//! 知识库：建索引 + 给管理页用的体检报告。

use serde::Serialize;
use std::time::Instant;

use sotto_index::{chunk_file, scan, Chunk, HybridRetriever, ScanConfig};

use crate::config::Config;

#[derive(Serialize, Clone)]
pub struct FileStat {
    pub rel: String,
    pub chunks: usize,
    pub chars: usize,
    pub bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct SkipStat {
    pub rel: String,
    pub reason: String,
}

#[derive(Serialize, Clone)]
pub struct KbReport {
    pub root: String,
    pub files: Vec<FileStat>,
    pub skipped: Vec<SkipStat>,
    pub chunks: usize,
    pub total_chars: usize,
    pub built_ms: f64,
}

pub struct Kb {
    pub retriever: HybridRetriever,
    pub report: KbReport,
}

impl Kb {
    pub fn build(cfg: &Config) -> Result<Kb, String> {
        let t = Instant::now();

        let mut sc = ScanConfig::new(&cfg.root);
        sc.max_file_bytes = cfg.max_file_kb.saturating_mul(1024);
        sc.excluded_dirs = cfg.excludes.clone();
        for inc in &cfg.includes {
            sc = sc.include(inc.clone());
        }

        let scanned = scan(&sc).map_err(|e| format!("扫描失败：{e}"))?;

        let mut chunks: Vec<Chunk> = Vec::new();
        let mut files: Vec<FileStat> = Vec::new();
        let mut failed: Vec<SkipStat> = Vec::new();

        for f in &scanned.files {
            let content = match std::fs::read_to_string(&f.path) {
                Ok(c) => c,
                Err(e) => {
                    // 编码坏了就记进失败清单，别让整个建库挂掉
                    failed.push(SkipStat {
                        rel: f.rel.clone(),
                        reason: format!("读不出来：{e}"),
                    });
                    continue;
                }
            };
            let own = chunk_file(&f.rel, &content, f.bytes);
            if own.is_empty() {
                failed.push(SkipStat {
                    rel: f.rel.clone(),
                    reason: "没有可索引的内容（空文件或全被过滤）".into(),
                });
                continue;
            }
            files.push(FileStat {
                rel: f.rel.clone(),
                chunks: own.len(),
                chars: own.iter().map(|c| c.chars).sum(),
                bytes: f.bytes,
            });
            chunks.extend(own);
        }

        let mut skipped: Vec<SkipStat> = scanned
            .skipped
            .iter()
            .map(|s| SkipStat {
                rel: s.rel.clone(),
                reason: s.reason.label().to_string(),
            })
            .collect();
        skipped.extend(failed);
        skipped.sort_by(|a, b| a.rel.cmp(&b.rel));

        let total_chars = chunks.iter().map(|c| c.chars).sum();
        let chunk_count = chunks.len();
        let retriever = HybridRetriever::new(chunks);

        Ok(Kb {
            retriever,
            report: KbReport {
                root: cfg.root.clone(),
                files,
                skipped,
                chunks: chunk_count,
                total_chars,
                built_ms: t.elapsed().as_secs_f64() * 1000.0,
            },
        })
    }
}
