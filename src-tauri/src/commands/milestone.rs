use crate::plan_scope::PlanScope;
use crate::project;
use crate::workflow_resolution::resolve_selected_mid_stage_step;

// ===================================================================
// V1 Console 命令：大阶段草稿 → 检查 → 批准 → 选择
// 所有阶段生成统一从后端持久化事实读取输入。
// ===================================================================

const MILESTONE_REGEN_SOURCE_CHECK_FAILED: &str = "check_failed";
const MILESTONE_REGEN_SOURCE_APPROVAL_REJECTED: &str = "approval_rejected";

fn validate_generated_count(layer: &str, actual: usize, limit: u32) -> Result<(), String> {
    if limit == 0 {
        return Err(format!("当前工作负载画像不允许生成{layer}"));
    }
    if actual == 0 {
        return Err(format!("AI 返回的{layer}列表为空；至少需要 1 个"));
    }
    if actual > limit as usize {
        return Err(format!(
            "{layer}数量超出工作负载画像上限：实际 {actual}，上限 {limit}"
        ));
    }
    Ok(())
}

fn model_context(
    project: &project::Project,
    purpose: crate::cost_ledger::ModelCallPurpose,
) -> crate::cost_ledger::ModelCallContext {
    crate::cost_ledger::ModelCallContext::for_project(project, purpose)
}

fn mark_model_output(
    project_name: &str,
    call_id: &str,
    outcome: crate::cost_ledger::ModelCallOutcome,
) {
    crate::cost_ledger::mark_call_outcome_best_effort(project_name, call_id, outcome);
}

#[derive(Debug, serde::Deserialize)]
struct MilestoneCheckResponse {
    passed: bool,
    summary: String,
    omissions: Vec<String>,
    overlaps: Vec<String>,
    out_of_scope: Vec<String>,
    ordering_issues: Vec<String>,
    suggestions: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct MidStageCheckResponse {
    passed: bool,
    summary: String,
    omissions: Vec<String>,
    overlaps: Vec<String>,
    ordering_issues: Vec<String>,
    suggestions: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ExecutionPlanCheckResponse {
    passed: bool,
    summary: String,
    omissions: Vec<String>,
    out_of_scope: Vec<String>,
    not_executable: Vec<String>,
    suggestions: Vec<String>,
}

fn normalized_stage_check_summary(
    summary: String,
    mut blocking_groups: Vec<Vec<String>>,
    suggestions: Vec<String>,
    model_passed: bool,
) -> (bool, String) {
    let mut blocking = Vec::new();
    for group in blocking_groups.drain(..) {
        blocking.extend(group);
    }
    let normalized =
        crate::autopilot_policy::normalize_plan_check_result(project::StagePlanCheckResult {
            passed: model_passed,
            omissions: blocking,
            out_of_scope: Vec::new(),
            not_executable: Vec::new(),
            suggestions,
            checked_at: String::new(),
        });
    let mut details = vec![format!(
        "{}：{}",
        if normalized.passed {
            "检查通过"
        } else {
            "检查未通过"
        },
        summary.trim()
    )];
    if !normalized.omissions.is_empty() {
        details.push(format!("硬阻断：{}", normalized.omissions.join("；")));
    }
    if !normalized.suggestions.is_empty() {
        details.push(format!("建议：{}", normalized.suggestions.join("；")));
    }
    (normalized.passed, details.join("\n"))
}

impl MilestoneCheckResponse {
    fn into_decision(self) -> (bool, String) {
        normalized_stage_check_summary(
            self.summary,
            vec![
                self.omissions,
                self.overlaps,
                self.out_of_scope,
                self.ordering_issues,
            ],
            self.suggestions,
            self.passed,
        )
    }
}

impl MidStageCheckResponse {
    fn into_decision(self) -> (bool, String) {
        normalized_stage_check_summary(
            self.summary,
            vec![self.omissions, self.overlaps, self.ordering_issues],
            self.suggestions,
            self.passed,
        )
    }
}

impl ExecutionPlanCheckResponse {
    fn into_result(self) -> project::StagePlanCheckResult {
        let _summary = self.summary;
        crate::autopilot_policy::normalize_plan_check_result(project::StagePlanCheckResult {
            passed: self.passed,
            omissions: self.omissions,
            out_of_scope: self.out_of_scope,
            not_executable: self.not_executable,
            suggestions: self.suggestions,
            checked_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

fn required_string(value: &serde_json::Value, field: &str, entity: &str) -> Result<String, String> {
    let result = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{}缺少必要字段 {}", entity, field))?;
    Ok(result.to_string())
}

fn required_string_array(
    value: &serde_json::Value,
    field: &str,
    entity: &str,
) -> Result<Vec<String>, String> {
    let items = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}缺少数组字段 {}", entity, field))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{}的 {} 包含空白或非文本项", entity, field))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if items.is_empty() {
        return Err(format!("{}的 {} 不能为空", entity, field));
    }
    Ok(items)
}

fn optional_string_array(
    value: &serde_json::Value,
    field: &str,
    entity: &str,
) -> Result<Vec<String>, String> {
    let Some(raw_items) = value.get(field) else {
        return Ok(Vec::new());
    };
    let items = raw_items
        .as_array()
        .ok_or_else(|| format!("{}的 {} 必须是数组", entity, field))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{}的 {} 包含空白或非文本项", entity, field))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

fn string_array(
    value: &serde_json::Value,
    field: &str,
    entity: &str,
) -> Result<Vec<String>, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}缺少数组字段 {}", entity, field))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{}的 {} 包含空白或非文本项", entity, field))
        })
        .collect()
}

fn required_u32(value: &serde_json::Value, field: &str, entity: &str) -> Result<u32, String> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .filter(|number| *number > 0 && *number <= u32::MAX as u64)
        .ok_or_else(|| format!("{}缺少正整数字段 {}", entity, field))?;
    Ok(number as u32)
}

fn required_u32_array(
    value: &serde_json::Value,
    field: &str,
    entity: &str,
) -> Result<Vec<u32>, String> {
    let mut result = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}缺少数组字段 {}", entity, field))?
        .iter()
        .map(|item| {
            item.as_u64()
                .filter(|number| *number > 0 && *number <= u32::MAX as u64)
                .map(|number| number as u32)
                .ok_or_else(|| format!("{}的 {} 包含非正整数项", entity, field))
        })
        .collect::<Result<Vec<_>, _>>()?;
    result.sort_unstable();
    if result.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(format!("{}的 {} 包含重复顺序号", entity, field));
    }
    Ok(result)
}

async fn generate_milestone_candidates(
    proj: &project::Project,
    regeneration_feedback: Option<&str>,
) -> Result<Vec<project::Milestone>, String> {
    let workload = crate::workload_policy::current_profile(proj)?;
    if proj.version_plan.trim().is_empty() {
        return Err("没有正式项目方案，无法生成大阶段。请先批准方案。".to_string());
    }

    let constitution_part1 = if proj.project_path.is_empty() {
        String::new()
    } else {
        let constitution_path = std::path::Path::new(&proj.project_path).join("CONSTITUTION.md");
        if constitution_path.exists() {
            std::fs::read_to_string(&constitution_path)
                .map_err(|error| format!("读取项目宪法失败：{}", error))?
        } else {
            String::new()
        }
    };

    let discussion_summary = proj
        .discussion_threads
        .first()
        .map(|thread| {
            thread
                .messages
                .iter()
                .filter(|message| message.role != "system")
                .map(|message| format!("[{}]: {}", message.role, message.content))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .map_or_else(String::new, |summary| summary);
    let feedback_section = regeneration_feedback
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
        .map_or_else(String::new, |feedback| {
            format!("\n\n=== 重新生成反馈 ===\n{}", feedback)
        });

    let user_message = format!(
        "项目名称：{}\n项目来源：{}\n项目路径：{}\n讨论修订号：{}\n\n\
         === 已批准项目方案 ===\n{}\n\n=== 宪法第 1 部分 ===\n{}\n\n\
         === 讨论摘要 ===\n{}{}",
        proj.name,
        match proj.entry_kind {
            project::ProjectEntryKind::NoProject => "从零开始",
            project::ProjectEntryKind::HalfProject => "改造已有项目",
        },
        proj.project_path,
        proj.discussion_revision,
        proj.version_plan,
        if constitution_part1.is_empty() {
            "（无）"
        } else {
            &constitution_part1
        },
        discussion_summary,
        feedback_section,
    );

    let stage_mode = if workload.use_mid_stage_layer {
        project::StageMode::Professional
    } else {
        project::StageMode::Quick
    };
    let topology = if workload.use_mid_stage_layer {
        "每个大阶段后续通过中阶段承载执行计划"
    } else {
        "每个大阶段后续直接承载执行计划"
    };
    let system_prompt = format!(
        "{}\n\n{}\n必须生成 1..={} 个大阶段；{}。模型输出不得包含 mid_stages 或 subtasks 的预生成内容。",
        crate::prompts::MILESTONE_GENERATION_PROMPT,
        crate::workload_policy::render_planning_constraints(workload),
        workload.max_milestones,
        topology,
    );
    // Inject context: working constitution, approved plan, discussion, Already constitution
    let context_injection = crate::constitution_context::build_context_injection(&proj);
    let augmented_user_message = if context_injection.is_empty() {
        user_message
    } else {
        format!("{}\n\n{}", context_injection, user_message)
    };
    let context = model_context(
        proj,
        crate::cost_ledger::ModelCallPurpose::MilestoneGeneration,
    );
    let response = crate::api::call_deepseek_api_inner_with_context(
        &system_prompt,
        &augmented_user_message,
        false,
        0.5,
        context.clone(),
    )
    .await?;
    let call_id = response.metadata.call_id.clone();
    let content = response.content;

    let raw_milestones: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry_with_context(&content, context)
            .await
            .map_err(|error| format!("解析大阶段 JSON 失败：{}", error))?;
    validate_generated_count("大阶段", raw_milestones.len(), workload.max_milestones)?;

    let milestones = raw_milestones
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let entity = format!("第 {} 个大阶段", index + 1);
            Ok(project::Milestone {
                id: uuid::Uuid::new_v4().to_string(),
                version: required_string(raw, "version", &entity)?,
                title: required_string(raw, "title", &entity)?,
                description: required_string(raw, "description", &entity)?,
                tech_stack: required_string(raw, "tech_stack", &entity)?,
                status: project::MilestoneStatus::Pending,
                mode: stage_mode.clone(),
                mid_stages: Vec::new(),
                subtasks: Vec::new(),
                qa_result: None,
                git_commit_hash: String::new(),
                decomposition_check: None,
                review_status: None,
                review_conclusion: None,
                approved_at: None,
                goal: required_string(raw, "goal", &entity)?,
                scope: required_string(raw, "scope", &entity)?,
                dependencies: optional_string_array(raw, "dependencies", &entity)?,
                expected_output: required_string(raw, "expected_output", &entity)?,
                acceptance_criteria: required_string_array(raw, "acceptance_criteria", &entity)?,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    mark_model_output(
        &proj.name,
        &call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_plan: true,
            ..Default::default()
        },
    );
    Ok(milestones)
}

/// 生成大阶段草稿（V1：后端读取正式项目事实，不接收前端传入的方案正文）
///
/// 1. 验证当前步骤为 MilestoneGeneration
/// 2. 读取 version_plan、宪法第 1 部分、讨论摘要
/// 3. 调用统一 DeepSeek 工作流模型生成结构化候选大阶段
/// 4. 验证每个候选大阶段包含必要字段
/// 5. 保存为 milestone_draft，转换到 MilestoneCheck
#[tauri::command]
pub(crate) async fn generate_milestone_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    if initial.workflow_state.current_step != project::WorkflowStep::MilestoneGeneration {
        return Err(format!(
            "当前步骤为 {:?}，首次生成只允许在 MilestoneGeneration 调用；检查或审批页面请使用 regenerate_milestone_draft",
            initial.workflow_state.current_step
        ));
    }
    let initial_revision = initial.workflow_state.data_revision;
    let initial_plan = initial.version_plan.clone();
    let candidates = generate_milestone_candidates(&initial, None).await?;
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneGeneration
        || proj.workflow_state.data_revision != initial_revision
        || proj.version_plan != initial_plan
    {
        return Err("生成期间项目事实已变化，未写入本次结果。请同步后重试。".to_string());
    }
    let draft = project::MilestoneDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        status: project::MilestoneDraftStatus::Pending,
        draft_kind: project::MilestoneDraftKind::Normal,
        candidate_milestones: candidates,
        check_result: None,
        generation_revision: proj.discussion_revision,
        source_plan_revision: proj.workflow_state.data_revision,
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved_at: None,
        regeneration_count: 0,
        previous_draft_id: None,
        last_regeneration_reason: None,
        last_regenerated_at: None,
        ..Default::default()
    };

    proj.milestone_draft = Some(draft);
    proj.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

#[tauri::command]
pub(crate) async fn regenerate_milestone_draft(
    project_name: String,
    current_draft_id: String,
    expected_data_revision: u64,
    feedback: String,
    source: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    let valid_source = match (&initial.workflow_state.current_step, source.as_str()) {
        (project::WorkflowStep::MilestoneCheck, MILESTONE_REGEN_SOURCE_CHECK_FAILED) => true,
        (project::WorkflowStep::MilestoneApproval, MILESTONE_REGEN_SOURCE_APPROVAL_REJECTED) => {
            true
        }
        _ => false,
    };
    if !valid_source {
        return Err(format!(
            "当前步骤 {:?} 与重新生成来源 {} 不匹配",
            initial.workflow_state.current_step, source
        ));
    }
    if initial.workflow_state.data_revision != expected_data_revision {
        return Err("项目修订号已变化，请同步最新项目后再重新生成。".to_string());
    }
    let old_draft = initial
        .milestone_draft
        .as_ref()
        .ok_or_else(|| "没有可重新生成的大阶段草稿。".to_string())?;
    if old_draft.draft_id != current_draft_id {
        return Err("大阶段草稿已变化，请同步后重试。".to_string());
    }
    let has_execution_facts = initial.milestones.iter().any(|milestone| {
        matches!(
            milestone.status,
            project::MilestoneStatus::InProgress | project::MilestoneStatus::Completed
        )
    });
    if has_execution_facts {
        return Err("已有执行中或已完成的大阶段，禁止重新生成；请使用审阅或回退流程。".to_string());
    }

    let effective_feedback = if feedback.trim().is_empty() {
        old_draft
            .check_result
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "请提供重新生成反馈。".to_string())?
            .to_string()
    } else {
        feedback.trim().to_string()
    };
    let initial_plan = initial.version_plan.clone();
    let old_regeneration_count = old_draft.regeneration_count;
    let candidates = generate_milestone_candidates(&initial, Some(&effective_feedback)).await?;

    let mut latest = crate::load_project(&project_name)?;
    let latest_draft = latest
        .milestone_draft
        .as_ref()
        .ok_or_else(|| "生成期间原草稿已不存在，未写入新草稿。".to_string())?;
    if latest.workflow_state.data_revision != expected_data_revision
        || latest.workflow_state.current_step != initial.workflow_state.current_step
        || latest_draft.draft_id != current_draft_id
        || latest.version_plan != initial_plan
    {
        return Err("生成期间项目或草稿已变化，未覆盖原草稿。请同步后重试。".to_string());
    }
    if latest.milestones.iter().any(|milestone| {
        matches!(
            milestone.status,
            project::MilestoneStatus::InProgress | project::MilestoneStatus::Completed
        )
    }) {
        return Err("生成期间出现了执行事实，未覆盖原草稿。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    latest.milestone_draft = Some(project::MilestoneDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        status: project::MilestoneDraftStatus::Pending,
        draft_kind: project::MilestoneDraftKind::Normal,
        candidate_milestones: candidates,
        check_result: None,
        generation_revision: latest.discussion_revision,
        source_plan_revision: expected_data_revision,
        generated_at: now.clone(),
        approved_at: None,
        regeneration_count: old_regeneration_count + 1,
        previous_draft_id: Some(current_draft_id),
        last_regeneration_reason: Some(effective_feedback),
        last_regenerated_at: Some(now.clone()),
        ..Default::default()
    });
    latest.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
    latest.workflow_state.data_revision += 1;
    latest.workflow_state.last_transition_at = now;
    crate::save_and_reload_project(&latest)
}

/// 检查大阶段草稿（V1：独立 AI 检查器核对候选大阶段与正式方案的一致性）
///
/// 1. 验证当前步骤为 MilestoneCheck
/// 2. 读取正式 version_plan 和候选大阶段
/// 3. 调用 AI 检查遗漏、重复、越界、顺序错误、不可执行内容
/// 4. 保存检查结果到 milestone_draft.check_result
/// 5. 检查通过 → MilestoneApproval；未通过 → 保留在 MilestoneCheck
#[tauri::command]
pub(crate) async fn check_milestone_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    // 1. 验证当前步骤
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneCheck {
        return Err(format!(
            "当前步骤为 {:?}，只有 MilestoneCheck 步骤可以检查大阶段草稿",
            proj.workflow_state.current_step
        ));
    }

    // 2. 获取草稿
    let draft = proj
        .milestone_draft
        .as_ref()
        .ok_or("没有大阶段草稿，请先生成大阶段。".to_string())?;

    if draft.candidate_milestones.is_empty() {
        return Err("候选大阶段列表为空，请重新生成。".to_string());
    }

    // 3. 序列化候选大阶段摘要（发送给检查器的内容）
    let candidates_summary: Vec<String> = draft
        .candidate_milestones
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let deps_text = if m.dependencies.is_empty() {
                "无".to_string()
            } else {
                m.dependencies.join("、")
            };
            format!(
                "{}. {} ({})\n   目标：{}\n   范围：{}\n   依赖：{}\n   预期输出：{}\n   验收标准：{}",
                i + 1,
                m.title,
                m.version,
                m.goal,
                m.scope,
                deps_text,
                m.expected_output,
                m.acceptance_criteria.join("；")
            )
        })
        .collect();

    let candidates_text = candidates_summary.join("\n\n");

    // 4. 构造检查上下文
    let check_context = format!(
        "=== 正式项目方案 ===\n{}\n\n=== 候选大阶段列表（共 {} 个） ===\n{}",
        proj.version_plan,
        draft.candidate_milestones.len(),
        candidates_text
    );

    // 5. 调用 AI 检查器
    let call_context = model_context(&proj, crate::cost_ledger::ModelCallPurpose::MilestoneCheck);
    let response = match crate::api::call_deepseek_api_json_with_context(
        crate::prompts::MILESTONE_CHECK_PROMPT,
        &check_context,
        call_context.clone(),
    )
    .await
    {
        Ok(response) => response,
        Err(e) => {
            return Err(format!("大阶段检查 AI 调用失败：{}", e));
        }
    };

    let check: MilestoneCheckResponse = crate::json_utils::parse_json_with_contract_and_context(
        &response.content,
        &crate::json_utils::MILESTONE_CHECK_JSON_CONTRACT,
        call_context,
    )
    .await
    .map_err(|error| format!("大阶段检查协议失败：{}", error))?;
    let (passed, summary) = check.into_decision();

    // 6. 重新加载并保存结果
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneCheck {
        return Err("当前项目已不在大阶段检查步骤，请刷新页面。".to_string());
    }

    apply_milestone_check_result(&mut proj, passed, summary)?;

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    let saved = crate::save_and_reload_project(&proj)?;
    mark_model_output(
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

fn apply_milestone_check_result(
    proj: &mut project::Project,
    passed: bool,
    summary: String,
) -> Result<(), String> {
    let draft = proj
        .milestone_draft
        .as_mut()
        .ok_or("没有大阶段草稿，请先生成大阶段。".to_string())?;
    draft.check_result = Some(summary);
    draft.status = if passed {
        project::MilestoneDraftStatus::CheckPassed
    } else {
        project::MilestoneDraftStatus::CheckFailed
    };

    if passed {
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
    }
    Ok(())
}

/// 批准大阶段草稿（V1：将候选大阶段复制为正式 milestones）
///
/// 1. 验证当前步骤为 MilestoneApproval
/// 2. 验证检查已通过
/// 3. 将候选列表复制为正式 milestones
/// 4. 转换到 MilestoneSelection（不得自动选中第一个）
#[tauri::command]
pub(crate) async fn approve_milestone_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 1. 验证当前步骤
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneApproval {
        return Err(format!(
            "当前步骤为 {:?}，只有 MilestoneApproval 步骤可以批准大阶段",
            proj.workflow_state.current_step
        ));
    }

    // 2. 获取草稿
    let draft = proj
        .milestone_draft
        .as_ref()
        .ok_or("没有大阶段草稿，请先生成并检查大阶段。".to_string())?;

    // 3. 只接受明确通过检查的草稿，禁止从非失败状态反向推断通过。
    if draft.status != project::MilestoneDraftStatus::CheckPassed {
        return Err("大阶段草稿尚未明确通过检查，无法批准。请先完成质量检查。".to_string());
    }
    if !draft
        .check_result
        .as_deref()
        .is_some_and(|result| !result.trim().is_empty())
    {
        return Err("大阶段草稿缺少有效检查结果，请先重新运行检查。".to_string());
    }

    // 4. 验证候选列表非空
    if draft.candidate_milestones.is_empty() {
        return Err("候选大阶段列表为空，无法批准。".to_string());
    }

    // 5. 校验：已有执行中或已完成的大阶段时，禁止替换
    let has_active_milestones = proj.milestones.iter().any(|m| {
        m.status == project::MilestoneStatus::InProgress
            || m.status == project::MilestoneStatus::Completed
    });
    if has_active_milestones {
        return Err(
            "已有执行中或已完成的大阶段，禁止替换正式大阶段列表。请通过大阶段审阅 A/B/C 分支调整。"
                .to_string(),
        );
    }

    // 6. 复制候选到正式 milestones
    proj.milestones = draft.candidate_milestones.clone();

    // 7. 更新草稿状态
    if let Some(ref mut d) = proj.milestone_draft {
        d.status = project::MilestoneDraftStatus::Approved;
        d.approved_at = Some(chrono::Utc::now().to_rfc3339());
    }

    // 8. 转换到 MilestoneSelection（不得自动选中第一个大阶段）
    proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
    // 大阶段批准是托管层的原子终点；同次保存释放自动驾驶互斥锁。
    proj.workflow_state.managed_flow_state = None;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 手动选择大阶段（V1：持久化 current_milestone_id，不得自动选中）
#[tauri::command]
pub(crate) async fn select_milestone(
    project_name: String,
    milestone_id: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 1. 验证当前步骤在合法范围内（MilestoneSelection 或后续选择步骤）
    let valid_steps = [
        project::WorkflowStep::MilestoneSelection,
        project::WorkflowStep::MidStageGeneration,
        project::WorkflowStep::MidStageCheck,
        project::WorkflowStep::MidStageApproval,
        project::WorkflowStep::MidStageSelection,
        project::WorkflowStep::PlanGeneration,
        project::WorkflowStep::PlanCheck,
        project::WorkflowStep::PlanApproving,
    ];
    if !valid_steps.contains(&proj.workflow_state.current_step) {
        return Err(format!(
            "当前步骤为 {:?}，不能在此步骤选择大阶段",
            proj.workflow_state.current_step
        ));
    }

    // 2. 验证 milestone_id 存在于正式 milestones 中
    let workload = crate::workload_policy::current_profile(&proj)?;
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| format!("大阶段 {} 不在正式大阶段列表中", milestone_id))?;
    let expected_mode = if workload.use_mid_stage_layer {
        project::StageMode::Professional
    } else {
        project::StageMode::Quick
    };
    if milestone.mode != expected_mode {
        return Err(format!(
            "大阶段拓扑与工作负载画像矛盾：画像要求 {:?}，当前为 {:?}",
            expected_mode, milestone.mode
        ));
    }

    // 3. 持久化选择
    proj.current_milestone_id = milestone_id.clone();
    proj.current_mid_stage_id.clear();
    proj.workflow_state.data_revision += 1;

    crate::save_and_reload_project(&proj)
}

