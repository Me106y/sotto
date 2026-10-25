//! Sotto 桌面壳。
//!
//! 这一层只做：开窗、管状态、把核心能力暴露给前端。
//! 业务逻辑在 `sotto-core` / `sotto-index` 和本目录的三个模块里。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod exec;
mod kb;

use std::sync::{Mutex, RwLock};
use std::time::Instant;

use serde::Serialize;
use tauri::{LogicalSize, Manager};

use config::Config;
use kb::{Kb, KbReport};

// ---------------------------------------------------------------- 状态

struct AppState {
    cfg: Mutex<Config>,
    kb: RwLock<Option<Kb>>,
    error: Mutex<Option<String>>,
}

impl AppState {
    fn rebuild(&self) -> Result<KbReport, String> {
        let cfg = self.cfg.lock().unwrap().clone();
        match Kb::build(&cfg) {
            Ok(kb) => {
                let report = kb.report.clone();
                *self.kb.write().unwrap() = Some(kb);
                *self.error.lock().unwrap() = None;
                Ok(report)
            }
            Err(e) => {
                *self.error.lock().unwrap() = Some(e.clone());
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------- DTO

#[derive(Serialize)]
struct StatusDto {
    ready: bool,
    chunks: usize,
    files: usize,
    scope: Vec<String>,
    built_ms: f64,
    error: Option<String>,
}

#[derive(Serialize)]
struct HitDto {
    rank: usize,
    source: String,
    heading: String,
    score: f64,
    /// 材料里的第一句——它才是能直接念出口的东西
    lead: String,
    body: String,
}

#[derive(Serialize)]
struct SearchDto {
    query: String,
    hits: Vec<HitDto>,
    took_ms: f64,
    chunks: usize,
    top_k: usize,
    degraded: bool,
}

fn status_of(state: &AppState) -> StatusDto {
    let cfg = state.cfg.lock().unwrap().clone();
    let guard = state.kb.read().unwrap();
    match guard.as_ref() {
        Some(kb) => {
            let mut scope = vec![kb.report.root.clone()];
            scope.extend(cfg.includes.clone());
            StatusDto {
                ready: true,
                chunks: kb.report.chunks,
                files: kb.report.files.len(),
                scope,
                built_ms: kb.report.built_ms,
                error: state.error.lock().unwrap().clone(),
            }
        }
        None => StatusDto {
            ready: false,
            chunks: 0,
            files: 0,
            scope: vec![cfg.root],
            built_ms: 0.0,
            error: state.error.lock().unwrap().clone(),
        },
    }
}

// ---------------------------------------------------------------- 命令

#[tauri::command]
fn status(state: tauri::State<'_, AppState>) -> StatusDto {
    status_of(&state)
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(state: tauri::State<'_, AppState>, cfg: Config) -> Result<StatusDto, String> {
    if !std::path::Path::new(&cfg.root).is_dir() {
        return Err(format!("目录不存在：{}", cfg.root));
    }
    cfg.save()?;
    *state.cfg.lock().unwrap() = cfg;
    state.rebuild()?;
    Ok(status_of(&state))
}

#[tauri::command]
fn kb_report(state: tauri::State<'_, AppState>) -> Result<KbReport, String> {
    let guard = state.kb.read().unwrap();
    guard
        .as_ref()
        .map(|k| k.report.clone())
        .ok_or_else(|| "索引还没建好".to_string())
}

#[tauri::command]
fn reindex(state: tauri::State<'_, AppState>) -> Result<KbReport, String> {
    state.rebuild()
}

#[tauri::command]
fn search(state: tauri::State<'_, AppState>, query: String, k: usize) -> Result<SearchDto, String> {
    let guard = state.kb.read().unwrap();
    let kb = guard.as_ref().ok_or_else(|| {
        state
            .error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "索引未就绪".into())
    })?;

    let top_k = if k == 0 {
        state.cfg.lock().unwrap().top_k
    } else {
        k
    };

    let t = Instant::now();
    let hits = kb.retriever.search(&query, top_k.max(1));
    let took_ms = t.elapsed().as_secs_f64() * 1000.0;
    let degraded = hits.is_empty() || hits.first().map(|h| h.score).unwrap_or(0.0) < 0.05;

    let hits = hits
        .into_iter()
        .enumerate()
        .map(|(i, h)| {
            let lead = pick_lead(&h.chunk.text);
            let body = plain(&h.chunk.excerpt(320));
            HitDto {
                rank: i + 1,
                source: h.chunk.source.clone(),
                heading: h.chunk.heading.clone(),
                score: h.score as f64,
                lead,
                body,
            }
        })
        .collect();

    Ok(SearchDto {
        query,
        hits,
        took_ms,
        chunks: kb.report.chunks,
        top_k,
        degraded,
    })
}

#[tauri::command]
async fn check(lang: String, code: String) -> Result<Vec<exec::Diagnostic>, String> {
    tauri::async_runtime::spawn_blocking(move || exec::check(&lang, &code))
        .await
        .map_err(|e| format!("检查线程出错：{e}"))?
}

#[tauri::command]
fn runtimes() -> Vec<exec::RuntimeInfo> {
    exec::runtimes()
}

#[tauri::command]
async fn run_code(lang: String, code: String, stdin: String) -> Result<exec::RunResult, String> {
    tauri::async_runtime::spawn_blocking(move || exec::run(&lang, &code, &stdin))
        .await
        .map_err(|e| format!("执行线程出错：{e}"))?
}

/// 自检专用：把前端结果落到文件，绕开截图/窗口标题权限。
#[tauri::command]
fn selftest_report(payload: String, tag: Option<String>) -> Result<(), String> {
    let name = match tag.as_deref() {
        Some(t) if !t.is_empty() => format!("/tmp/sotto-selftest-{t}.txt"),
        _ => "/tmp/sotto-selftest.txt".to_string(),
    };
    std::fs::write(name, payload).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 文本处理

/// 笔记常写成「**面试官问题：** … **我的回答：** …」，
/// 悬浮条上要念的是「我的回答」的第一句，不是问题本身。
fn pick_lead(text: &str) -> String {
    let mut body = text;
    if let Some(idx) = text.find("我的回答") {
        let rest = &text[idx..];
        if let Some(colon) = rest.find(|c| c == '：' || c == ':') {
            let offset = rest[colon..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            body = &text[(idx + colon + offset).min(text.len())..];
        }
    }
    plain(&sotto_core::prompt::first_sentence(body))
}

/// 去掉 Markdown 记号，屏幕上不该出现 `**` 和 `#`。
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut l = line.trim_start();
        while let Some(rest) = l.strip_prefix('#') {
            l = rest.trim_start();
        }
        out.push_str(l);
    }
    out.replace("**", "")
        .replace('`', "")
        .replace("__", "")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------- 启动

const LINT2_JS: &str = r#"
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  document.querySelector('[data-view=exam]').click();
  await wait(200);
  const ta = document.getElementById('code');
  const chip = (label) => [...document.querySelectorAll('.lang-chip')].find(b => b.textContent.includes(label));
  const read = () => {
    const p = document.getElementById('problems');
    return p.hidden ? '(无)' : (document.getElementById('pmText').textContent || '').trim().slice(0, 60);
  };

  chip('JavaScript').click();
  await wait(150);
  ta.value = 'const a = {';
  ta.dispatchEvent(new Event('input'));
  await wait(1400);
  const js = read();

  chip('C++17').click();
  await wait(150);
  ta.value = 'int main() { int x = ; return 0; }';
  ta.dispatchEvent(new Event('input'));
  await wait(1800);
  const cpp = read();

  await window.__TAURI__.core.invoke('selftest_report', {
    payload: 'js=' + js + ' | cpp=' + cpp, tag: 'lint2'
  });
})()
"#;

/// 静态检查探针：先塞一段坏代码，再塞一段好代码，看问题提示是否正确出现/消失。
const LINT_JS: &str = r#"
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  document.querySelector('[data-view=exam]').click();
  await wait(200);
  const ta = document.getElementById('code');
  const read = () => {
    const p = document.getElementById('problems');
    return {
      hidden: p.hidden,
      text: (document.getElementById('pmText').textContent || '').trim(),
      marked: document.querySelectorAll('#gutter .ln.err').length
    };
  };
  ta.value = 'def f():\n  return 1\nprint(f(';
  ta.dispatchEvent(new Event('input'));
  await wait(1600);
  const bad = read();
  ta.value = 'print(1)';
  ta.dispatchEvent(new Event('input'));
  await wait(1600);
  const good = read();
  const payload = 'bad=' + JSON.stringify(bad) + ' good=' + JSON.stringify(good);
  await window.__TAURI__.core.invoke('selftest_report', { payload, tag: 'lint' });
})()
"#;

/// 对齐探针：把行号基线和代码行基线逐行比对，差值应为 0。
/// 比截图可靠——截图只能靠眼睛，这个能给出小数。
const MEASURE_JS: &str = r#"
(async () => {
  const ta = document.getElementById('code');
  const pane = document.getElementById('editorBox');
  const lns = () => document.querySelectorAll('#gutter .ln');
  const frame = () => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));

