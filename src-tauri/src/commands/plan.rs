use crate::project;

/// 校验三项检查全部通过且未过期，返回具体错误说明。
fn validate_preflight_checks(proj: &project::Project) -> Result<(), String> {
    let check_types = [
        ("goal_completeness", "目标完整性检查"),
        ("reality_consistency", "现实一致性检查"),
        ("task_executability", "任务可执行性检查"),
    ];

    for (check_type, label) in &check_types {
        let result = proj
            .preflight_results
            .iter()
            .find(|r| r.check_type == *check_type)
            .ok_or_else(|| {
                format!("检查「{}」尚未执行，请先完成所有三项检查", label)
            })?;

        if !result.passed {
            return Err(format!("检查「{}」未通过，无法继续操作", label));
        }

        if result.stale || result.discussion_revision < proj.discussion_revision {
            return Err(format!(
                "检查「{}」已过期（讨论已更新），请重新检查",
                label
            ));
        }
    }

    Ok(())
}

/// 生成版本方案（必须三项检查全部通过且未过期，当前步骤为 ThreeChecks）
///
/// 方案生成期间讨论发生变化时，旧上下文生成的草稿不得保存。
/// AI 返回不完整草稿时不得进入审批页面。
#[tauri::command]
pub(crate) async fn generate_version_plan(
    project_name: String,
    expected_discussion_revision: u64,
    expected_data_revision: u64,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    // === 1. 校验当前步骤 ===
    if proj.workflow_state.current_step != project::WorkflowStep::ThreeChecks {
        return Err(format!(
            "当前步骤为 {:?}，只有通过三项检查后才能生成方案",
            proj.workflow_state.current_step
        ));
    }

    // === 2. 校验讨论未变化 ===
    if expected_discussion_revision != proj.discussion_revision {
        return Err(format!(
            "讨论已变化（前端修订号 {} 不等于后端修订号 {}），请刷新后重新生成",
            expected_discussion_revision, proj.discussion_revision
        ));
    }

    // === 2.5. 校验项目数据未变化 ===
    if expected_data_revision != proj.workflow_state.data_revision {
        return Err(format!(
            "项目数据已变化（前端数据修订号 {} 不等于后端数据修订号 {}），请刷新后重新生成",
            expected_data_revision, proj.workflow_state.data_revision
        ));
    }

    // === 3. Half Project: 基线必须已批准 ===
    if proj.entry_kind == project::ProjectEntryKind::HalfProject {
        let baseline_approved = proj
            .existing_baseline
            .as_ref()
            .map(|b| b.approved)
            .unwrap_or(false);
        if !baseline_approved {
            return Err("请先批准已有项目基线（Already Baseline），再生成方案。".to_string());
        }
    }

    // === 3.5. No Project: 项目路径必须是有效目录 ===
    if proj.entry_kind == project::ProjectEntryKind::NoProject {
        if !proj.project_path.is_empty() {
            let p = std::path::Path::new(&proj.project_path);
            if !p.is_dir() {
                return Err(format!(
                    "项目路径「{}」不是有效目录，无法生成方案",
                    proj.project_path
                ));
            }
        }
    }

    // === 3.6. 当前不存在待审批草稿 ===
    if let Some(ref existing) = proj.plan_draft {
        if existing.draft_status == project::DraftStatus::Pending {
            return Err(
                "已存在待审批草稿，请先处理（批准或驳回）后再生成新方案。".to_string()
            );
        }
    }

    // === 4. 强制校验三项检查 ===
    validate_preflight_checks(&proj)?;

    // === 4.5. 保存事实快照（AI 返回后校验未变化） ===
    let snapshot_step = proj.workflow_state.current_step.clone();
    let snapshot_discussion_revision = proj.discussion_revision;
    let snapshot_data_revision = proj.workflow_state.data_revision;
    let snapshot_project_path = proj.project_path.clone();

    let messages = proj
        .discussion_threads
        .first()
        .map(|t| t.messages.clone())
        .unwrap_or_default();

    // Build system prompt
    let system_prompt = format!(
        "{} {}",
        "你是一个产品战略顾问，角色名「策略产品经理」。\
         请根据以下对话历史，输出一份结构化的「版本方案摘要」。\
         使用 Markdown 格式，包含以下章节：\
         ## 项目愿景\n## 目标用户\n## 核心功能\n## 版本路径\n\
         每个版本路径下的版本要清晰列出。\
         回答风格：结构化、清晰、可直接用于执行。\
         在版本方案之后，用分隔符 ---CONSTITUTION_PART1--- 分隔宪法第 1 部分。\
         宪法第 1 部分包含：技术选型理由、架构决策记录、编码规范等长期规则。",
        crate::prompts::CONSTITUTION_PART1_PROMPT
    );

    let mut api_messages: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt,
    })];

    for msg in &messages {
        let api_role = if msg.role == "user" {
            "user"
        } else {
            "assistant"
        };
        api_messages.push(serde_json::json!({
            "role": api_role,
            "content": msg.content,
        }));
    }

    let ai_content = crate::api::call_deepseek_api_messages(api_messages, false, 0.5).await?;

    // === AI 返回后重新加载 Project，校验事实未变化 ===
    let current_proj = crate::load_project(&project_name)?;

    if current_proj.workflow_state.current_step != snapshot_step {
        return Err(
            "工作流步骤已变化（可能在生成期间发生了操作），请刷新后重新生成。".to_string()
        );
    }

    if current_proj.discussion_revision != snapshot_discussion_revision {
        return Err(
            "讨论已变化（可能在生成期间发送了新消息），请重新开始检查并生成方案。".to_string()
        );
    }

    if current_proj.workflow_state.data_revision != snapshot_data_revision {
        return Err(
            "项目数据已变化（可能在生成期间发生了操作），请刷新后重新生成。".to_string()
        );
    }

    if current_proj.project_path != snapshot_project_path {
        return Err(
            "项目路径已变化，请刷新后重新生成。".to_string()
        );
    }

    // 重新校验三项检查仍然有效
    validate_preflight_checks(&current_proj)?;

    // === AI 输出完整性校验 ===
    // 检查是否有宪法分隔标记
    let separator_pos = ai_content.find("---CONSTITUTION_PART1---");

    // Split into plan content and constitution part 1 draft
    let (plan_content, constitution_draft) =
        if let Some(pos) = separator_pos {
            let part1 = ai_content[pos + "---CONSTITUTION_PART1---".len()..]
                .trim()
                .to_string();
            let plan = ai_content[..pos].trim().to_string();
            (plan, part1)
        } else {
            (ai_content.trim().to_string(), String::new())
        };

    // 验证方案正文非空且不是只有空白/标题/错误说明
    let plan_trimmed = plan_content.trim();
    if plan_trimmed.is_empty() {
        return Err(
            "AI 未生成有效的项目方案正文。请返回讨论补充需求后重新生成。".to_string()
        );
    }
    // 检查是否只有标题没有实质内容（粗略判断：去除 Markdown 标题后仍有内容）
    let plan_without_headings = plan_trimmed
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if plan_without_headings.trim().is_empty() {
        return Err(
            "AI 生成的方案仅包含标题，缺少实质内容。请返回讨论补充需求后重新生成。".to_string()
        );
    }

    // 验证宪法第一部分分隔标记存在
    if separator_pos.is_none() {
        return Err(
            "AI 输出缺少宪法第一部分（---CONSTITUTION_PART1--- 分隔标记）。请重新生成。".to_string()
        );
    }

    // 验证宪法第一部分草稿非空
    if constitution_draft.trim().is_empty() {
        return Err(
            "AI 未生成有效的宪法第一部分草稿。请重新生成。".to_string()
        );
    }

    let draft = project::PlanDraft {
        draft_id: uuid::Uuid::new_v4().to_string(),
        draft_status: project::DraftStatus::Pending,
        plan_content: plan_content.clone(),
        constitution_part1_draft: constitution_draft,
        generation_revision: current_proj.discussion_revision,
        data_revision_at_generation: current_proj.workflow_state.data_revision,
        self_check_result: "".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved: false,
        approved_at: None,
        approved_at_discussion_revision: None,
        rejection_feedback: None,
        rejected_at: None,
        expired_at: None,
        superseded_at: None,
    };

    // Save draft to project, transition to PlanApproval (NOT writing CONSTITUTION.md yet)
    let mut proj = current_proj;
    proj.plan_draft = Some(draft);
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
    proj.workflow_state.data_revision += 1;
    crate::save_project(&proj)?;

    Ok(proj)
}

