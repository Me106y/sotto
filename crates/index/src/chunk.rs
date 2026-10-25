//! 切分。PRD §F4.4：
//! - 有 `## Qn` 的文件，一个问答对就是一个 chunk（天然对齐面试）。
//! - 其余按 1~2 级标题切，chunk ≤ 400 字，保留标题路径。
//! - 短版「XX-项目经历.md」整篇不切——它本来就是口播稿。

/// 单个 chunk 的字数上限
pub const MAX_CHUNK_CHARS: usize = 400;
/// 少于这个字数的段落会被并进前一块（除非前一块是问答）
pub const MIN_CHUNK_CHARS: usize = 40;
/// 短于这个字节数的「项目经历」文件整篇保留
pub const WHOLE_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    /// 一个问答对
    Qa,
    /// 一个普通小节
    Section,
    /// 整篇保留的短文件
    WholeFile,
}

impl ChunkKind {
    pub fn label(self) -> &'static str {
        match self {
            ChunkKind::Qa => "问答",
            ChunkKind::Section => "小节",
            ChunkKind::WholeFile => "整篇",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub source: String,
    /// 标题路径，例如 "SGS / 技术难点 / 冷启动"
    pub heading: String,
    pub kind: ChunkKind,
    pub text: String,
    pub chars: usize,
}

impl Chunk {
    /// 给检索器用的一行摘要
    pub fn excerpt(&self, max_chars: usize) -> String {
        let flat: String = self
            .text
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        let t = flat.trim();
        if t.chars().count() <= max_chars {
            t.to_string()
        } else {
            let mut s: String = t.chars().take(max_chars).collect();
            s.push('…');
            s
        }
    }
}

pub fn chunk_file(rel: &str, content: &str, bytes: u64) -> Vec<Chunk> {
    if bytes <= WHOLE_FILE_BYTES && rel.ends_with("项目经历.md") {
        let text = content.trim();
        if text.is_empty() {
            return Vec::new();
        }
        return vec![Chunk {
            source: rel.to_string(),
            heading: first_heading(content).unwrap_or_else(|| stem(rel)),
            kind: ChunkKind::WholeFile,
            chars: text.chars().count(),
            text: text.to_string(),
        }];
    }

    let mut chunks: Vec<Chunk> = Vec::new();
    for sec in split_sections(content) {
        push_section(&mut chunks, rel, sec);
    }
    merge_tiny(chunks)
}

#[derive(Debug)]
struct Section {
    /// 标题路径，最后一段就是本节标题
    path: Vec<String>,
    is_qa: bool,
    body: String,
}

fn split_sections(content: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut h1: Option<String> = None;
    let mut cur: Option<Section> = None;
    let mut preamble = String::new();

    for line in content.lines() {
        let line = line.trim_end();
        if let Some((level, title)) = heading_of(line) {
            if let Some(s) = cur.take() {
                sections.push(s);
            }
            if level == 1 {
                h1 = Some(title.clone());
            }
            let path = match (level, &h1) {
                (1, _) => vec![title.clone()],
                (_, Some(h)) if h != &title => vec![h.clone(), title.clone()],
                _ => vec![title.clone()],
            };
            cur = Some(Section {
                path,
                is_qa: title.trim_start().starts_with('Q'),
                body: String::new(),
            });
        } else if let Some(s) = cur.as_mut() {
            s.body.push_str(line);
            s.body.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    if let Some(s) = cur.take() {
        sections.push(s);
    }

    // 文首内容：整篇没有标题时必须保留（否则一篇短笔记会凭空消失），
    // 有标题时只有够长才值得单独成块。
    let preamble_len = preamble.trim().chars().count();
    if preamble_len > 0 && (sections.is_empty() || preamble_len >= MIN_CHUNK_CHARS) {
        sections.insert(
            0,
            Section {
                path: vec!["（文首）".to_string()],
                is_qa: false,
                body: preamble,
            },
        );
    }

    sections
}

/// 只认 1~2 级标题。`### 追问` 这类要留在正文里，
/// 否则会把一个问答对切碎，丢掉「问」的上下文。
fn heading_of(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=2).contains(&level) {
        return None;
    }
    let rest = line[level..].trim();
    if rest.is_empty() {
        return None;
    }
    Some((level, rest.to_string()))
}

fn push_section(out: &mut Vec<Chunk>, rel: &str, sec: Section) {
    let body = sec.body.trim();
    if body.is_empty() {
        return;
    }
    let heading = sec.path.join(" / ");
    let kind = if sec.is_qa {
        ChunkKind::Qa
    } else {
        ChunkKind::Section
    };

    if body.chars().count() <= MAX_CHUNK_CHARS {
        out.push(Chunk {
            source: rel.to_string(),
            heading,
            kind,
            chars: body.chars().count(),
            text: body.to_string(),
        });
        return;
    }

    for piece in split_long(body, MAX_CHUNK_CHARS) {
        let t = piece.trim();
        if t.is_empty() {
            continue;
        }
        out.push(Chunk {
            source: rel.to_string(),
            heading: heading.clone(),
            kind,
            chars: t.chars().count(),
            text: t.to_string(),
        });
    }
}

/// 在句末标点处断开，避免把一句话拦腰截断。
fn split_long(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut n = 0usize;
    for ch in text.chars() {
        cur.push(ch);
        n += 1;
        let boundary = matches!(ch, '。' | '！' | '？' | '；' | '\n');
        if n >= max || (boundary && n >= max * 3 / 5) {
            out.push(std::mem::take(&mut cur));
            n = 0;
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// 小块并进前一块；但绝不并进问答块——那会把两个问题揉成一个。
/// 合并后也不能超过上限，否则会养出 1500 字的大杂烩，
/// 检索时整块被拉进来，把真正的信号冲淡。
fn merge_tiny(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    for c in chunks {
        if c.chars < MIN_CHUNK_CHARS {
            if let Some(last) = out.last_mut() {
                if last.source == c.source
                    && last.kind != ChunkKind::Qa
                    && last.chars + c.chars <= MAX_CHUNK_CHARS
                {
                    last.text.push('\n');
                    last.text.push_str(&c.text);
                    last.chars = last.text.chars().count();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

fn first_heading(content: &str) -> Option<String> {
    content.lines().find_map(|l| {
        let t = l.trim_end();
        if t.starts_with("# ") {
            Some(t[2..].trim().to_string())
        } else {
            None
        }
    })
}

fn stem(rel: &str) -> String {
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".md")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qa_pairs_become_one_chunk_each() {
        let md = "# SGS 面试准备\n\n## Q1: 冷启动怎么做\n\n先用规则兜住，等样本够了再切模型。\n\n## Q2: 数据一致性\n\n用对账任务每天跑一遍，卡住就不许发版。\n";
        let chunks = chunk_file("面试准备/q.md", md, md.len() as u64);
        let qa: Vec<&Chunk> = chunks.iter().filter(|c| c.kind == ChunkKind::Qa).collect();
        assert_eq!(qa.len(), 2, "chunks={:?}", chunks);
        assert!(qa[0].heading.contains("Q1"));
        assert!(qa[0].heading.contains("SGS 面试准备"));
        assert_eq!(qa[0].text, "先用规则兜住，等样本够了再切模型。");
    }

    #[test]
    fn h3_stays_inside_the_qa_body() {
        let md = "## Q1: 冷启动\n\n结论先给。\n\n### 追问：具体数字\n\n前三周规则，一万条切模型。\n";
        let chunks = chunk_file("x.md", md, md.len() as u64);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("具体数字"));
        assert!(chunks[0].text.contains("一万条"));
    }

    #[test]
    fn short_project_note_is_kept_whole() {
        let md = "# MirrorView\n\n做了一个求职工作台。\n\n## 难点\n\n多租户隔离。\n";
        let chunks = chunk_file("项目经历/MirrorView/MirrorView-项目经历.md", md, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, ChunkKind::WholeFile);
        assert!(chunks[0].text.contains("多租户隔离"));
    }

    #[test]
    fn long_section_splits_at_sentence_boundaries() {
        let sentence = "这是一句用来撑长度的话。";
        let md = format!("## 长节\n\n{}", sentence.repeat(40));
        let chunks = chunk_file("x.md", &md, md.len() as u64);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.chars <= MAX_CHUNK_CHARS + 20, "too big: {}", c.chars);
            assert!(c.text.ends_with('。'), "cut mid-sentence: {:?}", c.text);
        }
    }

    #[test]
    fn plain_note_without_headings_still_yields_a_chunk() {
        let md = "没有标题的一段笔记，但内容足够长，应该作为一个块被保留下来，不能被丢掉。";
        let chunks = chunk_file("x.md", md, md.len() as u64);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.starts_with("没有标题"));
    }

    #[test]
    fn merging_small_sections_never_exceeds_the_cap() {
        let mut md = String::new();
        for i in 0..40 {
            md.push_str(&format!("## 小{i}\n{}\n\n", "短".repeat(30)));
        }
        let chunks = chunk_file("x.md", &md, md.len() as u64);
        assert!(chunks.len() > 1, "应该合并但不能并成一块");
        for c in &chunks {
            assert!(c.chars <= MAX_CHUNK_CHARS, "{} 字：{}", c.chars, c.heading);
        }
    }
}