  ta.value = Array.from({length: 14}, (_, i) => 'line_' + (i + 1) + ' = ' + i).join('\n');
  ta.dispatchEvent(new Event('input'));
  await frame();

  const cs = getComputedStyle(ta);
  const lh = parseFloat(cs.lineHeight);
  const padTop = parseFloat(cs.paddingTop);

  const deltas = () => {
    const ds = [];
    [0, 1, 3, 7, 11, 13].forEach(i => {
      const n = lns()[i];
      if (!n) return;
      const target = ta.getBoundingClientRect().top + padTop + i * lh;
      ds.push((n.getBoundingClientRect().top - target).toFixed(1));
    });
    return ds.join(',');
  };

  const plain = deltas();
  pane.scrollTop = 107;
  await frame();
  const scrolled = deltas();
  pane.scrollTop = 0;

  const payload = 'size=' + innerWidth + 'x' + innerHeight +
    ' lines=' + lns().length + ' lh=' + lh +
    ' plain=[' + plain + '] scrolled107=[' + scrolled + ']';
  await window.__TAURI__.core.invoke('selftest_report', { payload, tag: window.__TAG });
})()
"#;

const SELFTEST_JS: &str = r#"
(async () => {
  const text = (document.body.innerText || '').replace(/\s+/g, ' ').trim();
  let out = 'WIN[' + window.innerWidth + 'x' + window.innerHeight + '] DOM[' + text.slice(0, 30) + ']';
  try {
    const st = await window.__TAURI__.core.invoke('status');
    out += ' STATUS[' + st.ready + '/' + st.chunks + '/' + st.files + ']';
    const r = await window.__TAURI__.core.invoke('search', { query: '置信度分流', k: 2 });
    out += ' SEARCH[' + r.hits.length + '/' + r.took_ms.toFixed(1) + 'ms]';
    if (r.hits[0]) out += ' TOP[' + r.hits[0].source.split('/').pop() + ']';
    const rt = await window.__TAURI__.core.invoke('runtimes');
    out += ' RT[' + rt.filter(x => x.available).length + '/' + rt.length + ']';
    const kb = await window.__TAURI__.core.invoke('kb_report');
    out += ' KB[' + kb.files.length + '/' + kb.skipped.length + ']';
    const run = await window.__TAURI__.core.invoke('run_code', {
      lang: 'python', code: 'a,b=map(int,input().split())\nprint(a+b)', stdin: '1 2'
    });
    out += ' RUN[' + run.status + ':' + run.stdout.trim() + ':' + run.duration_ms.toFixed(0) + 'ms]';
    const bad = await window.__TAURI__.core.invoke('run_code', {
      lang: 'python', code: 'x=1\nprint(y)', stdin: ''
    });
    out += ' ERR[' + bad.status + '/' + bad.error_line + ']';
  } catch (e) { out += ' EXC[' + String(e).slice(0, 90) + ']'; }
  try { await window.__TAURI__.core.invoke('selftest_report', { payload: out }); }
  catch (e) { document.title = 'REPORT_FAIL'; }
})()
"#;

