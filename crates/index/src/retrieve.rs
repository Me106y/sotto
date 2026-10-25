//! 混合检索：BM25 归一化 + 字符二元组 Dice 相似度 + 标题命中加成。
//!
//! v1 不上向量库（PRD §F4.5）：200 来个 chunk 全在内存里，暴力算完就是微秒级。
//! 等 chunk 过 5000 再换 sqlite-vec。

use crate::bm25::{tokenize_query, Index};
use crate::chunk::Chunk;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Hit {
    pub chunk: Chunk,
    pub score: f32,
    pub bm25: f32,
    pub overlap: f32,
}

pub struct HybridRetriever {
    chunks: Vec<Chunk>,
    index: Index,
}

impl HybridRetriever {
    pub fn new(chunks: Vec<Chunk>) -> Self {
        let index = Index::build(&chunks);
        Self { chunks, index }
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn search(&self, query: &str, k: usize) -> Vec<Hit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let pool = self.index.search(query, (k * 4).max(20));
        if pool.is_empty() {
            return Vec::new();
        }
        let max_bm25 = pool.iter().map(|(_, s)| *s).fold(0.0f32, f32::max).max(1e-6);

        let q_grams = bigrams(query);
        let q_tokens = tokenize_query(query);

        let mut hits: Vec<Hit> = Vec::with_capacity(pool.len());
        for (doc, raw) in pool {
            let c = &self.chunks[doc];
            let doc_grams = bigrams(&format!("{} {}", c.heading, c.text));
            let overlap = dice(&q_grams, &doc_grams);

            let mut score = 0.7 * (raw / max_bm25) + 0.3 * overlap;

            // 标题里出现问题时，这条命中更可信（问「SGS 数据同步」，标题带 SGS 的优先）
            if heading_hits(&c.heading, &q_tokens) {
                score += 0.12;
            }

            hits.push(Hit {
                chunk: c.clone(),
                score,
                bm25: raw,
                overlap,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        hits
    }
}

fn heading_hits(heading: &str, tokens: &[String]) -> bool {
    if heading.is_empty() {
        return false;
    }
    let h = heading.to_lowercase();
    tokens
        .iter()
        .any(|t| t.chars().count() >= 2 && h.contains(&t.to_lowercase()))
}

fn bigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut set = HashSet::new();
    match chars.len() {
        0 => {}
        1 => {
            set.insert(chars[0].to_string());
        }
        _ => {
            for w in chars.windows(2) {
                set.insert(w.iter().collect::<String>());
            }
        }
    }
    set
}

fn dice(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    2.0 * inter / (a.len() + b.len()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkKind;

    fn chunk(source: &str, heading: &str, text: &str) -> Chunk {
        Chunk {
            source: source.to_string(),
            heading: heading.to_string(),
            kind: ChunkKind::Section,
            chars: text.chars().count(),
            text: text.to_string(),
        }
    }

    fn corpus() -> HybridRetriever {
        HybridRetriever::new(vec![
            chunk(
                "项目经历/SGS/SGS-项目经历.md",
                "SGS / 冷启动",
                "冷启动阶段先用规则引擎兜了三周，等埋点样本过万才切模型。",
            ),
            chunk(
                "项目经历/Carlsberg/Carlsberg-项目经历.md",
                "Carlsberg / 中间层",
                "统一中间层，把 Skill 和流水线解耦，三方系统靠 JIRA 需求串起来。",
            ),
            chunk(
                "学习/视频笔记/面试题｜RAG 治理.md",
                "RAG / 幻觉治理",
                "幻觉治理分四层：检索质量、置信度分流、答案校验、线上兜底。",
            ),
        ])
    }

    #[test]
    fn finds_the_right_project() {
        let r = corpus();
        let hits = r.search("你们 SGS 项目冷启动怎么解决的？", 3);
        assert!(!hits.is_empty());
        assert!(hits[0].chunk.source.contains("SGS"), "{:?}", hits[0]);
    }

    #[test]
    fn finds_knowledge_note_for_generic_question() {
        let r = corpus();
        let hits = r.search("RAG 的幻觉怎么治理", 3);
        assert!(hits[0].chunk.heading.contains("RAG"), "{:?}", hits[0]);
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(corpus().search("   ", 3).is_empty());
    }
}

// ---------------------------------------------------------------- 端口实现

/// 把 `HybridRetriever` 接成引擎认的 `sotto_core::Retriever`。
/// 引擎一行都不用改——这就是 trait 的意义。
impl sotto_core::Retriever for HybridRetriever {
    fn search(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<sotto_core::HitInfo>, sotto_core::EngineError> {
        Ok(self
            .search(query, k)
            .into_iter()
            .map(|h| sotto_core::HitInfo {
                chunk_id: stable_id(&h.chunk.source, &h.chunk.heading),
                source: h.chunk.source.clone(),
                heading: h.chunk.heading.clone(),
                score: h.score,
                // 注入上限 600 字：整篇项目经历块有 1200~1500 字，
                // 全塞进 prompt 会挤掉别的材料，也让模型开始跑题。
                excerpt: h.chunk.excerpt(600),
            })
            .collect())
    }
}

/// 稳定的 chunk 标识：不用给 Chunk 加 id 字段，也够 UI 追来源。
fn stable_id(source: &str, heading: &str) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source.bytes().chain(std::iter::once(0u8)).chain(heading.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h as usize
}
