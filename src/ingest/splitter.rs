use std::collections::HashSet;

pub enum ChunkMode {
    Sentence,
    Paragraph,
}

pub struct Splitter {
    pub min_len: usize,
    pub max_len: usize,
    pub mode: ChunkMode,
}

impl Splitter {
    pub fn split(&self, text: &str) -> Vec<String> {
        let raw = match self.mode {
            ChunkMode::Sentence => split_sentences(text),
            ChunkMode::Paragraph => split_paragraphs(text),
        };

        let mut seen: HashSet<String> = HashSet::new();
        raw.into_iter()
            .flat_map(|c| self.fit_to_max(c))
            .map(|c| c.trim().to_string())
            .filter(|c| self.is_valid(c))
            .filter(|c| seen.insert(c.clone()))
            .collect()
    }

    fn is_valid(&self, claim: &str) -> bool {
        let t = claim.trim();
        t.len() >= self.min_len && t.split_whitespace().count() >= 4
    }

    fn fit_to_max(&self, text: String) -> Vec<String> {
        if text.len() <= self.max_len {
            return vec![text];
        }
        chunk_on_words(&text, self.max_len)
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let c = chars[i];

        if (c == '.' || c == '!' || c == '?') && i + 1 < n {
            // Skip "..."
            if c == '.' && chars.get(i + 1) == Some(&'.') {
                current.push(c);
                i += 1;
                continue;
            }

            let next = chars[i + 1];
            let after_space = chars.get(i + 2).copied();
            let is_sentence_end = (next == ' ' || next == '\n')
                && after_space
                    .map(|ch| ch.is_uppercase() || ch == '"' || ch == '\'' || ch == '(')
                    .unwrap_or(true);

            if is_sentence_end {
                current.push(c);
                push_sentence(&mut sentences, &current);
                current = String::new();
                i += 2;
                continue;
            }
        }

        if c == '\r' {
            i += 1;
            continue;
        }
        if c == '\n' {
            // A newline immediately followed by a bullet/list marker starts a new
            // list item even without terminal punctuation on the previous line —
            // without this, a whole bulleted list merges into one run-on claim
            // (confirmed on a real PDF: bullets have no period per item, so the
            // hand-rolled scanner above never saw a sentence boundary between them).
            //
            // Some PDF extractors instead emit the bullet glyph trailing the
            // *previous* item ("...body weight. •\nBe physically active...") rather
            // than leading the next one — confirmed on the WHO fact sheet PDF used
            // in testing. Check both positions.
            if starts_bullet_item(&chars, i + 1) || ends_with_bullet_marker(&current) {
                push_sentence(&mut sentences, &current);
                current = String::new();
                i += 1;
                continue;
            }
            if !current.ends_with(' ') {
                current.push(' ');
            }
        } else {
            current.push(c);
        }
        i += 1;
    }

    push_sentence(&mut sentences, &current);

    sentences
}

/// Trims a sentence and strips a single leading or trailing bullet/list marker
/// (e.g. "• ", "- ", "3. ", or a trailing " •") so claim text reads as a clean
/// phrase, then pushes it if non-empty.
fn push_sentence(sentences: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let leading_stripped = if starts_bullet_item(&chars, 0) {
        let marker_end = bullet_marker_end(&chars, 0);
        chars[marker_end..].iter().collect::<String>().trim().to_string()
    } else {
        trimmed.to_string()
    };
    let cleaned = strip_trailing_bullet(&leading_stripped);
    if !cleaned.is_empty() {
        sentences.push(cleaned);
    }
}

/// True if the text accumulated so far ends (ignoring trailing whitespace) with
/// a bare bullet glyph — the marker for the *next* item, misplaced by a PDF
/// extractor that emits it before the line break instead of after.
fn ends_with_bullet_marker(s: &str) -> bool {
    matches!(s.trim_end().chars().last(), Some('•') | Some('‣') | Some('◦') | Some('·'))
}

