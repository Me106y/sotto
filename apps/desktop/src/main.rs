//! Sotto 桌面壳。
//!
//! 这一层只做三件事：开窗、读本地语料、把检索结果交给前端。
//! 所有业务逻辑在 `sotto-core` / `sotto-index` 里，这里不写判断。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::Manager;
use sotto_index::{build_corpus, HybridRetriever, ScanConfig};

#[derive(Serialize)]
struct HitDto {
    rank: usize,
    source: String,
    heading: String,
    score: f64,
    /// 材料里的第一句——它才是能直接念出口的东西
    lead: String,
    /// 其余部分，用作备查
    body: String,
}

#[derive(Serialize)]
struct SearchDto {
    query: String,
    hits: Vec<HitDto>,
    took_ms: f64,
    chunks: usize,
    files: usize,
    scope: Vec<String>,
}

#[derive(Serialize)]
struct StatusDto {
    ready: bool,
    chunks: usize,
    files: usize,
    scope: Vec<String>,
    error: Option<String>,
}

struct App {
    retriever: Option<HybridRetriever>,
    files: usize,
    scope: Vec<String>,
    error: Option<String>,
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// 作用域默认值。真实版本会从设置页读，M1 先用环境变量覆盖。
fn scope_config() -> (PathBuf, Vec<String>) {
    let root = std::env::var("SOTTO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join("Desktop/Note"));
    let includes = std::env::var("SOTTO_INCLUDE")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_else(|_| {
            vec![
                "项目经历".to_string(),
                "面试准备".to_string(),
                "学习".to_string(),
            ]
        });
    (root, includes)
}

/// 自检专用：把前端结果落到文件，绕开截图/窗口标题权限。
#[tauri::command]
fn selftest_report(payload: String) -> Result<(), String> {
    std::fs::write("/tmp/sotto-selftest.txt", payload).map_err(|e| e.to_string())
}

#[tauri::command]
fn status(state: tauri::State<'_, App>) -> StatusDto {
    StatusDto {
        ready: state.retriever.is_some(),
        chunks: state.retriever.as_ref().map(|r| r.len()).unwrap_or(0),
        files: state.files,
        scope: state.scope.clone(),
        error: state.error.clone(),
    }
}


/// 笔记里常写成「**面试官问题：** … **我的回答：** …」。
/// 悬浮条上要念的是「我的回答」的第一句，不是问题本身。
fn pick_lead(text: &str) -> String {
    let mut body = text;
    if let Some(idx) = text.find("我的回答") {
        let rest = &text[idx..];
        if let Some(colon) = rest.find(|c| c == '：' || c == ':') {
            let start = idx + colon + '：'.len_utf8().min(rest[colon..].chars().next().map(|c| c.len_utf8()).unwrap_or(1));
            body = &text[start.min(text.len())..];
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

#[tauri::command]
fn search(state: tauri::State<'_, App>, query: String, k: usize) -> Result<SearchDto, String> {
    let retriever = state
        .retriever
        .as_ref()
        .ok_or_else(|| state.error.clone().unwrap_or_else(|| "索引未就绪".into()))?;

    let t = Instant::now();
    let hits = retriever.search(&query, k.max(1));
    let took_ms = t.elapsed().as_secs_f64() * 1000.0;

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
        chunks: retriever.len(),
        files: state.files,
        scope: state.scope.clone(),
    })
}

const SELFTEST_JS: &str = r#"
(async () => {
  const text = (document.body.innerText || '').replace(/\s+/g, ' ').trim();
  let out = 'DOM[' + text.slice(0, 46) + ']';
  try {
    const st = await window.__TAURI__.core.invoke('status');
    out += ' STATUS[' + st.ready + '/' + st.chunks + '/' + st.files + ']';
    const r = await window.__TAURI__.core.invoke('search', { query: '置信度分流', k: 2 });
    out += ' SEARCH[' + r.hits.length + '/' + r.took_ms.toFixed(1) + 'ms]';
    if (r.hits[0]) out += ' TOP[' + r.hits[0].source.split('/').pop() + ']';
    out += ' LEAD[' + (r.hits[0] ? r.hits[0].lead.slice(0, 18) : '-') + ']';
  } catch (e) { out += ' ERR[' + String(e).slice(0, 80) + ']'; }
  try { await window.__TAURI__.core.invoke('selftest_report', { payload: out }); }
  catch (e) { document.title = 'REPORT_FAIL ' + String(e); }
})()
"#;

fn main() {
    let (root, includes) = scope_config();
    let scope = {
        let mut s = vec![root.to_string_lossy().to_string()];
        s.extend(includes.clone());
        s
    };

    let mut cfg = ScanConfig::new(&root);
    for i in &includes {
        cfg = cfg.include(i.clone());
    }

    let (retriever, files, error) = match build_corpus(&cfg) {
        Ok((chunks, scan)) => (Some(HybridRetriever::new(chunks)), scan.files.len(), None),
        Err(e) => (None, 0, Some(format!("读不到语料：{e}"))),
    };

    let app = App {
        retriever,
        files,
        scope,
        error,
    };

    tauri::Builder::default()
        .manage(app)
        .setup(|app| {
            // 自检：把前端渲染结果 + invoke 通路写进窗口标题，
            // 这样在没有截图权限的环境里也能验证 UI 真的活了。
            if std::env::var("SOTTO_SELFTEST").is_ok() {
                if let Some(win) = app.get_webview_window("main") {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1200));
                        let _ = win.eval(SELFTEST_JS);
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![search, status, selftest_report])
        .run(tauri::generate_context!())
        .expect("Sotto 启动失败");
}
