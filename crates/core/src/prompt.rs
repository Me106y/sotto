use crate::traits::PromptPayload;
use crate::types::HitInfo;

/// 答案字数上限。超过 120 字就是在写小作文，不是在给话。
pub const ANSWER_CHAR_LIMIT: usize = 120;

/// 第一句的口语上限：再长就念不完了。
pub const FIRST_SENTENCE_HINT: usize = 40;

pub fn system_prompt() -> String {
    format!(
        "你是 Sotto，正在替候选人说话。\n\
         \n\
         硬规则：\n\
         1. 只输出中文口语。不要 Markdown、不要编号、不要标题、不要 emoji。\n\
         2. 第一句是结论或立场，15~30 字，必须能直接念出口。\n\
         3. 之后 2~4 句支撑，每句一个具体事实、数字或动作。\n\
         4. 全文不超过 {limit} 字。\n\
         5. 只能用【材料】里的事实与数字。材料里没有的，说「这块我当时没细看」，不许编。\n\
         6. 不要出现「根据材料」「资料显示」「首先/其次」「综上所述」这类书面对接线。\n",
        limit = ANSWER_CHAR_LIMIT
    )
}

pub fn build(question: &str, hits: &[HitInfo]) -> PromptPayload {
    let mut user = String::new();
    user.push_str("【材料】\n");
    if hits.is_empty() {
        user.push_str("（没有命中任何材料）\n");
    } else {
        for (i, h) in hits.iter().enumerate() {
            user.push_str(&format!(
                "[{}/{}] {} · {}\n{}\n\n",
                i + 1,
                hits.len(),
                h.source,
                h.heading,
                h.excerpt.trim()
            ));
        }
    }
    user.push_str(&format!("【面试官的问题】\n{}\n\n【你的回答】", question.trim()));
    PromptPayload {
        system: system_prompt(),
        user,
    }
}

/// 取第一句：到第一个句末标点为止（含标点）。
/// 没有标点就截断到 60 字——宁可短，不要长。
pub fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for ch in trimmed.chars() {
        out.push(ch);
        if matches!(ch, '。' | '！' | '？' | '；' | '!' | '?' | '\n') {
            break;
        }
        if out.chars().count() >= 60 {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> HitInfo {
        HitInfo {
            chunk_id: 0,
            source: "项目经历/SGS/SGS-项目经历.md".into(),
            heading: "SGS / 技术难点".into(),
            score: 0.9,
            excerpt: "冷启动阶段用规则引擎兜了前三周。".into(),
        }
    }

    #[test]
    fn prompt_carries_the_hard_rules() {
        let p = build("冷启动怎么做的？", &[hit()]);
        assert!(p.system.contains("120"));
        assert!(p.system.contains("不许编"));
        assert!(p.system.contains("不要 Markdown"));
        assert!(p.user.contains("项目经历/SGS/SGS-项目经历.md"));
        assert!(p.user.contains("冷启动怎么做的？"));
    }

    #[test]
    fn empty_hits_still_build_a_prompt() {
        let p = build("你怎么看加班？", &[]);
        assert!(p.user.contains("没有命中任何材料"));
    }

    #[test]
    fn first_sentence_stops_at_terminator() {
        assert_eq!(
            first_sentence("我先说结论。后面还有很多细节。"),
            "我先说结论。"
        );
        assert_eq!(first_sentence("没有标点的一句话"), "没有标点的一句话");
    }

    #[test]
    fn first_sentence_truncates_when_too_long() {
        let long = "啊".repeat(200);
        let s = first_sentence(&long);
        assert!(s.chars().count() <= 61, "got {}", s.chars().count());
        assert!(s.ends_with('…'));
    }
}
