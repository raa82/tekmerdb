use crate::{log_info, log_warn, log_error};
use std::fs;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use crate::engine::pfo::Pfo;
use crate::engine::config::EngineConfig;
use crate::engine::sweep::evaluate_new_pfo;
use crate::engine::fingerprint::Fingerprinter;
use crate::engine::nli::NliClassifier;
use crate::storage::hot::{HotTier, InsertResult};
use crate::storage::crb::Crb;
use crate::storage::cold::ColdTier;
use crate::storage::source_cold::SourceColdTier;
use crate::engine::source_registry::SourceRegistry;

pub const HNSW_INDEX_PATH: &str  = "data/hnsw.index";
pub const COLD_PATH: &str        = "data/cold.parquet";
pub const SOURCE_CRB_PATH: &str  = "data/source_crb.bin";
pub const SOURCE_COLD_PATH: &str = "data/sources.parquet";

#[derive(Debug)]
pub enum EngineInsertResult {
    Inserted(uuid::Uuid),
    Duplicate(uuid::Uuid),
    DomainRejected(String),
}

pub struct Engine {
    hot: Arc<Mutex<HotTier>>,
    crb: Crb,
    fingerprinter: Fingerprinter,
    nli: NliClassifier,
    epoch: u32,
    seq: Arc<AtomicU32>,
    config: EngineConfig,
    domain_centroid: Arc<Mutex<Vec<f32>>>,
    centroid_count: Arc<AtomicUsize>,
    source_registry: Arc<Mutex<SourceRegistry>>,
}

fn current_epoch() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

pub fn make_seq_id(epoch: u32, seq: u32) -> u64 {
    ((epoch as u64) << 32) | (seq as u64)
}

pub fn decompose_seq_id(seq_id: u64) -> (u32, u32) {
    let epoch = (seq_id >> 32) as u32;
    let seq   = (seq_id & 0xFFFFFFFF) as u32;
    (epoch, seq)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() { return 0.0; }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 { return 0.0; }
    dot / (mag_a * mag_b)
}

fn update_centroid(centroid: &mut Vec<f32>, count: usize, new_fp: &[f32]) {
    if centroid.is_empty() {
        *centroid = new_fp.to_vec();
        return;
    }
    for (c, &n) in centroid.iter_mut().zip(new_fp.iter()) {
        *c = (*c * count as f32 + n) / (count + 1) as f32;
    }
}

impl Engine {
    // Convenience constructor using built-in defaults.
    // Used by external callers embedding TekmerDB without a config file.
    // main.rs uses new_with_config directly after loading tekmerdb-server.conf.
    #[allow(dead_code)]
    pub fn new(crb_path: &str) -> anyhow::Result<Self> {
        Self::new_with_config(crb_path, EngineConfig::default())
    }