/// 批准版本方案（必须三项检查全部通过且未过期，draft_id 匹配）
#[tauri::command]
pub(crate) async fn approve_version_plan(
    project_name: String,
    draft_id: String,
    generation_revision: u64,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // === 1. 校验当前步骤 ===
    if proj.workflow_state.current_step != project::WorkflowStep::PlanApproval {
        return Err(format!(
            "当前步骤为 {:?}，无法批准方案",
            proj.workflow_state.current_step
        ));
    }

    // === 2. 强制校验三项检查 ===
    validate_preflight_checks(&proj)?;

    let project_path = proj.project_path.clone();

    // Get the plan draft
    let draft = proj
        .plan_draft
        .as_ref()
        .ok_or("没有可批准的方案草稿，请先生成方案".to_string())?;

    // === 3. 验证 draft_id 匹配 ===
    if draft.draft_id != draft_id {
        return Err(format!(
            "草稿标识不匹配（前端 {} vs 后端 {}），请刷新后重试",
            draft_id, draft.draft_id
        ));
    }

    // === 4. 验证 generation_revision 匹配 ===
    if draft.generation_revision != generation_revision {
        return Err(format!(
            "草稿生成修订号不匹配（前端 {} vs 后端 {}），讨论可能已变化",
            generation_revision, draft.generation_revision
        ));
    }

    if draft.draft_status == project::DraftStatus::Approved {
        // 幂等返回已批准项目
        return Ok(proj);
    }
    if draft.draft_status != project::DraftStatus::Pending {
        return Err(format!(
            "当前草稿状态为 {:?}，只有待审批的草稿可以批准",
            draft.draft_status
        ));
    }

    // === 2.5. 验证草稿生成修订号等于当前讨论修订号 ===
    if draft.generation_revision != proj.discussion_revision {
        return Err(format!(
            "草稿生成时的讨论修订号（{}）与当前讨论修订号（{}）不一致。讨论可能已变化，请重新生成方案。",
            draft.generation_revision, proj.discussion_revision
        ));
    }

    // === 2.6. 验证草稿内容完整性（方案正文和宪法第一部分缺一不可） ===
    if draft.plan_content.trim().is_empty() {
        return Err("草稿方案正文为空，无法批准。请重新生成方案。".to_string());
    }
    if draft.constitution_part1_draft.trim().is_empty() {
        return Err("草稿缺少宪法第一部分，无法批准。请重新生成方案。".to_string());
    }

    // === 2.7. No Project: 验证项目目录仍可写 ===
    if !project_path.is_empty() {
        let p = std::path::Path::new(&project_path);
        if !p.is_dir() {
            return Err(format!(
                "项目路径「{}」无效或已被删除，无法写入宪法。",
                project_path
            ));
        }
    }

    // === 3. 原子写入：先写宪法，再存 Project；Project 失败则回退宪法 ===
    let constitution_path = if !project_path.is_empty() {
        Some(std::path::Path::new(&project_path).join("CONSTITUTION.md"))
    } else {
        None
    };

    // 保存旧宪法内容用于回退
    let old_constitution = constitution_path.as_ref().and_then(|p| {
        if p.exists() {
            std::fs::read_to_string(p).ok()
        } else {
            Some(String::new())
        }
    });

    if let Some(ref constitution_path) = constitution_path {
        let existing_content = old_constitution.as_deref().unwrap_or("");

        let constitution_full = if existing_content.contains("## 第 1 部分") {
            if existing_content.contains("## 第 2 部分") {
                existing_content.to_string()
            } else {
                format!(
                    "{}\n\n## 第 2 部分：项目当前状态\n（每个执行阶段通过后自动更新）\n",
                    existing_content.trim()
                )
            }
        } else {
            if !existing_content.is_empty() {
                format!(
                    "{}\n\n---\n\n{}\n\n## 第 2 部分：项目当前状态\n（每个执行阶段通过后自动更新）\n",
                    existing_content.trim(),
                    draft.constitution_part1_draft
                )
            } else {
                format!(
                    "{}\n\n## 第 2 部分：项目当前状态\n（每个执行阶段通过后自动更新）\n",
                    draft.constitution_part1_draft
                )
            }
        };

        let final_content = if let Some(ref baseline) = proj.existing_baseline {
            if baseline.approved && !constitution_full.contains("已有项目基线") {
                format!(
                    "{}\n\n### 已有项目基线\n{}\n\n技术栈：{}\n\n已完成能力：{}\n\n待处理能力：{}",
                    constitution_full,
                    baseline.project_summary,
                    baseline.tech_stack,
                    baseline.completed_capabilities.join("、"),
                    baseline.pending_capabilities.join("、"),
                )
            } else {
                constitution_full
            }
        } else {
            constitution_full
        };

        std::fs::write(constitution_path, &final_content)
            .map_err(|e| format!("写入 CONSTITUTION.md 失败：{}", e))?;
    }

    // Update project state in memory
    proj.version_plan = draft.plan_content.clone();
    if let Some(ref mut d) = proj.plan_draft {
        d.approved = true;
        d.draft_status = project::DraftStatus::Approved;
        d.approved_at = Some(chrono::Utc::now().to_rfc3339());
        d.approved_at_discussion_revision = Some(proj.discussion_revision);
    }

    proj.status = project::ProjectStatus::Planning;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
    proj.workflow_state.data_revision += 1;

    // 原子保存 Project；失败时回退宪法
    if let Err(save_err) = crate::save_project(&proj) {
        // 回退宪法到批准前内容
        if let Some(ref constitution_path) = constitution_path {
            let rollback_content = old_constitution.as_deref().unwrap_or("");
            if let Err(rollback_err) = std::fs::write(constitution_path, rollback_content) {
                return Err(format!(
                    "严重不一致：Project 保存失败（{}），且宪法回退也失败（{}）。请手动检查 CONSTITUTION.md 和项目数据文件是否一致。",
                    save_err, rollback_err
                ));
            }
        }
        return Err(format!("Project 保存失败，宪法已回退：{}", save_err));
    }

    Ok(proj)
}

