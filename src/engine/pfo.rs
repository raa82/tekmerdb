use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Healthcare,
    LawEnforcement,
    CriticalInfrastructure,
    Employment,
    Education,
    Migration,
    LegalInterpretation,
    Finance,
    General,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub source_id: Uuid,
    pub source_weight: f32,
    pub corroboration_count: u32,
    pub conflict_trigger_count: u32,
    pub effective_weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfo {
    pub id: Uuid,
    pub seq_id: u64,
    pub claim_text: String,
    pub semantic_fingerprint: Vec<f32>,
    pub confidence: f32,

    // source as declared by the agent — original string, always present, audit trail
    pub source: String,

    // resolved UUID from source registry — always populated at insert time
    // engine auto-registers unknown sources with neutral weight 0.5
    pub source_id: Uuid,

    pub corroboration_count: u32,
    pub conflict_refs: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub last_corroborated: Option<DateTime<Utc>>,
    pub domain: Domain,

    // runtime only — not persisted to CRB or Parquet
    #[serde(skip, default = "default_dirty")]
    pub dirty: bool,
}

fn default_dirty() -> bool {
    true
}