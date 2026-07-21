use crate::project;

// ===================================================================
// V1 Console 命令：大阶段草稿 → 检查 → 批准 → 选择
// 这些命令替代旧的 generate_milestones（前端传参）路径。
// ===================================================================

const MILESTONE_REGEN_SOURCE_CHECK_FAILED: &str = "check_failed";
const MILESTONE_REGEN_SOURCE_APPROVAL_REJECTED: &str = "approval_rejected";

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

async fn generate_milestone_candidates(
    proj: &project::Project,
    regeneration_feedback: Option<&str>,
) -> Result<Vec<project::Milestone>, String> {
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

    let system_prompt = format!(
        "{}\n\n当前项目模式：Professional。输出的每个大阶段应包含 mid_stages 字段（空列表）和 subtasks 字段（空列表）。",
        crate::prompts::MILESTONE_GENERATION_PROMPT
    );
    // Inject context: working constitution, approved plan, discussion, Already constitution
    let context_injection = crate::constitution_context::build_context_injection(&proj);
    let augmented_user_message = if context_injection.is_empty() {
        user_message
    } else {
        format!("{}\n\n{}", context_injection, user_message)
    };
    let content =
        crate::api::call_deepseek_api_inner(&system_prompt, &augmented_user_message, false, 0.5)
            .await?;

    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|error| format!("解析大阶段 JSON 失败：{}", error))?;
    if raw_milestones.is_empty() {
        return Err("AI 返回的大阶段列表为空，请重新生成。".to_string());
    }

    raw_milestones
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
                mode: project::StageMode::Professional,
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
            })
        })
        .collect()
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
    let check_result_str = match crate::api::call_deepseek_api_json(
        crate::prompts::MILESTONE_CHECK_PROMPT,
        &check_context,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("大阶段检查 AI 调用失败：{}", e));
        }
    };

    let check_json: serde_json::Value = serde_json::from_str(&check_result_str)
        .map_err(|e| format!("解析检查结果 JSON 失败：{}", e))?;

    let passed = check_json["passed"]
        .as_bool()
        .ok_or("检查结果缺少 'passed' 字段")?;
    let summary = check_json
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| "检查结果缺少有效 summary 字段".to_string())?
        .to_string();

    // 6. 重新加载并保存结果
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneCheck {
        return Err("当前项目已不在大阶段检查步骤，请刷新页面。".to_string());
    }

    if let Some(ref mut draft) = proj.milestone_draft {
        draft.check_result = Some(summary.clone());
    }

    if passed {
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
    } else {
        // 检查未通过 → 留在 MilestoneCheck，更新草稿状态
        if let Some(ref mut draft) = proj.milestone_draft {
            draft.status = project::MilestoneDraftStatus::CheckFailed;
        }
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
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

    // 3. 验证检查已通过（状态不能是 CheckFailed）
    if draft.status == project::MilestoneDraftStatus::CheckFailed {
        return Err("大阶段草稿检查未通过，无法批准。请根据检查反馈调整后重新生成。".to_string());
    }
    if draft.check_result.is_none() {
        return Err("大阶段草稿尚未经过检查，请先运行检查。".to_string());
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
    ];
    if !valid_steps.contains(&proj.workflow_state.current_step) {
        return Err(format!(
            "当前步骤为 {:?}，不能在此步骤选择大阶段",
            proj.workflow_state.current_step
        ));
    }

    // 2. 验证 milestone_id 存在于正式 milestones 中
    let milestone_exists = proj.milestones.iter().any(|m| m.id == milestone_id);
    if !milestone_exists {
        return Err(format!("大阶段 {} 不在正式大阶段列表中", milestone_id));
    }

    // 3. 持久化选择
    proj.current_milestone_id = milestone_id.clone();
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
        if context_injection.is_empty() { String::new() } else { format!("{}\n\n", context_injection) },
        if context_injection.is_empty() { String::new() } else { "---\n\n".to_string() },
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
    let reply =
        crate::api::call_deepseek_api_json(crate::prompts::MID_STAGE_GENERATION_PROMPT, &context)
            .await
            .map_err(|error| format!("中阶段生成 AI 调用失败：{}", error))?;
    let raw: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|error| format!("解析中阶段 JSON 失败：{}", error))?;
    if raw.is_empty() {
        return Err("AI 返回的中阶段列表为空，请重新生成。".to_string());
    }

    raw.iter()
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
            })
        })
        .collect()
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
    let milestone = initial
        .milestones
        .iter()
        .find(|milestone| milestone.id == initial.current_milestone_id)
        .ok_or_else(|| "当前大阶段不存在。".to_string())?;
    if milestone.mid_stages.iter().any(|mid_stage| {
        matches!(
            mid_stage.status,
            project::MidStageStatus::InProgress | project::MidStageStatus::Completed
        )
    }) {
        return Err("当前大阶段已有执行中或已完成的中阶段，禁止重新生成。".to_string());
    }
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
    let candidates =
        generate_mid_stage_candidates(&initial, &milestone_id, Some(&effective_feedback)).await?;

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
    let latest_milestone = latest
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "生成期间当前大阶段已不存在。".to_string())?;
    if latest_milestone.mid_stages.iter().any(|mid_stage| {
        matches!(
            mid_stage.status,
            project::MidStageStatus::InProgress | project::MidStageStatus::Completed
        )
    }) {
        return Err("生成期间出现了中阶段执行事实，未覆盖原草稿。".to_string());
    }

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

    let reply =
        crate::api::call_deepseek_api_json(crate::prompts::MID_STAGE_CHECK_PROMPT, &context)
            .await
            .map_err(|e| format!("中阶段检查 AI 调用失败：{}", e))?;

    let check: serde_json::Value =
        serde_json::from_str(&reply).map_err(|e| format!("解析检查结果失败：{}", e))?;

    let passed = check["passed"].as_bool().ok_or("缺少 passed 字段")?;
    let summary = check
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| "中阶段检查结果缺少有效 summary 字段".to_string())?
        .to_string();

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
    }

    proj.workflow_state.current_step = if passed {
        project::WorkflowStep::MidStageApproval
    } else {
        project::WorkflowStep::MidStageCheck // 留在检查步骤
    };
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 批准中阶段草稿（复制候选到正式中阶段列表）
#[tauri::command]
pub(crate) async fn approve_mid_stage_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

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
            "该大阶段已有执行中或已完成的中阶段，禁止替换。请通过回退流程修改。".to_string(),
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

    let milestone_id = &proj.current_milestone_id;
    if milestone_id.is_empty() {
        return Err("请先选择一个大阶段。".to_string());
    }

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。".to_string())?;

    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不在当前大阶段中。".to_string())?;

    proj.current_mid_stage_id = mid_stage_id.clone();

    // === 阶段二关键修复：根据中阶段已有事实动态决定下一步 ===
    // 不再固定跳转到 PlanGeneration，而是根据中阶段状态智能判断。
    let next_step = if mid.status == project::MidStageStatus::Completed {
        // 中阶段已完成 — 检查大阶段是否全部完成
        let all_done = ms
            .mid_stages
            .iter()
            .all(|m| m.status == project::MidStageStatus::Completed);
        if all_done {
            project::WorkflowStep::MilestoneReview
        } else {
            // 中阶段已完成但大阶段还有未完成的 — 留在选择页，提示用户选下一个
            project::WorkflowStep::MidStageSelection
        }
    } else if has_plan_execution_facts(mid) {
        // 已有执行事实（执行过小阶段）→ 回到 Execution
        project::WorkflowStep::Execution
    } else if mid.plan_approved_at.is_some() && mid.plan_revision > 0 {
        // 执行计划已批准但尚未执行 → Execution
        project::WorkflowStep::Execution
    } else if mid.plan_check_result.as_ref().is_some_and(|c| c.passed) {
        // 检查已通过但尚未批准 → PlanApproving
        project::WorkflowStep::PlanApproving
    } else if mid.plan_check_result.is_some() {
        // 检查未通过 → 回到 PlanCheck
        project::WorkflowStep::PlanCheck
    } else if mid.plan_generated_at.is_some() {
        // 有计划但未检查 → PlanCheck
        project::WorkflowStep::PlanCheck
    } else {
        // 没有任何计划 → 需要生成执行计划
        project::WorkflowStep::PlanGeneration
    };

    let now = chrono::Utc::now().to_rfc3339();
    proj.workflow_state.current_step = next_step;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

