use crate::prompt;
use crate::traits::{Asr, EngineError, Llm, PromptPayload, Retriever, Tts};
use crate::types::{Span, Stage, TurnReport};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub top_k: usize,
    /// 低于这个分数就当没命中，走降级
    pub min_score: f32,
    /// 音频层观测到的尾静音判定耗时（引擎自己测不了，由调用方提供）
    pub vad_tail: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            top_k: 5,
            min_score: 0.0,
            vad_tail: Duration::ZERO,
        }
    }
}

/// 一条问答流水线。四个依赖都是 trait，真实适配器和 mock 可以互换。
pub struct Engine<A, R, L, T>
where
    A: Asr,
    R: Retriever,
    L: Llm,
    T: Tts,
{
    asr: A,
    retriever: R,
    llm: L,
    tts: T,
    cfg: EngineConfig,
}

impl<A, R, L, T> Engine<A, R, L, T>
where
    A: Asr,
    R: Retriever,
    L: Llm,
    T: Tts,
{
    pub fn new(asr: A, retriever: R, llm: L, tts: T, cfg: EngineConfig) -> Self {
        Self {
            asr,
            retriever,
            llm,
            tts,
            cfg,
        }
    }

    /// 跑一轮。注意：这里的 TTFA 是「串行上界」——
    /// trait 是同步的，TTS 只能等生成返回才调用；真实实现里两者并行。
    /// 产品口径请用 `TurnReport::ttfa()`（按首句就绪时刻计算）。
    pub fn run_turn(&self, partial_question: &str) -> Result<TurnReport, EngineError> {
        let start = Instant::now();
        let mut spans: Vec<Span> = Vec::new();

        // 0. VAD 尾音判定：外部观测值
        spans.push(Span {
            stage: Stage::Vad,
            at: Duration::ZERO,
            took: self.cfg.vad_tail,
        });

        // 1. ASR 收敛
        let at = start.elapsed();
        let t = Instant::now();
        let asr = self.asr.finalize(partial_question)?;
        spans.push(Span {
            stage: Stage::AsrFinalize,
            at,
            took: t.elapsed(),
        });

        // 2. 检索
        let at = start.elapsed();
        let t = Instant::now();
        let hits = self.retriever.search(&asr.text, self.cfg.top_k)?;
        spans.push(Span {
            stage: Stage::Retrieve,
            at,
            took: t.elapsed(),
        });

        let mut degradations = Vec::new();
        let max_score = hits.iter().map(|h| h.score).fold(0.0f32, f32::max);
        if hits.is_empty() || max_score < self.cfg.min_score {
            degradations.push("没有材料命中，已降级为通用回答".to_string());
        }

        // 3. 生成：第一个 token 和「第一句就绪」分别打点
        let payload: PromptPayload = prompt::build(&asr.text, &hits);
        let at = start.elapsed();
        let t = Instant::now();
        let mut answer = String::new();
        let mut first_token_at: Option<Duration> = None;
        let mut first_sentence_ready: Option<Duration> = None;
        {
            let mut sink = |piece: &str| {
                if first_token_at.is_none() {
                    first_token_at = Some(t.elapsed());
                }
                answer.push_str(piece);
                if first_sentence_ready.is_none() && crate::prompt::char_count(&answer) >= 1 {
                    if has_terminator(&answer) {
                        first_sentence_ready = Some(t.elapsed());
                    }
                }
            };
            self.llm.stream(&payload, &mut sink)?;
        }
        let total_llm = t.elapsed();
        spans.push(Span {
            stage: Stage::FirstToken,
            at,
            took: first_token_at.unwrap_or(total_llm),
        });

        let first = prompt::first_sentence(&answer);
        let sentence_ready = first_sentence_ready.unwrap_or(total_llm);

        // 4. TTS 首包
        let at = start.elapsed();
        let t = Instant::now();
        self.tts.speak_first_sentence(&first)?;
        spans.push(Span {
            stage: Stage::TtsFirstPacket,
            at,
            took: t.elapsed(),
        });

        Ok(TurnReport {
            question: asr.text,
            spans,
            hits,
            answer,
            first_sentence: first,
            fs_ready: sentence_ready,
            degradations,
        })
    }
}

fn has_terminator(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '。' | '！' | '？' | '；' | '!' | '?' | '\n'))
}
