use crate::engine::pfo::Domain;

pub struct EngineConfig {
    // domain this engine instance accepts and stamps on every PFO
    pub domain: Domain,

    // minimum cosine similarity between a claim and the domain centroid
    // claims below this threshold are rejected as off-domain
    pub domain_threshold: f32,

    // number of inserts before domain enforcement begins
    // engine builds centroid from these initial inserts
    pub cold_start_inserts: usize,

    // number of HNSW candidates evaluated per sweep
    pub hnsw_top_k: usize,

    // seconds between Parquet flush cycles
    pub flush_interval_secs: u64,

    // minimum cosine similarity between two PFOs before NLI is invoked
    // at 0.90 claims are about the same specific subject
    // lower values cause false NLI conflicts between different topics
    pub similarity_floor: f32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            domain:               Domain::CriticalInfrastructure,
            domain_threshold:     0.70,
            cold_start_inserts:   10,
            hnsw_top_k:           20,
            flush_interval_secs:  10,
            similarity_floor:     0.75,
        }
    }
}