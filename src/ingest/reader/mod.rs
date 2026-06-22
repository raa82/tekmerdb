pub mod docx;
pub mod md;
pub mod pdf;
pub mod txt;
pub mod url;
pub mod wiki;

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DocType {
    Pdf,
    Docx,
    Txt,
    Md,
    Url,
    Wiki,
}

impl DocType {
    pub fn detect(input: &str, force: Option<&str>) -> Self {
        if let Some(t) = force {
            return match t.to_ascii_lowercase().as_str() {
                "pdf" => Self::Pdf,
                "docx" | "doc" => Self::Docx,
                "txt" => Self::Txt,
                "md" | "markdown" => Self::Md,
                "url" | "http" | "https" => Self::Url,
                "wiki" | "wikipedia" => Self::Wiki,
                _ => Self::detect(input, None),
            };
        }
        if input.starts_with("http://") || input.starts_with("https://") {
            return Self::Url;
        }
        let lower = input.to_ascii_lowercase();
        if lower.ends_with(".pdf") {
            return Self::Pdf;
        }
        if lower.ends_with(".docx") || lower.ends_with(".doc") {
            return Self::Docx;
        }
        if lower.ends_with(".md") || lower.ends_with(".wiki") {
            return Self::Md;
        }
        if lower.ends_with(".txt") {
            return Self::Txt;
        }
        // bare word (no path separators, no dots) → wikipedia article title
        if !input.contains('/') && !input.contains('\\') && !input.contains('.') {
            return Self::Wiki;
        }
        Self::Txt
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "DOCX",
            Self::Txt => "TXT",
            Self::Md => "Markdown",
            Self::Url => "URL",
            Self::Wiki => "Wikipedia",
        }
    }
}

pub async fn read_document(doc_type: DocType, input: &str) -> Result<String> {
    match doc_type {
        DocType::Pdf => {
            let path = input.to_string();
            tokio::task::spawn_blocking(move || pdf::read(&path))
                .await
                .map_err(|e| anyhow::anyhow!("task join error: {}", e))?
        }
        DocType::Docx => {
            let path = input.to_string();
            tokio::task::spawn_blocking(move || docx::read(&path))
                .await
                .map_err(|e| anyhow::anyhow!("task join error: {}", e))?
        }
        DocType::Txt => {
            let path = input.to_string();
            tokio::task::spawn_blocking(move || txt::read(&path))
                .await
                .map_err(|e| anyhow::anyhow!("task join error: {}", e))?
        }
        DocType::Md => {
            let path = input.to_string();
            tokio::task::spawn_blocking(move || md::read(&path))
                .await
                .map_err(|e| anyhow::anyhow!("task join error: {}", e))?
        }
        DocType::Url => url::read(input).await,
        DocType::Wiki => wiki::read(input).await,
    }
}
