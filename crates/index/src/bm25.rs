//! BM25。中文不做分词，用「字 + 字二元组」代替——
//! 这套土办法对技术名词（Redis、混合检索、冷启动）反而比词表分词更稳。

use crate::chunk::Chunk;
use std::collections::HashMap;

pub struct Index {
    df: HashMap<String, u32>,
    postings: HashMap<String, Vec<(usize, f32)>>,
    doc_len: Vec<f32>,
    avg_len: f32,
    n: usize,
}

impl Index {
    pub fn build(chunks: &[Chunk]) -> Self {
        let mut postings: HashMap<String, Vec<(usize, f32)>> = HashMap::new();
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut doc_len: Vec<f32> = Vec::with_capacity(chunks.len());

        for (i, c) in chunks.iter().enumerate() {
            let haystack = format!("{} {} {}", c.heading, c.source, c.text);
            let toks = tokenize(&haystack);
            doc_len.push(toks.len() as f32);

            let mut tf: HashMap<&str, u32> = HashMap::new();
            for t in &toks {
                *tf.entry(t.as_str()).or_insert(0) += 1;
            }
            for (t, f) in tf {
                postings
                    .entry(t.to_string())
                    .or_default()
                    .push((i, f as f32));
                *df.entry(t.to_string()).or_insert(0) += 1;
            }
        }

        let n = chunks.len();
        let avg_len = if n == 0 {
            0.0
        } else {
            doc_len.iter().sum::<f32>() / n as f32
        };

        Self {
            df,
            postings,
            doc_len,
            avg_len,
            n,
        }
    }

    /// 返回 (chunk 下标, BM25 分数)，按分数降序
    pub fn search(&self, query: &str, k: usize) -> Vec<(usize, f32)> {
        const K1: f32 = 1.2;
        const B: f32 = 0.75;

        let mut scores: HashMap<usize, f32> = HashMap::new();
        for t in tokenize_query(query) {
            let Some(plist) = self.postings.get(&t) else {
                continue;
            };
            let df = *self.df.get(&t).unwrap_or(&0) as f32;
            let idf = (1.0 + (self.n as f32 - df + 0.5) / (df + 0.5)).ln();
            for (doc, tf) in plist {
                let dl = self.doc_len[*doc];
                let norm = tf * (K1 + 1.0)
                    / (tf + K1 * (1.0 - B + B * dl / self.avg_len.max(1.0)));
                *scores.entry(*doc).or_insert(0.0) += idf * norm;
            }
        }

        let mut v: Vec<(usize, f32)> = scores.into_iter().collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(k);
        v
    }
}

/// 英文/数字按整词，中文**只按二元组**。
///
/// 一开始我连单字也索引，结果是长文档靠「的、怎、么、处」这些高频单字
/// 攒分，把真正的答案挤下去。二元组（置信、信度、分流）判别力高得多，
/// 单字只在孤立出现时才保留。
pub fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut cjk_run: Vec<char> = Vec::new();

    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '+' || ch == '#' || ch == '-' {
            flush_cjk(&mut cjk_run, &mut out);
            if !word.is_empty() && !word.chars().last().unwrap().is_ascii_alphanumeric() {
                out.push(std::mem::take(&mut word));
            }
            word.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            cjk_run.push(ch);
        } else {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            flush_cjk(&mut cjk_run, &mut out);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    flush_cjk(&mut cjk_run, &mut out);
    out
}

fn flush_cjk(run: &mut Vec<char>, out: &mut Vec<String>) {
    match run.len() {
        0 => {}
        1 => out.push(run[0].to_string()),
        _ => {
            for w in run.windows(2) {
                out.push(w.iter().collect());
            }
        }
    }
    run.clear();
}

/// 面试提问里的「问句套话」。它们在小语料里 df 很低，idf 却很高，
/// 于是「…怎么处理？」这种句型会把标题里带「怎么处理」的无关笔记顶上来。
/// 只在**查询侧**过滤，文档侧保留原词——文档里出现「怎么处理」是有信息量的。
pub const QUERY_STOP_BIGRAMS: &[&str] = &[
    "怎么", "么处", "处理", "如何", "何做", "什么", "么样", "为什", "么是", "一下", "时候", "你们",
    "我们", "那个", "这个", "可以", "能否", "具体", "说说", "讲讲", "介绍", "问下", "想问",
];

pub fn tokenize_query(s: &str) -> Vec<String> {
    tokenize(s)
        .into_iter()
        .filter(|t| !QUERY_STOP_BIGRAMS.contains(&t.as_str()))
        .collect()
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkKind;

    fn chunk(id: usize, text: &str) -> Chunk {
        Chunk {
            source: format!("doc{id}.md"),
            heading: String::new(),
            kind: ChunkKind::Section,
            chars: text.chars().count(),
            text: text.to_string(),
        }
    }

    #[test]
    fn tokenizer_keeps_ascii_words_and_cjk_bigrams() {
        let t = tokenize("Redis 的冷却");
        assert!(t.contains(&"redis".to_string()));
        assert!(t.contains(&"冷却".to_string()));
        // 单字不再进索引：高频单字会把长文档的分数抬虚
        assert!(!t.contains(&"冷".to_string()));
        assert!(t.contains(&"的冷".to_string()));
    }

    #[test]
    fn query_drops_interview_boilerplate() {
        let q = tokenize_query("你们那个系统怎么处理？");
        assert!(!q.contains(&"怎么".to_string()));
        assert!(!q.contains(&"处理".to_string()));
        // 文档侧不过滤，保留原词
        let d = tokenize("怎么处理");
        assert!(d.contains(&"怎么".to_string()));
    }

    #[test]
    fn bm25_ranks_the_containing_doc_first() {
        let idx = Index::build(&[
            chunk(0, "冷启动阶段先用规则引擎兜住三周"),
            chunk(1, "前端用 React 重写了整个工作台"),
            chunk(2, "部署走的是 K8s，灰度发布"),
        ]);
        let hits = idx.search("冷启动怎么做的", 3);
        assert_eq!(hits[0].0, 0, "hits={hits:?}");
        assert!(hits[0].1 > 0.0);
    }

    #[test]
    fn no_overlap_yields_no_hits() {
        let idx = Index::build(&[chunk(0, "完全无关的一段话")]);
        let hits = idx.search("量子计算", 3);
        assert!(hits.is_empty() || hits[0].1 < 1.0);
    }
}