/// 驳回方案：验证 draft_id，清除检查结果，回到 Discussion
#[tauri::command]
pub(crate) async fn reject_version_plan(
    project_name: String,
    draft_id: String,
    feedback: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // === 1. 校验当前步骤 ===
    if proj.workflow_state.current_step != project::WorkflowStep::PlanApproval {
        return Err(format!(
            "当前步骤为 {:?}，无法驳回方案",
            proj.workflow_state.current_step
        ));
    }

    // === 2. 验证 draft_id 匹配 ===
    let draft = proj
        .plan_draft
        .as_ref()
        .ok_or("没有可驳回的方案草稿。".to_string())?;

    if draft.draft_id != draft_id {
        return Err(format!(
            "草稿标识不匹配（前端 {} vs 后端 {}），请刷新后重试",
            draft_id, draft.draft_id
        ));
    }

    // === 3. 只有待审批草稿可以驳回 ===
    if draft.draft_status != project::DraftStatus::Pending {
        return Err(format!(
            "当前草稿状态为 {:?}，只有待审批的草稿可以驳回。已批准方案请使用「重新讨论方案」。",
            draft.draft_status
        ));
    }

    if feedback.trim().is_empty() {
        return Err("驳回反馈不能为空，请填写驳回原因。".to_string());
    }

    // Mark current draft as rejected and move to history
    if let Some(mut draft) = proj.plan_draft.take() {
        draft.draft_status = project::DraftStatus::Rejected;
        draft.rejection_feedback = Some(feedback);
        draft.rejected_at = Some(chrono::Utc::now().to_rfc3339());
        proj.draft_history.push(draft);
    }

    // Clear preflight results (they're now stale)
    // Don't clear version_plan — it was never set for a Pending draft
    proj.preflight_results.clear();

    // Return to discussion
    proj.workflow_state.current_step = project::WorkflowStep::Discussion;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.data_revision += 1;

    crate::save_project(&proj)?;
    Ok(proj)
}

