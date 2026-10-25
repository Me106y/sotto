use crate::types::HitInfo;
use std::fmt;

#[derive(Debug, Clone)]
pub enum EngineError {
    /// 能力不可用（没配 key、断网、平台不支持）
    Unavailable(String),
    /// 上游返回了错误
    Upstream(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Unavailable(m) => write!(f, "不可用：{m}"),
            EngineError::Upstream(m) => write!(f, "上游错误：{m}"),
        }
    }
}

impl std::error::Error for EngineError {}

pub struct PromptPayload {
    pub system: String,
    pub user: String,
}

pub struct AsrOutput {
    pub text: String,
}

pub trait Asr: Send + Sync {
    /// 把增量识别结果收敛成一句完整的问题（含纠错）。
    fn finalize(&self, partial: &str) -> Result<AsrOutput, EngineError>;
}

pub trait Retriever: Send + Sync {
    fn search(&self, query: &str, k: usize) -> Result<Vec<HitInfo>, EngineError>;
}

pub trait Llm: Send + Sync {
    /// 流式生成：每产出一段就调用一次 `sink`。
    /// 首字延迟由调用方在 sink 里打点，实现方不需要自己上报。
    fn stream(&self, prompt: &PromptPayload, sink: &mut dyn FnMut(&str)) -> Result<(), EngineError>;
}

pub trait Tts: Send + Sync {
    /// 只合成一句（首句优先出声）。分段送合成，不要整段等。
    fn speak_first_sentence(&self, sentence: &str) -> Result<(), EngineError>;
}
