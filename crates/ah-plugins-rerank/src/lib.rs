//! # ah-plugins-rerank
//!
//! 真实检索重排器(对齐 retrieval reranker):
//! - 把词法候选与向量候选按 doc_id+chunk 合并;
//! - 每条候选分数 = lexical_weight × bm25_norm + vector_weight × cosine_norm
//!   (两条路径各自按原始 score 在候选内归一化到 [0,1]);
//! - 多样性惩罚:已选结果与候选共享词(按空白分词)时按系数扣分;
//! - 按融合分数降序输出 top-k。确定性可测。

use std::collections::BTreeMap;
use std::sync::Arc;

use ah_contracts::keys::RERANK;
use ah_contracts::prelude::Effect;
use ah_contracts::rerank::{RerankConfig, RerankError, RerankedHit, Reranker};
use ah_contracts::retrieval::RetrievalHit;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 合并候选:key = doc_id + "\x00" + chunk。
fn merge_candidates(
    lexical: &[RetrievalHit],
    vector: &[RetrievalHit],
) -> Vec<(String, String, f64, f64)> {
    let mut map: BTreeMap<String, (String, String, f64, f64)> = BTreeMap::new();
    // 词法路径:记录原始分数(用于归一化)。
    let lex_max = lexical
        .iter()
        .map(|h| h.score)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    for hit in lexical {
        let key = format!("{}\x00{}", hit.doc_id, hit.chunk);
        let norm = hit.score / lex_max;
        match map.get_mut(&key) {
            Some(entry) => entry.2 = norm,
            None => {
                map.insert(key, (hit.doc_id.clone(), hit.chunk.clone(), norm, 0.0));
            }
        }
    }
    let vec_max = vector
        .iter()
        .map(|h| h.score)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    for hit in vector {
        let key = format!("{}\x00{}", hit.doc_id, hit.chunk);
        let norm = hit.score / vec_max;
        match map.get_mut(&key) {
            Some(entry) => entry.3 = norm,
            None => {
                map.insert(key, (hit.doc_id.clone(), hit.chunk.clone(), 0.0, norm));
            }
        }
    }
    map.into_values().collect()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// 真实重排器。
pub struct HybridReranker;

impl Seam for HybridReranker {}

impl Reranker for HybridReranker {
    fn rerank(
        &self,
        lexical: &[RetrievalHit],
        vector: &[RetrievalHit],
        k: usize,
        config: &RerankConfig,
    ) -> Result<Vec<RerankedHit>, RerankError> {
        if lexical.is_empty() && vector.is_empty() {
            return Err(RerankError("no candidates".to_string()));
        }
        if config.lexical_weight < 0.0 || config.vector_weight < 0.0 {
            return Err(RerankError("weights must be non-negative".to_string()));
        }
        let mut candidates = merge_candidates(lexical, vector);
        // 多样性惩罚:贪心选前 k,与已选共享词扣分。
        let mut selected: Vec<RerankedHit> = Vec::new();
        let mut selected_words: Vec<std::collections::HashSet<String>> = Vec::new();
        while !candidates.is_empty() && selected.len() < k {
            let mut best_idx = 0usize;
            let mut best_score = f64::NEG_INFINITY;
            for (idx, (_doc_id, chunk, lex_norm, vec_norm)) in candidates.iter().enumerate() {
                let fused = config.lexical_weight * lex_norm + config.vector_weight * vec_norm;
                // 多样性:与已选结果的共享词数惩罚。
                let chunk_words: std::collections::HashSet<String> =
                    words(chunk).into_iter().collect();
                let overlap: usize = selected_words
                    .iter()
                    .map(|sel| chunk_words.intersection(sel).count())
                    .sum();
                let score = fused - config.diversity_penalty * overlap as f64;
                if score > best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }
            let (doc_id, chunk, lex_norm, vec_norm) = candidates.remove(best_idx);
            let fused = config.lexical_weight * lex_norm + config.vector_weight * vec_norm;
            selected.push(RerankedHit {
                doc_id: doc_id.clone(),
                chunk: chunk.clone(),
                score: fused.clamp(0.0, 1.0),
            });
            selected_words.push(words(&chunk).into_iter().collect());
        }
        Ok(selected)
    }
}

/// rerank 插件:提供 rerank seam。
pub struct RerankPlugin;

impl Plugin for RerankPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rerank"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RERANK]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let reranker: Arc<dyn Reranker> = Arc::new(HybridReranker);
        Ok(vec![ctx.register(RERANK, reranker)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RERANK;
    use ah_contracts::rerank::Reranker;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn hit(doc: &str, chunk: &str, score: f64) -> RetrievalHit {
        RetrievalHit {
            doc_id: doc.to_string(),
            chunk: chunk.to_string(),
            score,
        }
    }

    #[test]
    fn rerank_fuses_lexical_and_vector_scores() {
        let reranker = HybridReranker;
        // doc A:词法 1.0 / 向量 0.5;doc B:词法 0.5 / 向量 1.0 → 等权重融合同为 0.75。
        let lexical = vec![hit("a", "alpha beta", 1.0), hit("b", "beta gamma", 0.5)];
        let vector = vec![hit("a", "alpha beta", 0.5), hit("b", "beta gamma", 1.0)];
        let result = reranker
            .rerank(&lexical, &vector, 10, &RerankConfig::default())
            .expect("rerank");
        assert_eq!(result.len(), 2);
        let a = result.iter().find(|r| r.doc_id == "a").expect("a");
        let b = result.iter().find(|r| r.doc_id == "b").expect("b");
        assert!((a.score - 0.75).abs() < 1e-9, "fused 0.75: {}", a.score);
        assert!((b.score - 0.75).abs() < 1e-9, "fused 0.75: {}", b.score);
    }

    #[test]
    fn diversity_penalty_reorders_redundant_candidates() {
        let reranker = HybridReranker;
        // 三个候选都与 query 相关;前两个共享词多 → 第三个因多样性上升。
        let lexical = vec![
            hit("d1", "rust compiler borrow checker", 1.0),
            hit("d2", "rust compiler borrow rules", 0.9),
            hit("d3", "cargo build system", 0.6),
        ];
        let config = RerankConfig {
            lexical_weight: 1.0,
            vector_weight: 0.0,
            diversity_penalty: 0.3,
        };
        let result = reranker.rerank(&lexical, &[], 3, &config).expect("rerank");
        assert_eq!(result.len(), 3);
        // d1 最高分仍第一。
        assert_eq!(result[0].doc_id, "d1");
        // 多样性惩罚使 d3(与 d1 共享少)比 d2 更早(0.6 vs 0.9-0.3×overlap)。
        let d3_pos = result.iter().position(|r| r.doc_id == "d3").expect("d3");
        assert!(
            d3_pos <= 1,
            "diversity promotes distinct candidate: {result:?}"
        );
    }

    #[test]
    fn errors_on_empty_and_bad_weights() {
        let reranker = HybridReranker;
        assert!(
            reranker
                .rerank(&[], &[], 3, &RerankConfig::default())
                .is_err(),
            "no candidates"
        );
        let bad = RerankConfig {
            lexical_weight: -0.1,
            ..Default::default()
        };
        let lexical = vec![hit("a", "x", 1.0)];
        assert!(
            reranker.rerank(&lexical, &[], 3, &bad).is_err(),
            "negative weight"
        );
    }

    #[test]
    fn plugin_registers_reranker() {
        let ctx = Context::new();
        let plugin: DynPlugin = StdArc::new(RerankPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let reranker = ctx.service::<dyn Reranker>(&RERANK).expect("rerank");
        let lexical = vec![hit("a", "hello world", 1.0)];
        let result = reranker
            .rerank(&lexical, &[], 1, &RerankConfig::default())
            .expect("rerank");
        assert_eq!(result.len(), 1);
        drop(effects);
        assert!(!ctx.has_service(&RERANK));
    }
}