// ===================================================================
// V1 中阶段命令：草稿 → 检查 → 批准 → 选择
// ===================================================================

async fn generate_mid_stage_candidates(
    proj: &project::Project,
    milestone_id: &str,
    regeneration_feedback: Option<&str>,
) -> Result<Vec<project::MidStage>, String> {
    let workload = crate::workload_policy::current_profile(proj)?;
    if !workload.use_mid_stage_layer {
        return Err("当前工作负载画像使用大阶段直挂计划，禁止生成中阶段。".to_string());
    }
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "当前选择的大阶段不存在。".to_string())?;
    let feedback_section = regeneration_feedback
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
        .map_or_else(String::new, |feedback| {
            format!("\n\n重新生成反馈：\n{}", feedback)
        });
    let context_injection = crate::constitution_context::build_context_injection(proj);
    let context = format!(
        "{}{}大阶段：{} ({})\n目标：{}\n范围：{}\n预期输出：{}\n验收标准：{}\n技术栈：{}\n\n项目方案：\n{}{}",
        if context_injection.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", context_injection)
        },
        if context_injection.is_empty() {
            String::new()
        } else {
            "---\n\n".to_string()
        },
        milestone.title,
        milestone.version,
        milestone.goal,
        milestone.scope,
        milestone.expected_output,
        milestone.acceptance_criteria.join("；"),
        milestone.tech_stack,
        proj.version_plan,
        feedback_section,
    );
    let call_context = model_context(
        proj,
        crate::cost_ledger::ModelCallPurpose::MidStageGeneration,
    );
    let prompt = format!(
        "{}\n\n{}\n必须生成 1..={} 个中阶段。",
        crate::prompts::MID_STAGE_GENERATION_PROMPT,
        crate::workload_policy::render_planning_constraints(workload),
        workload.max_mid_stages,
    );
    let response =
        crate::api::call_deepseek_api_json_with_context(&prompt, &context, call_context.clone())
            .await
            .map_err(|error| format!("中阶段生成 AI 调用失败：{}", error))?;
    let raw: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry_with_context(&response.content, call_context)
            .await
            .map_err(|error| format!("解析中阶段 JSON 失败：{}", error))?;
    validate_generated_count("中阶段", raw.len(), workload.max_mid_stages)?;

    let stages = raw
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let entity = format!("第 {} 个中阶段", index + 1);
            required_string(item, "goal", &entity)?;
            required_string(item, "scope", &entity)?;
            Ok(project::MidStage {
                id: uuid::Uuid::new_v4().to_string(),
                version: required_string(item, "version", &entity)?,
                title: required_string(item, "title", &entity)?,
                description: required_string(item, "description", &entity)?,
                tech_focus: required_string(item, "tech_focus", &entity)?,
                order: Some((index + 1) as i32),
                status: project::MidStageStatus::Pending,
                subtasks: Vec::new(),
                test_report: String::new(),
                domain: None,
                test_log: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
                approved_at: None,
                git_tag: String::new(),
                plan_check_result: None,
                plan_approved_at: None,
                plan_revision: 0,
                plan_draft_revision: 0,
                plan_generated_at: None,
                plan_regeneration_count: 0,
                last_plan_failure_fingerprint: String::new(),
                last_plan_issue_count: 0,
                plan_no_progress_count: 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    mark_model_output(
        &proj.name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_plan: true,
            ..Default::default()
        },
    );
    Ok(stages)
}

fn initial_mid_stage_baseline<'a>(
    proj: &'a project::Project,
    milestone_id: &str,
) -> Result<&'a project::Milestone, String> {
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "当前大阶段不存在。".to_string())?;
    if !milestone.mid_stages.is_empty() {
        return Err(
            "当前大阶段已有正式中阶段，请选择或恢复既有中阶段；首次完整草稿只允许用于空列表。"
                .to_string(),
        );
    }
    Ok(milestone)
}

fn validate_initial_mid_stage_draft(draft: &project::MidStageDraft) -> Result<(), String> {
    if draft.purpose != project::MidStageDraftPurpose::InitialFullList
        || !draft.allow_full_replacement
        || draft.base_mid_stage_revision != 0
        || !draft.retained_mid_stage_ids.is_empty()
        || draft.source_step != project::WorkflowStep::MidStageGeneration
    {
        return Err("当前草稿不是空基线上的首次完整中阶段列表，禁止使用整表批准命令。".to_string());
    }
    Ok(())
}

/// 生成中阶段草稿（V1：读取正式大阶段、项目方案、宪法，生成垂直切片中阶段）
#[tauri::command]
pub(crate) async fn generate_mid_stage_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    if initial.workflow_state.current_step != project::WorkflowStep::MidStageGeneration {
        return Err(format!(
            "当前步骤为 {:?}，首次生成只允许在 MidStageGeneration 调用；检查或审批页面请使用 regenerate_mid_stage_draft",
            initial.workflow_state.current_step
        ));
    }
    let milestone_id = initial.current_milestone_id.clone();
    if milestone_id.is_empty() {
        return Err("未选择大阶段，请先在执行树中选择一个大阶段。".to_string());
    }
    initial_mid_stage_baseline(&initial, &milestone_id)?;
    let initial_revision = initial.workflow_state.data_revision;
    let initial_plan = initial.version_plan.clone();
    let candidates = generate_mid_stage_candidates(&initial, &milestone_id, None).await?;
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MidStageGeneration
        || proj.workflow_state.data_revision != initial_revision
        || proj.current_milestone_id != milestone_id
        || proj.version_plan != initial_plan
    {
        return Err("生成期间项目事实已变化，未写入中阶段草稿。请同步后重试。".to_string());
    }
    initial_mid_stage_baseline(&proj, &milestone_id)?;
    let candidate_fingerprint =
        crate::autopilot_policy::mid_stage_candidate_fingerprint(&candidates);
    let draft = project::MidStageDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        milestone_id: milestone_id.clone(),
        status: project::MidStageDraftStatus::Pending,
        candidate_mid_stages: candidates,
        check_result: None,
        generation_revision: proj.discussion_revision,
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved_at: None,
        regeneration_count: 0,
        previous_draft_id: None,
        last_regeneration_reason: None,
        source_data_revision: initial_revision,
        last_check_failure_fingerprint: String::new(),
        last_candidate_fingerprint: candidate_fingerprint,
        no_progress_count: 0,
        purpose: project::MidStageDraftPurpose::InitialFullList,
        base_mid_stage_revision: 0,
        retained_mid_stage_ids: vec![],
        source_step: project::WorkflowStep::MidStageGeneration,
        allow_full_replacement: true,
    };

    proj.mid_stage_draft = Some(draft);
    proj.workflow_state.current_step = project::WorkflowStep::MidStageCheck;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

