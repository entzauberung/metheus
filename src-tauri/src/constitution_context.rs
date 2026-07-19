// constitution_context.rs — 构建注入到 AI 生成提示词中的"已有信息"上下文
use crate::project;

/// 构建注入到 AI 生成提示词中的"已有信息"上下文。
///
/// 按优先级排列：
/// 1. 工作宪法摘要（从 CONSTITUTION.md 提取，权重最高）
/// 2. 已批准方案（version_plan，权重高）
/// 3. 当前讨论摘要（最后 5 条消息）
/// 4. Already 宪法摘要（从 ALREADY_CONSTITUTION.md 提取，低权重）
///
/// 总长度限制为 ~3000 字符，多字节安全截断。
pub(crate) fn build_context_injection(proj: &project::Project) -> String {
    let mut parts: Vec<String> = Vec::new();
    let max_len: usize = 3000;

    // 1. 工作宪法摘要（最高优先级）
    if !proj.project_path.is_empty() {
        let constitution_path =
            std::path::Path::new(&proj.project_path).join("CONSTITUTION.md");
        if constitution_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&constitution_path) {
                if let Some(part1_start) = content.find("## 第 1 部分") {
                    let part1 = &content[part1_start..];
                    let part1_short: String = part1.chars().take(800).collect();
                    parts.push(format!(
                        "## 工作宪法（最高优先级）\n{}\n（截取自 CONSTITUTION.md 第 1 部分）",
                        part1_short
                    ));
                }
            }
        }
    }

    // 2. 已批准方案
    if !proj.version_plan.is_empty() {
        let plan_short: String = proj.version_plan.chars().take(600).collect();
        parts.push(format!(
            "## 已批准项目方案\n{}\n（截取自 version_plan）",
            plan_short
        ));
    }

    // 3. 当前讨论摘要（最后 5 条消息）
    if let Some(thread) = proj.discussion_threads.first() {
        let recent_msgs: Vec<&str> = thread
            .messages
            .iter()
            .rev()
            .take(5)
            .map(|m| m.content.as_str())
            .collect();
        if !recent_msgs.is_empty() {
            let mut discussion_text = String::new();
            for msg in recent_msgs.iter().rev() {
                let short: String = msg.chars().take(200).collect();
                discussion_text.push_str(&short);
                discussion_text.push('\n');
            }
            parts.push(format!("## 最近讨论摘要\n{}", discussion_text));
        }
    }

    // 4. Already 宪法摘要（最低权重）
    if let Some(ref baseline) = proj.existing_baseline {
        if !baseline.already_constitution_summary.is_empty() {
            parts.push(format!(
                "## 低权重背景参考（Already 项目）\n\
                 > 以下内容权重低于工作宪法和当前讨论，仅作背景了解。\n\
                 > 如有冲突，以工作宪法和当前讨论为准。\n\n{}",
                baseline.already_constitution_summary
            ));
        }
        if !baseline.tech_stack.is_empty() {
            parts.push(format!("## 已有项目技术栈\n{}", baseline.tech_stack));
        }
    }

    // Combine all parts, apply total length limit
    let combined = parts.join("\n\n---\n\n");
    let result: String = combined.chars().take(max_len).collect();

    if result.is_empty() {
        return String::new();
    }

    format!(
        "## 已有信息（按优先级排列，高优先级覆盖低优先级）\n\n{}\n\n---\n（已有信息结束）\n",
        result
    )
}
