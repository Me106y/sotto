//! 本地知识库索引：扫描 → 切分 → 混合检索。
//!
//! 全部跑在本地，不联网。查询延迟目标是 100ms 以内（PRD §6 的预算）。

pub mod bm25;
pub mod chunk;
pub mod retrieve;
pub mod scan;

pub use chunk::{chunk_file, Chunk, ChunkKind, MAX_CHUNK_CHARS};
pub use retrieve::{Hit, HybridRetriever};
pub use scan::{scan, ScanConfig, ScanResult, ScannedFile, SkipReason, Skipped};

use std::fs;

/// 扫描 + 读文件 + 切分，一次做完。
pub fn build_corpus(cfg: &ScanConfig) -> std::io::Result<(Vec<Chunk>, ScanResult)> {
    let result = scan(cfg)?;
    let mut chunks = Vec::new();
    for f in &result.files {
        let content = match fs::read_to_string(&f.path) {
            Ok(c) => c,
            Err(_) => continue, // 编码坏了就跳过，别让整个建库失败
        };
        chunks.extend(chunk_file(&f.rel, &content, f.bytes));
    }
    Ok((chunks, result))
}
