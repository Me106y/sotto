//! 应用配置：落盘在 `~/Library/Application Support/Sotto/config.json`。
//!
//! 这里只放「用户会改的东西」。模型 key 将来也走这里，但**不写进仓库**。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// 笔记根目录
    pub root: String,
    /// 参与索引的子目录（相对 root）
    pub includes: Vec<String>,
    /// 一律跳过的目录名
    pub excludes: Vec<String>,
    /// 单文件大小上限
    pub max_file_kb: u64,
    /// 每次检索取几条
    pub top_k: usize,
    /// 选中语言（模拟笔试）
    pub lang: String,

    // ---- 模型（豆包 / 火山方舟）----
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: f64,

    // ---- 语音 ----
    pub speak_enabled: bool,
    pub voice: String,
    /// `say` 的语速，中文大致是字/分钟；200 接近正常语速
    pub speech_rate: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: home()
                .join("Desktop/Note")
                .to_string_lossy()
                .to_string(),
            includes: vec![
                "项目经历".into(),
                "面试准备".into(),
                "学习".into(),
            ],
            excludes: vec![
                ".qoder".into(),
                ".obsidian".into(),
                ".git".into(),
                ".trash".into(),
                "node_modules".into(),
                "target".into(),
                ".venv".into(),
            ],
            max_file_kb: 512,
            top_k: 3,
            lang: "python".into(),
            api_key: String::new(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            model: "doubao-seed-2-0-mini-260428".into(),
            temperature: 0.4,
            speak_enabled: true,
            voice: "Tingting".into(),
            speech_rate: 210,
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        home()
            .join("Library")
            .join("Application Support")
            .join("Sotto")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.json")
    }

    /// 读不到就用默认值——配置文件坏了不该让 App 起不来。
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("序列化失败：{e}"))?;
        std::fs::write(Self::path(), json).map_err(|e| format!("写配置失败：{e}"))
    }

    /// 环境变量仍可覆盖，方便开发时临时切库。
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(root) = std::env::var("SOTTO_ROOT") {
            self.root = root;
        }
        if let Ok(inc) = std::env::var("SOTTO_INCLUDE") {
            self.includes = inc
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        self
    }
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}
