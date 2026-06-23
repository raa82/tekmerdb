mod display;
mod pipeline;
mod reader;
mod splitter;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::MultiProgress;
use pipeline::PipelineEvent;
use splitter::{ChunkMode, Splitter};
use std::io::Write;
use std::time::Instant;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(
    name = "tekmerdb-ingest",
    about = "Feed PDF, DOCX, TXT, MD, URL, or Wikipedia into a TekmerDB engine"
)]
struct Args {
    /// File path, URL, or Wikipedia article title
    input: String,

    /// Source name, e.g. "IEA Report 2024"
    #[arg(long, short = 's')]
    source: String,

    /// Domain override — otherwise queried from the engine via /status
    #[arg(long, short = 'd')]
    domain: Option<String>,

    /// Base confidence 0.0–1.0
    #[arg(long, default_value = "0.7")]
    confidence: f32,

    /// Engine base URL
    #[arg(long, short = 'e', default_value = "http://localhost:3000")]
    engine: String,

    /// Force document type: pdf|docx|txt|md|url|wiki
    #[arg(long = "type")]
    doc_type: Option<String>,

    /// Minimum claim length in chars
    #[arg(long, default_value = "40")]
    min_len: usize,

    /// Maximum claim length in chars
    #[arg(long, default_value = "600")]
    max_len: usize,

    /// Chunking strategy: sentence|paragraph
    #[arg(long, default_value = "sentence")]
    chunk_mode: String,

    /// Concurrent HTTP inserts
    #[arg(long, default_value = "4")]
    concurrency: usize,

    /// Parse and split only — no inserts
    #[arg(long)]
    dry_run: bool,

    /// Write NDJSON log to this file
    #[arg(long)]
    log: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let start = Instant::now();
    let doc_type = reader::DocType::detect(&args.input, args.doc_type.as_deref());

    // ── connectivity check + domain discovery (skip in dry-run) ─────────────
    let (pfo_count, domain, domain_src) = if !args.dry_run {
        match check_engine(&args.engine).await {
            Ok((count, engine_domain)) => {
                let (d, src) = if let Some(ref d) = args.domain {
                    (d.clone(), "CLI flag".to_string())
                } else {
                    (engine_domain, format!("engine: {}/status", args.engine))
                };
                (Some(count), d, src)
            }
            Err(e) => {
                eprintln!(
                    "\nerror: cannot reach TekmerDB engine at {}\n  {}\n\nHint: start the engine, or pass --engine <URL> / --dry-run",
                    args.engine, e
                );
                std::process::exit(1);
            }
        }
    } else {
        let d = args.domain.clone().unwrap_or_else(|| "General".to_string());
        let src = if args.domain.is_some() { "CLI flag".to_string() } else { "default".to_string() };
        (None, d, src)
    };

    // ── header ───────────────────────────────────────────────────────────────
    display::print_header(
        &args.source,
        &domain,
        &domain_src,
        &args.engine,
        pfo_count,
        &args.input,
    );
    if args.dry_run {
        println!("  Mode   : DRY RUN — claims will be parsed but not inserted");
        println!();
    }

    // ── phase 1: read document ───────────────────────────────────────────────
    let multi = MultiProgress::new();
    let t0 = Instant::now();
    let spin1 = multi.add(display::phase_spinner(&format!(
        "[1/3] Reading {} document...",
        doc_type.label()
    )));
    let text = reader::read_document(doc_type, &args.input)
        .await
        .with_context(|| format!("failed to read '{}'", args.input))?;
    display::finish_spinner(
        spin1,
        &format!("[1/3] Read {} — {} bytes", doc_type.label(), text.len()),
        t0.elapsed().as_millis(),
    );

    // ── phase 2: split into claims ───────────────────────────────────────────
    let t0 = Instant::now();
    let spin2 = multi.add(display::phase_spinner("[2/3] Splitting into claims..."));
    let chunk_mode = match args.chunk_mode.to_ascii_lowercase().as_str() {
        "paragraph" | "para" => ChunkMode::Paragraph,
        _ => ChunkMode::Sentence,
    };
    let splitter = Splitter {
        min_len: args.min_len,
        max_len: args.max_len,
        mode: chunk_mode,
    };
    let claims = splitter.split(&text);
    let avg_chars = if claims.is_empty() {
        0
    } else {
        claims.iter().map(|c| c.len()).sum::<usize>() / claims.len()
    };
    display::finish_spinner(
        spin2,
        &format!("[2/3] Split — {} claims  avg {} chars", claims.len(), avg_chars),
        t0.elapsed().as_millis(),
    );

