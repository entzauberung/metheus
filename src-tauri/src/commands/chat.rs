use crate::project;

const MAX_CONTEXT_MESSAGES: usize = 20;

#[tauri::command]
pub(crate) fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 简单单条消息发送（保留兼容）
#[tauri::command]
pub(crate) async fn send_message(message: String) -> Result<String, String> {
    crate::api::call_deepseek_api("", &message).await
}

/// 多角色对话命令（持久化版本）
/// 加载项目 → 写入用户消息 → 调用 AI → 写入 AI 回复 → 返回完整 Project
/// 前端应使用返回的完整 Project 替换本地状态，不再乐观插入消息。
#[tauri::command]
pub(crate) async fn chat_with_role(
    project_name: String,
    message: String,
    role: String,
    thread_id: String,
) -> Result<project::Project, String> {
    // 1. Load project from disk
    let mut proj = crate::load_project(&project_name)?;

    // 1.5. 方案已批准时拒绝聊天（用户必须先选择"重新讨论方案"）
    if proj.workflow_state.current_step == project::WorkflowStep::PlanApproval {
        if let Some(ref draft) = proj.plan_draft {
            if draft.draft_status == project::DraftStatus::Approved {
                return Err(
                    "方案已批准，聊天输入已锁定。如需修改方案，请使用「重新讨论方案」功能。".to_string()
                );
            }
        }
    }

    // 2. Find thread
    let thread_idx = proj.discussion_threads.iter().position(|t| t.id == thread_id)
        .ok_or_else(|| format!("讨论线程不存在: {}", thread_id))?;

    // 3. Select system prompt
    let system_prompt = match role.as_str() {
        "策略产品经理" => crate::prompts::STRATEGY_PROMPT,
        "产品经理" => crate::prompts::PM_PROMPT,
        "域负责人" => crate::prompts::DOMAIN_LEAD_PROMPT,
        "全栈技术顾问" => crate::prompts::TECH_PROMPT,
        "测试工程师" => crate::prompts::TEST_PROMPT,
        _ => return Err(format!("未知角色: {}", role)),
    };

    // 4. Create and persist user message (only user messages increment revision)
    let user_msg = project::Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: message.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        msg_type: None,
        approved: None,
        rejected: None,
        milestone_id: None,
    };

    proj.discussion_threads[thread_idx].messages.push(user_msg);
    // 只有用户有效需求消息递增 discussion_revision
    // AI、系统和总结消息不得增加修订号（用于检测检查/方案是否过期）
    proj.discussion_revision += 1;
    proj.workflow_state.data_revision += 1;

    // 用户发送新需求 → 标记旧检查结果为过期
    let now = chrono::Utc::now().to_rfc3339();
    for result in &mut proj.preflight_results {
        if !result.stale {
            result.stale = true;
            result.expired_at = Some(now.clone());
        }
    }

    // 用户发送新需求 → 将待审批草稿标记为过期并移入历史
    if let Some(ref draft) = proj.plan_draft {
        if draft.draft_status == project::DraftStatus::Pending {
            if let Some(mut expired_draft) = proj.plan_draft.take() {
                expired_draft.draft_status = project::DraftStatus::Expired;
                expired_draft.expired_at = Some(now);
                proj.draft_history.push(expired_draft);
            }
        }
    }

    // 5. Build AI context from thread history + project facts
    let thread = &proj.discussion_threads[thread_idx];
    let recent_messages: Vec<&project::Message> = thread.messages
        .iter()
        .rev()
        .take(MAX_CONTEXT_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let context = {
        let mut c = String::new();

        // Project context
        c.push_str(&format!("[项目: {}]\n", proj.name));
        c.push_str(&format!("[来源: {}]\n",
            match proj.entry_kind {
                project::ProjectEntryKind::NoProject => "从零开始",
                project::ProjectEntryKind::HalfProject => "改造已有项目",
            }
        ));
        c.push_str(&format!("[工作流步骤: {:?}]\n", proj.workflow_state.current_step));
        c.push_str(&format!("[讨论范围: {:?}]\n", proj.workflow_state.discussion_scope));

        // Phase 6: PauseAdjustment — inject focused context about current stage
        if proj.workflow_state.discussion_scope == project::DiscussionScope::PauseAdjustment {
            c.push_str("\n--- 暂停调整上下文（聚焦当前阶段）---\n");
            // Current milestone info
            if !proj.current_milestone_id.is_empty() {
                if let Some(ms) = proj.milestones.iter().find(|m| m.id == proj.current_milestone_id) {
                    c.push_str(&format!("当前大阶段: {} — {}\n", ms.title, ms.goal));
                    c.push_str(&format!("大阶段状态: {:?}\n", ms.status));
                }
            }
            // Current mid-stage info
            if !proj.current_mid_stage_id.is_empty() {
                if let Some(ms) = proj.milestones.iter().find(|m| m.id == proj.current_milestone_id) {
                    if let Some(mid) = ms.mid_stages.iter().find(|m| m.id == proj.current_mid_stage_id) {
                        c.push_str(&format!("当前中阶段: {} ({})\n", mid.title, mid.version));
                        c.push_str(&format!("中阶段状态: {:?}, 子任务数: {}\n", mid.status, mid.subtasks.len()));
                        // Subtask status summary
                        let pending = mid.subtasks.iter().filter(|s| s.status == project::SubtaskStatus::Pending).count();
                        let passed = mid.subtasks.iter().filter(|s| s.status == project::SubtaskStatus::Passed).count();
                        let awaiting = mid.subtasks.iter().filter(|s| s.status == project::SubtaskStatus::AwaitingConfirmation).count();
                        c.push_str(&format!("子任务进度: {} 待执行, {} 待确认, {} 已通过\n", pending, awaiting, passed));
                    }
                }
            }
            // Autopilot state
            if let Some(ref ap) = proj.workflow_state.autopilot_state {
                c.push_str(&format!("自动驾驶状态: {:?}, 最近动作: {}\n", ap.run_status, ap.last_action));
                if !ap.last_recovery_reason.is_empty() {
                    c.push_str(&format!("最近补救: {}\n", ap.last_recovery_reason));
                }
            }
            // Recent execution history (last 3 entries)
            let recent: Vec<_> = proj.execution_history.iter().rev().take(3).collect();
            for entry in recent.iter().rev() {
                c.push_str(&format!("执行记录: [{}] {}\n", entry.level, entry.text.chars().take(200).collect::<String>()));
            }
            c.push_str("--- 暂停调整上下文结束 ---\n\n");
        }

        // Existing baseline summary
        if let Some(ref baseline) = proj.existing_baseline {
            if baseline.approved {
                c.push_str(&format!("[已有项目技术栈: {}]\n", baseline.tech_stack));
                if !baseline.completed_capabilities.is_empty() {
                    c.push_str(&format!("[已完成: {}]\n", baseline.completed_capabilities.join(", ")));
                }
            }
        }

        // Discussion history
        for msg in &recent_messages {
            let display_role = if msg.role == "user" { "用户" } else { &msg.role };
            c.push_str(&format!("{}: {}\n", display_role, msg.content));
        }
        c
    };

    // 5.5. 用户消息已写入内存 → 立即保存 Project（用户消息落盘后再调用 AI）
    crate::save_project(&proj).map_err(|e| {
        format!("用户消息保存失败：{}。请重试。", e)
    })?;

    // 6. Call AI
    let reply = match crate::api::call_deepseek_api(system_prompt, &context).await {
        Ok(r) => r,
        Err(e) => {
            // AI 失败 — 用户消息已保存，写入系统失败提示后返回完整 Project
            let failure_msg = project::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "system".to_string(),
                content: format!(
                    "⚠️ 用户消息已保存，但 AI（{}）本次回复失败：{}。请稍后重试。",
                    role, e
                ),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                msg_type: Some("ai_failure".to_string()),
                approved: None,
                rejected: None,
                milestone_id: None,
            };
            proj.discussion_threads[thread_idx].messages.push(failure_msg);
            // 保存并返回完整 Project（用户消息 + 失败提示均已持久化）
            crate::save_project(&proj).map_err(|save_err| {
                format!("AI 调用失败（{}），且保存失败提示时也失败：{}", e, save_err)
            })?;
            return Ok(proj);
        }
    };

    // 7. Create and persist AI reply
    let ai_message = project::Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: role.clone(),
        content: reply,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        msg_type: None,
        approved: None,
        rejected: None,
        milestone_id: None,
    };

    proj.discussion_threads[thread_idx].messages.push(ai_message);

    // 8. Save project and return the complete updated Project
    crate::save_project(&proj)?;

    // 前端应使用此完整 Project 替换本地状态，消息已在 discussion_threads 中
    Ok(proj)
}
