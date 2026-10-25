//! Mock 适配器：可配置延迟的假实现。
//!
//! 它的唯一职责是**把管线和打点跑通**，让 M0 在没有 API key 的情况下
//! 也能验证「结构对不对、指标算得准不准」。
//! 真实延迟必须用真适配器测——mock 的 sleep 只是占位。

use crate::traits::{Asr, AsrOutput, EngineError, Llm, PromptPayload, Retriever, Tts};
use crate::types::HitInfo;
use std::time::Duration;

pub struct MockAsr {
    pub delay: Duration,
}

impl MockAsr {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms),
        }
    }
}

impl Asr for MockAsr {
    fn finalize(&self, partial: &str) -> Result<AsrOutput, EngineError> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        Ok(AsrOutput {
            text: partial.trim().to_string(),
        })
    }
}

pub struct MockRetriever {
    pub hits: Vec<HitInfo>,
    pub delay: Duration,
}

impl MockRetriever {
    pub fn new(hits: Vec<HitInfo>, delay_ms: u64) -> Self {
        Self {
            hits,
            delay: Duration::from_millis(delay_ms),
        }
    }

    pub fn scripted(delay_ms: u64) -> Self {
        Self::new(
            vec![HitInfo {
                chunk_id: 7,
                source: "项目经历/SGS/SGS-项目经历.md".into(),
                heading: "SGS / 冷启动".into(),
                score: 0.82,
                excerpt: "冷启动阶段先用规则引擎兜了三周，等埋点样本过万再切模型。".into(),
            }],
            delay_ms,
        )
    }
}

impl Retriever for MockRetriever {
    fn search(&self, _query: &str, k: usize) -> Result<Vec<HitInfo>, EngineError> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        Ok(self.hits.iter().take(k).cloned().collect())
    }
}

pub const SCRIPTED_ANSWER: &str =
    "冷启动我们是先用规则兜住的。前三周没有样本，就用运营给的关键词表硬扛。埋点攒到一万条才切模型，切完准确率从六成提到八成四。这段时间最烦的是两边口径对不上。";

pub struct MockLlm {
    pub ttft: Duration,
    pub per_char: Duration,
    pub answer: String,
}

impl MockLlm {
    pub fn new(ttft_ms: u64, per_char_ms: u64) -> Self {
        Self {
            ttft: Duration::from_millis(ttft_ms),
            per_char: Duration::from_millis(per_char_ms),
            answer: SCRIPTED_ANSWER.to_string(),
        }
    }
}

impl Llm for MockLlm {
    fn stream(
        &self,
        _prompt: &PromptPayload,
        sink: &mut dyn FnMut(&str),
    ) -> Result<(), EngineError> {
        if !self.ttft.is_zero() {
            std::thread::sleep(self.ttft);
        }
        for ch in self.answer.chars() {
            let mut buf = [0u8; 4];
            let s: &str = ch.encode_utf8(&mut buf);
            sink(s);
            if !self.per_char.is_zero() {
                std::thread::sleep(self.per_char);
            }
        }
        Ok(())
    }
}

pub struct MockTts {
    pub ttfb: Duration,
}

impl MockTts {
    pub fn new(ttfb_ms: u64) -> Self {
        Self {
            ttfb: Duration::from_millis(ttfb_ms),
        }
    }
}

impl Tts for MockTts {
    fn speak_first_sentence(&self, _sentence: &str) -> Result<(), EngineError> {
        if !self.ttfb.is_zero() {
            std::thread::sleep(self.ttfb);
        }
        Ok(())
    }
}
