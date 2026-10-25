//! 语音合成：macOS 的 `say`。
//!
//! 选它而不是云 TTS 的原因：**零延迟、不联网、不用第二个 key**。
//! 云 TTS 的首包要 200~400ms，而 `say` 是本地进程，几十毫秒就出声。
//! 代价是音色偏播报——先用着，等有了火山 TTS 的 appid 再换。
//!
//! 队列是必须的：答案按句流式产出，一句一句送进来；
//! 如果每句都直接 spawn，前一句会被后一句打断。

use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct Job {
    text: String,
    voice: String,
    rate: u32,
    generation: u64,
}

pub struct Speaker {
    tx: Sender<Job>,
    current: Arc<Mutex<Option<Child>>>,
    generation: Arc<AtomicU64>,
}

impl Speaker {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let current: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));

        let slot = current.clone();
        let gen_shared = generation.clone();
        thread::spawn(move || {
            for job in rx {
                // 新一轮提问时，队列里剩下的旧句子直接丢掉
                if job.generation != gen_shared.load(Ordering::SeqCst) {
                    continue;
                }
                let rate = job.rate.max(80).to_string();
                let mut child = match Command::new("say")
                    .arg("-v")
                    .arg(&job.voice)
                    .arg("-r")
                    .arg(&rate)
                    .arg(&job.text)
                    .spawn()
                {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                *slot.lock().unwrap() = Some(child);

                // 等这一句念完，再念下一句
                loop {
                    let mut guard = slot.lock().unwrap();
                    match guard.as_mut() {
                        Some(c) => match c.try_wait() {
                            Ok(Some(_)) => {
                                *guard = None;
                                break;
                            }
                            Ok(None) => {}
                            Err(_) => {
                                *guard = None;
                                break;
                            }
                        },
                        None => break, // 被 stop 拿走了
                    }
                    drop(guard);
                    thread::sleep(Duration::from_millis(15));
                }
            }
        });

        Self {
            tx,
            current,
            generation,
        }
    }

    /// 入队一句话。返回它属于第几代（新一轮提问会换代，旧句子自动作废）。
    pub fn enqueue(&self, text: &str, voice: &str, rate: u32) -> u64 {
        let generation = self.generation.load(Ordering::SeqCst);
        let _ = self.tx.send(Job {
            text: text.to_string(),
            voice: voice.to_string(),
            rate,
            generation,
        });
        generation
    }

    /// 立刻闭嘴：杀掉当前这句，并让队列里的后续句子失效。
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(mut child) = self.current.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 系统里装了哪些中文音色。
pub fn chinese_voices() -> Vec<String> {
    let out = Command::new("say").arg("-v").arg("?").output();
    let Ok(out) = out else {
        return vec!["Tingting".into()];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut voices: Vec<String> = text
        .lines()
        .filter(|l| l.contains("zh_CN"))
        .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
        .collect();
    voices.sort();
    voices.dedup();
    if voices.is_empty() {
        voices.push("Tingting".into());
    }
    voices
}

/// 合成一句话要多久（不含播放）。给「TTS 首包」这个指标一个可测的数。
pub fn synth_cost_ms(text: &str, voice: &str, rate: u32) -> f64 {
    let dir = std::env::temp_dir().join(format!("sotto-tts-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join("probe.aiff");
    let rate_s = rate.max(80).to_string();

    let started = Instant::now();
    let ok = Command::new("say")
        .arg("-v")
        .arg(voice)
        .arg("-r")
        .arg(&rate_s)
        .arg("-o")
        .arg(&out)
        .arg(text)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_dir(&dir);
    if ok {
        ms
    } else {
        -1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_always_yield_something() {
        let v = chinese_voices();
        assert!(!v.is_empty());
    }
}