    pub fn new_with_config(crb_path: &str, config: EngineConfig) -> anyhow::Result<Self> {
        fs::create_dir_all("data")?;

        let mut hot = HotTier::new();
        let cold = ColdTier::new(COLD_PATH)?;

        // 1. load Parquet checkpoint
        let (parquet_pfos, parquet_max_seq) = cold.read()?;
        let parquet_count = parquet_pfos.len();
        for pfo in parquet_pfos {
            hot.store.insert(pfo.id, pfo);
        }
        log_info!("[engine] loaded {} PFO(s) from Parquet — max seq: {}",
            parquet_count, parquet_max_seq);

        // 2. load HNSW index
        if let Err(e) = hot.load_index(HNSW_INDEX_PATH) {
            log_warn!("[engine] HNSW index load failed: {} — starting fresh", e);
        }

        // 3. replay CRB
        let crb = Crb::new(crb_path)?;
        let (crb_pfos, crb_max_seq) = crb.replay()?;
        let mut crb_replayed = 0;
        for pfo in crb_pfos {
            if pfo.seq_id > parquet_max_seq {
                let key = pfo.id.as_u128() as u64;
                if !pfo.semantic_fingerprint.is_empty() {
                    hot.index.reserve(hot.store.len() + 1).unwrap();
                    hot.index.add(key, &pfo.semantic_fingerprint).unwrap();
                    hot.id_map.insert(key, pfo.id);
                }
                hot.store.insert(pfo.id, pfo);
                crb_replayed += 1;
            }
        }
        log_info!("[engine] replayed {} new PFO(s) from CRB", crb_replayed);

        // 4. rebuild domain centroid
        let mut centroid: Vec<f32> = vec![];
        let mut centroid_count = 0usize;
        for pfo in hot.store.values() {
            if !pfo.semantic_fingerprint.is_empty() {
                update_centroid(&mut centroid, centroid_count, &pfo.semantic_fingerprint);
                centroid_count += 1;
            }
        }
        if centroid_count > 0 {
            log_info!("[engine] domain centroid rebuilt from {} existing PFO(s)", centroid_count);
        } else {
            log_info!("[engine] cold start — domain centroid will build from first {} inserts",
                config.cold_start_inserts);
        }

        // 5. restore seq counter
        let max_seq = crb_max_seq.max(parquet_max_seq);
        let now_epoch = current_epoch();
        let (stored_epoch, stored_seq) = decompose_seq_id(max_seq);
        let initial_seq = if stored_epoch == now_epoch { stored_seq } else { 0 };
        log_info!("[engine] seq restored — epoch: {} seq: {}", now_epoch, initial_seq);

        let seq = Arc::new(AtomicU32::new(initial_seq));
        let hot = Arc::new(Mutex::new(hot));
        let domain_centroid = Arc::new(Mutex::new(centroid));
        let centroid_count = Arc::new(AtomicUsize::new(centroid_count));

        // PFO background flush thread
        let hot_for_flush = Arc::clone(&hot);
        let flush_interval = config.flush_interval_secs;
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(flush_interval));
                let dirty_pfos = hot_for_flush.lock().unwrap().get_dirty();
                if dirty_pfos.is_empty() {
                    continue;
                }
                let all_pfos = hot_for_flush.lock().unwrap().get_all();
                if let Err(e) = cold.flush(&all_pfos) {
                    log_error!("[cold tier] flush error: {}", e);
                    continue;
                }
                {
                    let mut tier = hot_for_flush.lock().unwrap();
                    for pfo in &dirty_pfos {
                        tier.mark_clean(&pfo.id);
                    }
                }
                {
                    let tier = hot_for_flush.lock().unwrap();
                    if let Err(e) = tier.save_index(HNSW_INDEX_PATH) {
                        log_error!("[hot tier] HNSW save error: {}", e);
                    }
                }
                log_info!("[cold tier] flushed {} dirty PFO(s)", dirty_pfos.len());
            }
        });

        // 6. load source registry — Parquet then CRB replay
        let source_cold = SourceColdTier::new(SOURCE_COLD_PATH)?;
        let (parquet_sources, source_parquet_max_seq) = source_cold.read()?;
        let mut source_registry = SourceRegistry::new(SOURCE_CRB_PATH)?;
        let (crb_sources, source_crb_max_seq) = {
            let temp_crb = crate::storage::source_crb::SourceCrb::new(SOURCE_CRB_PATH)?;
            temp_crb.replay()?
        };
        let mut all_source_records = parquet_sources;
        for record in crb_sources {
            if record.seq_id > source_parquet_max_seq {
                all_source_records.push(record);
            }
        }
        source_registry.restore(all_source_records, source_parquet_max_seq, source_crb_max_seq);

        // source background flush thread
        let source_flush_interval = config.flush_interval_secs;
        let source_registry = Arc::new(Mutex::new(source_registry));
        let source_registry_for_flush = Arc::clone(&source_registry);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(source_flush_interval));
                let reg = source_registry_for_flush.lock().unwrap();
                let dirty = reg.get_dirty_records();
                if dirty.is_empty() {
                    continue;
                }
                let all = reg.get_all_records();
                drop(reg);
                if let Err(e) = source_cold.flush(&all) {
                    log_error!("[source_cold] flush error: {}", e);
                    continue;
                }
                source_registry_for_flush.lock().unwrap().mark_clean();
                log_info!("[source_cold] flushed {} dirty source(s)", dirty.len());
            }
        });

        Ok(Engine {
            hot,
            crb,
            fingerprinter: Fingerprinter::new("models/miniLM.onnx", "models/tokenizer.json")?,
            nli: NliClassifier::new("models/nli.onnx", "models/nli_tokenizer.json")?,
            epoch: now_epoch,
            seq,
            config,
            domain_centroid,
            centroid_count,
            source_registry,
        })
    }

    // resolve source name → UUID, auto-registering if unknown
    // called by insert_pfo handler before building the PFO
    pub fn resolve_source(&mut self, name: String) -> uuid::Uuid {
        let mut reg = self.source_registry.lock().unwrap();
        let source = reg.register(name, None);
        source.source_id
    }

    pub fn insert(&mut self, mut pfo: Pfo) -> anyhow::Result<EngineInsertResult> {
        pfo.semantic_fingerprint = self.fingerprinter.embed(&pfo.claim_text)?;

        let count = self.centroid_count.load(Ordering::SeqCst);

        if count >= self.config.cold_start_inserts {
            let centroid = self.domain_centroid.lock().unwrap();
            let similarity = cosine_similarity(&pfo.semantic_fingerprint, &centroid);
            log_info!("[engine] domain similarity: {:.3} (threshold: {:.2})",
                similarity, self.config.domain_threshold);
            if similarity < self.config.domain_threshold {
                log_warn!("[engine] DOMAIN REJECTED — similarity {:.3} below threshold {:.2}",
                    similarity, self.config.domain_threshold);
                return Ok(EngineInsertResult::DomainRejected(
                    format!("claim similarity {:.3} below domain threshold {:.2}",
                        similarity, self.config.domain_threshold)
                ));
            }
        } else {
            log_info!("[engine] cold start ({}/{}) — domain check skipped",
                count + 1, self.config.cold_start_inserts);
        }

        pfo.domain = self.config.domain.clone();

        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        pfo.seq_id = make_seq_id(self.epoch, seq);

        let mut tier = self.hot.lock().unwrap();
        let result = tier.insert(pfo.clone());

        match &result {
            InsertResult::Inserted(id) => {
                let id = *id;
                {
                    let mut centroid = self.domain_centroid.lock().unwrap();
                    update_centroid(&mut centroid, count, &pfo.semantic_fingerprint);
                    self.centroid_count.fetch_add(1, Ordering::SeqCst);
                }
                if let Err(e) = tier.save_index(HNSW_INDEX_PATH) {
                    log_error!("[hot tier] HNSW save error: {}", e);
                }
                drop(tier);
                self.crb.write(&pfo)?;
                let mut tier = self.hot.lock().unwrap();
                evaluate_new_pfo(&mut tier, &mut self.nli, &self.source_registry, id,
                    self.config.hnsw_top_k, self.config.similarity_floor);
                Ok(EngineInsertResult::Inserted(id))
            }
            InsertResult::Duplicate(existing_id) => {
                log_info!("[engine] duplicate detected — existing id: {}", existing_id);
                Ok(EngineInsertResult::Duplicate(*existing_id))
            }
        }
    }

    pub fn get(&self, id: &uuid::Uuid) -> Option<Pfo> {
        self.hot.lock().unwrap().get(id).cloned()
    }

    pub fn update_confidence(&mut self, id: &uuid::Uuid, new_confidence: f32) -> Option<f32> {
        let mut tier = self.hot.lock().unwrap();
        match tier.store.get_mut(id) {
            Some(pfo) => {
                let previous = pfo.confidence;
                pfo.confidence = new_confidence;
                pfo.dirty = true;
                log_info!("[engine] manual confidence update — id: {} {:.3} → {:.3}",
                    id, previous, new_confidence);
                Some(previous)
            }
            None => None,
        }
    }

    pub fn register_source(
        &mut self,
        name: String,
        domain: Option<String>,
    ) -> (uuid::Uuid, f32, u32, u32) {
        let mut reg = self.source_registry.lock().unwrap();
        let source = reg.register(name, domain);
        (
            source.source_id,
            source.effective_weight,
            source.corroboration_count,
            source.conflict_trigger_count,
        )
    }

    pub fn get_source_by_name(&self, name: &str) -> Option<crate::engine::pfo::Source> {
        self.source_registry.lock().unwrap()
            .get_by_name(name)
            .cloned()
    }

    pub fn get_source(&self, id: &uuid::Uuid) -> Option<(String, crate::engine::pfo::Source)> {
        let reg = self.source_registry.lock().unwrap();
        let name = reg.name_index.iter()
            .find(|(_, v)| *v == id)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "unknown".to_string());
        reg.get_by_id(id).map(|s| (name, s.clone()))
    }

    pub fn all_sources(&self) -> Vec<(String, crate::engine::pfo::Source)> {
        self.source_registry.lock().unwrap()
            .get_all_records()
            .into_iter()
            .map(|r| (r.name, r.source))
            .collect()
    }

    pub fn pfo_count(&self) -> usize {
        self.hot.lock().unwrap().store.len()
    }

    pub fn domain_name(&self) -> String {
        format!("{:?}", self.config.domain)
    }

    pub fn search(&mut self, query: &str, top_k: usize) -> anyhow::Result<Vec<Pfo>> {
        let fingerprint = self.fingerprinter.embed(query)?;
        let tier = self.hot.lock().unwrap();
        let results = tier.search(&fingerprint, top_k);
        let pfos = results.iter()
            .filter_map(|(id, _)| tier.store.get(id).cloned())
            .collect();
        Ok(pfos)
    }

    #[allow(dead_code)]
    pub fn replay(&self) -> anyhow::Result<(Vec<Pfo>, u64)> {
        Ok(self.crb.replay()?)
    }
}