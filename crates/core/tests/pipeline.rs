use std::sync::{Arc, Mutex};
use std::time::Duration;

use sotto_core::mock::{MockAsr, MockLlm, MockRetriever, MockTts};
use sotto_core::{Engine, EngineConfig, EngineError, Llm, PromptPayload, Stage};

fn engine(
    asr: u64,
    ret: u64,
    ttft: u64,
    tts: u64,
) -> Engine<MockAsr, MockRetriever, MockLlm, MockTts> {
    Engine::new(
        MockAsr::new(asr),
        MockRetriever::scripted(ret),
        MockLlm::new(ttft, 0),
        MockTts::new(tts),
        EngineConfig {
            top_k: 5,
            min_score: 0.0,
            vad_tail: Duration::from_millis(0),
        },
    )
}

#[test]
fn turn_records_every_stage_in_order() {
    let e = engine(1, 1, 1, 1);
    let r = e.run_turn("冷启动你们怎么解决的？").unwrap();

    let order: Vec<Stage> = r.spans.iter().map(|s| s.stage).collect();
    assert_eq!(
        order,
        vec![
            Stage::Vad,
            Stage::AsrFinalize,
            Stage::Retrieve,
            Stage::FirstToken,
            Stage::TtsFirstPacket
        ]
    );

    assert_eq!(r.question, "冷启动你们怎么解决的？");
    assert!(!r.hits.is_empty());
    assert!(r.degradations.is_empty());
    assert_eq!(r.first_sentence, "冷启动我们是先用规则兜住的。");
}

#[test]
fn product_ttfa_never_exceeds_the_serial_measurement() {
    let e = engine(1, 1, 1, 1);
    let r = e.run_turn("随便问问").unwrap();
    assert!(
        r.ttfa() <= r.ttfa_serial() + Duration::from_millis(10),
        "ttfa={:?} serial={:?}",
        r.ttfa(),
        r.ttfa_serial()
    );
}

#[test]
fn missing_material_is_reported_as_a_degradation() {
    let e = Engine::new(
        MockAsr::new(0),
        MockRetriever::new(Vec::new(), 0),
        MockLlm::new(0, 0),
        MockTts::new(0),
        EngineConfig::default(),
    );
    let r = e.run_turn("你最大的缺点是什么").unwrap();
    assert!(r.hits.is_empty());
    assert_eq!(r.degradations.len(), 1);
    assert!(r.degradations[0].contains("降级"));
}

/// 记录 prompt 的假模型：材料必须真的被塞进去，否则答案就是编的。
struct Spy {
    seen: Arc<Mutex<Option<String>>>,
}

impl Llm for Spy {
    fn stream(
        &self,
        prompt: &PromptPayload,
        sink: &mut dyn FnMut(&str),
    ) -> Result<(), EngineError> {
        *self.seen.lock().unwrap() = Some(format!("{}\n---\n{}", prompt.system, prompt.user));
        sink("好的，我按材料说。");
        Ok(())
    }
}

#[test]
fn retrieved_material_reaches_the_model() {
    let seen = Arc::new(Mutex::new(None));
    let e = Engine::new(
        MockAsr::new(0),
        MockRetriever::scripted(0),
        Spy { seen: seen.clone() },
        MockTts::new(0),
        EngineConfig::default(),
    );
    e.run_turn("冷启动怎么做的").unwrap();

    let text = seen.lock().unwrap().clone().expect("模型没被调用");
    assert!(
        text.contains("项目经历/SGS/SGS-项目经历.md"),
        "prompt 里没有材料来源：{text}"
    );
    assert!(text.contains("冷启动怎么做的"));
    assert!(text.contains("120"), "缺少字数硬约束");
}
