use std::fs;
use std::path::{Path, PathBuf};

/// 索引作用域。PRD §F4.2：必须手动圈选，不能自动猜。
#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub root: PathBuf,
    /// 相对 root 的目录名。空 = 扫 root 下全部
    pub includes: Vec<String>,
    pub max_file_bytes: u64,
    pub excluded_dirs: Vec<String>,
}

impl ScanConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            includes: Vec::new(),
            // 512 KB：再大就不是笔记了，是数据倾倒
            max_file_bytes: 512 * 1024,
            excluded_dirs: vec![
                ".qoder".into(),
                ".obsidian".into(),
                ".git".into(),
                ".trash".into(),
                "node_modules".into(),
                "target".into(),
                ".venv".into(),
            ],
        }
    }

    pub fn include(mut self, dir: impl Into<String>) -> Self {
        self.includes.push(dir.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub rel: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    ExcludedDir,
    TooLarge,
    Missing,
}

impl SkipReason {
    pub fn label(&self) -> &'static str {
        match self {
            SkipReason::ExcludedDir => "生成目录",
            SkipReason::TooLarge => "超过 512KB",
            SkipReason::Missing => "目录不存在",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Skipped {
    pub rel: String,
    pub reason: SkipReason,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<Skipped>,
    pub non_markdown: usize,
}

impl ScanResult {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

pub fn scan(cfg: &ScanConfig) -> std::io::Result<ScanResult> {
    let mut out = ScanResult::default();

    let starts: Vec<PathBuf> = if cfg.includes.is_empty() {
        vec![cfg.root.clone()]
    } else {
        cfg.includes.iter().map(|i| cfg.root.join(i)).collect()
    };

    let mut stack: Vec<PathBuf> = Vec::new();
    for s in starts {
        if !s.exists() {
            out.skipped.push(Skipped {
                rel: rel_path(&cfg.root, &s),
                reason: SkipReason::Missing,
            });
            continue;
        }
        stack.push(s);
    }

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };

            if ft.is_dir() {
                if cfg
                    .excluded_dirs
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(&name))
                {
                    out.skipped.push(Skipped {
                        rel: rel_path(&cfg.root, &path),
                        reason: SkipReason::ExcludedDir,
                    });
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() {
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    out.non_markdown += 1;
                    continue;
                }
                let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if bytes > cfg.max_file_bytes {
                    out.skipped.push(Skipped {
                        rel: rel_path(&cfg.root, &path),
                        reason: SkipReason::TooLarge,
                    });
                    continue;
                }
                out.files.push(ScannedFile {
                    rel: rel_path(&cfg.root, &path),
                    path,
                    bytes,
                });
            }
        }
    }

    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.skipped.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn rel_path(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sotto-scan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn generated_dirs_are_excluded_and_reported() {
        let root = tmpdir("gen");
        fs::create_dir_all(root.join(".qoder/wiki")).unwrap();
        fs::write(root.join(".qoder/wiki/a.md"), "x").unwrap();
        fs::write(root.join("keep.md"), "# hi\nbody").unwrap();

        let r = scan(&ScanConfig::new(&root)).unwrap();
        assert_eq!(r.files.len(), 1, "files={:?}", r.files);
        assert_eq!(r.files[0].rel, "keep.md");
        assert!(r
            .skipped
            .iter()
            .any(|s| s.reason == SkipReason::ExcludedDir && s.rel.contains(".qoder")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_files_are_skipped() {
        let root = tmpdir("big");
        let big = "x".repeat(1024);
        fs::write(root.join("big.md"), &big).unwrap();
        let mut cfg = ScanConfig::new(&root);
        cfg.max_file_bytes = 100;
        let r = scan(&cfg).unwrap();
        assert!(r.files.is_empty());
        assert_eq!(r.skipped[0].reason, SkipReason::TooLarge);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn includes_narrow_the_scope() {
        let root = tmpdir("inc");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/1.md"), "a").unwrap();
        fs::write(root.join("b/2.md"), "b").unwrap();

        let cfg = ScanConfig::new(&root).include("a");
        let r = scan(&cfg).unwrap();
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].rel, "a/1.md");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_include_is_reported_not_fatal() {
        let root = tmpdir("miss");
        let cfg = ScanConfig::new(&root).include("nope");
        let r = scan(&cfg).unwrap();
        assert!(r.files.is_empty());
        assert_eq!(r.skipped[0].reason, SkipReason::Missing);
        let _ = fs::remove_dir_all(&root);
    }
}
