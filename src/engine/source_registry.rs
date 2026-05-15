use crate::{log_info, log_warn, log_error};
use std::collections::HashMap;
use uuid::Uuid;
use crate::engine::pfo::Source;
use crate::storage::source_crb::{SourceCrb, SourceRecord};

pub struct SourceRegistry {
    pub sources: HashMap<Uuid, Source>,
    pub name_index: HashMap<String, Uuid>,
    pub dirty: HashMap<Uuid, bool>,
    crb: SourceCrb,
    seq: u64,
}

impl SourceRegistry {
    pub fn new(crb_path: &str) -> std::io::Result<Self> {
        Ok(SourceRegistry {
            sources: HashMap::new(),
            name_index: HashMap::new(),
            dirty: HashMap::new(),
            crb: SourceCrb::new(crb_path)?,
            seq: 0,
        })
    }

    pub fn restore(&mut self, records: Vec<SourceRecord>, parquet_max_seq: u64, crb_max_seq: u64) {
        for record in records {
            let normalised = Self::normalise(&record.name);
            self.name_index.insert(normalised, record.source.source_id);
            self.sources.insert(record.source.source_id, record.source);
        }
        self.seq = parquet_max_seq.max(crb_max_seq);
        log_info!("[source_registry] restored {} source(s) — seq: {}",
            self.sources.len(), self.seq);
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn normalise(name: &str) -> String {
        name.trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn register(&mut self, name: String, _domain: Option<String>) -> &Source {
        let normalised = Self::normalise(&name);

        if let Some(existing_id) = self.name_index.get(&normalised) {
            let source = self.sources.get(existing_id).unwrap();
            log_info!("[source_registry] existing source — name: '{}' id: {} effective_weight: {:.3}",
                normalised, existing_id, source.effective_weight);
            return self.sources.get(existing_id).unwrap();
        }

        let source_id = Uuid::new_v4();
        let source = Source {
            source_id,
            source_weight:          0.5,
            corroboration_count:    0,
            conflict_trigger_count: 0,
            effective_weight:       0.5,
        };

        let seq_id = self.next_seq();
        let record = SourceRecord {
            name: normalised.clone(),
            source: source.clone(),
            seq_id,
        };

        if let Err(e) = self.crb.write(&record) {
            log_error!("[source_registry] CRB write error on register: {}", e);
        }

        log_info!("[source_registry] new source registered — name: '{}' (normalised from: '{}') id: {} effective_weight: 0.500",
            normalised, name, source_id);

        self.sources.insert(source_id, source);
        self.name_index.insert(normalised, source_id);
        self.dirty.insert(source_id, true);
        self.sources.get(&source_id).unwrap()
    }

    pub fn get_by_id(&self, id: &Uuid) -> Option<&Source> {
        let source = self.sources.get(id);
        if let Some(s) = source {
            log_info!("[source_registry] source lookup — id: {} effective_weight: {:.3} corroborations: {} conflicts: {}",
                id, s.effective_weight, s.corroboration_count, s.conflict_trigger_count);
        } else {
            log_warn!("[source_registry] source not found — id: {}", id);
        }
        source
    }

    #[allow(dead_code)]
    pub fn get_by_name(&self, name: &str) -> Option<&Source> {
        let normalised = Self::normalise(name);
        self.name_index.get(&normalised)
            .and_then(|id| self.sources.get(id))
    }

    pub fn record_corroboration(&mut self, source_id: &Uuid) {
        if let Some(source) = self.sources.get_mut(source_id) {
            source.corroboration_count += 1;
            source.effective_weight = 1.0 - (1.0 - source.effective_weight) * 0.95;
            log_info!("[source_registry] corroboration recorded for {} — effective_weight: {:.3}",
                source_id, source.effective_weight);

            let name = self.name_index.iter()
                .find(|(_, v)| *v == source_id)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let seq_id = self.seq + 1;
            self.seq = seq_id;
            let record = SourceRecord { name, source: source.clone(), seq_id };
            if let Err(e) = self.crb.write(&record) {
                log_error!("[source_registry] CRB write error on corroboration: {}", e);
            }
            self.dirty.insert(*source_id, true);
        }
    }

    pub fn record_conflict(&mut self, source_id: &Uuid) {
        if let Some(source) = self.sources.get_mut(source_id) {
            source.conflict_trigger_count += 1;
            source.effective_weight = source.effective_weight * 0.90;
            log_info!("[source_registry] conflict recorded for {} — effective_weight: {:.3}",
                source_id, source.effective_weight);

            let name = self.name_index.iter()
                .find(|(_, v)| *v == source_id)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let seq_id = self.seq + 1;
            self.seq = seq_id;
            let record = SourceRecord { name, source: source.clone(), seq_id };
            if let Err(e) = self.crb.write(&record) {
                log_error!("[source_registry] CRB write error on conflict: {}", e);
            }
            self.dirty.insert(*source_id, true);
        }
    }

    pub fn effective_weight(&self, source_id: &Uuid) -> f32 {
        self.sources.get(source_id)
            .map(|s| s.effective_weight)
            .unwrap_or(0.5)
    }

    pub fn get_dirty_records(&self) -> Vec<SourceRecord> {
        self.dirty.keys()
            .filter_map(|id| {
                let source = self.sources.get(id)?;
                let name = self.name_index.iter()
                    .find(|(_, v)| *v == id)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                Some(SourceRecord { name, source: source.clone(), seq_id: self.seq })
            })
            .collect()
    }

    pub fn get_all_records(&self) -> Vec<SourceRecord> {
        self.sources.iter()
            .map(|(id, source)| {
                let name = self.name_index.iter()
                    .find(|(_, v)| *v == id)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                SourceRecord { name, source: source.clone(), seq_id: self.seq }
            })
            .collect()
    }

    pub fn mark_clean(&mut self) {
        self.dirty.clear();
    }
}