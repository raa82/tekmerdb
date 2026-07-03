use axum::{
    Router,
    routing::{get, post, patch},
    extract::{State, Path, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use chrono::Utc;
use crate::engine::engine::{Engine, EngineInsertResult};
use crate::engine::pfo::{Pfo, Domain};

pub type AppState = Arc<Mutex<Engine>>;

// ── existing types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct InsertRequest {
    pub claim_text: String,
    pub confidence: f32,
    pub source: String,       // agent declares source by name — engine resolves to UUID
    pub domain: Option<String>,
}

#[derive(Serialize)]
pub struct InsertResponse {
    pub status: String,
    pub id: Option<Uuid>,
    pub seq_id: Option<String>,
    pub confidence: Option<f32>,
    pub source_id: Option<Uuid>,  // resolved UUID returned for agent awareness
    pub message: Option<String>,
}

#[derive(Serialize)]
pub struct PfoResponse {
    pub id: Uuid,
    pub seq_id: String,
    pub claim_text: String,
    pub confidence: f32,
    pub source: String,           // original source string
    pub source_id: Uuid,          // resolved UUID
    pub corroboration_count: u32,
    pub conflict_refs: Vec<Uuid>,
    pub domain: String,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct ConfidenceUpdateRequest {
    pub confidence: f32,
    pub reason: String,
}

#[derive(Serialize)]
pub struct ConfidenceUpdateResponse {
    pub id: Uuid,
    pub confidence_before: f32,
    pub confidence_after: f32,
    pub reason: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub k: Option<usize>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub pfo_count: usize,
    pub domain: String,
}

// ── source registry types ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterSourceRequest {
    pub name: String,
    pub domain: Option<String>,
}

#[derive(Serialize)]
pub struct SourceResponse {
    pub source_id: Uuid,
    pub name: String,
    pub effective_weight: f32,
    pub corroboration_count: u32,
    pub conflict_trigger_count: u32,
}

#[derive(Deserialize)]
pub struct SourceNameParams {
    pub name: String,
}

// ── helpers ────────────────────────────────────────────────────────────────────

fn fmt_seq(seq_id: u64) -> String {
    let epoch = (seq_id >> 32) as u32;
    let seq   = (seq_id & 0xFFFFFFFF) as u32;
    format!("{}:{}", epoch, seq)
}

fn parse_domain(s: &str) -> Domain {
    match s {
        "Healthcare"             => Domain::Healthcare,
        "LawEnforcement"         => Domain::LawEnforcement,
        "CriticalInfrastructure" => Domain::CriticalInfrastructure,
        "Employment"             => Domain::Employment,
        "Education"              => Domain::Education,
        "Migration"              => Domain::Migration,
        "LegalInterpretation"    => Domain::LegalInterpretation,
        "Finance"                => Domain::Finance,
        _                        => Domain::General,
    }
}

fn pfo_to_response(p: &Pfo) -> PfoResponse {
    PfoResponse {
        id: p.id,
        seq_id: fmt_seq(p.seq_id),
        claim_text: p.claim_text.clone(),
        confidence: p.confidence,
        source: p.source.clone(),
        source_id: p.source_id,
        corroboration_count: p.corroboration_count,
        conflict_refs: p.conflict_refs.clone(),
        domain: format!("{:?}", p.domain),
        created_at: p.created_at.to_rfc3339(),
    }
}

// ── existing handlers (unchanged logic) ───────────────────────────────────────

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let engine = state.lock().unwrap();
    Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        pfo_count: engine.pfo_count(),
        domain: engine.domain_name(),
    })
}

