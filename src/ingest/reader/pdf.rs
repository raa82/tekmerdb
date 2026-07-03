use anyhow::Result;
use euclid::vec2;
use pdf_extract::{Document, MediaBox, OutputDev, OutputError, Transform};

pub fn read(path: &str) -> Result<String> {
    let mut doc =
        Document::load(path).map_err(|e| anyhow::anyhow!("PDF extraction failed: {}", e))?;
    if doc.is_encrypted() {
        let _ = doc.decrypt("");
    }

    let mut collector = LayoutCollector::new();
    pdf_extract::output_doc(&doc, &mut collector)
        .map_err(|e| anyhow::anyhow!("PDF extraction failed: {}", e))?;

    Ok(collector.into_text())
}

struct Glyph {
    x: f64,
    y: f64,
    width: f64,
    font_size: f64,
    is_word_start: bool,
    text: String,
}

/// Collects every glyph's position instead of streaming text directly, so pages
/// can be reordered into true reading order before stringifying. pdf-extract's
/// own `PlainTextOutput` emits characters in raw content-stream order with only
/// geometric line/word-break guessing — fine for single-column text, but on a
/// multi-column layout (confirmed on a real-world PDF used in testing) that
/// order does not match visual reading order at all, scrambling words across
/// columns. We fix that here by detecting a column gap and processing each
/// column's glyphs top-to-bottom before moving to the next.
struct LayoutCollector {
    pages: Vec<Vec<Glyph>>,
    at_word_start: bool,
}

impl LayoutCollector {
    fn new() -> Self {
        LayoutCollector { pages: Vec::new(), at_word_start: false }
    }

    fn into_text(self) -> String {
        self.pages
            .into_iter()
            .map(stringify_page)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl OutputDev for LayoutCollector {
    fn begin_page(
        &mut self,
        _page_num: u32,
        _media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        self.pages.push(Vec::new());
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        char: &str,
    ) -> Result<(), OutputError> {
        if let Some(page) = self.pages.last_mut() {
            // Font size alone isn't in the same units as trm.m31/m32 once the CTM
            // includes any scaling — transform it the same way pdf-extract's own
            // PlainTextOutput does, or word/line-gap thresholds below come out
            // wrong (confirmed: using raw font_size split real words in half).
            let scaled = trm.transform_vector(vec2(font_size, font_size));
            let transformed_font_size = (scaled.x * scaled.y).sqrt();
            page.push(Glyph {
                x: trm.m31,
                y: trm.m32,
                width: width * transformed_font_size,
                font_size: transformed_font_size,
                is_word_start: self.at_word_start,
                text: char.to_string(),
            });
        }
        self.at_word_start = false;
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        self.at_word_start = true;
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

fn stringify_page(glyphs: Vec<Glyph>) -> String {
    if glyphs.is_empty() {
        return String::new();
    }
    match detect_column_split(&glyphs) {
        Some(split_x) => {
            let (left, right): (Vec<Glyph>, Vec<Glyph>) =
                glyphs.into_iter().partition(|g| g.x < split_x);
            format!("{}\n\n{}", stringify_column(left), stringify_column(right))
        }
        None => stringify_column(glyphs),
    }
}

/// Looks for a single wide, mostly-empty vertical band roughly in the middle of
/// the page's text extent — a strong signal of a two-column layout's gutter.
/// Only ever proposes one split (two columns): good enough for the common case,
/// not a general N-column typesetting solution.
fn detect_column_split(glyphs: &[Glyph]) -> Option<f64> {
    const MIN_GLYPHS_TO_BOTHER: usize = 200;
    if glyphs.len() < MIN_GLYPHS_TO_BOTHER {
        return None;
    }

    let min_x = glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    let max_x = glyphs.iter().map(|g| g.x + g.width).fold(f64::NEG_INFINITY, f64::max);
    let span = max_x - min_x;
    if !span.is_finite() || span <= 0.0 {
        return None;
    }

    const BUCKETS: usize = 60;
    let bucket_width = span / BUCKETS as f64;
    let mut counts = vec![0u32; BUCKETS];
    for g in glyphs {
        let idx = (((g.x - min_x) / bucket_width) as usize).min(BUCKETS - 1);
        counts[idx] += 1;
    }

    // Ignore the outer margins — a gap there is just page margin, not a gutter.
    let margin = BUCKETS / 6;
    let mut best_run: Option<(usize, usize)> = None; // (start, len)
    let mut run_start: Option<usize> = None;
    for i in margin..(BUCKETS - margin) {
        if counts[i] == 0 {
            if run_start.is_none() {
                run_start = Some(i);
            }
        } else if let Some(start) = run_start.take() {
            let len = i - start;
            if best_run.map(|(_, best_len)| len > best_len).unwrap_or(true) {
                best_run = Some((start, len));
            }
        }
    }
    if let Some(start) = run_start {
        let len = BUCKETS - margin - start;
        if best_run.map(|(_, best_len)| len > best_len).unwrap_or(true) {
            best_run = Some((start, len));
        }
    }

    let (gap_start, gap_len) = best_run?;
    let avg_font_size = glyphs.iter().map(|g| g.font_size).sum::<f64>() / glyphs.len() as f64;
    if (gap_len as f64) * bucket_width < avg_font_size * 3.0 {
        return None;
    }

    let split_x = min_x + (gap_start as f64 + gap_len as f64 / 2.0) * bucket_width;

    // Require both sides to actually hold a meaningful share of the page's text —
    // otherwise this is probably a stray heading/pull-quote, not two columns.
    let left_count = glyphs.iter().filter(|g| g.x < split_x).count();
    let right_count = glyphs.len() - left_count;
    let min_share = glyphs.len() / 6;
    if left_count < min_share || right_count < min_share {
        return None;
    }

    Some(split_x)
}

/// Groups glyphs into visual lines (by y-proximity), orders lines top-to-bottom,
/// and words left-to-right within each line, then joins with newlines — this is
/// the actual reading-order reconstruction for one column (or the whole page,
/// when no column split was detected).
fn stringify_column(mut glyphs: Vec<Glyph>) -> String {
    if glyphs.is_empty() {
        return String::new();
    }
    glyphs.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    for g in glyphs {
        let starts_new_line = match lines.last() {
            Some(line) => {
                let ref_y = line[0].y;
                let tol = line[0].font_size.max(g.font_size) * 0.4;
                (ref_y - g.y).abs() > tol
            }
            None => true,
        };
        if starts_new_line {
            lines.push(vec![g]);
        } else {
            lines.last_mut().unwrap().push(g);
        }
    }

    let mut out = String::new();
    for (i, mut line) in lines.into_iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let mut last_end = f64::NEG_INFINITY;
        for g in &line {
            if g.is_word_start && last_end.is_finite() && g.x > last_end + g.font_size * 0.1 {
                out.push(' ');
            }
            out.push_str(&g.text);
            last_end = g.x + g.width;
        }
    }
    out
}
