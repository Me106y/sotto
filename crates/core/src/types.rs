use std::time::Duration;

/// 一次问答里被单独打点的阶段。顺序即真实发生的顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// 音频层观测到的尾静音判定耗时（由调用方提供，不在引擎内部产生）
    Vad,
    /// 末句 ASR 收敛
    AsrFinalize,
    /// 本地检索
    Retrieve,
    /// LLM 首 token
    FirstToken,
    /// TTS 首包
    TtsFirstPacket,
}

impl Stage {
    pub const ALL: [Stage; 5] = [
        Stage::Vad,
        Stage::AsrFinalize,
        Stage::Retrieve,
        Stage::FirstToken,
        Stage::TtsFirstPacket,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Stage::Vad => "VAD 收尾",
            Stage::AsrFinalize => "ASR 收敛",
            Stage::Retrieve => "本地检索",
            Stage::FirstToken => "LLM 首字",
            Stage::TtsFirstPacket => "TTS 首包",
        }
    }

    /// 这个阶段是否计入「首字可听」预算
    pub fn in_ttfa(self) -> bool {
        true
    }
}

/// 一个阶段的时间戳。`at` 是相对本轮开始的偏移，`took` 是本阶段耗时。
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub stage: Stage,
    pub at: Duration,
    pub took: Duration,
}

impl Span {
    pub fn took_ms(&self) -> f32 {
        self.took.as_secs_f32() * 1000.0
    }

    /// 本阶段结束时刻（相对本轮开始）
    pub fn end_ms(&self) -> f32 {
        (self.at + self.took).as_secs_f32() * 1000.0
    }
}

/// 一条检索命中，携带来源信息——悬浮条要把它显示出来。
#[derive(Debug, Clone)]
pub struct HitInfo {
    pub chunk_id: usize,
    pub source: String,
    pub heading: String,
    pub score: f32,
    pub excerpt: String,
}

/// 一轮问答的完整记录。UI 和指标都从它长出来。
#[derive(Debug, Clone, Default)]
pub struct TurnReport {
    pub question: String,
    pub spans: Vec<Span>,
    pub hits: Vec<HitInfo>,
    pub answer: String,
    pub first_sentence: String,
    /// 第一句生成完毕的时刻（相对本轮开始）。TTS 从这里就可以开口，
    /// 不必等全文生成完——这是产品口径的 TTFA 起点。
    pub fs_ready: Duration,
    /// 降级原因（无材料命中、断网兜底…），为空表示正常
    pub degradations: Vec<String>,
}

impl TurnReport {
    /// 产品口径：第一句就绪 + TTS 首包。TTS 与后续生成并行。
    /// 超过 3s 这个产品就废了。
    pub fn ttfa(&self) -> Duration {
        let tts = self
            .span(Stage::TtsFirstPacket)
            .map(|s| s.took)
            .unwrap_or_default();
        self.fs_ready + tts
    }

    /// 实测口径：同步流水线里所有阶段串行执行的总耗时。
    /// 真实实现会更快，因为 TTS 与生成重叠。
    pub fn ttfa_serial(&self) -> Duration {
        self.spans
            .iter()
            .map(|s| s.at + s.took)
            .max()
            .unwrap_or_default()
    }

    pub fn span(&self, stage: Stage) -> Option<Span> {
        self.spans.iter().copied().find(|s| s.stage == stage)
    }

    pub fn ttfa_ms(&self) -> f32 {
        self.ttfa().as_secs_f32() * 1000.0
    }

    pub fn ttfa_serial_ms(&self) -> f32 {
        self.ttfa_serial().as_secs_f32() * 1000.0
    }
}
