use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Inserted {
        pfo_id: String,
        claim: String,
        confidence: f32,
    },
    Duplicate {
        claim: String,
    },
    Rejected {
        claim: String,
        reason: Option<String>,
    },
    Error {
        claim: String,
        reason: String,
    },
}

impl PipelineEvent {
    pub fn claim(&self) -> &str {
        match self {
            Self::Inserted { claim, .. } => claim,
            Self::Duplicate { claim } => claim,
            Self::Rejected { claim, .. } => claim,
            Self::Error { claim, .. } => claim,
        }
    }
}

#[derive(Serialize)]
struct InsertRequest {
    claim_text: String,
    confidence: f32,
    source: String,
    domain: String,
}

#[derive(Deserialize)]
struct InsertResponse {
    status: String,
    id: Option<String>,
    confidence: Option<f32>,
    message: Option<String>,
}

pub struct PipelineConfig {
    pub engine_url: String,
    pub source: String,
    pub domain: String,
    pub confidence: f32,
    pub concurrency: usize,
    pub dry_run: bool,
}

pub async fn run(
    claims: Vec<String>,
    config: PipelineConfig,
    tx: mpsc::UnboundedSender<PipelineEvent>,
) -> Result<()> {
    let client = Arc::new(
        Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?,
    );
    let sem = Arc::new(Semaphore::new(config.concurrency));
    let config = Arc::new(config);
    let mut handles = Vec::with_capacity(claims.len());

    for claim in claims {
        let permit = sem.clone().acquire_owned().await?;
        let client = client.clone();
        let config = config.clone();
        let tx = tx.clone();

        handles.push(tokio::spawn(async move {
            let event = if config.dry_run {
                PipelineEvent::Inserted {
                    pfo_id: "dry-run".to_string(),
                    claim: claim.clone(),
                    confidence: config.confidence,
                }
            } else {
                insert_with_retry(&client, &config, &claim).await
            };
            let _ = tx.send(event);
            drop(permit);
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    Ok(())
}

async fn insert_with_retry(client: &Client, config: &PipelineConfig, claim: &str) -> PipelineEvent {
    let url = format!("{}/pfo", config.engine_url);
    let req = InsertRequest {
        claim_text: claim.to_string(),
        confidence: config.confidence,
        source: config.source.clone(),
        domain: config.domain.clone(),
    };

    for attempt in 0u32..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * (1u64 << attempt))).await;
        }

        match client.post(&url).json(&req).send().await {
            Err(e) => {
                if attempt == 2 {
                    return PipelineEvent::Error {
                        claim: claim.to_string(),
                        reason: format!("network error: {}", e),
                    };
                }
            }
            Ok(resp) => {
                if resp.status().is_client_error() {
                    return PipelineEvent::Error {
                        claim: claim.to_string(),
                        reason: format!("HTTP {}", resp.status().as_u16()),
                    };
                }
                match resp.json::<InsertResponse>().await {
                    Err(e) => {
                        if attempt == 2 {
                            return PipelineEvent::Error {
                                claim: claim.to_string(),
                                reason: format!("response parse error: {}", e),
                            };
                        }
                    }
                    Ok(r) => {
                        return match r.status.as_str() {
                            "inserted" => PipelineEvent::Inserted {
                                pfo_id: r.id.unwrap_or_default(),
                                claim: claim.to_string(),
                                confidence: r.confidence.unwrap_or(config.confidence),
                            },
                            "duplicate" => PipelineEvent::Duplicate {
                                claim: claim.to_string(),
                            },
                            "rejected" => PipelineEvent::Rejected {
                                claim: claim.to_string(),
                                reason: r.message,
                            },
                            other => PipelineEvent::Error {
                                claim: claim.to_string(),
                                reason: format!("unknown status '{}'", other),
                            },
                        };
                    }
                }
            }
        }
    }

    PipelineEvent::Error {
        claim: claim.to_string(),
        reason: "all retries exhausted".to_string(),
    }
}