// ===================================================================
// V1 执行计划命令：编译 → 检查 → 批准
// ===================================================================

fn has_plan_execution_facts(mid_stage: &project::MidStage) -> bool {
    !mid_stage.git_tag.is_empty()
        || mid_stage.subtasks.iter().any(|subtask| {
            matches!(
                subtask.status,
                project::SubtaskStatus::Executing
                    | project::SubtaskStatus::AwaitingConfirmation
                    | project::SubtaskStatus::Passed
            ) || subtask.auto_tag.as_ref().is_some_and(|tag| !tag.is_empty())
        })
}

fn plan_check_feedback(result: &project::StagePlanCheckResult) -> String {
    [
        ("遗漏", &result.omissions),
        ("越界", &result.out_of_scope),
        ("不可执行", &result.not_executable),
        ("建议", &result.suggestions),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .map(|(label, items)| format!("{}：{}", label, items.join("；")))
    .collect::<Vec<_>>()
    .join("\n")
}

async fn generate_execution_plan_tasks(
    proj: &project::Project,
    milestone_id: &str,
    mid_stage_id: &str,
    regeneration_feedback: Option<&str>,
) -> Result<Vec<project::Subtask>, String> {
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "大阶段不存在。".to_string())?;
    let mid_stage = milestone
        .mid_stages
        .iter()
        .find(|mid_stage| mid_stage.id == mid_stage_id)
        .ok_or_else(|| "中阶段不存在。".to_string())?;
    let feedback_section = regeneration_feedback
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
        .map_or_else(String::new, |feedback| {
            format!("\n\n重新生成反馈：\n{}", feedback)
        });
    let context_injection = crate::constitution_context::build_context_injection(proj);
    let context = format!(
        "{}中阶段：{} ({})\n描述：{}\n技术重点：{}\n\n所属大阶段：{} — {}\n\
         项目方案摘要（仅相关部分）：\n{}\n\n项目路径：{}\n\
         已有文件（仅作参考，不得无差别注入）：\n（由执行器在运行时按 evidence_files 精确读取）{}",
        if context_injection.is_empty() {
            String::new()
        } else {
            format!("{}\n\n---\n\n", context_injection)
        },
        mid_stage.title,
        mid_stage.version,
        mid_stage.description,
        mid_stage.tech_focus,
        milestone.title,
        milestone.goal,
        proj.version_plan.chars().take(1000).collect::<String>(),
        proj.project_path,
        feedback_section,
    );
    let reply = crate::api::call_deepseek_api_json(crate::prompts::EXECUTION_PLAN_PROMPT, &context)
        .await
        .map_err(|error| format!("执行计划生成 AI 调用失败：{}", error))?;
    match parse_execution_plan_tasks(&reply).await {
        Ok(tasks) => Ok(tasks),
        Err(validation_error) => {
            let repair_context = format!(
                "{}\n\n上一次输出未满足执行计划契约：{}\n请完整重新输出修正后的 JSON 数组。",
                context, validation_error
            );
            let repaired = crate::api::call_deepseek_api_json(
                crate::prompts::EXECUTION_PLAN_PROMPT,
                &repair_context,
            )
            .await
            .map_err(|error| format!("执行计划修订 AI 调用失败：{}", error))?;
            parse_execution_plan_tasks(&repaired)
                .await
                .map_err(|error| {
                    format!(
                        "执行计划修订后仍不满足契约：{}（首次错误：{}）",
                        error, validation_error
                    )
                })
        }
    }
}