#[tauri::command]
pub(crate) async fn regenerate_mid_stage_draft(
    project_name: String,
    current_draft_id: String,
    expected_data_revision: u64,
    feedback: String,
    source: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    let valid_source = matches!(
        (&initial.workflow_state.current_step, source.as_str()),
        (project::WorkflowStep::MidStageCheck, "check_failed")
            | (project::WorkflowStep::MidStageApproval, "approval_rejected")
    );
    if !valid_source {
        return Err(format!(
            "当前步骤 {:?} 与中阶段重新生成来源不匹配",
            initial.workflow_state.current_step
        ));
    }
    if initial.workflow_state.data_revision != expected_data_revision {
        return Err("项目修订号已变化，请同步后重试。".to_string());
    }
    let old_draft = initial
        .mid_stage_draft
        .as_ref()
        .ok_or_else(|| "没有可重新生成的中阶段草稿。".to_string())?;
    if old_draft.draft_id != current_draft_id
        || old_draft.milestone_id != initial.current_milestone_id
    {
        return Err("中阶段草稿或所属大阶段已变化，请同步后重试。".to_string());
    }
    validate_initial_mid_stage_draft(old_draft)?;
    initial_mid_stage_baseline(&initial, &initial.current_milestone_id)?;
    let effective_feedback = if feedback.trim().is_empty() {
        old_draft
            .check_result
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "请提供中阶段重新生成反馈。".to_string())?
            .to_string()
    } else {
        feedback.trim().to_string()
    };
    let milestone_id = initial.current_milestone_id.clone();
    let initial_plan = initial.version_plan.clone();
    let old_count = old_draft.regeneration_count;
    let old_failure_fingerprint = old_draft.last_check_failure_fingerprint.clone();
    let old_candidate_fingerprint = if old_draft.last_candidate_fingerprint.is_empty() {
        crate::autopilot_policy::mid_stage_candidate_fingerprint(&old_draft.candidate_mid_stages)
    } else {
        old_draft.last_candidate_fingerprint.clone()
    };
    let old_no_progress_count = old_draft.no_progress_count;
    let candidates =
        generate_mid_stage_candidates(&initial, &milestone_id, Some(&effective_feedback)).await?;
    let candidate_fingerprint =
        crate::autopilot_policy::mid_stage_candidate_fingerprint(&candidates);
    let candidate_unchanged = candidate_fingerprint == old_candidate_fingerprint;

    let mut latest = crate::load_project(&project_name)?;
    let latest_draft = latest
        .mid_stage_draft
        .as_ref()
        .ok_or_else(|| "生成期间原中阶段草稿已不存在。".to_string())?;
    if latest.workflow_state.data_revision != expected_data_revision
        || latest.workflow_state.current_step != initial.workflow_state.current_step
        || latest.current_milestone_id != milestone_id
        || latest_draft.draft_id != current_draft_id
        || latest.version_plan != initial_plan
    {
        return Err("生成期间项目或中阶段草稿已变化，未覆盖原草稿。".to_string());
    }
    validate_initial_mid_stage_draft(latest_draft)?;
    initial_mid_stage_baseline(&latest, &milestone_id)
        .map_err(|_| "生成期间出现了正式中阶段，未覆盖原草稿。".to_string())?;

    let now = chrono::Utc::now().to_rfc3339();
    latest.mid_stage_draft = Some(project::MidStageDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        milestone_id,
        status: project::MidStageDraftStatus::Pending,
        candidate_mid_stages: candidates,
        check_result: None,
        generation_revision: latest.discussion_revision,
        generated_at: now.clone(),
        approved_at: None,
        regeneration_count: old_count + 1,
        previous_draft_id: Some(current_draft_id),
        last_regeneration_reason: Some(effective_feedback),
        source_data_revision: expected_data_revision,
        last_check_failure_fingerprint: old_failure_fingerprint,
        last_candidate_fingerprint: candidate_fingerprint,
        no_progress_count: if candidate_unchanged {
            old_no_progress_count.saturating_add(1)
        } else {
            old_no_progress_count
        },
        purpose: project::MidStageDraftPurpose::InitialFullList,
        base_mid_stage_revision: 0,
        retained_mid_stage_ids: vec![],
        source_step: project::WorkflowStep::MidStageGeneration,
        allow_full_replacement: true,
    });
    latest.workflow_state.current_step = project::WorkflowStep::MidStageCheck;
    latest.workflow_state.data_revision += 1;
    latest.workflow_state.last_transition_at = now;
    crate::save_and_reload_project(&latest)
}