fn strip_trailing_bullet(s: &str) -> String {
    let trimmed = s.trim_end();
    if let Some(last) = trimmed.chars().last() {
        if matches!(last, '•' | '‣' | '◦' | '·') {
            return trimmed[..trimmed.len() - last.len_utf8()].trim_end().to_string();
        }
    }
    trimmed.to_string()
}

/// True if, after skipping leading spaces/tabs from `idx`, the text begins a
/// bullet or numbered list item: "•"/"‣"/"◦"/"·", "- ", "* ", or "<digits>." / "<digits>)".
fn starts_bullet_item(chars: &[char], idx: usize) -> bool {
    let n = chars.len();
    let mut i = idx;
    while i < n && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    if i >= n {
        return false;
    }
    match chars[i] {
        '•' | '‣' | '◦' | '·' => true,
        '-' | '*' => chars.get(i + 1) == Some(&' '),
        c if c.is_ascii_digit() => {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            matches!(chars.get(j), Some('.') | Some(')'))
        }
        _ => false,
    }
}

/// Index just past a bullet marker starting at `idx` (assumes `starts_bullet_item`
/// already returned true for this position), including one trailing space.
fn bullet_marker_end(chars: &[char], idx: usize) -> usize {
    let n = chars.len();
    let mut i = idx;
    while i < n && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    match chars.get(i) {
        Some('•') | Some('‣') | Some('◦') | Some('·') | Some('-') | Some('*') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while i < n && chars[i].is_ascii_digit() {
                i += 1;
            }
            i += 1; // skip '.' or ')'
        }
        _ => {}
    }
    if i < n && chars[i] == ' ' {
        i += 1;
    }
    i
}

fn split_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|p| p.split('\n').collect::<Vec<_>>().join(" "))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn chunk_on_words(text: &str, max_len: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut chunks = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let mut buf = String::new();
        while i < words.len() {
            let w = words[i];
            let needed = if buf.is_empty() {
                w.len()
            } else {
                buf.len() + 1 + w.len()
            };
            if needed > max_len && !buf.is_empty() {
                break;
            }
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(w);
            i += 1;
        }
        if !buf.is_empty() {
            chunks.push(buf);
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_bullet_list_without_terminal_punctuation() {
        let text = "Diabetes facts:\n\
                     • Globally, an estimated 346 million people have diabetes\n\
                     • Three out of four people with diabetes live in low- and middle-income countries\n\
                     • Nearly 3.4 million people globally die from consequences of high blood sugar every year";
        let out = split_sentences(text);
        assert_eq!(out.len(), 4);
        assert_eq!(out[1], "Globally, an estimated 346 million people have diabetes");
        assert_eq!(
            out[2],
            "Three out of four people with diabetes live in low- and middle-income countries"
        );
        assert_eq!(
            out[3],
            "Nearly 3.4 million people globally die from consequences of high blood sugar every year"
        );
    }

    #[test]
    fn does_not_split_on_hyphenated_word_at_line_start() {
        let text = "This is a long sentence that wraps onto\n-continuation text that should stay joined.";
        let out = split_sentences(text);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("wraps onto -continuation"));
    }

    #[test]
    fn splits_bullet_list_with_trailing_marker() {
        // Real pattern observed from pdf-extract on the WHO diabetes fact sheet:
        // the bullet glyph trails the previous item instead of leading the next.
        let text = "Achieve and maintain a healthy body weight. \u{2022}\n\
                     Be physically active at least 30 minutes on most days. \u{2022}\n\
                     Quit tobacco use.";
        let out = split_sentences(text);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], "Achieve and maintain a healthy body weight.");
        assert_eq!(out[1], "Be physically active at least 30 minutes on most days.");
        assert_eq!(out[2], "Quit tobacco use.");
    }

    #[test]
    fn regular_sentences_unaffected() {
        let text = "Type 1 diabetes is due to deficient insulin production. Type 2 diabetes results from ineffective use of insulin.";
        let out = split_sentences(text);
        assert_eq!(out.len(), 2);
    }
}
