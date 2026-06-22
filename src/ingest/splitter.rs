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
                let s = current.trim().to_string();
                if !s.is_empty() {
                    sentences.push(s);
                }
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
            if !current.ends_with(' ') {
                current.push(' ');
            }
        } else {
            current.push(c);
        }
        i += 1;
    }

    let tail = current.trim().to_string();
    if !tail.is_empty() {
        sentences.push(tail);
    }

    sentences
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
