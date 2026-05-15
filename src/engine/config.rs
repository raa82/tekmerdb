use crate::engine::pfo::Domain;
use std::fs;

pub struct EngineConfig {
    pub domain: Domain,
    pub domain_threshold: f32,
    pub cold_start_inserts: usize,
    pub hnsw_top_k: usize,
    pub flush_interval_secs: u64,
    pub similarity_floor: f32,
    pub engine_host: String,
    pub engine_port: u16,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            domain:               Domain::CriticalInfrastructure,
            domain_threshold:     0.70,
            cold_start_inserts:   10,
            hnsw_top_k:           20,
            flush_interval_secs:  10,
            similarity_floor:     0.90,
            engine_host:          "127.0.0.1".to_string(),
            engine_port:          3000,
        }
    }
}

impl EngineConfig {
    pub fn load(path: &str) -> Self {
        let mut cfg = EngineConfig::default();

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                println!("[config] {} not found — using defaults", path);
                return cfg;
            }
        };

        let table: toml::Table = match toml::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                println!("[config] parse error in {} — using defaults: {}", path, e);
                return cfg;
            }
        };

        // domain
        if let Some(v) = table.get("domain").and_then(|v| v.as_str()) {
            cfg.domain = parse_domain(v);
            println!("[config] domain = {}", v);
        }

        // domain_threshold
        if let Some(v) = table.get("domain_threshold").and_then(|v| v.as_float()) {
            cfg.domain_threshold = v as f32;
            println!("[config] domain_threshold = {}", v);
        }

        // cold_start_inserts
        if let Some(v) = table.get("cold_start_inserts").and_then(|v| v.as_integer()) {
            cfg.cold_start_inserts = v as usize;
            println!("[config] cold_start_inserts = {}", v);
        }

        // hnsw_top_k
        if let Some(v) = table.get("hnsw_top_k").and_then(|v| v.as_integer()) {
            cfg.hnsw_top_k = v as usize;
            println!("[config] hnsw_top_k = {}", v);
        }

        // flush_interval_secs
        if let Some(v) = table.get("flush_interval_secs").and_then(|v| v.as_integer()) {
            cfg.flush_interval_secs = v as u64;
            println!("[config] flush_interval_secs = {}", v);
        }

        // similarity_floor
        if let Some(v) = table.get("similarity_floor").and_then(|v| v.as_float()) {
            cfg.similarity_floor = v as f32;
            println!("[config] similarity_floor = {}", v);
        }

        // engine_host
        if let Some(v) = table.get("engine_host").and_then(|v| v.as_str()) {
            cfg.engine_host = v.to_string();
            println!("[config] engine_host = {}", v);
        }

        // engine_port
        if let Some(v) = table.get("engine_port").and_then(|v| v.as_integer()) {
            cfg.engine_port = v as u16;
            println!("[config] engine_port = {}", v);
        }

        println!("[config] loaded from {}", path);
        cfg
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.engine_host, self.engine_port)
    }
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
        _                        => {
            println!("[config] unknown domain '{}' — defaulting to General", s);
            Domain::General
        }
    }
}