// src-tauri/src/commands/checks.rs — 三项显式检查
use crate::project;

/// 三项检查的固定执行顺序（不可跳过或乱序）
const CHECK_ORDER: [&str; 3] = [
    "goal_completeness",
    "reality_consistency",
    "task_executability",
];

/// 检查项对应的中文标签
const CHECK_LABELS: [(&str, &str); 3] = [
    ("goal_completeness", "目标完整性检查"),
    ("reality_consistency", "现实一致性检查"),
    ("task_executability", "任务可执行性检查"),
];

/// 获取检查顺序
#[allow(dead_code)]
pub(crate) fn check_order() -> &'static [&'static str; 3] {
    &CHECK_ORDER
}

/// 运行三项检查中的一项。
///
/// 检查必须按顺序执行（目标完整性 → 现实一致性 → 任务可执行性），
/// 前一项未通过或已过期时不得执行后一项。
/// 前端传入其看到的讨论修订号和项目数据修订号，AI 返回后进行乐观并发校验。
/// 检查结果持久化到 Project.preflight_results，返回更新后的完整 Project。
#[tauri::command]
pub(crate) async fn run_preflight_check(
    project_name: String,
    check_type: String,
    _frontend_discussion_revision: u64,
    _frontend_data_revision: u64,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    // === 0. 校验工作流步骤 ===
    if proj.workflow_state.current_step != project::WorkflowStep::ThreeChecks {
        return Err(format!(
            "当前工作流步骤为 {:?}，只有 ThreeChecks 步骤可以运行检查",
            proj.workflow_state.current_step
        ));
    }

    // === 1. 校验检查类型 ===
    let check_idx = CHECK_ORDER
        .iter()
        .position(|c| *c == check_type)
        .ok_or_else(|| format!("未知的检查类型：{}", check_type))?;

    // === 2. 强制执行检查顺序：前一项必须有效（已通过、未过期、讨论修订号匹配） ===
    for i in 0..check_idx {
        let prev_type = CHECK_ORDER[i];
        let prev_valid = proj
            .preflight_results
            .iter()
            .any(|r| {
                r.check_type == prev_type
                    && r.passed
                    && !r.stale
                    && r.discussion_revision == proj.discussion_revision
            });
        if !prev_valid {
            let prev_label = CHECK_LABELS
                .iter()
                .find(|(t, _)| *t == prev_type)
                .map(|(_, l)| *l)
                .unwrap_or(prev_type);
            let curr_label = CHECK_LABELS
                .iter()
                .find(|(t, _)| *t == check_type)
                .map(|(_, l)| *l)
                .unwrap_or(&check_type);
            return Err(format!(
                "必须先通过「{}」检查（且未过期、讨论未变化）才能进行「{}」检查",
                prev_label, curr_label
            ));
        }
    }

    // === 3. 保存调用 AI 前的事实快照（用于 AI 返回后的并发校验） ===
    let snapshot_step = proj.workflow_state.current_step.clone();
    let snapshot_discussion_revision = proj.discussion_revision;
    let snapshot_data_revision = proj.workflow_state.data_revision;

    // === 4. 构建 AI 上下文 ===
    let discussion_messages = proj
        .discussion_threads
        .first()
        .map(|t| t.messages.clone())
        .unwrap_or_default();

    // Already 宪法低权重参考（仅 Half Project 且基线已批准时注入）
    let already_ref = if proj.entry_kind == project::ProjectEntryKind::HalfProject
        && proj.existing_baseline.as_ref().map(|b| b.approved).unwrap_or(false)
    {
        crate::constitution::read_already_constitution_reference(&proj.project_path)
    } else {
        String::new()
    };

    let context = format!(
        "项目名称：{}\n项目来源：{}\n项目路径：{}\n技术栈：{}\n讨论修订号：{}\n\n讨论历史：\n{}\n\n{}{}",
        proj.name,
        match proj.entry_kind {
            project::ProjectEntryKind::NoProject => "从零开始",
            project::ProjectEntryKind::HalfProject => "改造已有项目",
        },
        proj.project_path,
        proj.existing_baseline
            .as_ref()
            .map(|b| b.tech_stack.as_str())
            .unwrap_or("未检测"),
        proj.discussion_revision,
        discussion_messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n"),
        if let Some(ref baseline) = proj.existing_baseline {
            format!(
                "已有项目基线：\n已完成能力：{}\n待处理能力：{}\n风险：{}\n不确定项：{}",
                baseline.completed_capabilities.join("、"),
                baseline.pending_capabilities.join("、"),
                baseline.risks.join("、"),
                baseline.uncertainties.join("、"),
            )
        } else {
            "无已有项目基线（No Project）".to_string()
        },
        if already_ref.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", already_ref)
        }
    );

    let prompt = match check_type.as_str() {
        "goal_completeness" => crate::prompts::GOAL_COMPLETENESS_CHECK_PROMPT,
        "reality_consistency" => crate::prompts::REALITY_CONSISTENCY_CHECK_PROMPT,
        "task_executability" => crate::prompts::TASK_EXECUTABILITY_CHECK_PROMPT,
        _ => return Err(format!("未知的检查类型：{}", check_type)),
    };

    // === 5. 调用 AI（失败不按通过处理） ===
    let result_str = match crate::api::call_deepseek_api_json(prompt, &context).await {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("三项检查 AI 调用失败（{}）：{}", check_type, e));
        }
    };

    let result: serde_json::Value = serde_json::from_str(&result_str)
        .map_err(|e| format!("解析检查结果 JSON 失败（{}）：{}", check_type, e))?;

    // === 6. 提取字段（不使用 unwrap_or — 缺失字段按错误处理） ===
    let passed = result["passed"]
        .as_bool()
        .ok_or_else(|| {
            format!(
                "检查结果缺少 'passed' 字段（{}），原始响应：{}",
                check_type,
                &result_str[..result_str.len().min(300)]
            )
        })?;

    let summary = result["summary"]
        .as_str()
        .unwrap_or("（AI 未提供摘要）")
        .to_string();

    let issues: Vec<String> = result["issues"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let suggestions: Vec<String> = result["suggestions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // === 7. 重新加载 Project，防止竞态（检查期间用户可能发送了新消息） ===
    let current_proj = crate::load_project(&project_name)?;

    // 验证工作流步骤未变化
    if current_proj.workflow_state.current_step != snapshot_step {
        return Err("当前项目已不在三项检查步骤，请刷新页面。".to_string());
    }

    // 验证讨论修订号未变化
    if current_proj.discussion_revision != snapshot_discussion_revision {
        return Err(
            "讨论已变化（可能在检查期间发送了新消息），请重新开始检查。".to_string(),
        );
    }

    // 验证项目数据修订号未变化（防止并发写入）
    if current_proj.workflow_state.data_revision != snapshot_data_revision {
        return Err(
            "项目数据已变化（可能在检查期间发生了其他操作），请刷新页面后重新检查。".to_string(),
        );
    }

    // === 8. 构造并持久化检查结果 ===
    let check_result = project::PreflightCheckResult {
        check_type: check_type.clone(),
        passed,
        summary,
        issues,
        suggestions,
        discussion_revision: current_proj.discussion_revision,
        checked_at: chrono::Utc::now().to_rfc3339(),
        stale: false,
        expired_at: None,
    };

    // 覆盖同类型旧结果，使用重新加载后的项目数据
    let mut proj = current_proj;
    proj.preflight_results.retain(|r| r.check_type != check_type);
    proj.preflight_results.push(check_result);
    proj.workflow_state.data_revision += 1;

    crate::save_and_reload_project(&proj)
}
