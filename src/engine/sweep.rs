use crate::storage::hot::HotTier;
use crate::engine::nli::NliClassifier;
use crate::engine::source_registry::SourceRegistry;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug)]
enum ClaimRelationship {
    Contradiction,
    Corroboration,
    Subsumption,
    Uncertain,
    Unrelated,
}

fn classify_relationship(
    nli: &mut NliClassifier,
    claim_a: &str,
    claim_b: &str,
    similarity: f32,
) -> anyhow::Result<ClaimRelationship> {
    let (prob_c_ab, prob_e_ab, prob_n_ab) = nli.classify_probs(claim_a, claim_b)?;
    let (prob_c_ba, prob_e_ba, prob_n_ba) = nli.classify_probs(claim_b, claim_a)?;

    println!(
        "[nli] A→B  contradiction: {:.3}  entailment: {:.3}  neutral: {:.3}",
        prob_c_ab, prob_e_ab, prob_n_ab
    );
    println!(
        "[nli] B→A  contradiction: {:.3}  entailment: {:.3}  neutral: {:.3}",
        prob_c_ba, prob_e_ba, prob_n_ba
    );

    if prob_c_ab > 0.5 || prob_c_ba > 0.5 {
        return Ok(ClaimRelationship::Contradiction);
    }

    if prob_e_ab > 0.5 && prob_e_ba > 0.5 {
        return Ok(ClaimRelationship::Corroboration);
    }

    // synonym corroboration — very high cosine + moderate entailment both ways
    if similarity > 0.95 && prob_e_ab > 0.3 && prob_e_ba > 0.3 {
        println!("[sweep] synonym corroboration — very high cosine + moderate entailment");
        return Ok(ClaimRelationship::Corroboration);
    }

    // subsumption with cosine tiebreaker at very high similarity
    if (prob_e_ab > 0.5 && prob_n_ba > 0.5) || (prob_e_ba > 0.5 && prob_n_ab > 0.5) {
        if similarity > 0.95 {
            println!("[sweep] cosine tiebreaker — NLI asymmetry overridden by very high similarity");
            return Ok(ClaimRelationship::Corroboration);
        }
        return Ok(ClaimRelationship::Subsumption);
    }

    if prob_c_ab > 0.3 || prob_c_ba > 0.3 {
        return Ok(ClaimRelationship::Uncertain);
    }

    Ok(ClaimRelationship::Unrelated)
}

pub fn evaluate_new_pfo(
    tier: &mut HotTier,
    nli: &mut NliClassifier,
    source_registry: &Arc<Mutex<SourceRegistry>>,
    new_id: Uuid,
    top_k: usize,
    similarity_floor: f32,
) {
    let new_pfo = match tier.store.get(&new_id) {
        Some(p) => p.clone(),
        None => return,
    };

    if new_pfo.semantic_fingerprint.is_empty() {
        return;
    }

    let candidates = tier.search(&new_pfo.semantic_fingerprint, top_k + 1);

    // filter self and same source_id
    let scores: Vec<(Uuid, f32)> = candidates.into_iter()
        .filter(|(id, _)| *id != new_id)
        .filter(|(id, _)| {
            tier.store.get(id)
                .map(|c| c.source_id != new_pfo.source_id)
                .unwrap_or(false)
        })
        .collect();

    // compact similarity log
    let score_summary: Vec<String> = scores.iter().enumerate()
        .map(|(i, (_, s))| format!("{}/{}: {:.3}", i + 1, scores.len(), s))
        .collect();
    println!("[sweep] seq:{} similarities — {}",
        new_pfo.seq_id,
        if score_summary.is_empty() { "none".to_string() } else { score_summary.join(" | ") });

    for (candidate_id, similarity) in &scores {
        if *similarity < similarity_floor { continue; }

        let candidate = match tier.store.get(candidate_id) {
            Some(p) => p.clone(),
            None => continue,
        };

        let conf_new  = new_pfo.confidence;
        let conf_cand = candidate.confidence;

        println!("[sweep] NLI check — similarity: {:.3} — '{}' vs '{}'",
            similarity, new_pfo.claim_text, candidate.claim_text);

        match classify_relationship(nli, &new_pfo.claim_text, &candidate.claim_text, *similarity) {
            Ok(ClaimRelationship::Contradiction) => {
                println!("[sweep] CONFLICT — clear contradiction");
                if let Some(pfo) = tier.store.get_mut(&new_id) {
                    pfo.confidence = conf_new * 0.75;
                    pfo.dirty = true;
                    if !pfo.conflict_refs.contains(candidate_id) {
                        pfo.conflict_refs.push(*candidate_id);
                    }
                }
                if let Some(pfo) = tier.store.get_mut(candidate_id) {
                    pfo.confidence = conf_cand * 0.75;
                    pfo.dirty = true;
                    if !pfo.conflict_refs.contains(&new_id) {
                        pfo.conflict_refs.push(new_id);
                    }
                }
                // record conflict on the attacking (new) PFO's source
                {
                    let mut reg = source_registry.lock().unwrap();
                    reg.record_conflict(&new_pfo.source_id);
                }
            }
            Ok(ClaimRelationship::Corroboration) => {
                println!("[sweep] CORROBORATION — same fact confirmed");
                // use live effective_weight of new PFO's source
                let weight = {
                    let reg = source_registry.lock().unwrap();
                    reg.effective_weight(&new_pfo.source_id)
                };
                let new_conf = 1.0 - (1.0 - conf_new) * (1.0 - weight);
                println!("[sweep] corroboration weight: {:.3} → new confidence: {:.3}",
                    weight, new_conf);
                if let Some(pfo) = tier.store.get_mut(&new_id) {
                    pfo.confidence = new_conf;
                    pfo.corroboration_count += 1;
                    pfo.last_corroborated = Some(chrono::Utc::now());
                    pfo.dirty = true;
                }
                if let Some(pfo) = tier.store.get_mut(candidate_id) {
                    pfo.confidence = new_conf;
                    pfo.corroboration_count += 1;
                    pfo.last_corroborated = Some(chrono::Utc::now());
                    pfo.dirty = true;
                }
                // record corroboration on new PFO's source
                {
                    let mut reg = source_registry.lock().unwrap();
                    reg.record_corroboration(&new_pfo.source_id);
                }
            }
            Ok(ClaimRelationship::Subsumption) => {
                println!("[sweep] SUBSUMPTION — different scope, storing independently");
            }
            Ok(ClaimRelationship::Uncertain) => {
                println!("[sweep] UNCERTAIN — small confidence adjustment");
                if let Some(pfo) = tier.store.get_mut(&new_id) {
                    pfo.confidence = conf_new * 0.95;
                    pfo.dirty = true;
                }
                if let Some(pfo) = tier.store.get_mut(candidate_id) {
                    pfo.confidence = conf_cand * 0.95;
                    pfo.dirty = true;
                }
            }
            Ok(ClaimRelationship::Unrelated) => {
                println!("[sweep] unrelated — ignoring");
            }
            Err(e) => {
                println!("[sweep] NLI error: {}", e);
            }
        }
    }
}