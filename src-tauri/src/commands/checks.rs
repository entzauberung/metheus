// src-tauri/src/commands/checks.rs — 三项显式检查
use crate::project;
use serde::Deserialize;

const MAX_SCOPE_SIGNAL_COUNT: u32 = 100;

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

#[derive(Debug, Deserialize)]
struct RawCheckResponse {
    passed: bool,
    summary: String,
    issues: Vec<String>,
    suggestions: Vec<String>,
    #[serde(default)]
    scope_signals: Option<project::WorkloadSignals>,
}

#[derive(Debug)]
struct ParsedCheckResponse {
    passed: bool,
    summary: String,
    issues: Vec<String>,
    suggestions: Vec<String>,
    workload_profile: Option<project::WorkloadProfile>,
}

/// 获取检查顺序
#[allow(dead_code)]
pub(crate) fn check_order() -> &'static [&'static str; 3] {
    &CHECK_ORDER
}

fn validate_scope_signals(signals: &project::WorkloadSignals) -> Result<(), String> {
    for (label, value) in [
        (
            "external_integration_count",
            signals.external_integration_count,
        ),
        ("independent_domain_count", signals.independent_domain_count),
        ("deliverable_count", signals.deliverable_count),
    ] {
        if value > MAX_SCOPE_SIGNAL_COUNT {
            return Err(format!(
                "目标完整性检查的 scope_signals.{label} 超出上限 {MAX_SCOPE_SIGNAL_COUNT}"
            ));
        }
    }
    if signals.independent_domain_count == 0 || signals.deliverable_count == 0 {
        return Err(
            "目标完整性检查的 scope_signals 必须包含至少 1 个独立领域和 1 个交付物".to_string(),
        );
    }
    Ok(())
}

