//! 本地代码执行（模拟笔试用）。
//!
//! 只做「跑起来 + 把错误定位到行」，不做判题：
//! 没有隐藏用例、没有评分、没有沙箱隔离（就是本机跑）。
//! 这是 PRD F5 明确划的边界——它是练手感的沙盘，不是 LeetCode。

use serde::Serialize;
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
