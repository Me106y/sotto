//! 本地代码执行（模拟笔试用）。
//!
//! 只做「跑起来 + 把错误定位到行」，不做判题：
//! 没有隐藏用例、没有评分、没有沙箱隔离（就是本机跑）。
//! 这是 PRD F5 明确划的边界——它是练手感的沙盘，不是 LeetCode。

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(3);
const BUILD_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize, Clone)]
pub struct RuntimeInfo {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub version: String,
}

#[derive(Serialize)]
pub struct RunResult {
    /// ok | compile_error | runtime_error | timeout
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: f64,
    pub error_line: Option<u32>,
}

pub fn runtimes() -> Vec<RuntimeInfo> {
    let cpp = match cpp_compiler() {
        Some(bin) => probe("cpp", "C++17", bin, "--version"),
        None => RuntimeInfo {
            id: "cpp".into(),
            label: "C++17".into(),
            available: false,
            version: String::new(),
        },
    };
    vec![
        probe("python", "Python 3", "python3", "--version"),
        cpp,
        probe("go", "Go", "go", "version"),
    ]
}

fn probe(id: &str, label: &str, bin: &str, arg: &str) -> RuntimeInfo {
    match command(bin).arg(arg).output() {
        Ok(o) if o.status.success() => RuntimeInfo {
            id: id.to_string(),
            label: label.to_string(),
            available: true,
            version: first_line(&o.stdout).or_else(|| first_line(&o.stderr)).unwrap_or_default(),
        },
        _ => RuntimeInfo {
            id: id.to_string(),
            label: label.to_string(),
            available: false,
            version: String::new(),
        },
    }
}

