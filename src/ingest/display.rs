use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::io::Write;
use std::time::Duration;

pub struct ProgressDisplay {
    pub pb: ProgressBar,
    pub stats: ProgressBar,
}

#[derive(Default)]
pub struct IngestStats {
    pub inserted: u64,
    pub duplicates: u64,
    pub rejected: u64,
    pub conflicts: u64,
    pub errors: u64,
    pub total: u64,
    pub last_claim: String,
}

pub fn print_header(
    source: &str,
    domain: &str,
    domain_src: &str,
    engine: &str,
    pfo_count: Option<usize>,
    input: &str,
) {
    let conn = match pfo_count {
        Some(n) => format!("✓ connected  ({} PFOs indexed)", n),
        None => "dry-run — connectivity not checked".to_string(),
    };
    println!();
    println!("  TekmerDB Ingestor");
    println!("  {}", "─".repeat(52));
    println!("  Source : {}", source);
    println!("  Domain : {}  ({})", domain, domain_src);
    println!("  Engine : {}  {}", engine, conn);
    println!("  Input  : {}", input);
    println!();
}

pub fn phase_spinner(label: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

pub fn finish_spinner(pb: ProgressBar, detail: &str, elapsed_ms: u128) {
    pb.set_style(ProgressStyle::with_template("  ✓ {msg}").unwrap());
    pb.finish_with_message(format!(
        "{}  ({:.1}s)",
        detail,
        elapsed_ms as f64 / 1000.0
    ));
}

pub fn create_progress(multi: &MultiProgress, total: u64) -> ProgressDisplay {
    let pb = multi.add(ProgressBar::new(total));
    pb.set_style(
        ProgressStyle::with_template(
            "  [{wide_bar:.cyan/blue}] {pos}/{len}  {percent:.1}%  {per_sec:.2}/s  ETA {eta}",
        )
        .unwrap()
        .progress_chars("█░"),
    );
    pb.enable_steady_tick(Duration::from_millis(120));

    let stats = multi.add(ProgressBar::new(0));
    stats.set_style(ProgressStyle::with_template("  {msg}").unwrap());
    stats.set_message(format_stats_line(&IngestStats::default()));
    stats.enable_steady_tick(Duration::from_millis(120));

    ProgressDisplay { pb, stats }
}

pub fn update_progress(d: &ProgressDisplay, s: &IngestStats) {
    d.pb.set_position(s.total);
    d.stats.set_message(format_stats_line(s));
}

fn format_stats_line(s: &IngestStats) -> String {
    let last = if s.last_claim.is_empty() {
        String::new()
    } else {
        let preview: String = s.last_claim.chars().take(72).collect();
        format!("\n  » \"{}\"", preview)
    };
    format!(
        "Inserted: {}  |  Duplicates: {}  |  Rejected: {}  |  Conflicts: {}  |  Errors: {}{}",
        s.inserted, s.duplicates, s.rejected, s.conflicts, s.errors, last
    )
}

const BOX_W: usize = 52;

pub fn print_summary(source: &str, input: &str, s: &IngestStats, elapsed: Duration) {
    let pct = |n: u64| {
        if s.total == 0 {
            0.0f64
        } else {
            n as f64 / s.total as f64 * 100.0
        }
    };

    let elapsed_s = elapsed.as_secs_f64();
    let rate = if elapsed_s > 0.0 && s.total > 0 {
        format!("{:.1} claims/s", s.total as f64 / elapsed_s)
    } else {
        "—".to_string()
    };

    let short_input = if input.len() > BOX_W - 2 {
        format!("…{}", &input[input.len() - (BOX_W - 3)..])
    } else {
        input.to_string()
    };

    let line = |label: &str, value: &str| {
        let content = format!("  {}  {}", label, value);
        let pad = BOX_W.saturating_sub(content.chars().count() + 2);
        println!("  ║{}{}║", content, " ".repeat(pad));
    };

    let sep = "═".repeat(BOX_W);
    println!();
    println!("  ╔{}╗", sep);
    println!("  ║{}Ingestion Complete{}║", " ".repeat(2), " ".repeat(BOX_W - 20));
    println!("  ╠{}╣", sep);
    line("Source   :", source);
    line("Input    :", &short_input);
    line("Duration :", &format!("{:.1}s  ({})", elapsed_s, rate));
    println!("  ╠{}╣", sep);
    line("Claims   :", &format!("{} found", s.total));
    line(
        "Inserted :",
        &format!("{}   ({:.1}%)", s.inserted, pct(s.inserted)),
    );
    line(
        "Duplicate:",
        &format!("{}   ({:.1}%)", s.duplicates, pct(s.duplicates)),
    );
    line(
        "Rejected :",
        &format!("{}   ({:.1}%)", s.rejected, pct(s.rejected)),
    );
    line(
        "Conflicts:",
        &format!("{}   ({:.1}%)", s.conflicts, pct(s.conflicts)),
    );
    line(
        "Errors   :",
        &format!("{}   ({:.1}%)", s.errors, pct(s.errors)),
    );
    println!("  ╚{}╝", sep);
    println!();

    // ensure buffered output is flushed
    let _ = std::io::stdout().flush();
}