async fn parse_execution_plan_tasks(reply: &str) -> Result<Vec<project::Subtask>, String> {
    let raw: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|error| format!("解析执行计划 JSON 失败：{}", error))?;
    if raw.is_empty() {
        return Err("AI 返回的执行计划为空，请重新生成。".to_string());
    }

    let tasks = raw
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let entity = format!("第 {} 个小阶段", index + 1);
            let execution_prompt = required_string(item, "execution_prompt", &entity)?;
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
                order: (index + 1) as u32,
                goal: required_string(item, "goal", &entity)?,
                allowed_file_paths: required_string_array(item, "allowed_file_paths", &entity)?,
                new_file_paths: string_array(item, "new_file_paths", &entity)?,
                evidence_files: string_array(item, "evidence_files", &entity)?,
                context_summary: required_string(item, "context_summary", &entity)?,
                acceptance_criteria: required_string_array(item, "acceptance_criteria", &entity)?,
                stop_rules: required_string_array(item, "stop_rules", &entity)?,
                execution_prompt,
                confirmed_by_user: None,
                confirmed_at: None,
                confirmation_notes: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    crate::plan_contract::validate_subtasks(&tasks)?;
    Ok(tasks)
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
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        return Err("请先选择大阶段和中阶段。".to_string());
    }
    let initial_revision = initial.workflow_state.data_revision;
    let initial_plan = initial.version_plan.clone();
    let subtasks =
        generate_execution_plan_tasks(&initial, &milestone_id, &mid_stage_id, None).await?;
    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::PlanGeneration
        || proj.workflow_state.data_revision != initial_revision
        || proj.current_milestone_id != milestone_id
        || proj.current_mid_stage_id != mid_stage_id
        || proj.version_plan != initial_plan
    {
        return Err("生成期间项目事实已变化，未写入执行计划。请同步后重试。".to_string());
    }
    let ms = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。".to_string())?;
    let mid = ms
        .mid_stages
        .iter_mut()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不存在。".to_string())?;
    if has_plan_execution_facts(mid) {
        return Err("当前中阶段已有执行事实，禁止覆盖执行计划。".to_string());
    }
    mid.subtasks = subtasks;
    mid.plan_check_result = None;
    mid.plan_approved_at = None;
    mid.plan_revision = 0;
    mid.plan_draft_revision += 1;
    mid.plan_generated_at = Some(chrono::Utc::now().to_rfc3339());

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
    let milestone = initial
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "当前大阶段不存在。".to_string())?;
    let mid_stage = milestone
        .mid_stages
        .iter()
        .find(|mid_stage| mid_stage.id == mid_stage_id)
        .ok_or_else(|| "当前中阶段不存在。".to_string())?;
    if mid_stage.plan_draft_revision != expected_plan_draft_revision {
        return Err("执行计划草稿修订已变化，请同步后重试。".to_string());
    }
    if has_plan_execution_facts(mid_stage) {
        return Err(
            "执行计划已有执行进度或稳定标签，禁止直接重新生成；请使用回退流程。".to_string(),
        );
    }
    let effective_feedback = if feedback.trim().is_empty() {
        mid_stage
            .plan_check_result
            .as_ref()
            .map(plan_check_feedback)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "请提供执行计划重新生成反馈。".to_string())?
    } else {
        feedback.trim().to_string()
    };
    let initial_plan = initial.version_plan.clone();
    let old_regeneration_count = mid_stage.plan_regeneration_count;
    let subtasks = generate_execution_plan_tasks(
        &initial,
        &milestone_id,
        &mid_stage_id,
        Some(&effective_feedback),
    )
    .await?;

    let mut latest = crate::load_project(&project_name)?;
    if latest.workflow_state.data_revision != expected_data_revision
        || latest.workflow_state.current_step != initial.workflow_state.current_step
        || latest.current_milestone_id != milestone_id
        || latest.current_mid_stage_id != mid_stage_id
        || latest.version_plan != initial_plan
    {
        return Err("生成期间项目选择或正式方案已变化，未覆盖原执行计划。".to_string());
    }
    let latest_milestone = latest
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| "生成期间当前大阶段已不存在。".to_string())?;
    let latest_mid_stage = latest_milestone
        .mid_stages
        .iter_mut()
        .find(|mid_stage| mid_stage.id == mid_stage_id)
        .ok_or_else(|| "生成期间当前中阶段已不存在。".to_string())?;
    if latest_mid_stage.plan_draft_revision != expected_plan_draft_revision
        || has_plan_execution_facts(latest_mid_stage)
    {
        return Err("生成期间执行计划或执行事实已变化，未覆盖原计划。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    latest_mid_stage.subtasks = subtasks;
    latest_mid_stage.plan_check_result = None;
    latest_mid_stage.plan_approved_at = None;
    latest_mid_stage.plan_revision = 0;
    latest_mid_stage.plan_draft_revision += 1;
    latest_mid_stage.plan_generated_at = Some(now.clone());
    latest_mid_stage.plan_regeneration_count = old_regeneration_count + 1;
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

    let milestone_id = &proj.current_milestone_id;
    let mid_stage_id = &proj.current_mid_stage_id;

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == *mid_stage_id)
        .ok_or("中阶段不存在。")?;

    if let Err(error) = crate::plan_contract::validate_subtasks(&mid.subtasks) {
        let ms = proj
            .milestones
            .iter_mut()
            .find(|m| m.id == *milestone_id)
            .ok_or("大阶段不存在。")?;
        let mid = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == *mid_stage_id)
            .ok_or("中阶段不存在。")?;
        mid.plan_check_result = Some(project::StagePlanCheckResult {
            passed: false,
            omissions: vec![],
            out_of_scope: vec![],
            not_executable: vec![error],
            suggestions: vec!["请重新生成执行计划并补全合法的文件范围。".to_string()],
            checked_at: chrono::Utc::now().to_rfc3339(),
        });
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    let plan_text = mid
        .subtasks
        .iter()
        .enumerate()
        .map(|(i, st)| {
            format!(
                "{}. {} — goal: {} — files: [{}] — new: [{}] — criteria: [{}]",
                i + 1,
                st.title,
                st.goal,
                st.allowed_file_paths.join(", "),
                st.new_file_paths.join(", "),
                st.acceptance_criteria.join("; "),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let context = format!(
        "中阶段：{} — {}\n技术重点：{}\n\n执行计划（{} 个小阶段）：\n{}",
        mid.title,
        mid.description,
        mid.tech_focus,
        mid.subtasks.len(),
        plan_text
    );

    let reply =
        crate::api::call_deepseek_api_json(crate::prompts::EXECUTION_PLAN_CHECK_PROMPT, &context)
            .await
            .map_err(|e| format!("执行计划检查 AI 调用失败：{}", e))?;

    let check: serde_json::Value =
        serde_json::from_str(&reply).map_err(|e| format!("解析检查结果失败：{}", e))?;

    let passed = check["passed"].as_bool().ok_or("缺少 passed 字段")?;

    let mut proj = crate::load_project(&project_name)?;
    if proj.workflow_state.current_step != project::WorkflowStep::PlanCheck {
        return Err("当前项目已不在计划检查步骤。".to_string());
    }

    let ms = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter_mut()
        .find(|m| m.id == *mid_stage_id)
        .ok_or("中阶段不存在。")?;
    mid.plan_check_result = Some(project::StagePlanCheckResult {
        passed,
        omissions: arr_str(&check["omissions"]),
        out_of_scope: arr_str(&check["out_of_scope"]),
        not_executable: arr_str(&check["not_executable"]),
        suggestions: arr_str(&check["suggestions"]),
        checked_at: chrono::Utc::now().to_rfc3339(),
    });

    proj.workflow_state.current_step = if passed {
        project::WorkflowStep::PlanApproving
    } else {
        project::WorkflowStep::PlanCheck
    };
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
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

    let milestone_id = &proj.current_milestone_id;
    let mid_stage_id = &proj.current_mid_stage_id;

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == *mid_stage_id)
        .ok_or("中阶段不存在。")?;

    // Verify check passed
    match &mid.plan_check_result {
        Some(r) if r.passed => {}
        Some(_) => return Err("执行计划检查未通过，无法批准。".to_string()),
        None => return Err("执行计划尚未检查，请先运行检查。".to_string()),
    }

    if mid.subtasks.is_empty() {
        return Err("执行计划为空，无法批准。".to_string());
    }
    crate::plan_contract::validate_subtasks(&mid.subtasks)
        .map_err(|error| format!("执行计划契约无效，无法批准：{}", error))?;

    let workspace = crate::pipeline::get_execution_workspace_status_inner(&proj.project_path)?;
    if !workspace.ready {
        return Err(format!(
            "Git 工作区尚未满足批准条件：{}",
            workspace.status_message
        ));
    }
    crate::plan_contract::validate_subtasks_in_project(&mid.subtasks, &proj.project_path)
        .map_err(|error| format!("执行计划契约无效，无法批准：{}", error))?;

    // Idempotency: if already approved, ensure disk consistency
    if mid.plan_approved_at.is_some() && mid.plan_revision > 0 {
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

    // Write approval metadata
    let ms = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter_mut()
        .find(|m| m.id == *mid_stage_id)
        .ok_or("中阶段不存在。")?;
    mid.plan_approved_at = Some(now);
    mid.plan_revision = plan_rev;

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

    let milestone_id = proj.current_milestone_id.clone();
    if milestone_id.is_empty() {
        return Err("未选择大阶段。".to_string());
    }

    let milestone_title = {
        let ms = proj
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or("大阶段不存在。".to_string())?;

        if ms.mid_stages.is_empty() {
            return Err("当前大阶段没有中阶段。".to_string());
        }
        let all_complete = ms
            .mid_stages
            .iter()
            .all(|m| m.status == project::MidStageStatus::Completed);
        if !all_complete {
            return Err("大阶段尚有未完成的中阶段，无法进入审阅。".to_string());
        }

        ms.status = project::MilestoneStatus::Completed;
        ms.review_status = Some("pending_review".to_string());
        ms.review_conclusion = None;
        ms.title.clone()
    };

    proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
    proj.workflow_state.review_node_id = milestone_id.clone();
    if proj.workflow_state.autopilot_active {
        let autopilot = proj
            .workflow_state
            .autopilot_state
            .get_or_insert_with(project::AutopilotState::default);
        autopilot.active = true;
        autopilot.target_milestone_id = milestone_id.clone();
        autopilot.run_status = project::AutopilotRunStatus::WaitingMilestoneReview;
        autopilot.last_action = format!("到达大阶段边界：{}，等待人工 A/B/C", milestone_title);
        autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
        autopilot.error_message.clear();
    }
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 大阶段审阅决策：A（继续）/ B（修正过去）/ C（调整未来）
#[tauri::command]
pub(crate) async fn approve_milestone_outcome(
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
        }
        "C" => {
            proj.workflow_state.current_step = project::WorkflowStep::BranchDiscussion;
            proj.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
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
    let discussion = proj
        .discussion_threads
        .first()
        .map(|t| {
            t.messages
                .iter()
                .map(|m| format!("[{}]: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let context = format!(
        "大阶段：{}\n\n执行证据：{}\n\n分支讨论：{}",
        ms.title, evidence, discussion
    );

    let reply = crate::api::call_deepseek_api_inner(
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
    )
    .await
    .map_err(|e| format!("AI 调用失败：{}", e))?;

    Ok(reply)
}

/// C 分支：生成未来大阶段草稿（保留已完成，只生成后续）
#[tauri::command]
pub(crate) async fn generate_future_milestone_draft(
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    if proj.workflow_state.discussion_scope != project::DiscussionScope::AdjustFuture {
        return Err("当前不在 AdjustFuture 讨论范围。".to_string());
    }

    let milestone_id = &proj.current_milestone_id;
    let split_idx = proj
        .milestones
        .iter()
        .position(|m| m.id == *milestone_id)
        .ok_or("大阶段不存在。")?;

    // Completed milestones (up to and including current)
    let completed: Vec<&project::Milestone> = proj.milestones[..=split_idx].iter().collect();

    // Build context
    let completed_titles: Vec<String> = completed
        .iter()
        .map(|m| format!("- {} ({})", m.title, m.version))
        .collect();

    let context = format!(
        "项目方案：{}\n\n已完成大阶段：\n{}\n\n讨论反馈：\n{}\n\n\
         只生成上述已完成大阶段之后的后续大阶段。已完成大阶段必须完全保留。",
        proj.version_plan,
        completed_titles.join("\n"),
        proj.discussion_threads
            .first()
            .map(|t| t
                .messages
                .iter()
                .map(|m| format!("[{}]: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n"))
            .unwrap_or_default(),
    );

    let reply =
        crate::api::call_deepseek_api_json(crate::prompts::MILESTONE_GENERATION_PROMPT, &context)
            .await
            .map_err(|e| format!("AI 调用失败：{}", e))?;

    let raw: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析失败：{}", e))?;

    if raw.is_empty() {
        return Err("AI 返回的后续大阶段为空。".to_string());
    }

    let mut new_milestones: Vec<project::Milestone> = Vec::new();
    for r in &raw {
        new_milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: r["version"].as_str().unwrap_or("v0.0").to_string(),
            title: r["title"].as_str().unwrap_or("未命名").to_string(),
            description: r["description"].as_str().unwrap_or("").to_string(),
            tech_stack: r["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: project::StageMode::Professional,
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
    let retained_ids: Vec<String> = completed.iter().map(|m| m.id.clone()).collect();
    let future_ids: Vec<String> = new_milestones.iter().map(|m| m.id.clone()).collect();
    let ai_versions: Vec<String> = raw
        .iter()
        .map(|r| r["version"].as_str().unwrap_or("v0.0").to_string())
        .collect();

    // === 阶段六：数量守恒检查 ===
    // 计算分割点之后原有的大阶段数量（被替换的部分）
    let proj_for_count = crate::load_project(&project_name)?;
    let original_remaining = proj_for_count
        .milestones
        .len()
        .saturating_sub(split_idx + 1);
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

    // Save as milestone_draft with FutureOnly metadata
    let mut proj = crate::load_project(&project_name)?;
    let draft = project::MilestoneDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        status: project::MilestoneDraftStatus::Pending,
        draft_kind: project::MilestoneDraftKind::FutureOnly,
        candidate_milestones: new_milestones,
        check_result: None,
        generation_revision: proj.discussion_revision,
        source_plan_revision: proj.workflow_state.data_revision,
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved_at: None,
        regeneration_count: 0,
        previous_draft_id: None,
        last_regeneration_reason: None,
        last_regenerated_at: None,
        split_after_milestone_id: Some(milestone_id.clone()),
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
    proj.milestone_draft = Some(draft);
    proj.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
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

// ===================================================================
// 旧命令（保留兼容，新路径不使用）
// ===================================================================

#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn generate_milestones(
    version_plan: String,
    mode: String,
) -> Result<Vec<project::Milestone>, String> {
    //拼好提示词
    //2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        crate::prompts::PM_PROMPT, mode
    );
    let user_message = format!("请根据以下版本方案拆解为3-5个大阶段：\n{}", version_plan);
    let content =
        crate::api::call_deepseek_api_inner(&system_prompt, &user_message, false, 0.5).await?;
    //（上面的文字）JSON数组转化为Rust数组,json改动替换
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    //创建一个空的、可变的、专门用来存放 project::Milestone 结构体的数组，变量名叫 milestones
    let mut milestones: Vec<project::Milestone> = Vec::new();
    //把这个数组里面的每个对象，转化为Rust里Milestone结构体（补上id,状态...）
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: "".to_string(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[generate_milestones] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response = match crate::api::call_deepseek_api_inner(
        crate::prompts::QA_CHECK_PROMPT,
        &qa_user_message,
        false,
        0.1,
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("[generate_milestones] 质检 API 调用失败：{}，跳过质检", e);
            return Ok(milestones);
        }
    };

    // 步骤 4：使用 parse_json_with_retry 解析 AI 返回的 QAResult JSON
    let qa_result =
        match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await {
            Ok(mut result) => {
                result.checked_at = chrono::Utc::now().to_rfc3339();
                result
            }
            Err(e) => {
                eprintln!(
                    "[generate_milestones] 质检 JSON 解析失败：{}，默认判定为不通过",
                    e
                );
                project::QAResult {
                    passed: false,
                    reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                    details: vec![],
                    attention_points: vec![],
                    checked_at: chrono::Utc::now().to_rfc3339(),
                    warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                }
            }
        };

    // 步骤 5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///根据质检驳回的反馈，重新让产品经理拆解大阶段
///与 generate_milestones 的区别：user_message 中包含驳回原因，引导 AI 修正
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn regenerate_milestones_with_feedback(
    version_plan: String,
    mode: String,
    feedback: String,
) -> Result<Vec<project::Milestone>, String> {
    // 2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        crate::prompts::PM_PROMPT, mode
    );
    let user_message = format!(
        "上次拆解被需求质检驳回，原因：\n{}\n\n请根据此反馈，重新根据以下版本方案拆解为3-5个大阶段：\n{}",
        feedback, version_plan
    );
    let content =
        crate::api::call_deepseek_api_inner(&system_prompt, &user_message, false, 0.5).await?;
    // 5. 解析 JSON 数组
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    // 6. 构造 Milestone 结构体
    let mut milestones: Vec<project::Milestone> = Vec::new();
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: "".to_string(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 7.1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[regenerate_milestones_with_feedback] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 7.2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 7.3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response = match crate::api::call_deepseek_api_inner(
        crate::prompts::QA_CHECK_PROMPT,
        &qa_user_message,
        false,
        0.1,
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!(
                "[regenerate_milestones_with_feedback] 质检 API 调用失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 7.4：使用 parse_json_with_retry 解析 AI 返回的 QAResult JSON
    let qa_result =
        match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await {
            Ok(mut result) => {
                result.checked_at = chrono::Utc::now().to_rfc3339();
                result
            }
            Err(e) => {
                eprintln!(
                "[regenerate_milestones_with_feedback] 质检 JSON 解析失败：{}，默认判定为不通过",
                e
            );
                project::QAResult {
                    passed: false,
                    reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                    details: vec![],
                    attention_points: vec![],
                    checked_at: chrono::Utc::now().to_rfc3339(),
                    warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                }
            }
        };

    // 步骤 7.5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///中阶段控制
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn generate_mid_stages(
    _milestone_id: String,
    milestone_title: String,
    milestone_description: String,
    version_plan: String,
    mode: String,
    attention_points: Vec<String>,
) -> Result<Vec<project::MidStage>, String> {
    // 2. 构造 system prompt
    let mut system_prompt = format!(
        "{}\n\n当前项目模式：{}。请根据版本方案，将大阶段拆解为 3-6 个中阶段。\
         每个中阶段是一个垂直切片。",
        crate::prompts::DOMAIN_LEAD_PROMPT,
        mode
    );
    // 注入 attention_points（若不为空）
    if !attention_points.is_empty() {
        system_prompt.push_str("\n【需求关注点】\n该大阶段在需求对齐检查中确认了以下要点，请在拆分中阶段时确保覆盖：\n");
        for point in &attention_points {
            system_prompt.push_str(&format!("- {}\n", point));
        }
    }
    let user_message = format!(
        "请根据版本方案，为大阶段「{} - {}」拆解中阶段：\n{}",
        milestone_title, milestone_description, version_plan
    );
    let content =
        crate::api::call_deepseek_api_inner(&system_prompt, &user_message, false, 0.5).await?;
    // 5. 解析 JSON,json 解析改动
    let raw_mid_stages: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析中阶段 JSON 失败：{}", e))?;
    // 6. 转换成 MidStage 结构体
    let mut mid_stages: Vec<project::MidStage> = Vec::new();
    for (i, raw) in raw_mid_stages.iter().enumerate() {
        mid_stages.push(project::MidStage {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_focus: raw["tech_focus"].as_str().unwrap_or("").to_string(),
            order: Some((i + 1) as i32),
            status: project::MidStageStatus::Pending,
            subtasks: vec![],
            test_report: "".to_string(),
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
        });
    }
    Ok(mid_stages)
}

// src-tauri/src/lib.rs

/// 在保留已完成大阶段的前提下，根据用户反馈重新生成后续大阶段
///
/// 与 generate_milestones（首次全量生成）的区别：
/// - 只生成 after_milestone_id 之后的大阶段，不修改已完成的
/// - 接受用户反馈作为修正方向
/// - 将已完成大阶段的摘要作为上下文传给 AI
///
/// 1. 加载项目，定位 after_milestone_id 作为分割点
/// 2. 构造包含已完成摘要和用户反馈的 AI 请求
/// 3. 调用 AI 生成后续大阶段
/// 4. QA 质检 → 不通过则返回错误，不修改 project.json
/// 5. 填充 UUID / 时间戳，拼接新旧 milestones，持久化
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn regenerate_milestones_from_point(
    project_name: String,
    after_milestone_id: String,
    version_plan: String,
    mode: String,
    feedback: String,
    completed_summary: String,
) -> Result<String, String> {
    // 1. 加载项目
    let mut project = crate::load_project(&project_name)?;

    // 2. 定位分割点：找到 after_milestone_id 的索引
    let split_idx = if after_milestone_id.is_empty() {
        // 没有已完成的大阶段 → 退化为全量生成
        None
    } else {
        let mut found: Option<usize> = None;
        for (i, m) in project.milestones.iter().enumerate() {
            if m.id == after_milestone_id {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(idx) => Some(idx),
            None => return Err(format!("未找到指定的大阶段: {}", after_milestone_id)),
        }
    };

    // 3. 收集已完成大阶段的上下文信息
    let completed_milestones: Vec<&project::Milestone> = match split_idx {
        Some(idx) => project.milestones[..=idx].iter().collect(),
        None => vec![],
    };

    let completed_titles: Vec<String> = completed_milestones
        .iter()
        .map(|m| format!("- {} ({})", m.title, m.version))
        .collect();
    let completed_titles_str = if completed_titles.is_empty() {
        "（暂无已完成的大阶段）".to_string()
    } else {
        completed_titles.join("\n")
    };

    let next_version_hint = if let Some(last) = completed_milestones.last() {
        format!(
            "\n\n已有大阶段的最后一个版本是 {}，新生成的大阶段版本号应从 {} 之后开始。",
            last.version, last.version
        )
    } else {
        String::new()
    };

    // 4. 构造 AI 请求
    let system_prompt = crate::prompts::REGENERATE_MILESTONES_PROMPT.to_string();

    let user_message = format!(
        "版本方案：\n{}\n\n项目模式：{}\n\n已完成大阶段摘要：\n{}\n\n已完成大阶段列表：\n{}{}\n\n用户反馈：\n{}\n\n请根据以上信息，生成后续的大阶段（milestones）JSON 数组。",
        version_plan,
        mode,
        completed_summary,
        completed_titles_str,
        next_version_hint,
        if feedback.is_empty() { "（用户未提供额外反馈）" } else { &feedback }
    );

    let content =
        crate::api::call_deepseek_api_inner(&system_prompt, &user_message, false, 0.5).await?;

    // 8. 解析 AI 返回的 JSON 数组
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;

    // 9. 构造新的 Milestone 结构体
    let mut new_milestones: Vec<project::Milestone> = Vec::new();
    for raw in raw_milestones {
        new_milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
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
        });
    }

    // 10. QA 质检
    if !new_milestones.is_empty() {
        let milestones_json = match serde_json::to_string(&new_milestones) {
            Ok(json) => json,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_from_point] 大阶段 JSON 序列化失败：{}，跳过质检",
                    e
                );
                // 序列化失败不阻塞流程，跳过质检
                let merged = merge_milestones(completed_milestones, new_milestones);
                project.milestones = merged;
                let project = crate::save_and_reload_project(&project)?;
                let json_str = serde_json::to_string_pretty(&project)
                    .map_err(|e| format!("序列化项目文件失败: {}", e))?;
                return Ok(json_str);
            }
        };

        let qa_user_message = format!(
            "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
            version_plan, milestones_json
        );

        let qa_response = match crate::api::call_deepseek_api_inner(
            crate::prompts::QA_CHECK_PROMPT,
            &qa_user_message,
            false,
            0.1,
        )
        .await
        {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_from_point] 质检 API 调用失败：{}，跳过质检",
                    e
                );
                let merged = merge_milestones(completed_milestones, new_milestones);
                project.milestones = merged;
                let project = crate::save_and_reload_project(&project)?;
                let json_str = serde_json::to_string_pretty(&project)
                    .map_err(|e| format!("序列化项目文件失败: {}", e))?;
                return Ok(json_str);
            }
        };

        let qa_result =
            match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await
            {
                Ok(mut result) => {
                    result.checked_at = chrono::Utc::now().to_rfc3339();
                    result
                }
                Err(e) => {
                    eprintln!(
                    "[regenerate_milestones_from_point] 质检 JSON 解析失败：{}，默认判定为不通过",
                    e
                );
                    project::QAResult {
                        passed: false,
                        reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                            .to_string(),
                        details: vec![],
                        attention_points: vec![],
                        checked_at: chrono::Utc::now().to_rfc3339(),
                        warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                    }
                }
            };

        // 质检不通过 → 返回错误，不修改 project.json
        if !qa_result.passed {
            return Err(format!(
                "质检不通过：{}\n\n详细偏差：\n{}",
                qa_result.reason,
                qa_result
                    .details
                    .iter()
                    .map(|d| format!(
                        "- [{}] {}（相关需求：{}）",
                        d.issue_type, d.description, d.related_requirement
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        // 质检通过 → 写入每个新 milestone
        for milestone in &mut new_milestones {
            milestone.qa_result = Some(qa_result.clone());
        }
    }

    // 11. 拼接：保留已完成 + 新生成的
    let merged = merge_milestones(completed_milestones, new_milestones);
    project.milestones = merged;

    // 12. 持久化
    let project = crate::save_and_reload_project(&project)?;

    // 13. 返回完整 Project JSON
    let json_str =
        serde_json::to_string_pretty(&project).map_err(|e| format!("序列化项目文件失败: {}", e))?;

    Ok(json_str)
}

/// 将已完成的大阶段和新生成的大阶段拼接为一个列表
#[allow(dead_code)]
fn merge_milestones(
    completed: Vec<&project::Milestone>,
    new: Vec<project::Milestone>,
) -> Vec<project::Milestone> {
    let mut result: Vec<project::Milestone> = Vec::new();
    for m in completed {
        result.push(m.clone());
    }
    for m in new {
        result.push(m);
    }
    result
}

/// 在回退后，根据分割点重新生成后续 subtask 的执行计划
///
/// 保留分割点之前的已完成 subtask（含分割点自身），
/// 调用 AI 批量生成分割点之后的后续 subtask，并持久化到 project.json。
///
/// 与 handleGeneratePlanForMidStage 中逐个 generate_next_prompt 的区别：
/// - 本命令一次性生成多个后续 subtask，而非逐个生成
/// - 提供已完成 subtask 的上下文和 git diff，确保逻辑连贯
///
/// 1. 加载项目，定位 milestone → mid_stage → split subtask
/// 2. 收集已完成 subtask 上下文 + git diff
/// 3. 调用 AI 批量生成后续 subtask（JSON 数组）
/// 4. 拼接新旧 subtask，持久化
/// 5. 返回更新后的 mid_stage JSON
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn regenerate_plan_from_checkpoint(
    project_name: String,
    project_path: String,
    milestone_id: String,
    mid_stage_id: String,
    subtask_id: String,
) -> Result<String, String> {
    // 1. 加载项目
    let mut project = crate::load_project(&project_name)?;

    // 2. 定位目标 mid_stage
    let milestone = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or(format!("未找到大阶段: {}", milestone_id))?;
    let mid_stage = milestone
        .mid_stages
        .iter()
        .find(|ms| ms.id == mid_stage_id)
        .ok_or(format!("未找到中阶段: {}", mid_stage_id))?;

    let mid_stage_title = mid_stage.title.clone();
    let mid_stage_description = mid_stage.description.clone();

    // 3. 定位分割点：找到 subtask_id 对应的索引
    let split_idx = mid_stage
        .subtasks
        .iter()
        .position(|st| st.id == subtask_id)
        .ok_or(format!("未找到小阶段: {}", subtask_id))?;

    let total_count = mid_stage.subtasks.len();
    let remaining_count = total_count.saturating_sub(split_idx + 1);

    // 如果没有后续 subtask 需要生成，直接返回当前 mid_stage JSON
    if remaining_count == 0 {
        let json_str = serde_json::to_string_pretty(&mid_stage)
            .map_err(|e| format!("序列化中阶段失败: {}", e))?;
        return Ok(json_str);
    }

    // 4. 收集已完成 subtask 的上下文
    let completed_subtasks: Vec<String> = mid_stage.subtasks[..=split_idx]
        .iter()
        .map(|st| {
            let result_summary = match (&st.execution_result, &st.test_result) {
                (Some(exec), Some(test)) => {
                    if test.passed {
                        format!(
                            "通过 — {}",
                            exec.output.chars().take(100).collect::<String>()
                        )
                    } else {
                        format!("未通过 — {}", test.suggestion)
                    }
                }
                (Some(exec), None) => {
                    format!(
                        "已执行 — {}",
                        exec.output.chars().take(100).collect::<String>()
                    )
                }
                _ => "待执行".to_string(),
            };
            format!("- {}（结果：{}）", st.title, result_summary)
        })
        .collect();

    let completed_context = if completed_subtasks.is_empty() {
        "（暂无已完成的小阶段）".to_string()
    } else {
        completed_subtasks.join("\n")
    };

    // 5. 获取 git diff
    let git_diff = match std::process::Command::new("git")
        .args(["diff"])
        .current_dir(&project_path)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                let diff_str = String::from_utf8_lossy(&output.stdout).to_string();
                if diff_str.trim().is_empty() {
                    "（工作区干净，无未提交变更）".to_string()
                } else {
                    diff_str
                }
            } else {
                "（无法获取 git diff）".to_string()
            }
        }
        Err(_) => "（无法获取 git diff）".to_string(),
    };

    // 6. 构造 AI 请求
    let user_message = format!(
        "中阶段标题：{}\n\
         中阶段描述：{}\n\n\
         已完成小阶段：\n{}\n\n\
         分割点：已完成 {} 个小阶段，需要从第 {} 个小阶段开始生成。\n\n\
         需要生成数量：{}\n\n\
         当前项目代码变更（git diff）：\n{}",
        mid_stage_title,
        mid_stage_description,
        completed_context,
        split_idx + 1,
        split_idx + 2,
        remaining_count,
        git_diff
    );

    // 7. 调用 AI
    let reply = crate::api::call_deepseek_api_json(
        crate::prompts::REGENERATE_SUBTASKS_PROMPT,
        &user_message,
    )
    .await
    .map_err(|e| format!("AI 调用失败: {}", e))?;

    // 8. 解析 AI 返回的 JSON 数组
    let raw_subtasks: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析小阶段 JSON 失败：{}", e))?;

    // 9. 构建新的 subtask 列表
    let mut new_subtasks: Vec<project::Subtask> = Vec::new();

    // 保留已完成的 subtask（克隆原始数据）
    for st in mid_stage.subtasks[..=split_idx].iter() {
        new_subtasks.push(st.clone());
    }

    // 追加 AI 生成的新 subtask
    for raw in raw_subtasks {
        new_subtasks.push(project::Subtask {
            id: uuid::Uuid::new_v4().to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            prompt: raw["prompt"].as_str().unwrap_or("").to_string(),
            status: project::SubtaskStatus::Pending,
            test_report: String::new(),
            execution_result: None,
            test_result: None,
            retry_count: 0,
            auto_tag: None,
            order: 0,
            goal: String::new(),
            allowed_file_paths: vec!["tracked.txt".to_string()],
            new_file_paths: vec![],
            evidence_files: vec![],
            context_summary: String::new(),
            acceptance_criteria: vec![],
            stop_rules: vec![],
            execution_prompt: String::new(),
            confirmed_by_user: None,
            confirmed_at: None,
            confirmation_notes: None,
        });
    }

    // 10. 更新 project 中的 mid_stage subtasks
    {
        let ms = project
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or("更新时找不到大阶段".to_string())?;
        let mid = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == mid_stage_id)
            .ok_or("更新时找不到中阶段".to_string())?;
        mid.subtasks = new_subtasks;
    }

    // 11. 持久化
    let project = crate::save_and_reload_project(&project)?;

    // 12. 序列化并返回更新后的 mid_stage
    let updated_mid_stage = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .and_then(|ms| ms.mid_stages.iter().find(|m| m.id == mid_stage_id))
        .ok_or("序列化时找不到中阶段".to_string())?;

    let json_str = serde_json::to_string_pretty(updated_mid_stage)
        .map_err(|e| format!("序列化中阶段失败: {}", e))?;

    Ok(json_str)
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
    let summary = crate::api::call_deepseek_api_inner(
        crate::prompts::SUMMARIZE_MILESTONE_PROMPT,
        &user_message,
        false,
        0.3,
    )
    .await
    .map_err(|e| format!("AI 调用失败: {}", e))?;

    Ok(summary)
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

    fn completed_mid_stage() -> project::MidStage {
        project::MidStage {
            id: "mid-1".to_string(),
            title: "已完成中阶段".to_string(),
            version: "v0.1.1".to_string(),
            order: Some(1),
            status: project::MidStageStatus::Completed,
            subtasks: vec![],
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
        }
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
        }
    }

    fn review_project(project_name: &str, with_next: bool) -> project::Project {
        let mut proj = project::Project::new(project_name);
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

    #[tokio::test]
    async fn entering_review_persists_milestone_and_autopilot_boundary() -> Result<(), String> {
        let project_name = unique_project_name("enter-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = review_project(&project_name, false);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.workflow_state.review_node_id.clear();
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::Running;
        }
        crate::save_project(&proj)?;

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
        Ok(())
    }

    #[tokio::test]
    async fn branch_a_selects_next_target_and_resumes_autopilot() -> Result<(), String> {
        let project_name = unique_project_name("review-a-next");
        let _guard = ProjectDataGuard::new(&project_name)?;
        crate::save_project(&review_project(&project_name, true))?;

        let updated = approve_milestone_outcome(project_name, "A".to_string()).await?;
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

        let updated = approve_milestone_outcome(project_name, "A".to_string()).await?;
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
                approve_milestone_outcome(project_name.clone(), branch.to_string()).await?;
            assert_eq!(
                updated.workflow_state.current_step,
                project::WorkflowStep::BranchDiscussion
            );
            assert_eq!(updated.workflow_state.discussion_scope, scope);
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

            let duplicate = approve_milestone_outcome(project_name, branch.to_string()).await;
            assert!(duplicate.is_err());
        }
        Ok(())
    }
}
