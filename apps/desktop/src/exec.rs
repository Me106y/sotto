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
    let probes: [(&str, &str, &str, &str); 4] = [
        ("python", "Python 3", "python3", "--version"),
        ("javascript", "JavaScript", "node", "--version"),
        ("java", "Java", "javac", "-version"),
        ("cpp", "C++17", "g++", "--version"),
    ];
    probes
        .iter()
        .map(|(id, label, bin, arg)| {
            let out = Command::new(bin).arg(arg).output();
            match out {
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
        })
        .collect()
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
            let mut c = Command::new("python3");
            c.arg("main.py").current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        "javascript" => {
            write(dir, "main.js", code)?;
            let mut c = Command::new("node");
            c.arg("main.js").current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        "java" => {
            write(dir, "Main.java", code)?;
            let mut build = Command::new("javac");
            build.arg("-encoding").arg("UTF-8").arg("Main.java").current_dir(dir);
            let b = run_with_timeout(build, "", BUILD_TIMEOUT)?;
            if b.timed_out || b.status != Some(0) {
                return Ok(compile_failure(lang, b));
            }
            let mut c = Command::new("java");
            c.arg("-cp").arg(".").arg("Main").current_dir(dir);
            finish(lang, run_with_timeout(c, stdin, RUN_TIMEOUT)?)
        }
        "cpp" => {
            write(dir, "main.cpp", code)?;
            let mut build = Command::new("g++");
            build
                .arg("-O0")
                .arg("-std=c++17")
                .arg("-o")
                .arg("main")
                .arg("main.cpp")
                .current_dir(dir);
            let b = run_with_timeout(build, "", BUILD_TIMEOUT)?;
            if b.timed_out || b.status != Some(0) {
                return Ok(compile_failure(lang, b));
            }
            let mut c = Command::new(if cfg!(windows) { "main.exe" } else { "./main" });
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

/// 从编译/运行错误里抠出行号。四门语言各有各的格式，不引正则。
fn parse_error_line(lang: &str, stderr: &str) -> Option<u32> {
    let marker = match lang {
        "python" => "line ",
        "java" => ".java:",
        "cpp" => ".cpp:",
        "javascript" => ".js:",
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
    fn parses_java_compile_error() {
        let e = "Main.java:3: error: ';' expected";
        assert_eq!(parse_error_line("java", e), Some(3));
    }

    #[test]
    fn parses_cpp_and_js() {
        assert_eq!(parse_error_line("cpp", "main.cpp:12:5: error: expected ';'"), Some(12));
        assert_eq!(
            parse_error_line("javascript", "ReferenceError: x is not defined\n    at /tmp/a/main.js:4:1"),
            Some(4)
        );
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
            let mut c = Command::new("python3");
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
        "javascript" => {
            write(dir, "main.js", code)?;
            let mut c = Command::new("node");
            c.arg("--check").arg("main.js").current_dir(dir);
            let raw = run_with_timeout(c, "", Duration::from_secs(6))?;
            Ok(parse_js_check(&raw.stderr))
        }
        "java" => {
            write(dir, "Main.java", code)?;
            let mut c = Command::new("javac");
            c.arg("-encoding").arg("UTF-8").arg("Main.java").current_dir(dir);
            let raw = run_with_timeout(c, "", BUILD_TIMEOUT)?;
            Ok(parse_colon_errors(&raw.stderr, ".java"))
        }
        "cpp" => {
            write(dir, "main.cpp", code)?;
            let mut c = Command::new("g++");
            c.arg("-fsyntax-only")
                .arg("-std=c++17")
                .arg("main.cpp")
                .current_dir(dir);
            let raw = run_with_timeout(c, "", BUILD_TIMEOUT)?;
            Ok(parse_colon_errors(&raw.stderr, ".cpp"))
        }
        other => Err(format!("不支持的语言：{other}")),
    }
}

/// `main.cpp:12:5: error: expected ';'` / `Main.java:3: error: ';' expected`
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

/// node --check 的格式和 C 系不一样：行号在上一行，消息在最后。
fn parse_js_check(stderr: &str) -> Vec<Diagnostic> {
    let mut line_no = None;
    let mut message = String::new();
    for line in stderr.lines() {
        let t = line.trim();
        if line_no.is_none() {
            if let Some((n, _)) = leading_number_after(&t, ".js:") {
                line_no = Some(n);
            }
        }
        if t.contains("Error:") && message.is_empty() {
            message = t.to_string();
        }
    }
    match (line_no, message.is_empty()) {
        (Some(n), false) => vec![Diagnostic {
            line: n,
            column: None,
            severity: "error".into(),
            message,
        }],
        (Some(n), true) => vec![Diagnostic {
            line: n,
            column: None,
            severity: "error".into(),
            message: "语法错误".into(),
        }],
        _ => Vec::new(),
    }
}

fn leading_number_after<'a>(s: &'a str, marker: &str) -> Option<(u32, &'a str)> {
    let pos = s.find(marker)?;
    leading_number_plain(&s[pos + marker.len()..])
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
    fn parses_java_errors() {
        let e = "Main.java:3: error: ';' expected";
        let d = parse_colon_errors(e, ".java");
        assert_eq!(d[0].line, 3);
        assert_eq!(d[0].column, None);
    }

    #[test]
    fn parses_node_check() {
        let e = "/tmp/x/main.js:4\n  console.log(a b);\n              ^\n\nSyntaxError: Unexpected identifier 'b'";
        let d = parse_js_check(e);
        assert_eq!(d[0].line, 4);
        assert!(d[0].message.contains("Unexpected identifier"));
    }
}