/// 检查中阶段草稿
#[tauri::command]
pub(crate) async fn check_mid_stage_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::MidStageCheck {
        return Err(format!(
            "当前步骤为 {:?}，只有 MidStageCheck 步骤可以检查中阶段草稿",
            proj.workflow_state.current_step
        ));
    }

    let draft = proj
        .mid_stage_draft
        .as_ref()
        .ok_or("没有中阶段草稿，请先生成。".to_string())?;
    validate_initial_mid_stage_draft(draft)?;
    initial_mid_stage_baseline(&proj, &draft.milestone_id)?;

    let milestone = proj
        .milestones
        .iter()
        .find(|m| m.id == draft.milestone_id)
        .ok_or("关联的大阶段不存在。".to_string())?;

    let candidates_text = draft
        .candidate_mid_stages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            format!(
                "{}. {} — {} (tech: {})",
                i + 1,
                m.title,
                m.description,
                m.tech_focus
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let context = format!(
        "大阶段：{} — {}\n\n候选：\n{}",
        milestone.title, milestone.goal, candidates_text
    );

    let call_context = model_context(&proj, crate::cost_ledger::ModelCallPurpose::MidStageCheck);
    let response = crate::api::call_deepseek_api_json_with_context(
        crate::prompts::MID_STAGE_CHECK_PROMPT,
        &context,
        call_context.clone(),
    )
    .await
    .map_err(|e| format!("中阶段检查 AI 调用失败：{}", e))?;

    let check: MidStageCheckResponse = crate::json_utils::parse_json_with_contract_and_context(
        &response.content,
        &crate::json_utils::MID_STAGE_CHECK_JSON_CONTRACT,
        call_context,
    )
    .await
    .map_err(|error| format!("中阶段检查协议失败：{}", error))?;
    let (passed, summary) = check.into_decision();

    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MidStageCheck {
        return Err("当前项目已不在中阶段检查步骤，请刷新。".to_string());
    }

    if let Some(ref mut d) = proj.mid_stage_draft {
        d.check_result = Some(summary.clone());
        d.status = if passed {
            project::MidStageDraftStatus::Pending // 标记为待批准
        } else {
            project::MidStageDraftStatus::CheckFailed
        };
        if passed {
            d.last_check_failure_fingerprint.clear();
            d.no_progress_count = 0;
        } else {
            let fingerprint = crate::autopilot_policy::text_fingerprint(&summary);
            if !d.last_check_failure_fingerprint.is_empty()
                && d.last_check_failure_fingerprint == fingerprint
            {
                d.no_progress_count = d.no_progress_count.saturating_add(1);
            }
            d.last_check_failure_fingerprint = fingerprint;
            if d.last_candidate_fingerprint.is_empty() {
                d.last_candidate_fingerprint =
                    crate::autopilot_policy::mid_stage_candidate_fingerprint(
                        &d.candidate_mid_stages,
                    );
            }
        }
    }

    proj.workflow_state.current_step = if passed {
        project::WorkflowStep::MidStageApproval
    } else {
        project::WorkflowStep::MidStageCheck // 留在检查步骤
    };
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    let saved = crate::save_and_reload_project(&proj)?;
    mark_model_output(
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

/// 批准中阶段草稿（复制候选到正式中阶段列表）
#[tauri::command]
pub(crate) async fn approve_mid_stage_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj
        .mid_stage_draft
        .as_ref()
        .is_some_and(|draft| draft.status == project::MidStageDraftStatus::Approved)
    {
        return Ok(proj);
    }

    if proj.workflow_state.current_step != project::WorkflowStep::MidStageApproval {
        return Err(format!(
            "当前步骤为 {:?}，只有 MidStageApproval 步骤可以批准中阶段",
            proj.workflow_state.current_step
        ));
    }

    let draft = proj
        .mid_stage_draft
        .as_ref()
        .ok_or("没有中阶段草稿。".to_string())?;

    if draft.status == project::MidStageDraftStatus::CheckFailed {
        return Err("中阶段草稿检查未通过，无法批准。".to_string());
    }
    if draft.candidate_mid_stages.is_empty() {
        return Err("候选中阶段列表为空。".to_string());
    }
    validate_initial_mid_stage_draft(draft)?;

    // Find the milestone and copy candidates
    let milestone_id = draft.milestone_id.clone();
    let candidates = draft.candidate_mid_stages.clone();

    let ms = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == milestone_id)
        .ok_or("关联的大阶段不存在。".to_string())?;

    // 禁止覆盖已有执行进度的中阶段
    let has_active = ms.mid_stages.iter().any(|m| {
        m.status == project::MidStageStatus::InProgress
            || m.status == project::MidStageStatus::Completed
    });
    if has_active {
        return Err(
            "该大阶段已有执行中或已完成的中阶段，禁止整表替换；请按项目事实恢复执行或进入大阶段审阅。".to_string(),
        );
    }
    if !ms.mid_stages.is_empty() {
        return Err(
            "该大阶段已有正式中阶段，请选择或恢复既有中阶段；首次完整草稿不得替换正式列表。"
                .to_string(),
        );
    }

    ms.mid_stages = candidates;

    if let Some(ref mut d) = proj.mid_stage_draft {
        d.status = project::MidStageDraftStatus::Approved;
        d.approved_at = Some(chrono::Utc::now().to_rfc3339());
    }

    proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 手动选择中阶段
#[tauri::command]
pub(crate) async fn select_mid_stage(
    project_name: String,
    mid_stage_id: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    let valid_steps = [
        project::WorkflowStep::MidStageSelection,
        project::WorkflowStep::PlanGeneration,
        project::WorkflowStep::PlanCheck,
        project::WorkflowStep::PlanApproving,
        project::WorkflowStep::Execution,
    ];
    if !valid_steps.contains(&proj.workflow_state.current_step) {
        return Err(format!(
            "当前步骤 {:?} 不允许选择中阶段",
            proj.workflow_state.current_step
        ));
    }

    let milestone_id = proj.current_milestone_id.clone();
    if milestone_id.is_empty() {
        return Err("请先选择一个大阶段。".to_string());
    }

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。".to_string())?;
    let workload = crate::workload_policy::current_profile(&proj)?;
    if !workload.use_mid_stage_layer || ms.mode != project::StageMode::Professional {
        return Err("当前工作负载画像不允许选择中阶段。".to_string());
    }

    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不在当前大阶段中。".to_string())?;

    let next_step = resolve_selected_mid_stage_step(ms, mid)?;
    let now = chrono::Utc::now().to_rfc3339();
    proj.current_mid_stage_id = mid_stage_id;
    if next_step == project::WorkflowStep::MilestoneReview {
        crate::workflow_resolution::apply_milestone_review_boundary(
            &mut proj,
            &milestone_id,
            &now,
        )?;
    } else {
        proj.workflow_state.current_step = next_step;
    }
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 根据当前大阶段的持久化事实继续，不接受前端指定目标步骤。
#[tauri::command]
pub(crate) async fn continue_current_milestone(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneSelection {
        return Err(format!(
            "当前步骤为 {:?}，只有 MilestoneSelection 可以继续当前大阶段",
            proj.workflow_state.current_step
        ));
    }
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == proj.current_milestone_id)
        .ok_or_else(|| "请先选择有效的大阶段。".to_string())?;
    let workload = crate::workload_policy::current_profile(&proj)?;
    let expected_mode = if workload.use_mid_stage_layer {
        project::StageMode::Professional
    } else {
        project::StageMode::Quick
    };
    if milestone.mode != expected_mode {
        return Err(format!(
            "大阶段拓扑与工作负载画像矛盾：画像要求 {:?}，当前为 {:?}",
            expected_mode, milestone.mode
        ));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let changed = if milestone.mode == project::StageMode::Quick {
        let next_step = crate::workflow_resolution::resolve_direct_milestone_step(milestone)?;
        if next_step == project::WorkflowStep::MilestoneReview {
            let milestone_id = proj.current_milestone_id.clone();
            crate::workflow_resolution::apply_milestone_review_boundary(
                &mut proj,
                &milestone_id,
                &now,
            )?;
            true
        } else {
            let changed = proj.workflow_state.current_step != next_step
                || !proj.current_mid_stage_id.is_empty();
            proj.current_mid_stage_id.clear();
            proj.workflow_state.current_step = next_step;
            changed
        }
    } else {
        let route = crate::workflow_resolution::resolve_mid_stage_route(milestone);
        crate::workflow_resolution::apply_mid_stage_route(&mut proj, &route, &now)?
    };
    if changed {
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = now;
        return crate::save_and_reload_project(&proj);
    }
    Ok(proj)
}

// ===================================================================
// V1 执行计划命令：编译 → 检查 → 批准
// ===================================================================

fn plan_check_feedback(result: &project::StagePlanCheckResult) -> String {
    [
        ("遗漏", &result.omissions),
        ("越界", &result.out_of_scope),
        ("不可执行", &result.not_executable),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .map(|(label, items)| format!("{}：{}", label, items.join("；")))
    .collect::<Vec<_>>()
    .join("\n")
}

fn plan_check_tracking(
    scope: PlanScope,
    proj: &project::Project,
    result: &project::StagePlanCheckResult,
) -> (String, u32, u32) {
    if result.passed {
        return (String::new(), 0, 0);
    }
    let fingerprint = crate::autopilot_policy::plan_failure_fingerprint(result);
    let issue_count = crate::autopilot_policy::blocking_plan_issue_count(result);
    let repeated = !scope.last_plan_failure_fingerprint(proj).is_empty()
        && scope.last_plan_failure_fingerprint(proj) == fingerprint;
    let did_not_improve = issue_count > 0
        && scope.last_plan_issue_count(proj) > 0
        && issue_count >= scope.last_plan_issue_count(proj);
    let no_progress_count = if repeated || did_not_improve {
        scope.plan_no_progress_count(proj).saturating_add(1)
    } else {
        scope.plan_no_progress_count(proj)
    };
    (fingerprint, issue_count, no_progress_count)
}

async fn generate_execution_plan_tasks(
    proj: &project::Project,
    regeneration_feedback: Option<&str>,
) -> Result<Vec<project::Subtask>, String> {
    let workload = crate::workload_policy::current_profile(proj)?;
    let max_subtasks = workload.max_subtasks;
    let scope = PlanScope::resolve(proj)?;
    let milestone = scope.milestone(proj);
    let (target_kind, target_title, target_version, target_description, target_tech_focus) =
        match scope.mid_stage(proj) {
            Some(mid_stage) => (
                "中阶段",
                mid_stage.title.as_str(),
                mid_stage.version.as_str(),
                mid_stage.description.as_str(),
                mid_stage.tech_focus.as_str(),
            ),
            None => (
                "大阶段直挂计划",
                milestone.title.as_str(),
                milestone.version.as_str(),
                milestone.description.as_str(),
                milestone.tech_stack.as_str(),
            ),
        };
    let feedback_section = regeneration_feedback
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
        .map_or_else(String::new, |feedback| {
            format!("\n\n重新生成反馈：\n{}", feedback)
        });
    let context_injection = crate::constitution_context::build_context_injection(proj);
    let project_facts = crate::project_facts::planning_context(proj)?;
    let context = format!(
        "{}计划目标（{}）：{} ({})\n描述：{}\n技术重点：{}\n\n所属大阶段：{} — {}\n\
         项目方案摘要（仅相关部分）：\n{}\n\n项目路径：{}\n\
         当前项目事实（压缩扫描，不含完整文件）：\n{}\n\n\
         完整文件由执行器在运行时按 evidence_files 精确读取。\n\n{}\n必须生成 1..={} 个小阶段。{}",
        if context_injection.is_empty() {
            String::new()
        } else {
            format!("{}\n\n---\n\n", context_injection)
        },
        target_kind,
        target_title,
        target_version,
        target_description,
        target_tech_focus,
        milestone.title,
        milestone.goal,
        proj.version_plan.chars().take(1000).collect::<String>(),
        proj.project_path,
        project_facts,
        crate::workload_policy::render_planning_constraints(workload),
        max_subtasks,
        feedback_section,
    );
    let call_context = model_context(
        proj,
        crate::cost_ledger::ModelCallPurpose::ExecutionPlanGeneration,
    );
    let response = crate::api::call_deepseek_api_json_with_context(
        crate::prompts::EXECUTION_PLAN_PROMPT,
        &context,
        call_context.clone(),
    )
    .await
    .map_err(|error| format!("执行计划生成 AI 调用失败：{}", error))?;
    let mut repair_call_id = None;
    let mut tasks = match parse_execution_plan_tasks(
        &response.content,
        call_context.clone(),
        max_subtasks,
        workload,
    )
    .await
    {
        Ok(tasks) => tasks,
        Err(validation_error) => {
            let repair_context = format!(
                "{}\n\n上一次输出未满足执行计划契约：{}\n请完整重新输出修正后的 JSON 数组。",
                context, validation_error
            );
            let mut repair_context_metadata = call_context.clone();
            repair_context_metadata.purpose =
                Some(crate::cost_ledger::ModelCallPurpose::SchemaRepair);
            let repaired = crate::api::call_deepseek_api_json_with_context(
                crate::prompts::EXECUTION_PLAN_PROMPT,
                &repair_context,
                repair_context_metadata.clone(),
            )
            .await
            .map_err(|error| format!("执行计划修订 AI 调用失败：{}", error))?;
            repair_call_id = Some(repaired.metadata.call_id.clone());
            parse_execution_plan_tasks(
                &repaired.content,
                repair_context_metadata,
                max_subtasks,
                workload,
            )
            .await
            .map_err(|error| {
                format!(
                    "执行计划修订后仍不满足契约：{}（首次错误：{}）",
                    error, validation_error
                )
            })?
        }
    };
    let accepted_deviations = crate::project_facts::accepted_deviations(proj);
    for task in &mut tasks {
        let paths = crate::project_facts::snapshot_paths(task);
        task.fact_snapshot = Some(crate::project_facts::capture_with_identifiers(
            &proj.project_path,
            &paths,
            accepted_deviations.clone(),
            &task.required_identifiers,
        )?);
    }
    for call_id in std::iter::once(&response.metadata.call_id).chain(repair_call_id.iter()) {
        mark_model_output(
            &proj.name,
            call_id,
            crate::cost_ledger::ModelCallOutcome {
                produced_plan: true,
                produced_contract: true,
                produced_fact: true,
                ..Default::default()
            },
        );
    }
    Ok(tasks)
}

async fn parse_execution_plan_tasks(
    reply: &str,
    context: crate::cost_ledger::ModelCallContext,
    max_subtasks: u32,
    workload: &project::WorkloadProfile,
) -> Result<Vec<project::Subtask>, String> {
    let raw: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry_with_context(reply, context)
            .await
            .map_err(|error| format!("解析执行计划 JSON 失败：{}", error))?;
    validate_generated_count("小阶段", raw.len(), max_subtasks)?;

    let dependency_orders = raw
        .iter()
        .enumerate()
        .map(|(index, item)| {
            required_u32_array(
                item,
                "depends_on_orders",
                &format!("第 {} 个小阶段", index + 1),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut tasks = raw
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let entity = format!("第 {} 个小阶段", index + 1);
            let order = required_u32(item, "order", &entity)?;
            if order != index as u32 + 1 {
                return Err(format!(
                    "{}的 order 必须为 {}，实际为 {}",
                    entity,
                    index + 1,
                    order
                ));
            }
            let execution_prompt = required_string(item, "execution_prompt", &entity)?;
            let acceptance_criteria = required_string_array(item, "acceptance_criteria", &entity)?;
            let acceptance_criteria_meta =
                parse_acceptance_criteria_meta(item, &acceptance_criteria, &entity);
            Ok(project::Subtask {
                id: uuid::Uuid::new_v4().to_string(),
                title: required_string(item, "title", &entity)?,
                prompt: execution_prompt.clone(),
                status: project::SubtaskStatus::Pending,
                test_report: String::new(),
                execution_result: None,
                test_result: None,
                retry_count: 0,
                auto_tag: None,
                order,
                goal: required_string(item, "goal", &entity)?,
                allowed_file_paths: required_string_array(item, "allowed_file_paths", &entity)?,
                new_file_paths: string_array(item, "new_file_paths", &entity)?,
                evidence_files: string_array(item, "evidence_files", &entity)?,
                context_summary: required_string(item, "context_summary", &entity)?,
                acceptance_criteria,
                acceptance_criteria_meta,
                stop_rules: required_string_array(item, "stop_rules", &entity)?,
                execution_prompt,
                confirmed_by_user: None,
                confirmed_at: None,
                confirmation_notes: None,
                human_verification: None,
                required_identifiers: vec![],
                acceptance_ledger: vec![],
                fact_snapshot: None,
                plan_patch_revision: 0,
                depends_on: vec![],
                dependency_notes: required_string(item, "dependency_notes", &entity)?,
                contract_snapshot: None,
                child_tasks: vec![],
                expected_artifacts: vec![],
                related_symbols: vec![],
                read_file_paths: vec![],
                write_file_paths: vec![],
                split_basis: String::new(),
                independently_verifiable: false,
                future_parallel_safe: false,
                parent_criterion_indexes: vec![],
                aggregated_at: None,
                aggregation_source_task_ids: vec![],
                affected_deviation_criteria: vec![],
                aggregation_reason: String::new(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let task_ids = tasks
        .iter()
        .map(|task| (task.order, task.id.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (index, task) in tasks.iter_mut().enumerate() {
        task.depends_on = dependency_orders[index]
            .iter()
            .map(|dependency_order| {
                if *dependency_order >= task.order {
                    return Err(format!(
                        "第 {} 个小阶段的依赖顺序 {} 必须早于当前任务",
                        index + 1,
                        dependency_order
                    ));
                }
                task_ids.get(dependency_order).cloned().ok_or_else(|| {
                    format!(
                        "第 {} 个小阶段引用了不存在的依赖顺序 {}",
                        index + 1,
                        dependency_order
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    for task in &mut tasks {
        crate::plan_contract::hydrate_subtask_contract(task, workload);
    }
    crate::plan_contract::validate_subtasks(&tasks)?;
    Ok(tasks)
}

fn parse_acceptance_criteria_meta(
    item: &serde_json::Value,
    criteria: &[String],
    entity: &str,
) -> Vec<crate::provability::AcceptanceCriterion> {
    let declared = item
        .get("acceptance_criteria_meta")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|raw| {
                    let text = raw
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let provability = raw
                        .get("provability")
                        .and_then(serde_json::Value::as_str)
                        .and_then(parse_provability_label)
                        .unwrap_or(crate::provability::Provability::Unprovable);
                    crate::provability::AcceptanceCriterion {
                        text,
                        provability,
                        provability_source: crate::provability::ProvabilitySource::PlanningExplicit,
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let normalized = crate::provability::normalize_metadata(criteria, &declared);
    if declared.len() != criteria.len()
        || normalized.iter().any(|criterion| {
            criterion.provability_source == crate::provability::ProvabilitySource::SystemInferred
        })
    {
        eprintln!(
            "{}的验收可证明性标签缺失或偏乐观，已执行本地保守校准",
            entity
        );
    }
    normalized
}

fn parse_provability_label(value: &str) -> Option<crate::provability::Provability> {
    match value.trim().to_ascii_lowercase().as_str() {
        "deterministic" => Some(crate::provability::Provability::Deterministic),
        "automatedtest" | "automated_test" => Some(crate::provability::Provability::AutomatedTest),
        "semanticreview" | "semantic_review" => {
            Some(crate::provability::Provability::SemanticReview)
        }
        "humanreview" | "human_review" => Some(crate::provability::Provability::HumanReview),
        "unprovable" => Some(crate::provability::Provability::Unprovable),
        _ => None,
    }
}

/// 生成执行计划（V1：动态任务数量，精准上下文注入）
#[tauri::command]
pub(crate) async fn generate_execution_plan(
    project_name: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    if initial.workflow_state.current_step != project::WorkflowStep::PlanGeneration {
        return Err(format!(
            "当前步骤为 {:?}，首次生成只允许在 PlanGeneration 调用；检查或审批页面请使用 regenerate_execution_plan",
            initial.workflow_state.current_step
        ));
    }
    let milestone_id = initial.current_milestone_id.clone();
    let mid_stage_id = initial.current_mid_stage_id.clone();
    let initial_scope = PlanScope::resolve(&initial)?;
    let initial_revision = initial.workflow_state.data_revision;
    let initial_plan = initial.version_plan.clone();
    let subtasks = generate_execution_plan_tasks(&initial, None).await?;
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::PlanGeneration
        || proj.workflow_state.data_revision != initial_revision
        || proj.current_milestone_id != milestone_id
        || proj.current_mid_stage_id != mid_stage_id
        || proj.version_plan != initial_plan
    {
        return Err("生成期间项目事实已变化，未写入执行计划。请同步后重试。".to_string());
    }
    let scope = PlanScope::resolve(&proj)?;
    if scope.kind() != initial_scope.kind() || scope.has_execution_facts(&proj) {
        return Err("当前计划目标已有执行事实或拓扑已变化，禁止覆盖执行计划。".to_string());
    }
    scope.set_generated_plan(&mut proj, subtasks, chrono::Utc::now().to_rfc3339(), 0, 0);

    proj.workflow_state.current_step = project::WorkflowStep::PlanCheck;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

#[tauri::command]
pub(crate) async fn regenerate_execution_plan(
    project_name: String,
    expected_data_revision: u64,
    expected_plan_draft_revision: u64,
    feedback: String,
    source: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    let valid_source = matches!(
        (&initial.workflow_state.current_step, source.as_str()),
        (project::WorkflowStep::PlanCheck, "check_failed")
            | (project::WorkflowStep::PlanApproving, "approval_rejected")
    );
    if !valid_source {
        return Err(format!(
            "当前步骤 {:?} 与执行计划重新生成来源不匹配",
            initial.workflow_state.current_step
        ));
    }
    if initial.workflow_state.data_revision != expected_data_revision {
        return Err("项目修订号已变化，请同步后重试。".to_string());
    }
    let milestone_id = initial.current_milestone_id.clone();
    let mid_stage_id = initial.current_mid_stage_id.clone();
    let initial_scope = PlanScope::resolve(&initial)?;
    if initial_scope.plan_draft_revision(&initial) != expected_plan_draft_revision {
        return Err("执行计划草稿修订已变化，请同步后重试。".to_string());
    }
    if initial_scope.has_execution_facts(&initial) {
        return Err(
            "执行计划已有执行进度或稳定标签，禁止直接重新生成；请使用回退流程。".to_string(),
        );
    }
    let effective_feedback = if feedback.trim().is_empty() {
        initial_scope
            .plan_check_result(&initial)
            .map(plan_check_feedback)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "请提供执行计划重新生成反馈。".to_string())?
    } else {
        feedback.trim().to_string()
    };
    let initial_plan = initial.version_plan.clone();
    let old_regeneration_count = initial_scope.plan_regeneration_count(&initial);
    let old_no_progress_count = initial_scope.plan_no_progress_count(&initial);
    let subtasks = generate_execution_plan_tasks(&initial, Some(&effective_feedback)).await?;

    let mut latest = crate::load_project(&project_name)?;
    if latest.workflow_state.data_revision != expected_data_revision
        || latest.workflow_state.current_step != initial.workflow_state.current_step
        || latest.current_milestone_id != milestone_id
        || latest.current_mid_stage_id != mid_stage_id
        || latest.version_plan != initial_plan
    {
        return Err("生成期间项目选择或正式方案已变化，未覆盖原执行计划。".to_string());
    }
    let latest_scope = PlanScope::resolve(&latest)?;
    if latest_scope.kind() != initial_scope.kind()
        || latest_scope.plan_draft_revision(&latest) != expected_plan_draft_revision
        || latest_scope.has_execution_facts(&latest)
    {
        return Err("生成期间执行计划或执行事实已变化，未覆盖原计划。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    latest_scope.set_generated_plan(
        &mut latest,
        subtasks,
        now.clone(),
        old_regeneration_count.saturating_add(1),
        old_no_progress_count,
    );
    latest.workflow_state.current_step = project::WorkflowStep::PlanCheck;
    if let Some(autopilot) = latest.workflow_state.autopilot_state.as_mut() {
        if autopilot.recovery_action == project::AutopilotRecoveryAction::RegenerateExecutionPlan {
            autopilot.run_status = project::AutopilotRunStatus::Paused;
            autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            autopilot.error_message.clear();
            autopilot.last_action = "执行计划已重新生成，等待重新检查".to_string();
            autopilot.last_action_at = now.clone();
        }
    }
    latest.workflow_state.data_revision += 1;
    latest.workflow_state.last_transition_at = now;
    crate::save_and_reload_project(&latest)
}

/// 检查执行计划
#[tauri::command]
pub(crate) async fn check_stage_plan(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::PlanCheck {
        return Err(format!(
            "当前步骤 {:?} 不允许检查执行计划",
            proj.workflow_state.current_step
        ));
    }

    let max_subtasks = crate::workload_policy::current_profile(&proj)?.max_subtasks;
    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    let scope = PlanScope::resolve(&proj)?;
    let plan_draft_revision = scope.plan_draft_revision(&proj);

    let deterministic =
        crate::plan_deterministic_checks::check_execution_plan(scope.subtasks(&proj), max_subtasks);
    if !deterministic.is_empty() {
        let result = project::StagePlanCheckResult {
            passed: false,
            omissions: deterministic.omissions,
            out_of_scope: deterministic.out_of_scope,
            not_executable: deterministic.not_executable,
            suggestions: vec!["修复以上结构硬阻断后再运行语义检查。".to_string()],
            checked_at: chrono::Utc::now().to_rfc3339(),
        };
        let (fingerprint, issue_count, no_progress_count) =
            plan_check_tracking(scope, &proj, &result);
        scope.set_plan_check_result(
            &mut proj,
            result,
            fingerprint,
            issue_count,
            no_progress_count,
        );
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    let order_by_id = scope
        .subtasks(&proj)
        .iter()
        .map(|task| (task.id.as_str(), task.order))
        .collect::<std::collections::BTreeMap<_, _>>();
    let plan_text = scope
        .subtasks(&proj)
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let dependency_orders = st
                .depends_on
                .iter()
                .filter_map(|id| order_by_id.get(id.as_str()))
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}. {} — goal: {} — files: [{}] — new: [{}] — evidence: [{}] — criteria: [{}] — depends_on_orders: [{}] — dependency_notes: {} — implementation: {}",
                i + 1,
                st.title,
                st.goal,
                st.allowed_file_paths.join(", "),
                st.new_file_paths.join(", "),
                st.evidence_files.join(", "),
                st.acceptance_criteria.join("; "),
                dependency_orders,
                st.dependency_notes,
                st.execution_prompt,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let project_facts = crate::project_facts::planning_context(&proj)?;
    let milestone = scope.milestone(&proj);
    let (target_kind, target_title, target_description, target_tech_focus) =
        match scope.mid_stage(&proj) {
            Some(mid_stage) => (
                "中阶段",
                mid_stage.title.as_str(),
                mid_stage.description.as_str(),
                mid_stage.tech_focus.as_str(),
            ),
            None => (
                "大阶段直挂计划",
                milestone.title.as_str(),
                milestone.description.as_str(),
                milestone.tech_stack.as_str(),
            ),
        };

    let context = format!(
        "计划目标（{}）：{} — {}\n技术重点：{}\n\n当前项目事实（与计划生成使用相同的压缩扫描）：\n{}\n\n执行计划（{} 个小阶段）：\n{}",
        target_kind,
        target_title,
        target_description,
        target_tech_focus,
        project_facts,
        scope.subtasks(&proj).len(),
        plan_text
    );

    let call_context = model_context(
        &proj,
        crate::cost_ledger::ModelCallPurpose::ExecutionPlanCheck,
    );
    let response = crate::api::call_deepseek_api_json_with_context(
        crate::prompts::EXECUTION_PLAN_CHECK_PROMPT,
        &context,
        call_context.clone(),
    )
    .await
    .map_err(|e| format!("执行计划检查 AI 调用失败：{}", e))?;

    let check: ExecutionPlanCheckResponse =
        crate::json_utils::parse_json_with_contract_and_context(
            &response.content,
            &crate::json_utils::EXECUTION_PLAN_CHECK_JSON_CONTRACT,
            call_context,
        )
        .await
        .map_err(|error| format!("执行计划检查协议失败：{}", error))?;
    let result = check.into_result();
    let passed = result.passed;

    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::PlanCheck
        || proj.current_milestone_id != milestone_id
        || proj.current_mid_stage_id != mid_stage_id
    {
        return Err("当前项目已不在原计划检查目标。".to_string());
    }
    let latest_scope = PlanScope::resolve(&proj)?;
    if latest_scope.kind() != scope.kind()
        || latest_scope.plan_draft_revision(&proj) != plan_draft_revision
    {
        return Err("计划检查期间计划目标或草稿修订已变化，未写入检查结果。".to_string());
    }
    let (fingerprint, issue_count, no_progress_count) =
        plan_check_tracking(latest_scope, &proj, &result);
    latest_scope.set_plan_check_result(
        &mut proj,
        result,
        fingerprint,
        issue_count,
        no_progress_count,
    );

    proj.workflow_state.current_step = if passed {
        project::WorkflowStep::PlanApproving
    } else {
        project::WorkflowStep::PlanCheck
    };
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    let saved = crate::save_and_reload_project(&proj)?;
    mark_model_output(
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

/// 批准执行计划（写入 plan_revision 和批准时间）
#[tauri::command]
pub(crate) async fn approve_stage_plan(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::PlanApproving {
        return Err(format!(
            "当前步骤 {:?} 不允许批准执行计划",
            proj.workflow_state.current_step
        ));
    }

    let scope = PlanScope::resolve(&proj)?;
    if let Some(check) = scope.plan_check_result_mut(&mut proj) {
        *check = crate::autopilot_policy::normalize_plan_check_result(check.clone());
    }

    // Verify check passed
    match scope.plan_check_result(&proj) {
        Some(r) if r.passed => {}
        Some(_) => return Err("执行计划检查未通过，无法批准。".to_string()),
        None => return Err("执行计划尚未检查，请先运行检查。".to_string()),
    }

    if scope.subtasks(&proj).is_empty() {
        return Err("执行计划为空，无法批准。".to_string());
    }
    crate::plan_contract::validate_subtasks(scope.subtasks(&proj))
        .map_err(|error| format!("执行计划契约无效，无法批准：{}", error))?;

    let workspace = crate::pipeline::get_execution_workspace_status_inner(&proj.project_path)?;
    if !workspace.ready {
        return Err(format!(
            "Git 工作区尚未满足批准条件：{}",
            workspace.status_message
        ));
    }
    crate::plan_contract::validate_subtasks_in_project(scope.subtasks(&proj), &proj.project_path)
        .map_err(|error| format!("执行计划契约无效，无法批准：{}", error))?;

    // Idempotency: if already approved, ensure disk consistency
    if scope.plan_approved_at(&proj).is_some() && scope.plan_revision(&proj) > 0 {
        if proj.workflow_state.current_step == project::WorkflowStep::PlanApproving {
            // Repair stale step: migrate to Execution
            proj.workflow_state.current_step = project::WorkflowStep::Execution;
            proj.workflow_state.data_revision += 1;
            proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
            return crate::save_and_reload_project(&proj);
        }
        // 非修复路径也统一返回磁盘最终事实，不再返回未保存的内存对象
        return crate::save_and_reload_project(&proj);
    }

    let now = chrono::Utc::now().to_rfc3339();
    let plan_rev = proj.workflow_state.data_revision + 1;

    scope.approve_plan(&mut proj, now, plan_rev);

    // Transition to Execution — plan is now frozen, ready for execution
    proj.workflow_state.current_step = project::WorkflowStep::Execution;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

// ===================================================================
// V1 大阶段审阅 A/B/C 分支命令
// ===================================================================

/// 进入大阶段审阅（检测当前大阶段所有中阶段完成后由前端调用）
#[tauri::command]
pub(crate) async fn enter_milestone_review(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !matches!(
        proj.workflow_state.current_step,
        project::WorkflowStep::Execution
            | project::WorkflowStep::MilestoneSelection
            | project::WorkflowStep::Discussion
    ) {
        return Err(format!(
            "当前步骤 {:?} 不能直接进入 MilestoneReview。",
            proj.workflow_state.current_step
        ));
    }

    let milestone_id = proj.current_milestone_id.clone();
    let now = chrono::Utc::now().to_rfc3339();
    crate::workflow_resolution::apply_milestone_review_boundary(&mut proj, &milestone_id, &now)?;
    proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

async fn approve_milestone_outcome_state(
    project_name: String,
    branch: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneReview {
        return Err(format!(
            "当前步骤 {:?} 不是 MilestoneReview",
            proj.workflow_state.current_step
        ));
    }

    if !matches!(branch.as_str(), "A" | "B" | "C") {
        return Err(format!("未知分支：{}（仅支持 A/B/C）", branch));
    }

    let milestone_id = proj.current_milestone_id.clone();
    let now = chrono::Utc::now().to_rfc3339();
    let review_cycle_id = format!(
        "{}:{}",
        milestone_id,
        if proj.workflow_state.last_transition_at.is_empty() {
            now.as_str()
        } else {
            proj.workflow_state.last_transition_at.as_str()
        }
    );
    let current_idx = proj
        .milestones
        .iter()
        .position(|milestone| milestone.id == milestone_id)
        .ok_or("大阶段不存在。".to_string())?;
    let next_target = proj
        .milestones
        .iter()
        .skip(current_idx + 1)
        .find(|milestone| milestone.status != project::MilestoneStatus::Completed)
        .map(|milestone| (milestone.id.clone(), milestone.title.clone()));

    {
        let milestone = proj
            .milestones
            .get_mut(current_idx)
            .ok_or("大阶段不存在。".to_string())?;
        milestone.review_conclusion = Some(branch.clone());
        match branch.as_str() {
            "A" => {
                milestone.review_status = Some("approved".to_string());
                milestone.approved_at = Some(now.clone());
            }
            "B" => milestone.review_status = Some("needs_fix".to_string()),
            "C" => milestone.review_status = Some("future_adjusted".to_string()),
            _ => {}
        }
    }

    match branch.as_str() {
        "A" => match next_target {
            Some((next_id, next_title)) => {
                proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
                proj.workflow_state.review_node_id.clear();
                proj.current_milestone_id = next_id.clone();
                proj.current_mid_stage_id.clear();
                if proj.workflow_state.autopilot_active {
                    proj.workflow_state.autopilot_target_milestone_id = next_id.clone();
                    let autopilot = proj
                        .workflow_state
                        .autopilot_state
                        .get_or_insert_with(project::AutopilotState::default);
                    autopilot.active = true;
                    autopilot.target_milestone_id = next_id;
                    autopilot.run_status = project::AutopilotRunStatus::Running;
                    autopilot.last_action = format!("大阶段审阅通过，继续：{}", next_title);
                    autopilot.last_action_at = now.clone();
                    autopilot.error_message.clear();
                }
            }
            None => {
                proj.workflow_state.current_step = project::WorkflowStep::Completed;
                proj.workflow_state.top_level_phase = project::TopLevelPhase::Completed;
                proj.workflow_state.review_node_id.clear();
                proj.workflow_state.autopilot_active = false;
                proj.workflow_state.autopilot_target_milestone_id.clear();
                proj.workflow_state.autopilot_state = None;
                proj.current_mid_stage_id.clear();
            }
        },
        "B" => {
            proj.workflow_state.current_step = project::WorkflowStep::BranchDiscussion;
            proj.workflow_state.discussion_scope = project::DiscussionScope::FixPast;
            proj.activate_discussion_thread(
                project::DiscussionScope::FixPast,
                &milestone_id,
                &review_cycle_id,
            );
        }
        "C" => {
            proj.workflow_state.current_step = project::WorkflowStep::BranchDiscussion;
            proj.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
            proj.activate_discussion_thread(
                project::DiscussionScope::AdjustFuture,
                &milestone_id,
                &review_cycle_id,
            );
        }
        _ => {}
    }

    if matches!(branch.as_str(), "B" | "C") && proj.workflow_state.autopilot_active {
        let autopilot = proj
            .workflow_state
            .autopilot_state
            .get_or_insert_with(project::AutopilotState::default);
        autopilot.active = true;
        autopilot.target_milestone_id = milestone_id;
        autopilot.run_status = project::AutopilotRunStatus::Paused;
        autopilot.last_action = format!("大阶段审阅选择 {}，等待人工后续流程", branch);
        autopilot.last_action_at = now.clone();
        autopilot.error_message.clear();
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 大阶段审阅决策：A（继续）/ B（修正过去）/ C（调整未来）。
/// A 分支进入下一大阶段时重新接续后端自动驾驶作业。
#[tauri::command]
pub(crate) async fn approve_milestone_outcome(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    branch: String,
) -> Result<project::Project, String> {
    let project = approve_milestone_outcome_state(project_name.clone(), branch).await?;
    let should_start = project.workflow_state.autopilot_active
        && project
            .workflow_state
            .autopilot_state
            .as_ref()
            .is_some_and(|autopilot| autopilot.run_status == project::AutopilotRunStatus::Running);
    if should_start {
        state
            .autopilot_runtime
            .start(state.pipeline_state.clone(), project_name)
            .await?;
    }
    Ok(project)
}

/// B 分支：AI 生成回退建议（基于失败证据、测试结果、稳定标签、用户反馈）
#[tauri::command]
pub(crate) async fn suggest_rollback_checkpoint(project_name: String) -> Result<String, String> {
    let proj = crate::load_project(&project_name)?;

    if proj.workflow_state.discussion_scope != project::DiscussionScope::FixPast {
        return Err("当前不在 FixPast 讨论范围。".to_string());
    }

    let milestone_id = &proj.current_milestone_id;
    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;

    // Collect evidence
    let mut evidence = String::new();
    for mid in &ms.mid_stages {
        evidence.push_str(&format!("\n中阶段 {} ({}):\n", mid.title, mid.version));
        for st in &mid.subtasks {
            let status = match st.status {
                project::SubtaskStatus::Passed => "✅ 通过",
                project::SubtaskStatus::Rejected => "❌ 驳回",
                project::SubtaskStatus::AwaitingConfirmation => "⏳ 待确认",
                _ => "—",
            };
            evidence.push_str(&format!(
                "  - {} [{}] tag:{}\n",
                st.title,
                status,
                st.auto_tag.as_deref().unwrap_or("无")
            ));
            if let Some(ref t) = st.test_result {
                if !t.passed {
                    evidence.push_str(&format!("    测试失败：{}\n", t.suggestion));
                }
            }
        }
    }

    // Get branch discussion messages
    let discussion_thread = proj
        .active_discussion_thread()
        .filter(|thread| {
            thread.scope == project::DiscussionScope::FixPast
                && thread.status == project::DiscussionThreadStatus::Open
                && thread.milestone_id == proj.current_milestone_id
        })
        .ok_or("当前 FixPast 活动讨论线程不存在或作用域错误，请同步项目状态。")?;
    let discussion = discussion_thread
        .messages
        .iter()
        .map(|m| format!("[{}]: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let context = format!(
        "大阶段：{}\n\n执行证据：{}\n\n分支讨论：{}",
        ms.title, evidence, discussion
    );

    let response = crate::api::call_deepseek_api_inner_with_context(
        "你是一个项目诊断专家。根据大阶段执行证据和用户反馈，\
         分析应该回退到哪个稳定检查点，并给出理由。\
         输出纯文本（非 JSON），包含：\
         1. 推荐的检查点（任务名 + Git 标签）\
         2. 回退理由（引用失败证据）\
         3. 回退后需要重新执行的范围\
         长度 100-200 字。",
        &context,
        false,
        0.3,
        model_context(&proj, crate::cost_ledger::ModelCallPurpose::Decision),
    )
    .await
    .map_err(|e| format!("AI 调用失败：{}", e))?;

    mark_model_output(
        &project_name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_plan: true,
            ..Default::default()
        },
    );
    Ok(response.content)
}

fn active_future_discussion_thread(
    proj: &project::Project,
) -> Result<&project::DiscussionThread, String> {
    proj.active_discussion_thread()
        .filter(|thread| {
            thread.scope == project::DiscussionScope::AdjustFuture
                && thread.status == project::DiscussionThreadStatus::Open
                && thread.milestone_id == proj.current_milestone_id
        })
        .ok_or_else(|| {
            "当前 AdjustFuture 活动讨论线程不存在或作用域错误，请同步项目状态。".to_string()
        })
}

fn future_planning_fact_summary(
    proj: &project::Project,
    retained_milestone_ids: &[String],
) -> Result<String, String> {
    let retained_scope = retained_milestone_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let current_code_facts =
        crate::project_facts::planning_context_for_milestones(proj, &retained_scope)
            .map_err(|error| format!("无法读取未来规划所需的最新项目事实：{}", error))?;
    let completed_outcomes = proj
        .milestones
        .iter()
        .filter(|milestone| retained_scope.contains(&milestone.id))
        .map(|milestone| {
            format!(
                "- {} ({})：目标={}；预期输出={}",
                milestone.title,
                milestone.version,
                if milestone.goal.trim().is_empty() {
                    "未记录"
                } else {
                    milestone.goal.trim()
                },
                if milestone.expected_output.trim().is_empty() {
                    "未记录"
                } else {
                    milestone.expected_output.trim()
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let baseline_capabilities = proj
        .existing_baseline
        .as_ref()
        .map(|baseline| baseline.completed_capabilities.join("、"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "无额外基线能力记录".to_string());
    let baseline_unresolved = proj
        .existing_baseline
        .as_ref()
        .map(|baseline| {
            baseline
                .pending_capabilities
                .iter()
                .chain(baseline.risks.iter())
                .chain(baseline.uncertainties.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("、")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "无额外未解决基线问题".to_string());
    Ok(format!(
        "当前代码事实（压缩扫描，不含完整文件）：\n{}\n\n已完成大阶段事实：\n{}\n\n初始基线能力：{}\n基线未解决问题：{}",
        current_code_facts,
        if completed_outcomes.is_empty() {
            "无已完成大阶段记录"
        } else {
            &completed_outcomes
        },
        baseline_capabilities,
        baseline_unresolved,
    ))
}

fn validate_future_draft_source(
    proj: &project::Project,
    draft: &project::MilestoneDraft,
) -> Result<(), String> {
    if draft.expired {
        return Err(format!(
            "未来规划草稿已过期：{}。请重新生成。",
            draft
                .expiration_reason
                .as_deref()
                .unwrap_or("来源事实已变化")
        ));
    }
    if draft.source_thread_id.is_empty() {
        return Err("未来规划草稿缺少来源讨论线程，请重新生成。".to_string());
    }
    let thread = active_future_discussion_thread(proj)?;
    if thread.id != draft.source_thread_id {
        return Err("未来规划草稿的来源讨论线程已变化，请重新生成。".to_string());
    }
    if thread.revision != draft.source_thread_revision {
        return Err(format!(
            "未来规划讨论已更新（草稿修订 {}，当前修订 {}），请重新生成。",
            draft.source_thread_revision, thread.revision
        ));
    }
    let expected_revision = draft.source_data_revision.saturating_add(1);
    if draft.source_data_revision == 0 || proj.workflow_state.data_revision != expected_revision {
        return Err("项目事实在未来草稿生成后已变化，请重新生成。".to_string());
    }
    if draft.split_after_milestone_id.as_deref() != Some(proj.current_milestone_id.as_str()) {
        return Err("未来规划草稿的分割点已变化，请重新生成。".to_string());
    }
    let split_idx = proj
        .milestones
        .iter()
        .position(|milestone| milestone.id == proj.current_milestone_id)
        .ok_or_else(|| "当前大阶段不存在，请同步项目状态。".to_string())?;
    let retained_ids = proj.milestones[..=split_idx]
        .iter()
        .map(|milestone| milestone.id.clone())
        .collect::<Vec<_>>();
    if retained_ids != draft.retained_milestone_ids {
        return Err("未来规划草稿的保留大阶段已变化，请重新生成。".to_string());
    }
    Ok(())
}

/// C 分支：生成未来大阶段草稿（保留已完成，只生成后续）
#[tauri::command]
pub(crate) async fn generate_future_milestone_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;
    let workload = crate::workload_policy::current_profile(&proj)?;

    if proj.workflow_state.discussion_scope != project::DiscussionScope::AdjustFuture {
        return Err("当前不在 AdjustFuture 讨论范围。".to_string());
    }

    if !matches!(
        proj.workflow_state.current_step,
        project::WorkflowStep::BranchDiscussion | project::WorkflowStep::FuturePlanApproval
    ) {
        return Err("当前步骤不允许生成未来大阶段草稿。".to_string());
    }

    let source_thread = active_future_discussion_thread(&proj)?;
    let source_thread_id = source_thread.id.clone();
    let source_thread_revision = source_thread.revision;
    let source_data_revision = proj.workflow_state.data_revision;
    let source_step = proj.workflow_state.current_step.clone();
    let milestone_id = proj.current_milestone_id.clone();
    let split_idx = proj
        .milestones
        .iter()
        .position(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。")?;

    // Completed milestones (up to and including current)
    let completed: Vec<&project::Milestone> = proj.milestones[..=split_idx].iter().collect();

    // Build context
    let completed_titles: Vec<String> = completed
        .iter()
        .map(|m| format!("- {} ({})", m.title, m.version))
        .collect();
    let retained_ids: Vec<String> = completed.iter().map(|m| m.id.clone()).collect();
    let remaining_milestone_limit = workload
        .max_milestones
        .saturating_sub(retained_ids.len() as u32);
    if remaining_milestone_limit == 0 {
        return Err(format!(
            "当前画像最多允许 {} 个大阶段，已保留 {} 个，无法再生成未来大阶段。",
            workload.max_milestones,
            retained_ids.len()
        ));
    }
    let discussion = source_thread
        .messages
        .iter()
        .map(|m| format!("[{}]: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let context = format!(
        "项目方案：{}\n\n当前项目事实：\n{}\n\n已完成大阶段：\n{}\n\n讨论反馈：\n{}\n\n\
         只生成上述已完成大阶段之后的后续大阶段。已完成大阶段必须完全保留。\n\n{}\n必须生成 1..={} 个后续大阶段。",
        proj.version_plan,
        future_planning_fact_summary(&proj, &retained_ids)?,
        completed_titles.join("\n"),
        discussion,
        crate::workload_policy::render_planning_constraints(workload),
        remaining_milestone_limit,
    );

    let call_context = model_context(
        &proj,
        crate::cost_ledger::ModelCallPurpose::MilestoneGeneration,
    );
    let response = crate::api::call_deepseek_api_json_with_context(
        crate::prompts::MILESTONE_GENERATION_PROMPT,
        &context,
        call_context.clone(),
    )
    .await
    .map_err(|e| format!("AI 调用失败：{}", e))?;

    let raw: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry_with_context(&response.content, call_context)
            .await
            .map_err(|e| format!("解析失败：{}", e))?;

    validate_generated_count("后续大阶段", raw.len(), remaining_milestone_limit)?;
    let mut new_milestones: Vec<project::Milestone> = Vec::new();
    for r in &raw {
        new_milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: r["version"].as_str().unwrap_or("v0.0").to_string(),
            title: r["title"].as_str().unwrap_or("未命名").to_string(),
            description: r["description"].as_str().unwrap_or("").to_string(),
            tech_stack: r["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if workload.use_mid_stage_layer {
                project::StageMode::Professional
            } else {
                project::StageMode::Quick
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: r["goal"].as_str().unwrap_or("").to_string(),
            scope: r["scope"].as_str().unwrap_or("").to_string(),
            dependencies: arr_str(&r["dependencies"]),
            expected_output: r["expected_output"].as_str().unwrap_or("").to_string(),
            acceptance_criteria: arr_str(&r["acceptance_criteria"]),
            ..Default::default()
        });
    }

    // === 阶段五关键修复：版本归一化 ===
    // AI 输出的 version 仅作参考，系统基于最后一个保留阶段重新计算版本序列
    let last_retained_version = completed
        .last()
        .map(|m| m.version.clone())
        .unwrap_or_else(|| "v0.0".to_string());
    let normalized = normalize_future_versions(&last_retained_version, &new_milestones);
    if normalized.is_empty() {
        return Err("版本归一化失败：无法为未来大阶段生成唯一递增版本号。".to_string());
    }
    // Apply normalized versions
    for (i, ms) in new_milestones.iter_mut().enumerate() {
        if i < normalized.len() {
            ms.version = normalized[i].clone();
        }
    }

    // Collect metadata
    let future_ids: Vec<String> = new_milestones.iter().map(|m| m.id.clone()).collect();
    let ai_versions: Vec<String> = raw
        .iter()
        .map(|r| r["version"].as_str().unwrap_or("v0.0").to_string())
        .collect();

    // === 阶段六：数量守恒检查 ===
    // 计算分割点之后原有的大阶段数量（被替换的部分）
    let original_remaining = proj.milestones.len().saturating_sub(split_idx + 1);
    let new_count = new_milestones.len();
    let count_expansion = new_count > original_remaining.saturating_mul(3) / 2
        && new_count.saturating_sub(original_remaining) > 1;

    // === 阶段六：粒度一致性检查 ===
    let mut granularity_issues: Vec<String> = Vec::new();
    for (i, fm) in new_milestones.iter().enumerate() {
        if fm.goal.is_empty() && fm.description.is_empty() {
            granularity_issues.push(format!(
                "未来大阶段 #{}「{}」缺少目标和描述，可能为空壳阶段。",
                i + 1,
                fm.title
            ));
        }
        if fm.scope.is_empty() {
            granularity_issues.push(format!(
                "未来大阶段 #{}「{}」缺少范围边界，粒度可能不足。",
                i + 1,
                fm.title
            ));
        }
        if fm.acceptance_criteria.is_empty() {
            granularity_issues.push(format!(
                "未来大阶段 #{}「{}」缺少验收标准。",
                i + 1,
                fm.title
            ));
        }
    }
    let granularity_ok = granularity_issues.is_empty();

    // Save only if the selected discussion and project facts still match the
    // snapshot used for model generation.
    let mut latest = crate::load_project(&project_name)?;
    let latest_thread = active_future_discussion_thread(&latest)?;
    let latest_retained_ids = latest
        .milestones
        .iter()
        .take(split_idx + 1)
        .map(|milestone| milestone.id.clone())
        .collect::<Vec<_>>();
    if latest.workflow_state.current_step != source_step
        || latest.workflow_state.data_revision != source_data_revision
        || latest.current_milestone_id != milestone_id
        || latest_thread.id != source_thread_id
        || latest_thread.revision != source_thread_revision
        || latest_retained_ids != retained_ids
    {
        return Err("未来草稿生成期间讨论或项目事实已变化，未保存旧上下文结果。".to_string());
    }
    let previous_draft = latest
        .milestone_draft
        .as_ref()
        .filter(|draft| draft.draft_kind == project::MilestoneDraftKind::FutureOnly);
    let draft = project::MilestoneDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        status: project::MilestoneDraftStatus::Pending,
        draft_kind: project::MilestoneDraftKind::FutureOnly,
        candidate_milestones: new_milestones,
        check_result: None,
        generation_revision: source_thread_revision,
        source_plan_revision: source_data_revision,
        source_thread_id,
        source_thread_revision,
        source_data_revision,
        expired: false,
        expiration_reason: None,
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved_at: None,
        regeneration_count: previous_draft
            .map(|draft| draft.regeneration_count.saturating_add(1))
            .unwrap_or(0),
        previous_draft_id: previous_draft.map(|draft| draft.draft_id.clone()),
        last_regeneration_reason: None,
        last_regenerated_at: None,
        split_after_milestone_id: Some(milestone_id),
        retained_milestone_ids: retained_ids,
        future_candidate_ids: future_ids,
        original_ai_versions: ai_versions,
        normalized_versions: normalized,
        versions_normalized: true,
        original_remaining_count: Some(original_remaining),
        new_future_count: Some(new_count),
        count_expansion_warning: count_expansion,
        granularity_check_passed: granularity_ok,
        granularity_issues,
    };
    latest.milestone_draft = Some(draft);
    latest.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
    latest.workflow_state.data_revision += 1;
    latest.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_project(&latest)?;
    mark_model_output(
        &project_name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_plan: true,
            ..Default::default()
        },
    );
    crate::load_project(&project_name)
}

/// C 分支：批准未来大阶段（替换正式 future milestones）
#[tauri::command]
pub(crate) async fn approve_future_milestones(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::FuturePlanApproval {
        return Err("当前不在 FuturePlanApproval 步骤。".to_string());
    }

    let draft = proj
        .milestone_draft
        .as_ref()
        .ok_or("没有未来大阶段草稿。".to_string())?;

    // === 阶段五关键修复：批准前一致性校验 ===
    if draft.draft_kind != project::MilestoneDraftKind::FutureOnly {
        return Err("当前草稿不是 FutureOnly 类型，请使用普通大阶段批准流程。".to_string());
    }
    if draft.status != project::MilestoneDraftStatus::Pending {
        return Err("当前未来规划草稿不是待审批状态。".to_string());
    }
    validate_future_draft_source(&proj, draft)?;
    if draft.split_after_milestone_id.is_none() {
        return Err("未来规划草稿缺少分割点元数据，请重新生成。".to_string());
    }
    if draft.retained_milestone_ids.is_empty() {
        return Err("未来规划草稿缺少保留阶段列表，请重新生成。".to_string());
    }
    if draft.candidate_milestones.is_empty() {
        return Err("未来候选大阶段为空，无法批准。".to_string());
    }
    if !draft.versions_normalized {
        return Err("未来规划版本未归一化，请重新生成草稿。".to_string());
    }

    // === 阶段六：粒度校验 — 有空壳阶段时拒绝批准 ===
    if !draft.granularity_check_passed && !draft.granularity_issues.is_empty() {
        return Err(format!(
            "未来规划粒度校验未通过：\n{}\n\n请返回讨论补充信息后重新生成。",
            draft.granularity_issues.join("\n")
        ));
    }

    // === 阶段六：数量膨胀预警 — 不阻断批准，但记录原因 ===
    if draft.count_expansion_warning {
        let orig = draft.original_remaining_count.unwrap_or(0);
        let new = draft.new_future_count.unwrap_or(0);
        eprintln!(
            "[future_milestones] 数量膨胀预警：原剩余 {} 个大阶段，新生成 {} 个。请确认用户是否明确要求扩展范围。",
            orig, new
        );
    }

    // Verify no completed milestone appears in future candidates
    let retained_set: std::collections::HashSet<&str> = draft
        .retained_milestone_ids
        .iter()
        .map(|s| s.as_str())
        .collect();
    for fm in &draft.candidate_milestones {
        if retained_set.contains(fm.id.as_str()) {
            return Err(format!(
                "校验失败：未来候选大阶段「{}」({}) 与保留阶段冲突。请重新生成草稿。",
                fm.title, fm.version
            ));
        }
    }

    // Verify future versions are unique and don't duplicate retained versions
    let retained_versions: std::collections::HashSet<String> = proj
        .milestones
        .iter()
        .filter(|m| retained_set.contains(m.id.as_str()))
        .map(|m| m.version.clone())
        .collect();
    let mut seen_versions: std::collections::HashSet<String> = retained_versions.clone();
    for fm in &draft.candidate_milestones {
        if seen_versions.contains(&fm.version) {
            return Err(format!(
                "版本冲突：未来大阶段「{}」版本 {} 与已有阶段重复。请重新生成草稿。",
                fm.title, fm.version
            ));
        }
        seen_versions.insert(fm.version.clone());
    }

    let milestone_id = &proj.current_milestone_id;
    let split_idx = proj
        .milestones
        .iter()
        .position(|m| m.id == *milestone_id)
        .unwrap_or(0);

    // Keep past milestones, replace future ones
    let past: Vec<project::Milestone> = proj.milestones[..=split_idx].iter().cloned().collect();
    let future = draft.candidate_milestones.clone();

    proj.milestones = past;
    proj.milestones.extend(future);

    if let Some(ref mut d) = proj.milestone_draft {
        d.status = project::MilestoneDraftStatus::Approved;
        d.approved_at = Some(chrono::Utc::now().to_rfc3339());
    }

    proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
    proj.close_active_discussion_thread();
    proj.workflow_state.discussion_scope = project::DiscussionScope::FirstDiscussion;
    proj.current_milestone_id.clear();
    proj.current_mid_stage_id.clear();
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// Helper: extract string array from JSON value
fn arr_str(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 版本归一化：基于最后一个保留阶段的版本，为未来阶段生成唯一递增版本序列。
///
/// 规则：
/// 1. 解析 last_retained_version（如 "v0.3" → major=0, minor=3）
/// 2. 从 minor+1 开始，为每个未来阶段分配递增版本
/// 3. 返回与 future_milestones 等长的版本号列表
fn normalize_future_versions(
    last_retained_version: &str,
    future_milestones: &[project::Milestone],
) -> Vec<String> {
    let n = future_milestones.len();
    if n == 0 {
        return vec![];
    }

    // Parse last retained version like "v0.3" → (0, 3)
    let (major, mut minor) = parse_version(last_retained_version);

    let mut versions = Vec::with_capacity(n);
    for _ in 0..n {
        minor += 1;
        versions.push(format!("v{}.{}", major, minor));
    }
    versions
}

/// Parse a version string like "v0.3" or "v1.2.3" into (major, minor).
/// Falls back to (0, 0) on parse failure.
fn parse_version(v: &str) -> (u32, u32) {
    let v = v.trim_start_matches('v').trim_start_matches('V');
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 2 {
        let major = parts[0].parse::<u32>().unwrap_or(0);
        let minor = parts[1].parse::<u32>().unwrap_or(0);
        (major, minor)
    } else if parts.len() == 1 {
        (parts[0].parse::<u32>().unwrap_or(0), 0)
    } else {
        (0, 0)
    }
}

/// 大阶段完成后的 AI 自然语言总结
///
/// 基于大阶段的执行统计数据（中阶段完成情况、测试通过率、Git 标签等），
/// 调用 AI 生成一段自然语言总结和下一步建议。
/// 纯文本输出，与第一层前端统计表格配合使用。
///
/// 1. 加载项目，定位目标 milestone
/// 2. 收集中阶段/子任务统计数据
/// 3. 调用 AI 生成自然语言总结
/// 4. 返回纯文本总结
#[tauri::command]
pub(crate) async fn summarize_milestone(
    project_name: String,
    milestone_id: String,
) -> Result<String, String> {
    // 1. 加载项目
    let project = crate::load_project(&project_name)?;

    // 2. 定位目标 milestone
    let milestone = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or(format!("未找到指定的大阶段: {}", milestone_id))?;

    let title = &milestone.title;
    let version = &milestone.version;

    // 3. 收集中阶段统计数据
    let mid_stages = &milestone.mid_stages;
    let total_mid_stages = mid_stages.len();
    let completed_count = mid_stages
        .iter()
        .filter(|ms| ms.status == project::MidStageStatus::Completed)
        .count();
    let failed_count = mid_stages
        .iter()
        .filter(|ms| ms.status == project::MidStageStatus::Rejected)
        .count();

    // Git 标签列表
    let tags: Vec<&str> = mid_stages
        .iter()
        .filter_map(|ms| {
            if ms.git_tag.is_empty() {
                None
            } else {
                Some(ms.git_tag.as_str())
            }
        })
        .collect();
    let tags_line = if tags.is_empty() {
        "无".to_string()
    } else {
        tags.join("、")
    };

    // 4. 收集子任务测试通过率
    let mut total_subtasks: usize = 0;
    let mut passed_subtasks: usize = 0;
    for mid in mid_stages {
        for st in &mid.subtasks {
            total_subtasks += 1;
            if let Some(ref test_result) = st.test_result {
                if test_result.passed {
                    passed_subtasks += 1;
                }
            }
        }
    }
    let pass_rate = if total_subtasks > 0 {
        format!(
            "{}%（{}/{}）",
            ((passed_subtasks as f64 / total_subtasks as f64) * 100.0).round() as u32,
            passed_subtasks,
            total_subtasks
        )
    } else {
        "N/A".to_string()
    };

    // 5. 项目剩余大阶段数
    let milestone_idx = project
        .milestones
        .iter()
        .position(|m| m.id == milestone_id)
        .unwrap_or(0);
    let remaining = project.milestones.len().saturating_sub(milestone_idx + 1);

    // 6. 构造 user message
    let user_message = format!(
        "大阶段：{}（{}）\n\n\
         中阶段统计：\n\
         - 总数：{}\n\
         - 已完成：{}\n\
         - 失败：{}\n\
         - Git 标签：{}\n\n\
         子任务测试通过率：{}\n\n\
         项目剩余大阶段数：{} 个",
        title,
        version,
        total_mid_stages,
        completed_count,
        failed_count,
        tags_line,
        pass_rate,
        remaining
    );

    // 7. 调用 AI（纯文本模式，低 temperature = 0.3，语气中性）
    let response = crate::api::call_deepseek_api_inner_with_context(
        crate::prompts::SUMMARIZE_MILESTONE_PROMPT,
        &user_message,
        false,
        0.3,
        model_context(
            &project,
            crate::cost_ledger::ModelCallPurpose::MilestoneCheck,
        ),
    )
    .await
    .map_err(|e| format!("AI 调用失败: {}", e))?;

    mark_model_output(
        &project_name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_evidence: true,
            ..Default::default()
        },
    );
    Ok(response.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct ProjectDataGuard {
        path: PathBuf,
    }

    impl ProjectDataGuard {
        fn new(project_name: &str) -> Result<Self, String> {
            Ok(Self {
                path: crate::project_data_path(project_name)?,
            })
        }
    }

    impl Drop for ProjectDataGuard {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_file(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("清理测试项目 {} 失败：{}", self.path.display(), error);
                }
            }
        }
    }

    fn unique_project_name(label: &str) -> String {
        format!("test-{}-{}", label, uuid::Uuid::new_v4())
    }

    #[test]
    fn provability_closeout_plan_labels_are_checked_and_missing_labels_are_inferred() {
        let raw = serde_json::json!({
            "acceptance_criteria_meta": [
                {"text": "视觉表现与打磨前一致", "provability": "SemanticReview"},
                {"text": "cargo test 测试通过", "provability": "AutomatedTest"}
            ]
        });
        let criteria = vec![
            "视觉表现与打磨前一致".to_string(),
            "cargo test 测试通过".to_string(),
            "令人满意的最终结果".to_string(),
        ];
        let metadata = parse_acceptance_criteria_meta(&raw, &criteria, "测试任务");
        assert_eq!(metadata.len(), criteria.len());
        assert_eq!(
            metadata[0].provability,
            crate::provability::Provability::HumanReview
        );
        assert_eq!(
            metadata[1].provability,
            crate::provability::Provability::AutomatedTest
        );
        assert_eq!(
            metadata[2].provability_source,
            crate::provability::ProvabilitySource::SystemInferred
        );
    }

    fn completed_mid_stage() -> project::MidStage {
        project::MidStage {
            id: "mid-1".to_string(),
            title: "已完成中阶段".to_string(),
            version: "v0.1.1".to_string(),
            order: Some(1),
            status: project::MidStageStatus::Completed,
            subtasks: vec![project::Subtask {
                id: "subtask-1".to_string(),
                status: project::SubtaskStatus::Passed,
                ..Default::default()
            }],
            domain: None,
            test_log: None,
            created_at: String::new(),
            description: String::new(),
            tech_focus: String::new(),
            test_report: String::new(),
            completed_at: Some("2026-07-20T00:00:00Z".to_string()),
            approved_at: None,
            git_tag: String::new(),
            plan_check_result: None,
            plan_approved_at: None,
            plan_revision: 0,
            plan_draft_revision: 0,
            plan_generated_at: None,
            plan_regeneration_count: 0,
            last_plan_failure_fingerprint: String::new(),
            last_plan_issue_count: 0,
            plan_no_progress_count: 0,
        }
    }

    fn professional_workload_profile() -> project::WorkloadProfile {
        crate::workload_policy::classify(
            project::WorkloadSignals {
                has_frontend: true,
                has_backend: true,
                has_persistence: false,
                has_auth_or_roles: false,
                external_integration_count: 0,
                independent_domain_count: 3,
                deliverable_count: 3,
                high_risk: false,
            },
            None,
            0,
        )
        .expect("professional test profile")
    }

    fn test_milestone(
        id: &str,
        title: &str,
        status: project::MilestoneStatus,
    ) -> project::Milestone {
        project::Milestone {
            id: id.to_string(),
            version: "v0.1".to_string(),
            title: title.to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status,
            mode: project::StageMode::Professional,
            mid_stages: vec![completed_mid_stage()],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
            ..Default::default()
        }
    }

    fn review_project(project_name: &str, with_next: bool) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(professional_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
        proj.workflow_state.review_node_id = "milestone-1".to_string();
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_target_milestone_id = "milestone-1".to_string();
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::WaitingMilestoneReview,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut current = test_milestone(
            "milestone-1",
            "当前大阶段",
            project::MilestoneStatus::Completed,
        );
        current.review_status = Some("pending_review".to_string());
        proj.milestones.push(current);
        if with_next {
            proj.milestones.push(test_milestone(
                "milestone-2",
                "下一大阶段",
                project::MilestoneStatus::Pending,
            ));
        }
        proj
    }

    fn quick_completed_review_project(project_name: &str) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::Small,
        ));
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.workflow_state.data_revision = 6;
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        proj.current_milestone_id = "milestone-1".to_string();
        proj.milestones.push(project::Milestone {
            id: "milestone-1".to_string(),
            title: "Quick completed".to_string(),
            status: project::MilestoneStatus::Completed,
            mode: project::StageMode::Quick,
            subtasks: [
                project::SubtaskStatus::Passed,
                project::SubtaskStatus::AcceptedDeviation,
                project::SubtaskStatus::Skipped,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, status)| project::Subtask {
                id: format!("quick-task-{}", index + 1),
                status,
                ..Default::default()
            })
            .collect(),
            review_status: Some("approved".to_string()),
            review_conclusion: Some("A".to_string()),
            ..Default::default()
        });
        proj
    }

    fn milestone_draft(status: project::MilestoneDraftStatus) -> project::MilestoneDraft {
        project::MilestoneDraft {
            status,
            candidate_milestones: vec![test_milestone(
                "milestone-draft-1",
                "候选大阶段",
                project::MilestoneStatus::Pending,
            )],
            check_result: Some("检查通过".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn mid_stage_initial_contract_old_json_defaults_to_full_list() -> Result<(), String> {
        let mut value = serde_json::to_value(project::MidStageDraft::default())
            .map_err(|error| error.to_string())?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| "中阶段草稿未序列化为对象".to_string())?;
        for field in [
            "purpose",
            "base_mid_stage_revision",
            "retained_mid_stage_ids",
            "source_step",
            "allow_full_replacement",
        ] {
            object.remove(field);
        }
        let restored: project::MidStageDraft =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        assert_eq!(
            restored.purpose,
            project::MidStageDraftPurpose::InitialFullList
        );
        assert_eq!(restored.base_mid_stage_revision, 0);
        assert!(restored.retained_mid_stage_ids.is_empty());
        assert_eq!(
            restored.source_step,
            project::WorkflowStep::MidStageGeneration
        );
        assert!(restored.allow_full_replacement);
        Ok(())
    }

    #[tokio::test]
    async fn mid_stage_initial_contract_rejects_existing_pending_list() -> Result<(), String> {
        let project_name = unique_project_name("initial-mid-stage-existing");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::MidStageApproval;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "已有中阶段的大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages[0].status = project::MidStageStatus::Pending;
        milestone.mid_stages[0].completed_at = None;
        let existing_id = milestone.mid_stages[0].id.clone();
        let mut candidate = completed_mid_stage();
        candidate.id = "replacement".to_string();
        candidate.status = project::MidStageStatus::Pending;
        candidate.completed_at = None;
        proj.milestones.push(milestone);
        proj.mid_stage_draft = Some(project::MidStageDraft {
            milestone_id: "milestone-1".to_string(),
            candidate_mid_stages: vec![candidate],
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let error = approve_mid_stage_draft(project_name.clone())
            .await
            .expect_err("已有 Pending 中阶段时必须拒绝整表替换");
        assert!(error.contains("选择或恢复既有中阶段"));
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(persisted.milestones[0].mid_stages[0].id, existing_id);
        Ok(())
    }

    #[tokio::test]
    async fn mid_stage_initial_contract_repeated_approval_is_idempotent() -> Result<(), String> {
        let project_name = unique_project_name("initial-mid-stage-idempotent");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
        proj.workflow_state.data_revision = 7;
        proj.mid_stage_draft = Some(project::MidStageDraft {
            status: project::MidStageDraftStatus::Approved,
            approved_at: Some("2026-07-30T00:00:00Z".to_string()),
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let approved = approve_mid_stage_draft(project_name).await?;
        assert_eq!(approved.workflow_state.data_revision, 7);
        assert_eq!(
            approved.workflow_state.current_step,
            project::WorkflowStep::MidStageSelection
        );
        Ok(())
    }

    #[tokio::test]
    async fn workflow_closure_e2e_existing_mid_stage_continues_without_new_draft(
    ) -> Result<(), String> {
        let project_name = unique_project_name("existing-mid-stage-e2e");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workload_profile = Some(professional_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "已有中阶段的大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages[0].id = "mid-completed".to_string();
        milestone.mid_stages[0].order = Some(1);
        let mut pending = completed_mid_stage();
        pending.id = "mid-next".to_string();
        pending.title = "下一个既有中阶段".to_string();
        pending.order = Some(2);
        pending.status = project::MidStageStatus::Pending;
        pending.completed_at = None;
        milestone.mid_stages.push(pending);
        proj.milestones.push(milestone);
        crate::save_project(&proj)?;

        let continued = continue_current_milestone(project_name).await?;
        assert_eq!(continued.current_mid_stage_id, "mid-next");
        assert_eq!(
            continued.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );
        assert!(continued.mid_stage_draft.is_none());
        assert_eq!(continued.milestones[0].mid_stages.len(), 2);
        assert_eq!(
            continued.milestones[0].mid_stages[0].status,
            project::MidStageStatus::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_continue_quick_completed_builds_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("continue-quick-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = quick_completed_review_project(&project_name);
        let before_revision = proj.workflow_state.data_revision;
        crate::save_project(&proj)?;

        let updated = continue_current_milestone(project_name).await?;
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(updated.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            updated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(updated.milestones[0].review_conclusion.is_none());
        assert_eq!(updated.workflow_state.data_revision, before_revision + 1);
        assert_eq!(
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("Quick Review 缺少 autopilot 边界".to_string())?
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_continue_professional_completed_builds_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("continue-professional-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.workflow_state.data_revision = 7;
        proj.milestones[0].review_status = Some("needs_fix".to_string());
        proj.milestones[0].review_conclusion = Some("B".to_string());
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::Running;
        }
        crate::save_project(&proj)?;

        let updated = continue_current_milestone(project_name).await?;
        assert_eq!(updated.workflow_state.data_revision, 8);
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(updated.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            updated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(updated.milestones[0].review_conclusion.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_select_last_completed_mid_stage_builds_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("select-final-mid-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
        proj.workflow_state.data_revision = 11;
        proj.milestones[0].review_status = Some("approved".to_string());
        proj.milestones[0].review_conclusion = Some("A".to_string());
        let mut final_stage = completed_mid_stage();
        final_stage.id = "mid-2".to_string();
        final_stage.subtasks[0].id = "subtask-2".to_string();
        proj.milestones[0].mid_stages.push(final_stage);
        crate::save_project(&proj)?;

        let updated = select_mid_stage(project_name, "mid-2".to_string()).await?;
        assert_eq!(updated.workflow_state.data_revision, 12);
        assert_eq!(updated.current_mid_stage_id, "mid-2");
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(updated.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            updated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(updated.milestones[0].review_conclusion.is_none());
        Ok(())
    }

    fn execution_plan_json(second_dependencies: serde_json::Value) -> String {
        serde_json::json!([
            {
                "order": 1,
                "title": "建立状态",
                "goal": "建立状态模型",
                "allowed_file_paths": ["src/main.ts"],
                "new_file_paths": [],
                "evidence_files": ["src/main.ts"],
                "context_summary": "读取当前状态模型并保持现有字段兼容，先建立后续交互所需的最小状态接口。",
                "acceptance_criteria": ["提供 `loadState` 函数"],
                "stop_rules": ["发现范围外变更时停止"],
                "execution_prompt": "实现状态模型",
                "depends_on_orders": [],
                "dependency_notes": "首个任务没有前置依赖"
            },
            {
                "order": 2,
                "title": "绑定交互",
                "goal": "绑定状态交互",
                "allowed_file_paths": ["src/main.ts"],
                "new_file_paths": [],
                "evidence_files": ["src/main.ts"],
                "context_summary": "使用上一任务建立的状态接口绑定交互，并保持已有事件初始化顺序不变。",
                "acceptance_criteria": ["调用 `loadState`"],
                "stop_rules": ["缺少状态接口时停止"],
                "execution_prompt": "绑定交互",
                "depends_on_orders": second_dependencies,
                "dependency_notes": "依赖第一个任务建立状态接口"
            }
        ])
        .to_string()
    }

    #[test]
    fn future_draft_source_requires_same_thread_revision_and_project_facts() {
        let mut proj = review_project("future-source", true);
        proj.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
        proj.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
        proj.workflow_state.data_revision = 10;
        let thread_id = proj.activate_discussion_thread(
            project::DiscussionScope::AdjustFuture,
            "milestone-1",
            "cycle-1",
        );
        let draft = project::MilestoneDraft {
            draft_kind: project::MilestoneDraftKind::FutureOnly,
            source_thread_id: thread_id.clone(),
            source_thread_revision: 0,
            source_data_revision: 9,
            split_after_milestone_id: Some("milestone-1".to_string()),
            retained_milestone_ids: vec!["milestone-1".to_string()],
            ..Default::default()
        };
        validate_future_draft_source(&proj, &draft).expect("matching future source");

        proj.discussion_threads
            .iter_mut()
            .find(|thread| thread.id == thread_id)
            .expect("future thread")
            .revision = 1;
        let error = validate_future_draft_source(&proj, &draft)
            .expect_err("thread revision changes must expire approval");
        assert!(error.contains("讨论已更新"));
    }

    #[test]
    fn future_planning_context_uses_current_code_and_nested_deviations() {
        let root = std::env::temp_dir().join(format!(
            "metheus-future-planning-facts-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("feature.rs"),
            "fn live_future_planning_capability() {}",
        )
        .unwrap();

        let mut proj = review_project("future-current-facts", true);
        proj.project_path = root.to_string_lossy().to_string();
        proj.milestones[0].goal = "交付已验证能力".to_string();
        proj.milestones[0].expected_output = "可运行的当前实现".to_string();
        let mut deviation = project::Subtask {
            id: "nested-deviation".to_string(),
            title: "动态叶子偏差".to_string(),
            ..Default::default()
        };
        deviation.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "保留当前兼容边界".to_string(),
            verified_at: String::new(),
            original_test_failure: String::new(),
            resolution: project::HumanResolution::AcceptDeviation,
            accepted_criteria: vec![1],
            dependency_check: String::new(),
            action_source: String::new(),
            execution_result_fingerprint: String::new(),
            task_tree_revision: 0,
            project_revision: 0,
        });
        let mut parent = project::Subtask {
            id: "dynamic-parent".to_string(),
            title: "动态父任务".to_string(),
            ..Default::default()
        };
        parent.child_tasks = vec![deviation];
        proj.milestones[0].mid_stages[0].subtasks = vec![parent];

        let mut future_deviation = project::Subtask {
            id: "future-deviation".to_string(),
            title: "未执行未来偏差".to_string(),
            status: project::SubtaskStatus::AcceptedDeviation,
            ..Default::default()
        };
        future_deviation.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "未来条件".to_string(),
            status: project::AcceptanceStatus::AcceptedDeviation,
            ..Default::default()
        }];
        proj.milestones[1].subtasks = vec![future_deviation];

        let summary = future_planning_fact_summary(&proj, &["milestone-1".to_string()])
            .expect("当前项目事实应可用于未来规划");
        assert!(summary.contains("live_future_planning_capability"));
        assert!(summary.contains("动态叶子偏差"));
        assert!(summary.contains("保留当前兼容边界"));
        assert!(summary.contains("交付已验证能力"));
        assert!(summary.contains("可运行的当前实现"));
        assert!(!summary.contains("未执行未来偏差"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn future_planning_context_fails_closed_without_current_project_facts() {
        let mut proj = review_project("future-facts-unavailable", true);
        proj.project_path = std::env::temp_dir()
            .join(format!("missing-future-facts-{}", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .to_string();

        let error = future_planning_fact_summary(&proj, &["milestone-1".to_string()])
            .expect_err("事实扫描不可用时不得退回旧基线摘要");
        assert!(error.contains("无法读取未来规划所需的最新项目事实"));
    }

    #[tokio::test]
    async fn workflow_closure_e2e_future_discussion_requires_regeneration() -> Result<(), String> {
        let project_name = unique_project_name("future-closure-e2e");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = review_project(&project_name, true);
        crate::save_project(&proj)?;

        let discussion =
            approve_milestone_outcome_state(project_name.clone(), "C".to_string()).await?;
        let thread = discussion
            .active_discussion_thread()
            .filter(|thread| thread.scope == project::DiscussionScope::AdjustFuture)
            .ok_or_else(|| "C 分支未创建专属讨论线程".to_string())?;
        let thread_id = thread.id.clone();
        let thread_revision = thread.revision;
        let source_data_revision = discussion.workflow_state.data_revision;

        let mut generated = discussion;
        let mut future = test_milestone(
            "milestone-future",
            "重新规划的未来大阶段",
            project::MilestoneStatus::Pending,
        );
        future.version = "v0.2".to_string();
        generated.milestone_draft = Some(project::MilestoneDraft {
            status: project::MilestoneDraftStatus::Pending,
            draft_kind: project::MilestoneDraftKind::FutureOnly,
            candidate_milestones: vec![future],
            source_thread_id: thread_id.clone(),
            source_thread_revision: thread_revision,
            source_data_revision,
            split_after_milestone_id: Some("milestone-1".to_string()),
            retained_milestone_ids: vec!["milestone-1".to_string()],
            future_candidate_ids: vec!["milestone-future".to_string()],
            normalized_versions: vec!["v0.2".to_string()],
            versions_normalized: true,
            granularity_check_passed: true,
            ..Default::default()
        });
        generated.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
        generated.workflow_state.data_revision += 1;
        crate::save_project(&generated)?;

        let mut continued = crate::load_project(&project_name)?;
        let active = continued
            .discussion_threads
            .iter_mut()
            .find(|thread| thread.id == thread_id)
            .ok_or_else(|| "未来讨论线程丢失".to_string())?;
        active.revision += 1;
        crate::commands::chat::invalidate_future_milestone_draft(&mut continued);
        continued.workflow_state.data_revision += 1;
        crate::save_project(&continued)?;

        let stale_error = approve_future_milestones(project_name.clone())
            .await
            .expect_err("继续讨论后的旧草稿必须拒绝批准");
        assert!(stale_error.contains("过期") || stale_error.contains("修订"));

        let mut regenerated = crate::load_project(&project_name)?;
        let latest_thread_revision = regenerated
            .active_discussion_thread()
            .ok_or_else(|| "活动未来讨论线程丢失".to_string())?
            .revision;
        let regeneration_source_revision = regenerated.workflow_state.data_revision;
        let draft = regenerated
            .milestone_draft
            .as_mut()
            .ok_or_else(|| "未来草稿丢失".to_string())?;
        draft.source_thread_revision = latest_thread_revision;
        draft.source_data_revision = regeneration_source_revision;
        draft.expired = false;
        draft.expiration_reason = None;
        regenerated.workflow_state.data_revision += 1;
        crate::save_project(&regenerated)?;

        let approved = approve_future_milestones(project_name).await?;
        assert_eq!(approved.milestones.len(), 2);
        assert_eq!(approved.milestones[0].id, "milestone-1");
        assert_eq!(approved.milestones[1].id, "milestone-future");
        assert_eq!(
            approved.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(approved
            .workflow_state
            .active_discussion_thread_id
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn execution_plan_maps_dependency_orders_to_stable_ids() -> Result<(), String> {
        let tasks = parse_execution_plan_tasks(
            &execution_plan_json(serde_json::json!([1])),
            crate::cost_ledger::ModelCallContext::default(),
            8,
            &crate::workload_policy::test_profile(project::WorkloadScale::System),
        )
        .await?;
        assert_eq!(tasks[1].depends_on, vec![tasks[0].id.clone()]);
        assert!(!tasks[1].dependency_notes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn execution_plan_rejects_forward_dependencies() {
        let result = parse_execution_plan_tasks(
            &execution_plan_json(serde_json::json!([2])),
            crate::cost_ledger::ModelCallContext::default(),
            8,
            &crate::workload_policy::test_profile(project::WorkloadScale::System),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execution_plan_rejects_profile_count_overflow() {
        let result = parse_execution_plan_tasks(
            &execution_plan_json(serde_json::json!([1])),
            crate::cost_ledger::ModelCallContext::default(),
            1,
            &crate::workload_policy::test_profile(project::WorkloadScale::Micro),
        )
        .await;
        assert!(result
            .unwrap_err()
            .contains("小阶段数量超出工作负载画像上限：实际 2，上限 1"));
    }

    #[test]
    fn adaptive_execution_contract_generation_count_is_profile_bounded() {
        assert!(validate_generated_count("大阶段", 1, 1).is_ok());
        assert!(validate_generated_count("大阶段", 0, 3)
            .unwrap_err()
            .contains("至少需要 1 个"));
        assert!(validate_generated_count("大阶段", 4, 3)
            .unwrap_err()
            .contains("实际 4，上限 3"));
    }

    #[test]
    fn milestone_check_result_clears_a_previous_failure_on_pass() -> Result<(), String> {
        let mut proj = project::Project::new("milestone-check-transition");
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
        proj.milestone_draft = Some(milestone_draft(project::MilestoneDraftStatus::CheckFailed));

        apply_milestone_check_result(&mut proj, true, "复检通过".to_string())?;

        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneApproval
        );
        let draft = proj
            .milestone_draft
            .as_ref()
            .ok_or("草稿缺失".to_string())?;
        assert_eq!(draft.status, project::MilestoneDraftStatus::CheckPassed);
        assert_eq!(draft.check_result.as_deref(), Some("复检通过"));
        Ok(())
    }

    #[tokio::test]
    async fn milestone_approval_requires_check_passed() -> Result<(), String> {
        for status in [
            project::MilestoneDraftStatus::Pending,
            project::MilestoneDraftStatus::CheckFailed,
        ] {
            let project_name = unique_project_name("milestone-approval-rejected");
            let _guard = ProjectDataGuard::new(&project_name)?;
            let mut proj = project::Project::new(&project_name);
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
            proj.milestone_draft = Some(milestone_draft(status));
            crate::save_project(&proj)?;

            assert!(approve_milestone_draft(project_name).await.is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn milestone_approval_atomically_releases_managed_flow() -> Result<(), String> {
        let project_name = unique_project_name("milestone-approval-managed");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
        proj.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            ..Default::default()
        });
        proj.milestone_draft = Some(milestone_draft(project::MilestoneDraftStatus::CheckPassed));
        crate::save_project(&proj)?;

        let updated = approve_milestone_draft(project_name).await?;
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(updated.workflow_state.managed_flow_state.is_none());
        assert_eq!(
            updated.milestone_draft.as_ref().map(|draft| &draft.status),
            Some(&project::MilestoneDraftStatus::Approved)
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_entering_review_persists_single_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("enter-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.workflow_state.review_node_id.clear();
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::Running;
        }
        crate::save_project(&proj)?;
        let before_revision = proj.workflow_state.data_revision;

        let updated = enter_milestone_review(project_name).await?;
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(updated.workflow_state.review_node_id, "milestone-1");
        let milestone = updated
            .milestones
            .first()
            .ok_or("进入审阅后大阶段缺失".to_string())?;
        assert_eq!(milestone.status, project::MilestoneStatus::Completed);
        assert_eq!(milestone.review_status.as_deref(), Some("pending_review"));
        assert!(milestone.review_conclusion.is_none());
        assert_eq!(
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("进入审阅后自动驾驶状态缺失".to_string())?
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        assert_eq!(updated.workflow_state.data_revision, before_revision + 1);
        assert_eq!(
            updated.workflow_state.last_transition_at,
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("自动驾驶状态缺失".to_string())?
                .last_action_at
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_entering_review_rejects_illegal_source_without_save(
    ) -> Result<(), String> {
        let project_name = unique_project_name("enter-review-illegal-source");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::PlanGeneration;
        proj.workflow_state.data_revision = 9;
        proj.milestones[0].status = project::MilestoneStatus::InProgress;
        proj.milestones[0].review_status = Some("approved".to_string());
        crate::save_project(&proj)?;

        let error = enter_milestone_review(project_name.clone())
            .await
            .expect_err("illegal source must fail");
        assert!(error.contains("不能直接进入"));
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(persisted.workflow_state.data_revision, 9);
        assert_eq!(
            persisted.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );
        assert_eq!(
            persisted.milestones[0].status,
            project::MilestoneStatus::InProgress
        );
        assert_eq!(
            persisted.milestones[0].review_status.as_deref(),
            Some("approved")
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_discussion_reentry_resets_review_and_preserves_thread(
    ) -> Result<(), String> {
        let project_name = unique_project_name("enter-review-discussion");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::Discussion;
        proj.workflow_state.data_revision = 4;
        proj.milestones[0].review_status = Some("needs_fix".to_string());
        proj.milestones[0].review_conclusion = Some("B".to_string());
        proj.discussion_threads.push(project::DiscussionThread {
            id: "review-thread".to_string(),
            title: "审阅讨论".to_string(),
            node_id: "milestone-1".to_string(),
            milestone_id: "milestone-1".to_string(),
            scope: project::DiscussionScope::FixPast,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let updated = enter_milestone_review(project_name).await?;
        assert_eq!(updated.workflow_state.data_revision, 5);
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(
            updated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(updated.milestones[0].review_conclusion.is_none());
        assert!(updated
            .discussion_threads
            .iter()
            .any(|thread| thread.id == "review-thread"));
        Ok(())
    }

    #[tokio::test]
    async fn branch_a_selects_next_target_and_resumes_autopilot() -> Result<(), String> {
        let project_name = unique_project_name("review-a-next");
        let _guard = ProjectDataGuard::new(&project_name)?;
        crate::save_project(&review_project(&project_name, true))?;

        let updated = approve_milestone_outcome_state(project_name, "A".to_string()).await?;
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert_eq!(updated.current_milestone_id, "milestone-2");
        assert!(updated.current_mid_stage_id.is_empty());
        assert_eq!(
            updated.workflow_state.autopilot_target_milestone_id,
            "milestone-2"
        );
        let autopilot = updated
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("A 分支继续后自动驾驶状态缺失".to_string())?;
        assert_eq!(autopilot.target_milestone_id, "milestone-2");
        assert_eq!(autopilot.run_status, project::AutopilotRunStatus::Running);
        assert_eq!(
            updated.milestones[0].review_status.as_deref(),
            Some("approved")
        );
        Ok(())
    }

    #[tokio::test]
    async fn final_branch_a_completes_project_and_closes_autopilot() -> Result<(), String> {
        let project_name = unique_project_name("review-a-final");
        let _guard = ProjectDataGuard::new(&project_name)?;
        crate::save_project(&review_project(&project_name, false))?;

        let updated = approve_milestone_outcome_state(project_name, "A".to_string()).await?;
        assert_eq!(
            updated.workflow_state.current_step,
            project::WorkflowStep::Completed
        );
        assert_eq!(
            updated.workflow_state.top_level_phase,
            project::TopLevelPhase::Completed
        );
        assert!(!updated.workflow_state.autopilot_active);
        assert!(updated.workflow_state.autopilot_state.is_none());
        assert!(updated
            .workflow_state
            .autopilot_target_milestone_id
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn branches_b_and_c_pause_autopilot_and_reject_duplicate_submit() -> Result<(), String> {
        let cases = [
            ("B", "needs_fix", project::DiscussionScope::FixPast),
            (
                "C",
                "future_adjusted",
                project::DiscussionScope::AdjustFuture,
            ),
        ];

        for (branch, review_status, scope) in cases {
            let project_name = unique_project_name(&format!("review-{}", branch));
            let _guard = ProjectDataGuard::new(&project_name)?;
            crate::save_project(&review_project(&project_name, true))?;

            let updated =
                approve_milestone_outcome_state(project_name.clone(), branch.to_string()).await?;
            assert_eq!(
                updated.workflow_state.current_step,
                project::WorkflowStep::BranchDiscussion
            );
            assert_eq!(updated.workflow_state.discussion_scope, scope);
            let active_thread = updated
                .active_discussion_thread()
                .ok_or("B/C 分支活动线程缺失".to_string())?;
            assert_ne!(active_thread.id, "thread-init");
            assert_eq!(active_thread.scope, scope);
            assert_eq!(active_thread.milestone_id, "milestone-1");
            assert_eq!(active_thread.status, project::DiscussionThreadStatus::Open);
            assert!(!active_thread.review_cycle_id.is_empty());
            assert_eq!(
                updated.milestones[0].review_status.as_deref(),
                Some(review_status)
            );
            assert_eq!(
                updated
                    .workflow_state
                    .autopilot_state
                    .as_ref()
                    .ok_or("B/C 分支后自动驾驶状态缺失".to_string())?
                    .run_status,
                project::AutopilotRunStatus::Paused
            );

            let duplicate = approve_milestone_outcome_state(project_name, branch.to_string()).await;
            assert!(duplicate.is_err());
        }
        Ok(())
    }

    async fn assert_branch_discussion_reenters_complete_review(
        branch: &str,
        scope: project::DiscussionScope,
    ) -> Result<(), String> {
        let project_name = unique_project_name(&format!("review-{}-reentry", branch));
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut seeded = review_project(&project_name, false);
        crate::pipeline::write_execution_history_with_source(
            &mut seeded,
            "info",
            project::ExecutionEventType::SystemAdvance,
            project::OperationSource::System,
            "审阅前历史事件".to_string(),
            Some("milestone-1"),
            None,
            None,
        );
        crate::save_project(&seeded)?;

        let branched =
            approve_milestone_outcome_state(project_name.clone(), branch.to_string()).await?;
        assert_eq!(branched.workflow_state.discussion_scope, scope);
        let thread_id = branched
            .active_discussion_thread()
            .ok_or("B/C 分支活动线程缺失".to_string())?
            .id
            .clone();
        let discussion = crate::commands::workflow::transition_workflow(
            project_name.clone(),
            "Discussion".to_string(),
            format!("test: {} 分支继续讨论", branch),
        )
        .await?;
        assert_eq!(
            discussion.workflow_state.current_step,
            project::WorkflowStep::Discussion
        );
        let before_review_revision = discussion.workflow_state.data_revision;

        let reviewed = crate::commands::workflow::transition_workflow(
            project_name,
            "MilestoneReview".to_string(),
            format!("test: {} 分支重返审阅", branch),
        )
        .await?;
        assert_eq!(
            reviewed.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(
            reviewed.workflow_state.top_level_phase,
            project::TopLevelPhase::Console
        );
        assert_eq!(
            reviewed.workflow_state.data_revision,
            before_review_revision + 1
        );
        assert_eq!(reviewed.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            reviewed.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(reviewed.milestones[0].review_conclusion.is_none());
        assert!(reviewed
            .discussion_threads
            .iter()
            .any(|thread| thread.id == thread_id));
        assert!(reviewed
            .execution_history
            .iter()
            .any(|entry| entry.text == "审阅前历史事件"));
        assert_eq!(
            reviewed
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("重返 Review 缺少 autopilot 边界".to_string())?
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_branch_b_discussion_reenters_complete_review(
    ) -> Result<(), String> {
        assert_branch_discussion_reenters_complete_review("B", project::DiscussionScope::FixPast)
            .await
    }

    #[tokio::test]
    async fn adaptive_execution_contract_branch_c_discussion_reenters_complete_review(
    ) -> Result<(), String> {
        assert_branch_discussion_reenters_complete_review(
            "C",
            project::DiscussionScope::AdjustFuture,
        )
        .await
    }

    #[tokio::test]
    async fn adaptive_execution_contract_plain_discussion_without_milestone_rejects_review(
    ) -> Result<(), String> {
        let project_name = unique_project_name("plain-discussion-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::Small,
        ));
        proj.workflow_state.current_step = project::WorkflowStep::Discussion;
        proj.workflow_state.data_revision = 5;
        crate::save_project(&proj)?;

        let error = crate::commands::workflow::transition_workflow(
            project_name.clone(),
            "MilestoneReview".to_string(),
            "test: 普通讨论不得旁路进入审阅".to_string(),
        )
        .await
        .expect_err("missing milestone facts must block Review");
        assert!(error.contains("未选择大阶段"));
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(persisted.workflow_state.data_revision, 5);
        assert_eq!(
            persisted.workflow_state.current_step,
            project::WorkflowStep::Discussion
        );
        Ok(())
    }
}
