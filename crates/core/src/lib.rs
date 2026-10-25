//! Sotto 核心引擎：平台无关，无网络依赖。
//!
//! 设计约束（见 PRD §8.2）：
//! - 这一层不许出现任何 `#[cfg(target_os)]`，也不许直接引用平台 API。
//! - 所有外部能力（ASR / LLM / TTS / 检索）都通过 trait 注入，
//!   真实适配器与 mock 各写一份，引擎本身不知道对面是谁。

pub mod budget;
pub mod mock;
pub mod pipeline;
pub mod prompt;
pub mod traits;
pub mod types;

pub use budget::{Budget, BudgetResult, StageModel};
pub use pipeline::{Engine, EngineConfig};
pub use traits::{Asr, AsrOutput, EngineError, Llm, PromptPayload, Retriever, Tts};
pub use types::{HitInfo, Span, Stage, TurnReport};
