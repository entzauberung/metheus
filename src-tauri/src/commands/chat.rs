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
                    "方案已批准，聊天输入已锁定。如需修改方案，请使用「重新讨论方案」功能。"
                        .to_string(),
                );
            }
        }
    }

    // 2. Find thread
    let thread_idx = proj
        .discussion_threads
        .iter()
        .position(|t| t.id == thread_id)
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

    // 5. Build AI context: 统一上下文注入链 + 聊天元数据 + 完整消息历史
    let thread = &proj.discussion_threads[thread_idx];
    let recent_messages: Vec<&project::Message> = thread
        .messages
        .iter()
        .rev()
        .take(MAX_CONTEXT_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let context = {
        let mut c = String::new();

        // 聊天元数据（轻量，仅标识当前会话位置）
        c.push_str(&format!("[项目: {}]\n", proj.name));
        c.push_str(&format!(
            "[工作流步骤: {:?}]\n",
            proj.workflow_state.current_step
        ));
        c.push_str(&format!(
            "[讨论范围: {:?}]\n",
            proj.workflow_state.discussion_scope
        ));

        // 统一上下文注入链：项目事实（宪法、基线、方案、讨论摘要等）
        // 与 milestone / mid-stage / plan 生成共用同一来源
        let injection = crate::constitution_context::build_context_injection(&proj);
        if !injection.is_empty() {
            c.push_str(&injection);
            c.push('\n');
        }

        // 完整讨论历史（覆盖 build_context_injection 中的摘要版本）
        c.push_str("## 讨论历史\n");
        for msg in &recent_messages {
            let display_role = if msg.role == "user" {
                "用户"
            } else {
                &msg.role
            };
            c.push_str(&format!("{}: {}\n", display_role, msg.content));
        }
        c
    };

    // 5.5. 用户消息已写入内存 → 立即保存并重读 Project（用户消息落盘后再调用 AI）
    proj = crate::save_and_reload_project(&proj)
        .map_err(|e| format!("用户消息保存失败：{}。请重试。", e))?;

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
            proj.discussion_threads[thread_idx]
                .messages
                .push(failure_msg);
            // 保存并返回完整 Project（用户消息 + 失败提示均已持久化）
            return crate::save_and_reload_project(&proj).map_err(|save_err| {
                format!("AI 调用失败（{}），且保存失败提示时也失败：{}", e, save_err)
            });
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

    proj.discussion_threads[thread_idx]
        .messages
        .push(ai_message);

    // 8. Save and reload project, return disk-verified Project
    crate::save_and_reload_project(&proj)
}
