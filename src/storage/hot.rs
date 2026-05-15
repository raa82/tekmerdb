use std::collections::HashMap;
use uuid::Uuid;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};
use crate::engine::pfo::Pfo;

const DUPLICATE_THRESHOLD: f32 = 0.99;

#[derive(Debug)]
pub enum InsertResult {
    Inserted(Uuid),
    Duplicate(Uuid),
}

pub struct HotTier {
    pub store: HashMap<Uuid, Pfo>,
    pub index: Index,
    pub id_map: HashMap<u64, Uuid>,
}

impl HotTier {
    pub fn new() -> Self {
        let options = IndexOptions {
            dimensions: 384,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };
        let index = Index::new(&options).expect("failed to create HNSW index");
        HotTier {
            store: HashMap::new(),
            index,
            id_map: HashMap::new(),
        }
    }

    pub fn save_index(&self, path: &str) -> anyhow::Result<()> {
        self.index.save(path)?;
        Ok(())
    }

    pub fn load_index(&mut self, path: &str) -> anyhow::Result<()> {
        if !std::path::Path::new(path).exists() {
            return Ok(());
        }
        self.index.load(path)?;

        // rebuild id_map from store
        for uuid in self.store.keys() {
            let key = uuid.as_u128() as u64;
            self.id_map.insert(key, *uuid);
        }

        println!("[hot tier] loaded HNSW index — {} vectors", self.index.size());
        Ok(())
    }

    pub fn insert(&mut self, mut pfo: Pfo) -> InsertResult {
        // duplicate check via HNSW distance
        if !pfo.semantic_fingerprint.is_empty() && self.index.size() > 0 {
            let results = self.index.search(&pfo.semantic_fingerprint, 1).unwrap();
            if let (Some(&key), Some(&distance)) = (
                results.keys.first(),
                results.distances.first(),
            ) {
                let similarity = 1.0 - distance;
                if similarity > DUPLICATE_THRESHOLD {
                    if let Some(&existing_id) = self.id_map.get(&key) {
                        return InsertResult::Duplicate(existing_id);
                    }
                }
            }
        }

        let key = pfo.id.as_u128() as u64;
        if !pfo.semantic_fingerprint.is_empty() {
            self.index.reserve(self.store.len() + 1).unwrap();
            self.index.add(key, &pfo.semantic_fingerprint).unwrap();
            self.id_map.insert(key, pfo.id);
        }

        pfo.dirty = true; // mark as needing flush
        self.store.insert(pfo.id, pfo.clone());
        InsertResult::Inserted(pfo.id)
    }

    pub fn mark_clean(&mut self, id: &Uuid) {
        if let Some(pfo) = self.store.get_mut(id) {
            pfo.dirty = false;
        }
    }

    pub fn get_dirty(&self) -> Vec<Pfo> {
        self.store.values()
            .filter(|p| p.dirty)
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &Uuid) -> Option<&Pfo> {
        self.store.get(id)
    }

    pub fn get_all(&self) -> Vec<Pfo> {
        self.store.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn search(&self, fingerprint: &[f32], count: usize) -> Vec<(Uuid, f32)> {
        if self.index.size() == 0 {
            return vec![];
        }
        let results = self.index.search(fingerprint, count).unwrap();
        results.keys.iter()
            .zip(results.distances.iter())
            .filter_map(|(k, d)| {
                self.id_map.get(k).map(|uuid| (*uuid, 1.0 - d)) // distance → similarity
            })
            .collect()
    }
}