async fn insert_pfo(
    State(state): State<AppState>,
    Json(req): Json<InsertRequest>,
) -> Result<Json<InsertResponse>, StatusCode> {
    if req.source.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let domain = req.domain.as_deref()
        .map(parse_domain)
        .unwrap_or(Domain::CriticalInfrastructure);

    // resolve source name → UUID, auto-registering if unknown
    let source_id = {
        let mut engine = state.lock().unwrap();
        engine.resolve_source(req.source.clone())
    };

    let pfo = Pfo {
        id: Uuid::new_v4(),
        seq_id: 0,
        claim_text: req.claim_text,
        semantic_fingerprint: vec![],
        confidence: req.confidence,
        source: req.source,
        source_id,
        corroboration_count: 0,
        conflict_refs: vec![],
        created_at: Utc::now(),
        last_corroborated: None,
        domain,
        dirty: true,
    };

    let mut engine = state.lock().unwrap();
    match engine.insert(pfo).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
        EngineInsertResult::Inserted(id) => {
            let p = engine.get(&id).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(InsertResponse {
                status: "inserted".to_string(),
                id: Some(id),
                seq_id: Some(fmt_seq(p.seq_id)),
                confidence: Some(p.confidence),
                source_id: Some(p.source_id),
                message: None,
            }))
        }
        EngineInsertResult::Duplicate(id) => {
            let p = engine.get(&id).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(InsertResponse {
                status: "duplicate".to_string(),
                id: Some(id),
                seq_id: Some(fmt_seq(p.seq_id)),
                confidence: Some(p.confidence),
                source_id: Some(p.source_id),
                message: None,
            }))
        }
        EngineInsertResult::DomainRejected(reason) => {
            Ok(Json(InsertResponse {
                status: "rejected".to_string(),
                id: None,
                seq_id: None,
                confidence: None,
                source_id: None,
                message: Some(reason),
            }))
        }
    }
}

async fn get_pfo(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PfoResponse>, StatusCode> {
    let engine = state.lock().unwrap();
    match engine.get(&id) {
        Some(p) => Ok(Json(pfo_to_response(&p))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn update_confidence(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ConfidenceUpdateRequest>,
) -> Result<Json<ConfidenceUpdateResponse>, StatusCode> {
    if req.confidence < 0.0 || req.confidence > 1.0 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if req.reason.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let mut engine = state.lock().unwrap();
    match engine.update_confidence(&id, req.confidence) {
        Some(confidence_before) => {
            Ok(Json(ConfidenceUpdateResponse {
                id,
                confidence_before,
                confidence_after: req.confidence,
                reason: req.reason,
                updated_at: Utc::now().to_rfc3339(),
            }))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn search_pfos(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<PfoResponse>>, StatusCode> {
    let top_k = params.k.unwrap_or(10);
    let mut engine = state.lock().unwrap();
    let results = engine.search(&params.q, top_k)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(results.iter().map(pfo_to_response).collect()))
}

// ── source registry handlers ───────────────────────────────────────────────────

async fn register_source(
    State(state): State<AppState>,
    Json(req): Json<RegisterSourceRequest>,
) -> Result<Json<SourceResponse>, StatusCode> {
    if req.name.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let mut engine = state.lock().unwrap();
    let (source_id, effective_weight, corroboration_count, conflict_trigger_count) =
        engine.register_source(req.name.clone(), req.domain);

    Ok(Json(SourceResponse {
        source_id,
        name: req.name,
        effective_weight,
        corroboration_count,
        conflict_trigger_count,
    }))
}

async fn get_source_by_name(
    State(state): State<AppState>,
    Query(params): Query<SourceNameParams>,
) -> Result<Json<SourceResponse>, StatusCode> {
    if params.name.trim().is_empty() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let engine = state.lock().unwrap();
    match engine.get_source_by_name(&params.name) {
        Some(source) => Ok(Json(SourceResponse {
            source_id: source.source_id,
            name: params.name,
            effective_weight: source.effective_weight,
            corroboration_count: source.corroboration_count,
            conflict_trigger_count: source.conflict_trigger_count,
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_source(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SourceResponse>, StatusCode> {
    let engine = state.lock().unwrap();
    match engine.get_source(&id) {
        Some((name, source)) => Ok(Json(SourceResponse {
            source_id: id,
            name,
            effective_weight: source.effective_weight,
            corroboration_count: source.corroboration_count,
            conflict_trigger_count: source.conflict_trigger_count,
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn list_sources(State(state): State<AppState>) -> Json<Vec<SourceResponse>> {
    let engine = state.lock().unwrap();
    let sources = engine.all_sources()
        .into_iter()
        .map(|(name, source)| SourceResponse {
            source_id: source.source_id,
            name,
            effective_weight: source.effective_weight,
            corroboration_count: source.corroboration_count,
            conflict_trigger_count: source.conflict_trigger_count,
        })
        .collect();
    Json(sources)
}

// ── router ─────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/status",              get(status))
        .route("/pfo",                 post(insert_pfo))
        .route("/pfo/:id",             get(get_pfo))
        .route("/pfo/:id/confidence",  patch(update_confidence))
        .route("/search",              get(search_pfos))
        .route("/source",              post(register_source).get(get_source_by_name))
        .route("/source/all",          get(list_sources))
        .route("/source/:id",          get(get_source))
        .with_state(state)
}