fn parse_check_response(
    check_type: &str,
    result_str: &str,
    baseline: Option<&project::ExistingProjectBaseline>,
    discussion_revision: u64,
) -> Result<ParsedCheckResponse, String> {
    let raw: RawCheckResponse = serde_json::from_str(result_str).map_err(|error| {
        format!(
            "检查结果协议错误（{check_type}）：{error}；要求 passed/summary/issues/suggestions 均为正确类型"
        )
    })?;
    if raw.summary.trim().is_empty() {
        return Err(format!(
            "检查结果协议错误（{check_type}）：summary 不能为空"
        ));
    }

    // issues 只承载硬阻断；建议项不能把检查改为不通过。
    let passed = raw.issues.is_empty();
    let _model_passed = raw.passed;
    let workload_profile = if check_type == "goal_completeness" {
        let signals = raw.scope_signals.ok_or_else(|| {
            "目标完整性检查结果缺少必填 scope_signals，未写入工作负载画像".to_string()
        })?;
        validate_scope_signals(&signals)?;
        if passed {
            Some(crate::workload_policy::classify(
                signals,
                baseline,
                discussion_revision,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(ParsedCheckResponse {
        passed,
        summary: raw.summary,
        issues: raw.issues,
        suggestions: raw.suggestions,
        workload_profile,
    })
}

fn check_depth_context(profile: &project::WorkloadProfile) -> String {
    let rules = match profile.check_depth {
        project::WorkloadCheckDepth::Lean => {
            "Lean：保留三个业务维度，只把目标缺失、现实矛盾、不可交付和必需依赖缺失列为 issues；系统级架构材料缺失只能作为 suggestion。"
        }
        project::WorkloadCheckDepth::Standard => {
            "Standard：保留三个业务维度，检查范围边界、现有事实、可验证交付物、关键依赖和实现顺序；只有影响交付的缺口列为 issues。"
        }
        project::WorkloadCheckDepth::Strict => {
            "Strict：保留三个业务维度，并要求跨端、数据库、权限、外部集成和依赖顺序证据；相关必需事实缺失时列为 issues。"
        }
    };
    format!(
        "后端工作负载画像（不可改写）：\n{}\n检查分级规则：{}\n所有可选优化只能写入 suggestions，不能使 passed=false。",
        crate::workload_policy::render_planning_constraints(profile),
        rules
    )
}

fn ensure_snapshot_is_current(
    project: &project::Project,
    snapshot_step: &project::WorkflowStep,
    snapshot_discussion_revision: u64,
    snapshot_data_revision: u64,
) -> Result<(), String> {
    if project.workflow_state.current_step != *snapshot_step {
        return Err("当前项目已不在三项检查步骤，请刷新页面。".to_string());
    }
    if project.discussion_revision != snapshot_discussion_revision {
        return Err("讨论已变化（可能在检查期间发送了新消息），请重新开始检查。".to_string());
    }
    if project.workflow_state.data_revision != snapshot_data_revision {
        return Err(
            "项目数据已变化（可能在检查期间发生了其他操作），请刷新页面后重新检查。".to_string(),
        );
    }
    Ok(())
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

    if proj.workflow_state.current_step != project::WorkflowStep::ThreeChecks {
        return Err(format!(
            "当前工作流步骤为 {:?}，只有 ThreeChecks 步骤可以运行检查",
            proj.workflow_state.current_step
        ));
    }

    let check_idx = CHECK_ORDER
        .iter()
        .position(|candidate| *candidate == check_type)
        .ok_or_else(|| format!("未知的检查类型：{}", check_type))?;

    for prev_type in CHECK_ORDER.iter().take(check_idx) {
        let prev_valid = proj.preflight_results.iter().any(|result| {
            result.check_type == *prev_type
                && result.passed
                && !result.stale
                && result.discussion_revision == proj.discussion_revision
        });
        if !prev_valid {
            let prev_label = CHECK_LABELS
                .iter()
                .find(|(candidate, _)| candidate == prev_type)
                .map(|(_, label)| *label)
                .unwrap_or(prev_type);
            let curr_label = CHECK_LABELS
                .iter()
                .find(|(candidate, _)| *candidate == check_type)
                .map(|(_, label)| *label)
                .unwrap_or(&check_type);
            return Err(format!(
                "必须先通过「{}」检查（且未过期、讨论未变化）才能进行「{}」检查",
                prev_label, curr_label
            ));
        }
    }

    let workload_context = if check_type == "goal_completeness" {
        "本项必须在同一次响应中返回完整 scope_signals；模型只声明范围事实，规模由后端确定性计算。"
            .to_string()
    } else {
        check_depth_context(crate::workload_policy::current_profile(&proj)?)
    };

    let snapshot_step = proj.workflow_state.current_step.clone();
    let snapshot_discussion_revision = proj.discussion_revision;
    let snapshot_data_revision = proj.workflow_state.data_revision;

    let discussion_messages = proj
        .discussion_threads
        .first()
        .map(|thread| thread.messages.clone())
        .unwrap_or_default();
    let already_ref = if proj.entry_kind == project::ProjectEntryKind::HalfProject
        && proj
            .existing_baseline
            .as_ref()
            .is_some_and(|baseline| baseline.approved)
    {
        crate::constitution::read_already_constitution_reference(&proj.project_path)
    } else {
        String::new()
    };

    let baseline_context = if let Some(ref baseline) = proj.existing_baseline {
        format!(
            "已有项目基线：\n扫描文件数：{}\n证据摘要：{}\n已完成能力：{}\n待处理能力：{}\n风险：{}\n不确定项：{}\n清单文件：{}\n源文件摘要数：{}",
            baseline.scanned_files.len(),
            baseline.evidence_summary,
            baseline.completed_capabilities.join("、"),
            baseline.pending_capabilities.join("、"),
            baseline.risks.join("、"),
            baseline.uncertainties.join("、"),
            baseline
                .manifest_details
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join("、"),
            baseline.source_abstracts.len(),
        )
    } else {
        "无已有项目基线（No Project）".to_string()
    };
    let context = format!(
        "项目名称：{}\n项目来源：{}\n项目路径：{}\n技术栈：{}\n讨论修订号：{}\n\n{}\n\n讨论历史：\n{}\n\n{}{}",
        proj.name,
        match proj.entry_kind {
            project::ProjectEntryKind::NoProject => "从零开始",
            project::ProjectEntryKind::HalfProject => "改造已有项目",
        },
        proj.project_path,
        proj.existing_baseline
            .as_ref()
            .map(|baseline| baseline.tech_stack.as_str())
            .unwrap_or("未检测"),
        proj.discussion_revision,
        workload_context,
        discussion_messages
            .iter()
            .filter(|message| message.role != "system")
            .map(|message| format!("[{}]: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n"),
        baseline_context,
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

    let model_context = crate::cost_ledger::ModelCallContext::for_project(
        &proj,
        crate::cost_ledger::ModelCallPurpose::PreflightCheck,
    );
    let response = crate::api::call_deepseek_api_json_with_context(prompt, &context, model_context)
        .await
        .map_err(|error| format!("三项检查 AI 调用失败（{}）：{}", check_type, error))?;
    let parsed = parse_check_response(
        &check_type,
        &response.content,
        proj.existing_baseline.as_ref(),
        snapshot_discussion_revision,
    )?;

    let current_proj = crate::load_project(&project_name)?;
    ensure_snapshot_is_current(
        &current_proj,
        &snapshot_step,
        snapshot_discussion_revision,
        snapshot_data_revision,
    )?;

    let check_result = project::PreflightCheckResult {
        check_type: check_type.clone(),
        passed: parsed.passed,
        summary: parsed.summary,
        issues: parsed.issues,
        suggestions: parsed.suggestions,
        discussion_revision: current_proj.discussion_revision,
        checked_at: chrono::Utc::now().to_rfc3339(),
        stale: false,
        expired_at: None,
    };

    let mut proj = current_proj;
    if check_type == "goal_completeness" {
        proj.workload_profile = parsed.workload_profile;
    } else {
        crate::workload_policy::current_profile(&proj)?;
    }
    proj.preflight_results
        .retain(|result| result.check_type != check_type);
    proj.preflight_results.push(check_result);
    let all_checks_passed = CHECK_ORDER.iter().all(|required| {
        proj.preflight_results.iter().any(|result| {
            result.check_type == *required
                && result.passed
                && !result.stale
                && result.discussion_revision == proj.discussion_revision
        })
    });
    if all_checks_passed {
        crate::workload_policy::current_profile(&proj)?;
        proj.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    }
    proj.workflow_state.data_revision += 1;

    let saved = crate::save_and_reload_project(&proj)?;
    crate::cost_ledger::mark_call_outcome_best_effort(
        &project_name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_evidence: true,
            produced_fact: true,
            ..Default::default()
        },
    );
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(scope_signals: serde_json::Value) -> String {
        serde_json::json!({
            "passed": true,
            "summary": "范围完整",
            "issues": [],
            "suggestions": ["可选优化"],
            "scope_signals": scope_signals,
        })
        .to_string()
    }

    fn micro_signals() -> serde_json::Value {
        serde_json::json!({
            "has_frontend": true,
            "has_backend": false,
            "has_persistence": false,
            "has_auth_or_roles": false,
            "external_integration_count": 0,
            "independent_domain_count": 1,
            "deliverable_count": 1,
            "high_risk": false,
        })
    }

    #[test]
    fn no_project_goal_check_builds_profile_from_the_existing_call() {
        let parsed =
            parse_check_response("goal_completeness", &response(micro_signals()), None, 3).unwrap();
        let profile = parsed.workload_profile.unwrap();
        assert_eq!(profile.scale, project::WorkloadScale::Micro);
        assert_eq!(profile.discussion_revision, 3);
    }

    #[test]
    fn half_project_large_repository_only_lifts_micro_to_small() {
        let baseline = project::ExistingProjectBaseline {
            scan_complete: true,
            scanned_files: (0..200).map(|index| format!("src/{index}.rs")).collect(),
            ..Default::default()
        };
        let parsed = parse_check_response(
            "goal_completeness",
            &response(micro_signals()),
            Some(&baseline),
            4,
        )
        .unwrap();
        assert_eq!(
            parsed.workload_profile.unwrap().scale,
            project::WorkloadScale::Small
        );
    }

    #[test]
    fn adaptive_execution_contract_invalid_scope_signals_fail_protocol() {
        let missing = serde_json::json!({
            "passed": true,
            "summary": "完整",
            "issues": [],
            "suggestions": [],
        })
        .to_string();
        assert!(parse_check_response("goal_completeness", &missing, None, 1)
            .unwrap_err()
            .contains("scope_signals"));

        let mut invalid = micro_signals();
        invalid["deliverable_count"] = serde_json::json!(101);
        assert!(
            parse_check_response("goal_completeness", &response(invalid), None, 1)
                .unwrap_err()
                .contains("超出上限")
        );
    }

    #[test]
    fn suggestions_never_make_a_check_fail() {
        let parsed =
            parse_check_response("goal_completeness", &response(micro_signals()), None, 1).unwrap();
        assert!(parsed.passed);
        assert_eq!(parsed.suggestions, vec!["可选优化"]);
    }

    #[test]
    fn concurrent_discussion_revision_is_rejected_before_write() {
        let mut project = project::Project::new("concurrent-check");
        project.workflow_state.current_step = project::WorkflowStep::ThreeChecks;
        project.workflow_state.data_revision = 8;
        project.discussion_revision = 6;
        let error = ensure_snapshot_is_current(&project, &project::WorkflowStep::ThreeChecks, 5, 8)
            .unwrap_err();
        assert!(error.contains("讨论已变化"));
        assert!(project.workload_profile.is_none());
    }

    #[test]
    fn adaptive_execution_contract_check_depths_keep_three_dimensions() {
        for (scope, expected) in [
            (
                project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: false,
                    has_persistence: false,
                    has_auth_or_roles: false,
                    external_integration_count: 0,
                    independent_domain_count: 1,
                    deliverable_count: 1,
                    high_risk: false,
                },
                "Lean",
            ),
            (
                project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: false,
                    has_persistence: false,
                    has_auth_or_roles: false,
                    external_integration_count: 0,
                    independent_domain_count: 3,
                    deliverable_count: 4,
                    high_risk: false,
                },
                "Standard",
            ),
            (
                project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: true,
                    has_persistence: true,
                    has_auth_or_roles: true,
                    external_integration_count: 0,
                    independent_domain_count: 3,
                    deliverable_count: 4,
                    high_risk: false,
                },
                "Strict",
            ),
        ] {
            let profile = crate::workload_policy::classify(scope, None, 1).unwrap();
            let context = check_depth_context(&profile);
            assert!(context.contains(expected));
            assert!(context.contains("三个业务维度"));
            assert!(context.contains("suggestions"));
        }
    }

    #[test]
    fn prompt_contract_requires_signals_and_preserves_all_checks() {
        assert!(crate::prompts::GOAL_COMPLETENESS_CHECK_PROMPT.contains("scope_signals"));
        assert!(crate::prompts::GOAL_COMPLETENESS_CHECK_PROMPT.contains("禁止直接判断项目规模"));
        assert!(crate::prompts::REALITY_CONSISTENCY_CHECK_PROMPT.contains("suggestions"));
        assert!(crate::prompts::TASK_EXECUTABILITY_CHECK_PROMPT.contains("suggestions"));
        assert_eq!(check_order(), &CHECK_ORDER);
    }
}