/// 进入控制台（严格验证批准事实，任一条件不满足则拒绝）
#[tauri::command]
pub(crate) async fn enter_console(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // === 1. 验证顶层阶段 ===
    if proj.workflow_state.top_level_phase != project::TopLevelPhase::FirstDiscussion {
        return Err(format!(
            "当前顶层阶段为 {:?}，只有 FirstDiscussion 可进入 Console",
            proj.workflow_state.top_level_phase
        ));
    }

    // === 2. 验证当前步骤 ===
    if proj.workflow_state.current_step != project::WorkflowStep::PlanApproval {
        return Err(format!(
            "当前步骤为 {:?}，只有 PlanApproval 步骤可进入 Console",
            proj.workflow_state.current_step
        ));
    }

    // === 3. 验证草稿存在 ===
    let draft = proj
        .plan_draft
        .as_ref()
        .ok_or("没有方案草稿，无法进入控制台。请先生成并批准方案。".to_string())?;

    // === 4. 验证草稿已批准 ===
    if draft.draft_status != project::DraftStatus::Approved {
        return Err(format!(
            "草稿状态为 {:?}，尚未批准。请在方案审批页面先批准方案草稿。",
            draft.draft_status
        ));
    }

    // === 5. 验证批准时间存在 ===
    if draft.approved_at.is_none() {
        return Err("方案批准记录异常（缺少批准时间），请联系管理员。".to_string());
    }

    // === 6. 验证正式 version_plan 存在 ===
    if proj.version_plan.is_empty() {
        return Err("正式方案内容缺失，请重新批准方案以写入 version_plan。".to_string());
    }

    // === 7. 验证草案 == 正式方案（未被篡改） ===
    if proj.version_plan != draft.plan_content {
        return Err("正式方案内容与批准草稿不一致，请返回讨论重新生成方案。".to_string());
    }

    // === 8. 验证批准修订号等于当前讨论修订号（批准后未发送新需求） ===
    let approved_revision = draft.approved_at_discussion_revision.unwrap_or(0);
    if approved_revision != proj.discussion_revision {
        return Err(format!(
            "批准时的讨论修订号（{}）与当前讨论修订号（{}）不一致。讨论可能在批准后发生了变化，请重新讨论并批准方案。",
            approved_revision, proj.discussion_revision
        ));
    }

    proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
    proj.workflow_state.current_step = project::WorkflowStep::MilestoneGeneration;
    proj.workflow_state.data_revision += 1;

    crate::save_project(&proj)?;
    Ok(proj)
}