fn main() {
    let cfg = Config::load().with_env_overrides();
    // 第一次启动就把默认配置写出来，让设置页有东西可看、可改。
    if !Config::path().exists() {
        let _ = cfg.save();
    }
    let state = AppState {
        kb: RwLock::new(None),
        error: Mutex::new(None),
        cfg: Mutex::new(cfg),
    };
    if let Err(e) = state.rebuild() {
        *state.error.lock().unwrap() = Some(e);
    }

    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            // 不依赖配置能不能被读到：显式定尺寸、居中、显示。
            // 有一次启动时窗口以默认尺寸且不可见地创建了，加这层兜底。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_size(LogicalSize::new(1040.0, 720.0));
                let _ = win.center();
                let _ = win.show();
                let _ = win.set_focus();
            }

            if let Ok(mode) = std::env::var("SOTTO_SELFTEST") {
                if let Some(win) = app.get_webview_window("main") {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(2500));
                        if mode == "align" {
                            // 切到笔试页，再灌一段 14 行的代码，然后分别在
                            // 普通窗口 / 全屏下量行号与代码的基线差
                            let _ = win.eval("document.querySelector('[data-view=exam]').click();");
                            std::thread::sleep(std::time::Duration::from_millis(300));
                            std::thread::sleep(std::time::Duration::from_millis(300));
                            let _ = win.eval(&format!("window.__TAG='normal';{MEASURE_JS}"));
                            std::thread::sleep(std::time::Duration::from_millis(900));
                            let _ = win.set_fullscreen(true);
                            std::thread::sleep(std::time::Duration::from_millis(1600));
                            let _ = win.eval(&format!("window.__TAG='full';{MEASURE_JS}"));
                            std::thread::sleep(std::time::Duration::from_millis(900));
                            let _ = win.set_fullscreen(false);
                        } else if mode == "lint" {
                            let _ = win.eval(LINT_JS);
                        } else if mode == "lint2" {
                            let _ = win.eval(LINT2_JS);
                        } else {
                            let _ = win.eval(SELFTEST_JS);
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            search,
            get_settings,
            save_settings,
            kb_report,
            reindex,
            runtimes,
            run_code,
            check,
            selftest_report
        ])
        .run(tauri::generate_context!())
        .expect("Sotto 启动失败");
}
