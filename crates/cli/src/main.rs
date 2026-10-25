//! Sotto CLI —— M0 的验证工具。
//!
//! 这里没有 UI，只有四件能在没有 API key 的情况下验证的事：
//! 1. 语料能不能被正确扫出来、切对（scan / chunk）
//! 2. 检索能不能命中真实材料（ask / verify）
//! 3. 管线打点对不对（latency，mock 延迟占位）
//! 4. 延迟预算在哪一步崩（budget，Monte Carlo）

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use sotto_core::mock::{MockAsr, MockLlm, MockRetriever, MockTts};
use sotto_core::{Budget, Engine, EngineConfig, Stage};
use sotto_index::{build_corpus, HybridRetriever, ScanConfig};

const VALUE_FLAGS: &[&str] = &[
    "--root",
    "--include",
    "--k",
    "--file",
    "--trials",
    "--vad",
    "--asr",
    "--retrieve",
    "--ttft",
    "--tts",
    "--seed",
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];

    let result = match cmd {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "scan" => cmd_scan(rest),
        "chunk" => cmd_chunk(rest),
        "ask" => cmd_ask(rest),
        "verify" => cmd_verify(rest),
        "prompt" => cmd_prompt(rest),
        "chat" => cmd_chat(rest),
        "latency" => cmd_latency(rest),
        "budget" => cmd_budget(rest),
        other => {
            eprintln!("未知子命令：{other}\n");
            print_help();
            Ok(())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------- 子命令

fn cmd_scan(args: &[String]) -> Result<(), String> {
    let cfg = scan_cfg(args);
    let r = sotto_index::scan(&cfg).map_err(|e| e.to_string())?;

    println!("作用域：{}", cfg.root.display());
    if !cfg.includes.is_empty() {
        println!("包含子目录：{}", cfg.includes.join("、"));
    }
    println!("Markdown：{} 篇 / {:.1} KB", r.files.len(), r.total_bytes() as f64 / 1024.0);
    println!("非 Markdown 文件（跳过）：{} 个", r.non_markdown);

    println!("\n按目录：");
    let mut by_dir: Vec<(String, usize, u64)> = Vec::new();
    for f in &r.files {
        let dir = top_dir(&f.rel);
        match by_dir.iter_mut().find(|x| x.0 == dir) {
            Some(x) => {
                x.1 += 1;
                x.2 += f.bytes;
            }
            None => by_dir.push((dir, 1, f.bytes)),
        }
    }
    by_dir.sort_by(|a, b| b.2.cmp(&a.2));
    for (dir, n, bytes) in &by_dir {
        println!("  {:<28} {:>4} 篇  {:>8.1} KB", dir, n, *bytes as f64 / 1024.0);
    }

    if !r.skipped.is_empty() {
        println!("\n被跳过（{}）：", r.skipped.len());
        for s in r.skipped.iter().take(12) {
            println!("  [{}] {}", s.reason.label(), s.rel);
        }
        if r.skipped.len() > 12 {
            println!("  …… 另有 {} 项", r.skipped.len() - 12);
        }
    }
    Ok(())
}

fn cmd_chunk(args: &[String]) -> Result<(), String> {
    let cfg = scan_cfg(args);
    let (chunks, scan) = build_corpus(&cfg).map_err(|e| e.to_string())?;

    let total_chars: usize = chunks.iter().map(|c| c.chars).sum();
    let max = chunks.iter().map(|c| c.chars).max().unwrap_or(0);
    let avg = if chunks.is_empty() {
        0.0
    } else {
        total_chars as f64 / chunks.len() as f64
    };

    println!("文件：{} 篇", scan.files.len());
    println!(
        "chunk：{} 个 / 共 {:.1} 万字 / 平均 {:.0} 字 / 最长 {} 字",
        chunks.len(),
        total_chars as f64 / 10_000.0,
        avg,
        max
    );

    let qa = chunks.iter().filter(|c| c.kind.label() == "问答").count();
    let sec = chunks.iter().filter(|c| c.kind.label() == "小节").count();
    let whole = chunks.iter().filter(|c| c.kind.label() == "整篇").count();
    println!("构成：问答 {qa} / 小节 {sec} / 整篇 {whole}");

    println!("\n前 5 个 chunk：");
    for c in chunks.iter().take(5) {
        println!(
            "\n  [{}] {} · {}  ({} 字)",
            c.kind.label(),
            c.source,
            c.heading,
            c.chars
        );
        println!("  {}", c.excerpt(80));
    }
    Ok(())
}

fn cmd_ask(args: &[String]) -> Result<(), String> {
    let k: usize = flag_value(args, "--k")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let question = positional(args).join(" ");
    if question.trim().is_empty() {
        return Err("用法：sotto ask --root <笔记目录> \"你的问题\"".into());
    }

    let cfg = scan_cfg(args);
    let (chunks, _) = build_corpus(&cfg).map_err(|e| e.to_string())?;
    let retriever = HybridRetriever::new(chunks);

    let t = Instant::now();
    let hits = retriever.search(&question, k);
    let took = t.elapsed();

    println!("问题：{question}");
    println!(
        "索引 {} 个 chunk，检索耗时 {:.2} ms\n",
        retriever.len(),
        took.as_secs_f64() * 1000.0
    );

    if hits.is_empty() {
        println!("（没有命中）");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        println!(
            "{}. score {:.3}  (bm25 {:.2} / 重合 {:.2})",
            i + 1,
            h.score,
            h.bm25,
            h.overlap
        );
        println!("   {} · {}", h.chunk.source, h.chunk.heading);
        println!("   {}\n", h.chunk.excerpt(140));
    }
    Ok(())
}

/// 问题文件格式：每行 `问题|||关键词1,关键词2|||期望路径片段`
///
/// - 只给关键词：宽松判定，top-k 里出现任意关键词就算命中（容易假阳）
/// - 再给期望路径：严格判定，top-k 里必须出现该路径才算命中（建议都写）
/// 把真实材料拼成最终 prompt，看看模型到底会看到什么。
/// 这一步最容易出问题：材料塞错、来源丢失、注入过长。
fn cmd_prompt(args: &[String]) -> Result<(), String> {
    let k: usize = flag_value(args, "--k")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let question = positional(args).join(" ");
    if question.trim().is_empty() {
        return Err("用法：sotto prompt --root <笔记目录> <问题>".into());
    }

    let cfg = scan_cfg(args);
    let (chunks, _) = build_corpus(&cfg).map_err(|e| e.to_string())?;
    let retriever = HybridRetriever::new(chunks);
    let hits = <HybridRetriever as sotto_core::Retriever>::search(&retriever, &question, k)
        .map_err(|e| e.to_string())?;
    let p = sotto_core::prompt::build(&question, &hits);

    println!("== system ==");
    println!("{}", p.system);
    println!("== user ==");
    println!("{}", p.user);
    println!(
        "\n---------\n材料 {} 条 / user {} 字",
        hits.len(),
        sotto_core::prompt::char_count(&p.user)
    );
    Ok(())
}

/// 交互式检索：给 Dock / 终端用的最小可用形态。
/// 现在只显示「命中了哪几段材料」，不假装生成答案——模型还没接。
fn cmd_chat(args: &[String]) -> Result<(), String> {
    use std::io::{self, BufRead, Write};

    let k: usize = flag_value(args, "--k")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let cfg = scan_cfg(args);
    let (chunks, scan) = build_corpus(&cfg).map_err(|e| e.to_string())?;
    let retriever = HybridRetriever::new(chunks);

    println!("Sotto · M0");
    println!(
        "{} 篇笔记 → {} 个 chunk   作用域：{}",
        scan.files.len(),
        retriever.len(),
        cfg.root.display()
    );
    println!("模型未接入，下面只显示检索到的材料。输入问题回车，q 退出。\n");

    let stdin = io::stdin();
    loop {
        print!("问> ");
        let _ = io::stdout().flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let q = line.trim();
        if q.is_empty() {
            continue;
        }
        if matches!(q, "q" | "quit" | "exit") {
            break;
        }
        if q == "?" {
            println!("直接输问题。检索范围：{}，相似度前 {} 条\n", cfg.root.display(), k);
            continue;
        }

        let t = Instant::now();
        let hits = retriever.search(q, k);
        let ms = t.elapsed().as_secs_f64() * 1000.0;

        if hits.is_empty() {
            println!("   没命中。可能是作用域没圈到，或者这条笔记还没写。\n");
            continue;
        }
        for (i, h) in hits.iter().enumerate() {
            println!("  {}. [{}] {:.2}分", i + 1, h.chunk.source, h.score);
            println!("     {}", h.chunk.heading);
            println!("     {}", h.chunk.excerpt(110));
        }
        println!("  —— {} 条 / {:.1} ms\n", hits.len(), ms);
    }
    println!("拜拜。");
    Ok(())
}

struct Case {
    question: String,
    keywords: Vec<String>,
    expect_path: Option<String>,
}

fn cmd_verify(args: &[String]) -> Result<(), String> {
    let file = flag_value(args, "--file").ok_or("需要 --file <问题文件>")?;
    let k: usize = flag_value(args, "--k")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let cfg = scan_cfg(args);
    let (chunks, _) = build_corpus(&cfg).map_err(|e| e.to_string())?;
    let retriever = HybridRetriever::new(chunks);

    let raw = fs::read_to_string(&file).map_err(|e| format!("读不到 {file}：{e}"))?;
    let cases: Vec<Case> = raw
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let parts: Vec<&str> = l.split("|||").collect();
            Case {
                question: parts.first().map(|x| x.trim().to_string()).unwrap_or_default(),
                keywords: parts
                    .get(1)
                    .map(|x| {
                        x.split(&[',', '，'][..])
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                expect_path: parts
                    .get(2)
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty()),
            }
        })
        .collect();

    println!("索引 {} 个 chunk，共 {} 个问题\n", retriever.len(), cases.len());

    let mut passed = 0;
    let mut strict_passed = 0;
    let mut strict_total = 0;
    let mut rank_sum = 0usize;
    let mut total_ms = 0.0f64;

    for case in &cases {
        let t = Instant::now();
        let hits = retriever.search(&case.question, k);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        total_ms += ms;

        let (ok, note) = judge(case, &hits);
        if ok {
            passed += 1;
        }
        if case.expect_path.is_some() {
            strict_total += 1;
            if let Some(pos) = hits
                .iter()
                .position(|h| h.chunk.source.contains(case.expect_path.as_deref().unwrap_or("")))
            {
                strict_passed += 1;
                rank_sum += pos + 1;
            }
        }

        println!("{} {}", if ok { "✅" } else { "❌" }, case.question);
        match hits.first() {
            Some(h) => println!(
                "    → {} · {}   ({:.2} ms)",
                h.chunk.source, h.chunk.heading, ms
            ),
            None => println!("    → 无命中"),
        }
        if !ok {
            println!("      {note}");
        }
    }

    let rate = passed as f64 / cases.len().max(1) as f64 * 100.0;
    println!(
        "\n宽松召回：{}/{} = {:.0}%   平均检索 {:.2} ms",
        passed,
        cases.len(),
        rate,
        total_ms / cases.len().max(1) as f64
    );
    if strict_total > 0 {
        println!(
            "严格召回（要求命中指定文件）：{}/{} = {:.0}%，命中时平均排名 {:.1}",
            strict_passed,
            strict_total,
            strict_passed as f64 / strict_total as f64 * 100.0,
            rank_sum as f64 / strict_passed.max(1) as f64
        );
    }
    Ok(())
}

fn judge(case: &Case, hits: &[sotto_index::Hit]) -> (bool, String) {
    if let Some(expect) = &case.expect_path {
        return match hits.iter().position(|h| h.chunk.source.contains(expect.as_str())) {
            Some(pos) => (true, format!("命中，排名第 {}", pos + 1)),
            None => (
                false,
                format!("期望文件《{expect}》没进 top-{}", hits.len()),
            ),
        };
    }
    let hay = hits
        .iter()
        .map(|h| format!("{} {} {}", h.chunk.heading, h.chunk.source, h.chunk.text).to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let ok = case.keywords.is_empty() || case.keywords.iter().any(|kw| hay.contains(kw));
    (ok, if ok { "关键词命中".into() } else { "关键词未命中".into() })
}

fn cmd_latency(args: &[String]) -> Result<(), String> {
    let trials: usize = flag_value(args, "--trials")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let ms = |name: &str, default: u64| -> u64 {
        flag_value(args, name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };

    let vad = ms("--vad", 0);
    let asr = ms("--asr", 150);
    let retrieve = ms("--retrieve", 100);
    let ttft = ms("--ttft", 400);
    let tts = ms("--tts", 300);

    println!("mock 延迟：VAD {vad}ms / ASR {asr}ms / 检索 {retrieve}ms / 首字 {ttft}ms / TTS {tts}ms");
    println!("注意：这些是占位值，真实延迟必须用真适配器测。\n");

    let engine = Engine::new(
        MockAsr::new(asr),
        MockRetriever::scripted(retrieve),
        MockLlm::new(ttft, 0),
        MockTts::new(tts),
        EngineConfig {
            top_k: 5,
            min_score: 0.0,
            vad_tail: std::time::Duration::from_millis(vad),
        },
    );

    let mut per_stage: Vec<Vec<f32>> = vec![Vec::new(); Stage::ALL.len()];
    let mut ttfa: Vec<f32> = Vec::new();
    let mut serial: Vec<f32> = Vec::new();

    for _ in 0..trials {
        let r = engine.run_turn("冷启动你们怎么解决的？").map_err(|e| e.to_string())?;
        for (i, st) in Stage::ALL.iter().enumerate() {
            if let Some(s) = r.span(*st) {
                per_stage[i].push(s.took_ms());
            }
        }
        ttfa.push(r.ttfa_ms());
        serial.push(r.ttfa_serial_ms());
    }

    println!("{:<12} {:>8} {:>8}", "阶段", "P50", "P90");
    for (i, st) in Stage::ALL.iter().enumerate() {
        let p50 = pct(&mut per_stage[i], 0.50);
        let p90 = pct(&mut per_stage[i], 0.90);
        println!("{:<12} {:>8.0} {:>8.0}", st.label(), p50, p90);
    }
    println!();

    let ttfa_p50 = pct(&mut ttfa, 0.50);
    let ttfa_p90 = pct(&mut ttfa, 0.90);
    println!(
        "{:<12} {:>8.0} {:>8.0}  （产品口径：首句就绪 + TTS 首包）",
        "TTFA", ttfa_p50, ttfa_p90
    );

    let serial_p50 = pct(&mut serial, 0.50);
    let serial_p90 = pct(&mut serial, 0.90);
    println!(
        "{:<12} {:>8.0} {:>8.0}  （同步串行上界）",
        "TTFA serial", serial_p50, serial_p90
    );
    Ok(())
}

fn cmd_budget(args: &[String]) -> Result<(), String> {
    let trials: usize = flag_value(args, "--trials")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let seed: u64 = flag_value(args, "--seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(42);

    let budget = Budget::prd_default();
    let r = budget.simulate(trials, seed);

    println!("延迟预算模型（PRD §6），{trials} 次模拟，seed {seed}\n");
    println!("{:<12} {:>8} {:>8} {:>8}", "阶段", "min", "mode", "max");
    for s in &budget.stages {
        println!(
            "{:<12} {:>8.0} {:>8.0} {:>8.0}",
            s.stage.label(),
            s.min_ms,
            s.mode_ms,
            s.max_ms
        );
    }

    println!("\n合计：P50 {:.0} ms / P90 {:.0} ms / P99 {:.0} ms", r.p50, r.p90, r.p99);
    println!(
        "落在 1500ms 内：{:.1}%    落在 2500ms 内：{:.1}%",
        r.under_1500, r.under_2500
    );

    println!("\n各阶段平均耗时：");
    let mut means = r.stage_mean.clone();
    means.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (st, mean) in &means {
        println!("  {:<12} {:>7.0} ms", st.label(), mean);
    }

    println!("\n敏感性（把某一阶段整体放大后的 P90）：");
    for st in Stage::ALL {
        let x15 = budget.scaled(st, 1.5).simulate(trials / 4, seed);
        let x20 = budget.scaled(st, 2.0).simulate(trials / 4, seed);
        println!(
            "  {:<12} 基线 {:>5.0} → ×1.5 {:>5.0} → ×2.0 {:>5.0}",
            st.label(),
            r.p90,
            x15.p90,
            x20.p90
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- 工具

fn print_help() {
    println!(
        "sotto —— 面试实时助手 M0 验证工具

用法：
  sotto scan    --root <笔记目录> [--include 子目录]...
      扫描作用域，看会索引哪些文件、跳过了什么

  sotto chunk   --root <笔记目录> [--include 子目录]...
      切分并统计 chunk（问答对是否被正确识别）

  sotto ask     --root <笔记目录> [--k 5] <问题>
      跑一次检索，看命中什么

  sotto prompt  --root <笔记目录> [--k 5] <问题>
      打印真实材料拼出来的 prompt（system + user），用于核对注入质量

  sotto chat    --root <笔记目录> [--include 子目录]... [--k 3]
      交互式检索（Dock 图标启动的就是它）

  sotto verify  --root <笔记目录> --file 问题.txt [--k 3]
      批量验证召回率。每行：问题|||关键词|||期望路径片段

  sotto latency [--trials 5] [--vad 0] [--asr 150] [--retrieve 100] [--ttft 400] [--tts 300]
      跑 mock 管线，验证打点与阶段耗时

  sotto budget  [--trials 20000] [--seed 42]
      延迟预算 Monte Carlo，看哪一步最可能崩

示例：
  sotto chunk  --root ~/Desktop/Note --include 项目经历 --include 面试准备 --include 学习
  sotto verify --root ~/Desktop/Note --include 项目经历 --include 面试准备 --file examples/questions.txt"
    );
}

fn scan_cfg(args: &[String]) -> ScanConfig {
    let root: PathBuf = flag_value(args, "--root")
        .unwrap_or_else(|| ".".to_string())
        .into();
    let mut cfg = ScanConfig::new(&root);
    for inc in flag_all(args, "--include") {
        cfg = cfg.include(inc);
    }
    cfg
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == name {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
        i += 1;
    }
    None
}

fn flag_all(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == name {
            if let Some(v) = args.get(i + 1) {
                out.push(v.clone());
            }
            i += 2;
            continue;
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            out.push(v.to_string());
        }
        i += 1;
    }
    out
}

fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            if VALUE_FLAGS.contains(&a.as_str()) {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(a.clone());
        i += 1;
    }
    out
}

fn top_dir(rel: &str) -> String {
    match rel.split_once('/') {
        Some((d, _)) => d.to_string(),
        None => "（根目录）".to_string(),
    }
}

fn pct(v: &mut Vec<f32>, q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((v.len() - 1) as f32) * q).round() as usize;
    v[idx.min(v.len() - 1)]
}
