// constitution_context.rs — 构建注入到 AI 生成提示词中的"已有信息"上下文
use crate::project;

/// 构建注入到 AI 生成提示词中的"已有信息"上下文。
///
/// 按优先级排列：
/// 1. 工作宪法摘要（从 CONSTITUTION.md 提取，权重最高）
/// 2. 已批准方案（version_plan，权重高）
/// 3. 当前讨论摘要（最后 5 条消息）
/// 4. 项目来源与基线信息（Half Project / NoProject）
/// 5. Already 宪法摘要（从 ALREADY_CONSTITUTION.md 提取，低权重）
///
/// 总长度限制为 ~3000 字符，多字节安全截断。
///
/// 此函数是聊天和生成上下文的统一来源。
/// chat_with_role 也复用此函数获取项目级事实，再追加完整消息历史。
pub(crate) fn build_context_injection(proj: &project::Project) -> String {
    let mut parts: Vec<String> = Vec::new();
    let max_len: usize = 3000;

    // 1. 工作宪法摘要（最高优先级）
    if !proj.project_path.is_empty() {
        let constitution_path = std::path::Path::new(&proj.project_path).join("CONSTITUTION.md");
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

    // 4. 项目来源与基线信息（统一聊天与生成的事实来源）
    parts.push(format!(
        "## 项目来源\n{}",
        match proj.entry_kind {
            project::ProjectEntryKind::NoProject => "从零开始新项目",
            project::ProjectEntryKind::HalfProject => "改造已有项目（Half Project）",
        }
    ));

    if let Some(ref baseline) = proj.existing_baseline {
        if baseline.approved {
            // 基线已批准 — 注入完整基线信息
            if !baseline.tech_stack.is_empty() {
                parts.push(format!("## 已有项目技术栈\n{}", baseline.tech_stack));
            }
            if !baseline.completed_capabilities.is_empty() {
                parts.push(format!(
                    "## 已完成能力\n{}",
                    baseline.completed_capabilities.join(", ")
                ));
            }
            if !baseline.pending_capabilities.is_empty() {
                parts.push(format!(
                    "## 待完成能力\n{}",
                    baseline.pending_capabilities.join(", ")
                ));
            }
            if !baseline.risks.is_empty() {
                parts.push(format!("## 已知风险\n{}", baseline.risks.join(", ")));
            }
        } else {
            // 基线存在但尚未批准 — 防御性输出
            parts.push(format!("## 项目路径\n{}", proj.project_path));
            parts.push(
                "## 注意\n已有项目基线尚未批准，请先完成基线分析。当前仅能依赖项目路径和讨论内容。"
                    .to_string(),
            );
        }
    } else if proj.entry_kind == project::ProjectEntryKind::HalfProject {
        // Half Project 但基线尚未生成 — 防御性输出
        parts.push(format!("## 项目路径\n{}", proj.project_path));
        parts.push(
            "## 注意\n这是已有项目模式（Half Project），但项目分析尚未完成。\n\
             当前只能依赖已有路径和当前讨论。后续基线分析完成后会补充完整信息。"
                .to_string(),
        );
    }

    // 5. Already 宪法摘要（最低权重）
    if let Some(ref baseline) = proj.existing_baseline {
        if !baseline.already_constitution_summary.is_empty() {
            parts.push(format!(
                "## 低权重背景参考（Already 项目）\n\
                 > 以下内容权重低于工作宪法和当前讨论，仅作背景了解。\n\
                 > 如有冲突，以工作宪法和当前讨论为准。\n\n{}",
                baseline.already_constitution_summary
            ));
        }
    }

    // 6. Already 宪法全文参考（最低权重，从磁盘文件读取）
    if let Some(ref baseline) = proj.existing_baseline {
        if baseline.approved && !proj.project_path.is_empty() {
            let already_ref =
                crate::constitution::read_already_constitution_reference(&proj.project_path);
            if !already_ref.is_empty() {
                // Take a shorter excerpt since this is lower priority
                let shortened: String = already_ref.chars().take(800).collect();
                parts.push(shortened);
            }
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