    if claims.is_empty() {
        eprintln!(
            "\nNo claims extracted from '{}'. Try --min-len or --chunk-mode paragraph.",
            args.input
        );
        return Ok(());
    }

    // ── phase 3: ingest ──────────────────────────────────────────────────────
    let ingest_label = format!(
        "[3/3] Ingesting {} claims{}",
        claims.len(),
        if args.dry_run { " (dry run)" } else { "" }
    );
    let spin3 = multi.add(display::phase_spinner(&format!("{}...", ingest_label)));
    let progress = display::create_progress(&multi, claims.len() as u64);

    let (tx, mut rx) = mpsc::unbounded_channel::<PipelineEvent>();
    let cfg = pipeline::PipelineConfig {
        engine_url: args.engine.clone(),
        source: args.source.clone(),
        domain: domain.clone(),
        confidence: args.confidence,
        concurrency: args.concurrency,
        dry_run: args.dry_run,
    };

    let pipeline_handle = tokio::spawn(pipeline::run(claims, cfg, tx));

    let mut log_writer: Option<std::io::BufWriter<std::fs::File>> = match &args.log {
        None => None,
        Some(p) => match std::fs::File::create(p) {
            Ok(f) => Some(std::io::BufWriter::new(f)),
            Err(e) => {
                eprintln!("warning: cannot create log file '{}': {}", p, e);
                None
            }
        },
    };

    let mut stats = display::IngestStats::default();
    display::finish_spinner(spin3, &ingest_label, 0);

    while let Some(event) = rx.recv().await {
        let preview: String = event.claim().chars().take(72).collect();
        match &event {
            PipelineEvent::Inserted { .. } => stats.inserted += 1,
            PipelineEvent::Duplicate { .. } => stats.duplicates += 1,
            PipelineEvent::Rejected { .. } => stats.rejected += 1,
            PipelineEvent::Error { .. } => stats.errors += 1,
        }
        stats.total += 1;
        stats.last_claim = preview;
        if let Some(ref mut w) = log_writer {
            let _ = writeln!(w, "{}", event_to_ndjson(&event));
        }
        display::update_progress(&progress, &stats);
    }

    pipeline_handle.await??;
    progress.pb.finish_and_clear();
    progress.stats.finish_and_clear();

    display::print_summary(&args.source, &args.input, &stats, start.elapsed());
    Ok(())
}

async fn check_engine(engine_url: &str) -> Result<(usize, String)> {
    #[derive(serde::Deserialize)]
    struct StatusResponse {
        #[allow(dead_code)]
        status: String,
        #[allow(dead_code)]
        version: String,
        pfo_count: usize,
        domain: String,
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let url = format!("{}/status", engine_url);
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("cannot connect to {}", url))?;

    let health: StatusResponse = resp
        .json()
        .await
        .context("engine /status returned an unexpected response")?;

    Ok((health.pfo_count, health.domain))
}

fn event_to_ndjson(event: &PipelineEvent) -> String {
    let ts = chrono::Utc::now().to_rfc3339();
    let obj = match event {
        PipelineEvent::Inserted { pfo_id, claim, confidence } => serde_json::json!({
            "ts": ts, "result": "inserted",
            "claim": claim, "pfo_id": pfo_id, "confidence": confidence
        }),
        PipelineEvent::Duplicate { claim } => serde_json::json!({
            "ts": ts, "result": "duplicate", "claim": claim
        }),
        PipelineEvent::Rejected { claim, reason } => serde_json::json!({
            "ts": ts, "result": "rejected", "claim": claim, "reason": reason
        }),
        PipelineEvent::Error { claim, reason } => serde_json::json!({
            "ts": ts, "result": "error", "claim": claim, "reason": reason
        }),
    };
    obj.to_string()
}
