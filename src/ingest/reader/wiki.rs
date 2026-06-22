use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct WikiSummary {
    extract: Option<String>,
}

#[derive(Deserialize)]
struct ActionResponse {
    query: Option<ActionQuery>,
}

#[derive(Deserialize)]
struct ActionQuery {
    pages: Option<HashMap<String, WikiPage>>,
}

#[derive(Deserialize)]
struct WikiPage {
    extract: Option<String>,
}

pub async fn read(title: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("tekmerdb-ingest/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let slug = title.replace(' ', "_");

    // Try REST summary first (fast, plain text)
    let summary_url = format!(
        "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
        slug
    );
    if let Ok(resp) = client.get(&summary_url).send().await {
        if resp.status().is_success() {
            if let Ok(w) = resp.json::<WikiSummary>().await {
                if let Some(text) = w.extract {
                    if text.len() > 100 {
                        return Ok(text);
                    }
                }
            }
        }
    }

    // Fall back to Action API for full article plain text
    let action_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts&explaintext=1&redirects=1&titles={}&format=json",
        slug
    );

    let resp = client.get(&action_url).send().await?;
    let data: ActionResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Wikipedia API response parse error: {}", e))?;

    if let Some(query) = data.query {
        if let Some(pages) = query.pages {
            for page in pages.values() {
                if let Some(text) = &page.extract {
                    if !text.is_empty() {
                        return Ok(text.clone());
                    }
                }
            }
        }
    }

    anyhow::bail!("no content found for Wikipedia article '{}'", title)
}
