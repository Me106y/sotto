//! 豆包（火山方舟）客户端。
//!
//! 用的是 Ark 的 `/responses` 接口，不是老的 chat/completions：
//! - `instructions` 当 system prompt
//! - `thinking: disabled` 关掉推理——面试场景没有时间等它想
//! - `stream: true` 拿 SSE，只有流式才测得出首字延迟

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::config::Config;

pub struct ArkClient {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: f64,
}

#[derive(Debug, Clone)]
pub struct StreamOutcome {
    pub text: String,
    /// 首个 delta 的时间：这才是产品口径的「开口」时刻
    pub ttft_ms: f64,
    pub total_ms: f64,
    pub deltas: usize,
}

impl ArkClient {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            api_key: cfg.api_key.clone(),
            base_url: cfg.base_url.clone(),
            model: cfg.model.clone(),
            temperature: cfg.temperature,
        }
    }

    pub fn configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn stream<F>(
        &self,
        instructions: &str,
        user: &str,
        mut on_delta: F,
    ) -> Result<StreamOutcome, String>
    where
        F: FnMut(&str),
    {
        if !self.configured() {
            return Err("没有配置 API key".into());
        }
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "thinking": { "type": "disabled" },
            "temperature": self.temperature,
            "instructions": instructions,
            "stream": true,
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": user }]
            }]
        });

        let started = Instant::now();
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .timeout(Duration::from_secs(60));

        let response = match request.send_string(&body.to_string()) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                let brief: String = text.chars().take(240).collect();
                return Err(format!("HTTP {code}：{brief}"));
            }
            Err(e) => return Err(format!("请求失败：{e}")),
        };

        let mut reader = BufReader::new(response.into_reader());
        let mut line = String::new();
        let mut text = String::new();
        let mut ttft: Option<f64> = None;
        let mut deltas = 0usize;

        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("读流失败：{e}"))?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim_end();
            let Some(rest) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let payload = rest.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };
            match v.get("type").and_then(|x| x.as_str()) {
                Some("response.output_text.delta") => {
                    if let Some(d) = v.get("delta").and_then(|x| x.as_str()) {
                        if ttft.is_none() {
                            ttft = Some(started.elapsed().as_secs_f64() * 1000.0);
                        }
                        text.push_str(d);
                        deltas += 1;
                        on_delta(d);
                    }
                }
                Some("response.completed") | Some("response.failed") => break,
                _ => {}
            }
        }

        if text.trim().is_empty() {
            return Err("模型没有返回任何内容".into());
        }

        Ok(StreamOutcome {
            text,
            ttft_ms: ttft.unwrap_or_else(|| started.elapsed().as_secs_f64() * 1000.0),
            total_ms: started.elapsed().as_secs_f64() * 1000.0,
            deltas,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_is_reported_before_any_request() {
        let client = ArkClient {
            api_key: String::new(),
            base_url: "http://127.0.0.1:9".into(),
            model: "x".into(),
            temperature: 0.4,
        };
        let err = client.stream("s", "u", |_| {}).unwrap_err();
        assert!(err.contains("API key"));
    }
}
