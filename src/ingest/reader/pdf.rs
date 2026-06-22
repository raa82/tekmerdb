use anyhow::Result;

pub fn read(path: &str) -> Result<String> {
    pdf_extract::extract_text(path)
        .map_err(|e| anyhow::anyhow!("PDF extraction failed: {}", e))
}
