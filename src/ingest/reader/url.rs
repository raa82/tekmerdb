use anyhow::Result;
use scraper::{Html, Selector};

pub async fn read(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("tekmerdb-ingest/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let html = client.get(url).send().await?.text().await?;
    Ok(extract_body_text(&html))
}

fn extract_body_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let sel = Selector::parse(
        "p, h1, h2, h3, h4, h5, h6, li, blockquote, article"
    )
    .unwrap();

    document
        .select(&sel)
        .map(|el| el.text().collect::<Vec<_>>().join(" ").trim().to_string())
        .filter(|s| !s.is_empty() && s.split_whitespace().count() > 3)
        .collect::<Vec<_>>()
        .join("\n")
}