/// 找 C++ 编译器。`g++` 不是每个平台都有，`clang++` 次之，`c++` 兜底。
fn cpp_compiler() -> Option<&'static str> {
    ["g++", "clang++", "c++"].into_iter().find(|bin| {
        command(bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// 从 Finder 启动的 GUI 进程拿不到 shell 的 PATH（只有 `/usr/bin:/bin:...`），
/// Homebrew 装的 go 会「明明装了却找不到」。先查 PATH，再补常见目录。
fn resolve_bin(bin: &str) -> Option<String> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(bin);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let p = Path::new(dir).join(bin);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

fn command(bin: &str) -> Command {
    Command::new(resolve_bin(bin).unwrap_or_else(|| bin.to_string()))
}

/// `<bits/stdc++.h>` 只有 GCC 提供，macOS 的 clang 没有。牛客/LeetCode 的 C++ 代码
/// 几乎都从这一行开始，所以在临时目录里放一份等价的标准头，用 `-I .` 把它找出来。
const CPP_SHIM: &str = r#"#pragma once
#include <algorithm>
#include <array>
#include <bitset>
#include <cassert>
#include <cctype>
#include <climits>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <functional>
#include <iomanip>
#include <iostream>
#include <iterator>
#include <limits>
#include <list>
#include <map>
#include <numeric>
#include <queue>
#include <set>
#include <sstream>
#include <stack>
#include <string>
#include <tuple>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>
"#;

fn write_cpp_shim(dir: &Path) -> Result<(), String> {
    let bits = dir.join("bits");
    std::fs::create_dir_all(&bits).map_err(|e| format!("建临时目录失败：{e}"))?;
    std::fs::write(bits.join("stdc++.h"), CPP_SHIM).map_err(|e| format!("写文件失败：{e}"))
}

/// GCC/libstdc++ 自带 `<bits/stdc++.h>`，clang + libc++（macOS）没有。
/// 先问一下编译器，别在本来就有的平台上把系统头遮掉（那会少了 PBDS 之类的扩展）。
fn has_system_bits_header(compiler: &str) -> bool {
    let mut probe = command(compiler);
    probe.arg("-fsyntax-only").arg("-x").arg("c++").arg("-");
    match run_with_timeout(probe, "#include <bits/stdc++.h>\n", Duration::from_secs(5)) {
        Ok(raw) => !raw.timed_out && raw.status == Some(0),
        Err(_) => false,
    }
}

pub fn run(lang: &str, code: &str, stdin: &str) -> Result<RunResult, String> {
    let dir = std::env::temp_dir().join(format!(
        "sotto-run-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;

    let result = std::panic::catch_unwind(|| run_in(lang, code, stdin, &dir))
        .unwrap_or_else(|_| Err("执行时崩了".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run_in(lang: &str, code: &str, stdin: &str, dir: &Path) -> Result<RunResult, String> {
    match lang {
        "python" => {
            write(dir, "main.py", code)?;
            let mut c = command("python3");
            c.arg("main.py").current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        "cpp" => {
            write(dir, "main.cpp", code)?;
            let compiler = cpp_compiler().ok_or("本机没有找到 C++ 编译器")?;
            let mut build = command(compiler);
            build.arg("-O0").arg("-std=c++17");
            if code.contains("bits/stdc++.h") && !has_system_bits_header(compiler) {
                write_cpp_shim(dir)?;
                build.arg("-I").arg(".");
            }
            build
                .arg("-o")
                .arg("main")
                .arg("main.cpp")
                .current_dir(dir);
            let b = run_with_timeout(build, "", BUILD_TIMEOUT)?;
            if b.timed_out || b.status != Some(0) {
                return Ok(compile_failure(lang, b));
            }
            let mut c = command(if cfg!(windows) { "main.exe" } else { "./main" });
            c.current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        "go" => {
            write(dir, "main.go", code)?;
            let mut build = command("go");
            build
                .arg("build")
                .arg("-o")
                .arg("main")
                .arg("main.go")
                .current_dir(dir);
            let b = run_with_timeout(build, "", BUILD_TIMEOUT)?;
            if b.timed_out || b.status != Some(0) {
                return Ok(compile_failure(lang, b));
            }
            let mut c = command(if cfg!(windows) { "main.exe" } else { "./main" });
            c.current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        other => Err(format!("不支持的语言：{other}")),
    }
}

fn write(dir: &Path, name: &str, code: &str) -> Result<(), String> {
    std::fs::write(dir.join(name), code).map_err(|e| format!("写文件失败：{e}"))
}

fn compile_failure(lang: &str, raw: Raw) -> RunResult {
    RunResult {
        status: "compile_error".into(),
        error_line: parse_error_line(lang, &raw.stderr),
        stdout: raw.stdout,
        stderr: raw.stderr,
        exit_code: raw.status,
        duration_ms: raw.ms,
    }
}

fn finish(lang: &str, raw: Raw) -> Result<RunResult, String> {
    let status = if raw.timed_out {
        "timeout"
    } else if raw.status == Some(0) {
        "ok"
    } else {
        "runtime_error"
    };
    let error_line = if status == "ok" {
        None
    } else {
        parse_error_line(lang, &raw.stderr)
    };
    Ok(RunResult {
        status: status.into(),
        stdout: raw.stdout,
        stderr: raw.stderr,
        exit_code: raw.status,
        duration_ms: raw.ms,
        error_line,
    })
}

struct Raw {
    status: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    ms: f64,
}

fn run_with_timeout(mut cmd: Command, stdin_data: &str, timeout: Duration) -> Result<Raw, String> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("启动失败：{e}"))?;

    if let Some(mut si) = child.stdin.take() {
        let data = stdin_data.as_bytes().to_vec();
        // 用户代码可能根本不读 stdin，写不进去就算了
        std::thread::spawn(move || {
            let _ = si.write_all(&data);
        });
    }

    let mut so = child.stdout.take().ok_or("拿不到 stdout")?;
    let mut se = child.stderr.take().ok_or("拿不到 stderr")?;
    let h_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = so.read_to_end(&mut buf);
        buf
    });
    let h_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = se.read_to_end(&mut buf);
        buf
    });

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };

    let out = h_out.join().unwrap_or_default();
    let err = h_err.join().unwrap_or_default();

    Ok(Raw {
        status: status.and_then(|s| s.code()),
        stdout: String::from_utf8_lossy(&out).to_string(),
        stderr: String::from_utf8_lossy(&err).to_string(),
        timed_out,
        ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

/// 从编译/运行错误里抠出行号。三门语言各有各的格式，不引正则。
fn parse_error_line(lang: &str, stderr: &str) -> Option<u32> {
    let marker = match lang {
        "python" => "line ",
        "cpp" => ".cpp:",
        "go" => ".go:",
        _ => return None,
    };

    for line in stderr.lines() {
        if let Some(pos) = line.find(marker) {
            let rest = &line[pos + marker.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

fn first_line(bytes: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(bytes);
    s.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_traceback_line() {
        let e = "Traceback (most recent call last):\n  File \"/tmp/x/main.py\", line 7, in <module>\n    boom()\nZeroDivisionError: division by zero";
        assert_eq!(parse_error_line("python", e), Some(7));
    }

    #[test]
    fn parses_go_compile_error() {
        let e = "# command-line-arguments\n./main.go:3:5: declared and not used: x";
        assert_eq!(parse_error_line("go", e), Some(3));
    }

    #[test]
    fn parses_cpp_error() {
        assert_eq!(parse_error_line("cpp", "main.cpp:12:5: error: expected ';'"), Some(12));
    }

    #[test]
    fn no_line_when_clean() {
        assert_eq!(parse_error_line("python", ""), None);
    }
}

// ---------------------------------------------------------------- 静态检查

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub column: Option<u32>,
    pub severity: String,
    pub message: String,
}

/// 不执行代码，只让编译器/解释器做语法检查。
/// 能给的是「编译期」错误——语法、类型、引用；运行时的 NameError、除零仍然只能跑出来。
pub fn check(lang: &str, code: &str) -> Result<Vec<Diagnostic>, String> {
    if code.trim().is_empty() {
        return Ok(Vec::new());
    }
    let dir = std::env::temp_dir().join(format!(
        "sotto-check-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;

    let res = check_in(lang, code, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    res
}

const PY_CHECK: &str = r#"
import sys, json
src = open(sys.argv[1], encoding='utf-8').read()
try:
    compile(src, 'main.py', 'exec')
    print('[]')
except SyntaxError as e:
    print(json.dumps([{'line': e.lineno or 1, 'column': e.offset, 'severity': 'error', 'message': e.msg}]))
except Exception as e:
    print(json.dumps([{'line': 1, 'column': None, 'severity': 'error', 'message': str(e)}]))
"#;

fn check_in(lang: &str, code: &str, dir: &Path) -> Result<Vec<Diagnostic>, String> {
    match lang {
        "python" => {
            write(dir, "main.py", code)?;
            let mut c = command("python3");
            c.arg("-c").arg(PY_CHECK).arg("main.py").current_dir(dir);
            let raw = run_with_timeout(c, "", Duration::from_secs(6))?;
            let text = raw.stdout.trim();
            if text.is_empty() {
                return Ok(Vec::new());
            }
            let parsed: Vec<Diagnostic> =
                serde_json::from_str(text).map_err(|e| format!("解析检查结果失败：{e}"))?;
            Ok(parsed)
        }
        "cpp" => {
            write(dir, "main.cpp", code)?;
            let compiler = cpp_compiler().ok_or("本机没有找到 C++ 编译器")?;
            let mut c = command(compiler);
            c.arg("-fsyntax-only").arg("-std=c++17");
            if code.contains("bits/stdc++.h") && !has_system_bits_header(compiler) {
                write_cpp_shim(dir)?;
                c.arg("-I").arg(".");
            }
            c.arg("main.cpp").current_dir(dir);
            let raw = run_with_timeout(c, "", BUILD_TIMEOUT)?;
            Ok(parse_colon_errors(&raw.stderr, ".cpp"))
        }
        "go" => {
            write(dir, "main.go", code)?;
            let mut c = command("go");
            c.arg("build")
                .arg("-o")
                .arg("main")
                .arg("main.go")
                .current_dir(dir);
            let raw = run_with_timeout(c, "", BUILD_TIMEOUT)?;
            Ok(parse_go_errors(&raw.stderr))
        }
        other => Err(format!("不支持的语言：{other}")),
    }
}

/// `main.cpp:12:5: error: expected ';'`
fn parse_colon_errors(stderr: &str, ext: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let Some(pos) = line.find(ext) else { continue };
        let rest = &line[pos + ext.len()..];
        let rest = rest.strip_prefix(':').unwrap_or(rest);

        let Some((line_no, rest)) = leading_number_plain(rest) else {
            continue;
        };
        let rest = rest.strip_prefix(':').unwrap_or(rest);

        let (col, rest) = match leading_number_plain(rest) {
            Some((c, r)) => (Some(c), r),
            None => (None, rest),
        };
        let rest = rest.strip_prefix(':').unwrap_or(rest);

        let Some(epos) = rest.find("error:") else { continue };
        let msg = rest[epos + 6..].trim();
        if msg.is_empty() {
            continue;
        }
        out.push(Diagnostic {
            line: line_no,
            column: col,
            severity: "error".into(),
            message: msg.to_string(),
        });
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// Go 的编译错误：`./main.go:4:1: syntax error: unexpected }`。
fn parse_go_errors(stderr: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let Some(pos) = line.find(".go:") else { continue };
        let rest = &line[pos + 4..];

        let Some((line_no, rest)) = leading_number_plain(rest) else {
            continue;
        };
        let rest = rest.strip_prefix(':').unwrap_or(rest);

        let (col, rest) = match leading_number_plain(rest) {
            Some((c, r)) => (Some(c), r),
            None => (None, rest),
        };
        let rest = rest.strip_prefix(':').unwrap_or(rest).trim();
        if rest.is_empty() {
            continue;
        }
        out.push(Diagnostic {
            line: line_no,
            column: col,
            severity: "error".into(),
            message: rest.to_string(),
        });
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn leading_number_plain(s: &str) -> Option<(u32, &str)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n = digits.parse().ok()?;
    Some((n, &s[digits.len()..]))
}

#[cfg(test)]
mod check_tests {
    use super::*;

    #[test]
    fn parses_cpp_errors() {
        let e = "main.cpp:12:5: error: expected ';' after expression\nmain.cpp:14:1: note: x";
        let d = parse_colon_errors(e, ".cpp");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 12);
        assert_eq!(d[0].column, Some(5));
        assert!(d[0].message.contains("expected"));
    }

    #[test]
    fn parses_go_errors() {
        let e = "# command-line-arguments\n./main.go:4:1: syntax error: unexpected }, expected expression";
        let d = parse_go_errors(e);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 4);
        assert_eq!(d[0].column, Some(1));
        assert!(d[0].message.contains("syntax error"));
    }
}
