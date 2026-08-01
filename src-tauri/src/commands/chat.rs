use crate::project;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

const MAX_CONTEXT_MESSAGES: usize = 20;
const MAX_CHAT_MESSAGE_CHARS: usize = 20_000;
const CANCELLED_MESSAGE_TYPE: &str = "ai_cancelled";
const INTERRUPTED_MESSAGE_TYPE: &str = "ai_interrupted";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum ChatStreamEvent {
    Started {
        request_id: String,
        thread_id: String,
        role: String,
    },
    UserSaved {
        request_id: String,
        thread_id: String,
        role: String,
        message: project::Message,
    },
    ReplyStarted {
        request_id: String,
        thread_id: String,
        role: String,
        message_id: String,
        timestamp: u64,
    },
    Delta {
        request_id: String,
        thread_id: String,
        role: String,
        text: String,
    },
    Completed {
        request_id: String,
        thread_id: String,
        role: String,
        message_id: String,
    },
    Cancelled {
        request_id: String,
        thread_id: String,
        role: String,
        message_id: Option<String>,
    },
    Failed {
        request_id: String,
        thread_id: String,
        role: String,
        message_id: Option<String>,
        error: String,
        retryable: bool,
    },
}

#[derive(Debug)]
struct PreparedChat {
    user_message: project::Message,
    context: String,
    system_prompt: &'static str,
}

enum ChatInput {
    NewMessage(String),
    RetryUserMessage(String),
}

fn send_stream_event(
    channel: &Channel<ChatStreamEvent>,
    event: ChatStreamEvent,
) -> Result<(), String> {
    channel
        .send(event)
        .map_err(|error| format!("发送聊天流事件失败：{error}"))
}

fn mark_discussion_call(project_name: &str, call_id: &str, produced_change: bool) {
    if !produced_change {
        return;
    }
    crate::cost_ledger::mark_call_outcome_best_effort(
        project_name,
        call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_change: true,
            ..Default::default()
        },
    );
}

#[tauri::command]
pub(crate) fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 简单单条消息发送（保留兼容）
#[tauri::command]
pub(crate) async fn send_message(message: String) -> Result<String, String> {
    crate::api::call_deepseek_api("", &message).await
}

/// 多角色对话命令（保留兼容）。用户消息先落盘，AI 终态只落盘一次。
#[tauri::command]
pub(crate) async fn chat_with_role(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    message: String,
    role: String,
    thread_id: String,
) -> Result<project::Project, String> {
    let _lease = runtime.begin(
        format!("compat-{}", uuid::Uuid::new_v4()),
        project_name.clone(),
        thread_id.clone(),
        role.clone(),
    )?;
    let prepared = prepare_chat(
        runtime.inner(),
        &project_name,
        &thread_id,
        &role,
        ChatInput::NewMessage(message),
    )?;
    let model_context = crate::cost_ledger::ModelCallContext::for_project(
        &crate::load_project(&project_name)?,
        crate::cost_ledger::ModelCallPurpose::Discussion,
    );
    match crate::api::call_deepseek_api_inner_with_context(
        prepared.system_prompt,
        &prepared.context,
        false,
        0.5,
        model_context,
    )
    .await
    {
        Ok(response) => {
            let project = persist_reply(
                runtime.inner(),
                &project_name,
                &thread_id,
                new_reply_message(
                    &role,
                    response.content,
                    None,
                    &prepared.user_message.id,
                    uuid::Uuid::new_v4().to_string(),
                    now_millis(),
                ),
            )?;
            crate::cost_ledger::mark_call_outcome_best_effort(
                &project_name,
                &response.metadata.call_id,
                crate::cost_ledger::ModelCallOutcome {
                    produced_change: true,
                    ..Default::default()
                },
            );
            Ok(project)
        }
        Err(error) => persist_reply(
            runtime.inner(),
            &project_name,
            &thread_id,
            new_reply_message(
                &role,
                "本次回复未生成内容。".to_string(),
                Some(INTERRUPTED_MESSAGE_TYPE),
                &prepared.user_message.id,
                uuid::Uuid::new_v4().to_string(),
                now_millis(),
            ),
        )
        .map_err(|save_error| format!("AI 调用失败（{error}），且中断状态保存失败：{save_error}")),
    }
}

#[tauri::command]
pub(crate) async fn chat_with_role_stream(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    message: String,
    role: String,
    thread_id: String,
    request_id: String,
    on_event: Channel<ChatStreamEvent>,
) -> Result<project::Project, String> {
    run_chat_stream(
        runtime.inner(),
        project_name,
        role,
        thread_id,
        request_id,
        ChatInput::NewMessage(message),
        on_event,
    )
    .await
}

#[tauri::command]
pub(crate) async fn regenerate_chat_reply_stream(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    user_message_id: String,
    role: String,
    thread_id: String,
    request_id: String,
    on_event: Channel<ChatStreamEvent>,
) -> Result<project::Project, String> {
    run_chat_stream(
        runtime.inner(),
        project_name,
        role,
        thread_id,
        request_id,
        ChatInput::RetryUserMessage(user_message_id),
        on_event,
    )
    .await
}

#[tauri::command]
pub(crate) async fn chat_with_role_runtime(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    message: String,
    role: String,
    thread_id: String,
) -> Result<crate::runtime_snapshot::RuntimeMutationResult, String> {
    chat_with_role(runtime, project_name.clone(), message, role, thread_id).await?;
    crate::runtime_snapshot::mutation_result(
        &project_name,
        None,
        crate::runtime_snapshot::RuntimeActionSummary::silent("chat_with_role"),
        false,
    )
}

#[tauri::command]
pub(crate) async fn chat_with_role_stream_runtime(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    message: String,
    role: String,
    thread_id: String,
    request_id: String,
    on_event: Channel<ChatStreamEvent>,
) -> Result<crate::runtime_snapshot::RuntimeMutationResult, String> {
    chat_with_role_stream(
        runtime,
        project_name.clone(),
        message,
        role,
        thread_id,
        request_id,
        on_event,
    )
    .await?;
    crate::runtime_snapshot::mutation_result(
        &project_name,
        None,
        crate::runtime_snapshot::RuntimeActionSummary::silent("chat_with_role_stream"),
        false,
    )
}

#[tauri::command]
pub(crate) async fn regenerate_chat_reply_stream_runtime(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    project_name: String,
    user_message_id: String,
    role: String,
    thread_id: String,
    request_id: String,
    on_event: Channel<ChatStreamEvent>,
) -> Result<crate::runtime_snapshot::RuntimeMutationResult, String> {
    regenerate_chat_reply_stream(
        runtime,
        project_name.clone(),
        user_message_id,
        role,
        thread_id,
        request_id,
        on_event,
    )
    .await?;
    crate::runtime_snapshot::mutation_result(
        &project_name,
        None,
        crate::runtime_snapshot::RuntimeActionSummary::silent("regenerate_chat_reply_stream"),
        false,
    )
}

async fn run_chat_stream(
    runtime: &crate::chat_runtime::ChatRuntimeState,
    project_name: String,
    role: String,
    thread_id: String,
    request_id: String,
    input: ChatInput,
    on_event: Channel<ChatStreamEvent>,
) -> Result<project::Project, String> {
    let lease = runtime.begin(
        request_id.clone(),
        project_name.clone(),
        thread_id.clone(),
        role.clone(),
    )?;
    let active = lease.active().clone();
    send_stream_event(
        &on_event,
        ChatStreamEvent::Started {
            request_id: active.request_id.clone(),
            thread_id: active.thread_id.clone(),
            role: active.role.clone(),
        },
    )?;

    let prepared = match prepare_chat(runtime, &project_name, &thread_id, &role, input) {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = send_stream_event(
                &on_event,
                ChatStreamEvent::Failed {
                    request_id,
                    thread_id,
                    role,
                    message_id: None,
                    error: error.clone(),
                    retryable: false,
                },
            );
            return Err(error);
        }
    };
    let reply_id = uuid::Uuid::new_v4().to_string();
    let reply_timestamp = now_millis();
    if let Err(event_error) = send_stream_event(
        &on_event,
        ChatStreamEvent::UserSaved {
            request_id: request_id.clone(),
            thread_id: thread_id.clone(),
            role: role.clone(),
            message: prepared.user_message.clone(),
        },
    ) {
        let project = persist_channel_interruption(
            runtime,
            &project_name,
            &thread_id,
            &role,
            String::new(),
            &prepared.user_message.id,
            &reply_id,
            reply_timestamp,
            event_error,
        )?;
        lease.finish();
        return Ok(project);
    }

    if let Err(event_error) = send_stream_event(
        &on_event,
        ChatStreamEvent::ReplyStarted {
            request_id: request_id.clone(),
            thread_id: thread_id.clone(),
            role: role.clone(),
            message_id: reply_id.clone(),
            timestamp: reply_timestamp,
        },
    ) {
        let project = persist_channel_interruption(
            runtime,
            &project_name,
            &thread_id,
            &role,
            String::new(),
            &prepared.user_message.id,
            &reply_id,
            reply_timestamp,
            event_error,
        )?;
        lease.finish();
        return Ok(project);
    }

    let cancellation = active.cancellation_flag();
    let mut partial_reply = String::new();
    let model_context = crate::cost_ledger::ModelCallContext::for_project(
        &crate::load_project(&project_name)?,
        crate::cost_ledger::ModelCallPurpose::Discussion,
    );
    let stream_result = crate::api::call_deepseek_api_stream_with_context(
        prepared.system_prompt,
        &prepared.context,
        cancellation,
        |delta| {
            record_delta_before_emit(&mut partial_reply, delta, || {
                send_stream_event(
                    &on_event,
                    ChatStreamEvent::Delta {
                        request_id: request_id.clone(),
                        thread_id: thread_id.clone(),
                        role: role.clone(),
                        text: delta.to_string(),
                    },
                )
            })
        },
        model_context,
    )
    .await;

    match stream_result {
        Ok(response) => {
            if active.is_cancelled() {
                let produced_change = !partial_reply.trim().is_empty();
                let project = persist_terminal_reply(
                    runtime,
                    &project_name,
                    &thread_id,
                    &role,
                    partial_reply,
                    CANCELLED_MESSAGE_TYPE,
                    &prepared.user_message.id,
                    &reply_id,
                    reply_timestamp,
                )?;
                mark_discussion_call(&project_name, &response.metadata.call_id, produced_change);
                let _ = send_stream_event(
                    &on_event,
                    ChatStreamEvent::Cancelled {
                        request_id,
                        thread_id,
                        role,
                        message_id: Some(reply_id),
                    },
                );
                lease.finish();
                return Ok(project);
            }
            let project = persist_reply(
                runtime,
                &project_name,
                &thread_id,
                new_reply_message(
                    &role,
                    response.content,
                    None,
                    &prepared.user_message.id,
                    reply_id.clone(),
                    reply_timestamp,
                ),
            )
            .map_err(|error| {
                let _ = send_stream_event(
                    &on_event,
                    ChatStreamEvent::Failed {
                        request_id: request_id.clone(),
                        thread_id: thread_id.clone(),
                        role: role.clone(),
                        message_id: Some(reply_id.clone()),
                        error: format!("最终回复保存失败：{error}。请同步项目后重试。"),
                        retryable: false,
                    },
                );
                format!("最终回复保存失败：{error}")
            })?;
            mark_discussion_call(&project_name, &response.metadata.call_id, true);
            let _ = send_stream_event(
                &on_event,
                ChatStreamEvent::Completed {
                    request_id,
                    thread_id,
                    role,
                    message_id: reply_id,
                },
            );
            lease.finish();
            Ok(project)
        }
        Err(error) if error.is_cancelled() => {
            let produced_change = !partial_reply.trim().is_empty();
            let project = persist_terminal_reply(
                runtime,
                &project_name,
                &thread_id,
                &role,
                partial_reply,
                CANCELLED_MESSAGE_TYPE,
                &prepared.user_message.id,
                &reply_id,
                reply_timestamp,
            )?;
            mark_discussion_call(&project_name, &error.metadata().call_id, produced_change);
            let _ = send_stream_event(
                &on_event,
                ChatStreamEvent::Cancelled {
                    request_id,
                    thread_id,
                    role,
                    message_id: Some(reply_id),
                },
            );
            lease.finish();
            Ok(project)
        }
        Err(error) if active.is_cancelled() => {
            let produced_change = !partial_reply.trim().is_empty();
            let project = persist_terminal_reply(
                runtime,
                &project_name,
                &thread_id,
                &role,
                partial_reply,
                CANCELLED_MESSAGE_TYPE,
                &prepared.user_message.id,
                &reply_id,
                reply_timestamp,
            )?;
            mark_discussion_call(&project_name, &error.metadata().call_id, produced_change);
            let _ = send_stream_event(
                &on_event,
                ChatStreamEvent::Cancelled {
                    request_id,
                    thread_id,
                    role,
                    message_id: Some(reply_id),
                },
            );
            lease.finish();
            Ok(project)
        }
        Err(error) => {
            let produced_change = !partial_reply.trim().is_empty();
            let call_id = error.metadata().call_id.clone();
            let error = error.to_string();
            let project = persist_terminal_reply(
                runtime,
                &project_name,
                &thread_id,
                &role,
                partial_reply,
                INTERRUPTED_MESSAGE_TYPE,
                &prepared.user_message.id,
                &reply_id,
                reply_timestamp,
            )
            .map_err(|save_error| {
                format!("AI 回复失败（{error}），且中断状态保存失败：{save_error}")
            })?;
            mark_discussion_call(&project_name, &call_id, produced_change);
            let _ = send_stream_event(
                &on_event,
                ChatStreamEvent::Failed {
                    request_id,
                    thread_id,
                    role,
                    message_id: Some(reply_id),
                    error,
                    retryable: true,
                },
            );
            lease.finish();
            Ok(project)
        }
    }
}

#[tauri::command]
pub(crate) fn cancel_chat_stream(
    runtime: State<'_, crate::chat_runtime::ChatRuntimeState>,
    request_id: String,
    thread_id: String,
) -> Result<bool, String> {
    runtime.cancel(&request_id, &thread_id)
}

fn prepare_chat(
    runtime: &crate::chat_runtime::ChatRuntimeState,
    project_name: &str,
    thread_id: &str,
    role: &str,
    input: ChatInput,
) -> Result<PreparedChat, String> {
    let system_prompt = system_prompt_for_role(role)?;
    runtime.with_project_mutation(project_name, || {
        let mut project = crate::load_project(project_name)?;
        ensure_chat_is_unlocked(&project)?;
        ensure_discussion_thread_is_active(&project, thread_id)?;
        let thread_idx = find_thread_index(&project, thread_id)?;

        let (user_message, context_end_message_id) = match input {
            ChatInput::NewMessage(message) => {
                let content = message.trim();
                if content.is_empty() {
                    return Err("消息内容不能为空".to_string());
                }
                if content.chars().count() > MAX_CHAT_MESSAGE_CHARS {
                    return Err(format!(
                        "消息内容超过 {MAX_CHAT_MESSAGE_CHARS} 个字符的上限"
                    ));
                }
                let user_message = project::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "user".to_string(),
                    content: content.to_string(),
                    timestamp: now_millis(),
                    msg_type: None,
                    approved: None,
                    rejected: None,
                    milestone_id: None,
                    reply_to_message_id: None,
                };
                project.discussion_threads[thread_idx]
                    .messages
                    .push(user_message.clone());
                project.discussion_threads[thread_idx].revision = project.discussion_threads
                    [thread_idx]
                    .revision
                    .saturating_add(1);
                if project.discussion_threads[thread_idx].scope
                    == project::DiscussionScope::FirstDiscussion
                {
                    project.discussion_revision = project.discussion_revision.saturating_add(1);
                    invalidate_discussion_derivatives(&mut project);
                } else if project.discussion_threads[thread_idx].scope
                    == project::DiscussionScope::AdjustFuture
                {
                    invalidate_future_milestone_draft(&mut project);
                }
                project.workflow_state.data_revision =
                    project.workflow_state.data_revision.saturating_add(1);
                (user_message, None)
            }
            ChatInput::RetryUserMessage(user_message_id) => {
                let message = project.discussion_threads[thread_idx]
                    .messages
                    .iter()
                    .find(|message| message.id == user_message_id && message.role == "user")
                    .cloned()
                    .ok_or_else(|| format!("找不到可重新生成的用户消息: {user_message_id}"))?;
                (message, Some(user_message_id))
            }
        };

        let context = build_chat_context(&project, thread_idx, context_end_message_id.as_deref());
        if context_end_message_id.is_none() {
            crate::save_and_reload_project(&project)
                .map_err(|error| format!("用户消息保存失败：{error}。请重试。"))?;
        }

        Ok(PreparedChat {
            user_message,
            context,
            system_prompt,
        })
    })
}

fn ensure_chat_is_unlocked(project: &project::Project) -> Result<(), String> {
    if project.workflow_state.current_step == project::WorkflowStep::PlanApproval
        && project
            .plan_draft
            .as_ref()
            .is_some_and(|draft| draft.draft_status == project::DraftStatus::Approved)
    {
        return Err(
            "方案已批准，聊天输入已锁定。如需修改方案，请使用「重新讨论方案」功能。".to_string(),
        );
    }
    Ok(())
}

fn system_prompt_for_role(role: &str) -> Result<&'static str, String> {
    match role {
        "策略产品经理" => Ok(crate::prompts::STRATEGY_PROMPT),
        "产品经理" => Ok(crate::prompts::PM_PROMPT),
        "域负责人" => Ok(crate::prompts::DOMAIN_LEAD_PROMPT),
        "全栈技术顾问" => Ok(crate::prompts::TECH_PROMPT),
        "测试工程师" => Ok(crate::prompts::TEST_PROMPT),
        _ => Err(format!("未知角色: {role}")),
    }
}

fn find_thread_index(project: &project::Project, thread_id: &str) -> Result<usize, String> {
    project
        .discussion_threads
        .iter()
        .position(|thread| thread.id == thread_id)
        .ok_or_else(|| format!("讨论线程不存在: {thread_id}"))
}

fn ensure_discussion_thread_is_active(
    project: &project::Project,
    thread_id: &str,
) -> Result<(), String> {
    let active_id = project.workflow_state.active_discussion_thread_id.as_str();
    if active_id.is_empty() {
        return Err("活动讨论线程尚未完成对账，请同步项目状态。".to_string());
    }
    if active_id != thread_id {
        return Err(format!(
            "讨论线程已切换（当前活动线程为 {}），请同步项目状态。",
            active_id
        ));
    }
    let thread = project
        .discussion_threads
        .iter()
        .find(|candidate| candidate.id == thread_id)
        .ok_or_else(|| format!("活动讨论线程不存在: {thread_id}"))?;
    if thread.status != project::DiscussionThreadStatus::Open {
        return Err("当前讨论线程已关闭，请同步项目状态。".to_string());
    }
    if thread.scope != project.workflow_state.discussion_scope {
        return Err("活动讨论线程与当前讨论范围不一致，请同步项目状态。".to_string());
    }
    if thread.scope != project::DiscussionScope::FirstDiscussion
        && thread.milestone_id != project.current_milestone_id
    {
        return Err("活动讨论线程与当前大阶段不一致，请同步项目状态。".to_string());
    }
    Ok(())
}

fn invalidate_discussion_derivatives(project: &mut project::Project) {
    let now = chrono::Utc::now().to_rfc3339();
    for result in &mut project.preflight_results {
        if !result.stale {
            result.stale = true;
            result.expired_at = Some(now.clone());
        }
    }
    if project
        .plan_draft
        .as_ref()
        .is_some_and(|draft| draft.draft_status == project::DraftStatus::Pending)
    {
        if let Some(mut expired_draft) = project.plan_draft.take() {
            expired_draft.draft_status = project::DraftStatus::Expired;
            expired_draft.expired_at = Some(now);
            project.draft_history.push(expired_draft);
        }
    }
}

pub(crate) fn invalidate_future_milestone_draft(project: &mut project::Project) {
    if let Some(draft) = project.milestone_draft.as_mut().filter(|draft| {
        draft.draft_kind == project::MilestoneDraftKind::FutureOnly
            && draft.status != project::MilestoneDraftStatus::Approved
            && !draft.expired
    }) {
        draft.expired = true;
        draft.expiration_reason = Some("来源讨论线程新增了消息".to_string());
    }
}

fn build_chat_context(
    project: &project::Project,
    thread_idx: usize,
    end_message_id: Option<&str>,
) -> String {
    let thread = &project.discussion_threads[thread_idx];
    let end_index = end_message_id
        .and_then(|id| thread.messages.iter().position(|message| message.id == id))
        .map(|index| index + 1)
        .unwrap_or(thread.messages.len());
    let recent_messages = thread.messages[..end_index]
        .iter()
        .filter(|message| message_is_context_eligible(message))
        .rev()
        .take(MAX_CONTEXT_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev();

    // The shared injection includes a discussion summary, so give it the same
    // filtered snapshot used below. Otherwise interrupted or post-retry messages
    // could leak back into the model context through the summary.
    let mut context_project = project.clone();
    let mut context_thread = thread.clone();
    context_thread.messages = thread.messages[..end_index]
        .iter()
        .filter(|message| message_is_context_eligible(message))
        .cloned()
        .collect();
    context_project.discussion_threads = vec![context_thread];

    let mut context = String::new();
    context.push_str(&format!("[项目: {}]\n", project.name));
    context.push_str(&format!(
        "[工作流步骤: {:?}]\n",
        project.workflow_state.current_step
    ));
    context.push_str(&format!(
        "[讨论范围: {:?}]\n",
        project.workflow_state.discussion_scope
    ));
    let injection = crate::constitution_context::build_context_injection(&context_project);
    if !injection.is_empty() {
        context.push_str(&injection);
        context.push('\n');
    }
    context.push_str("## 讨论历史\n");
    for message in recent_messages {
        let display_role = if message.role == "user" {
            "用户"
        } else {
            &message.role
        };
        context.push_str(&format!("{}: {}\n", display_role, message.content));
    }
    context
}

fn message_is_context_eligible(message: &project::Message) -> bool {
    !matches!(
        message.msg_type.as_deref(),
        Some(CANCELLED_MESSAGE_TYPE | INTERRUPTED_MESSAGE_TYPE | "ai_failure")
    )
}

fn persist_terminal_reply(
    runtime: &crate::chat_runtime::ChatRuntimeState,
    project_name: &str,
    thread_id: &str,
    role: &str,
    content: String,
    message_type: &str,
    user_message_id: &str,
    reply_id: &str,
    timestamp: u64,
) -> Result<project::Project, String> {
    let content = if content.is_empty() {
        "本次回复未生成内容。".to_string()
    } else {
        content
    };
    persist_reply(
        runtime,
        project_name,
        thread_id,
        new_reply_message(
            role,
            content,
            Some(message_type),
            user_message_id,
            reply_id.to_string(),
            timestamp,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_channel_interruption(
    runtime: &crate::chat_runtime::ChatRuntimeState,
    project_name: &str,
    thread_id: &str,
    role: &str,
    content: String,
    user_message_id: &str,
    reply_id: &str,
    timestamp: u64,
    event_error: String,
) -> Result<project::Project, String> {
    persist_terminal_reply(
        runtime,
        project_name,
        thread_id,
        role,
        content,
        INTERRUPTED_MESSAGE_TYPE,
        user_message_id,
        reply_id,
        timestamp,
    )
    .map_err(|save_error| format!("{event_error}，且 Channel 中断状态保存失败：{save_error}"))
}

fn record_delta_before_emit(
    partial_reply: &mut String,
    delta: &str,
    emit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    partial_reply.push_str(delta);
    emit()
}

fn new_reply_message(
    role: &str,
    content: String,
    message_type: Option<&str>,
    user_message_id: &str,
    reply_id: String,
    timestamp: u64,
) -> project::Message {
    project::Message {
        id: reply_id,
        role: role.to_string(),
        content,
        timestamp,
        msg_type: message_type.map(str::to_string),
        approved: None,
        rejected: None,
        milestone_id: None,
        reply_to_message_id: Some(user_message_id.to_string()),
    }
}

fn persist_reply(
    runtime: &crate::chat_runtime::ChatRuntimeState,
    project_name: &str,
    thread_id: &str,
    reply: project::Message,
) -> Result<project::Project, String> {
    runtime.with_project_mutation(project_name, || {
        let mut latest = crate::load_project(project_name)?;
        append_reply_to_project(&mut latest, thread_id, reply)?;
        crate::save_and_reload_project(&latest)
    })
}

fn append_reply_to_project(
    latest: &mut project::Project,
    thread_id: &str,
    reply: project::Message,
) -> Result<(), String> {
    ensure_discussion_thread_is_active(latest, thread_id)?;
    let thread_idx = find_thread_index(latest, thread_id)?;
    let user_message_id = reply
        .reply_to_message_id
        .as_deref()
        .ok_or_else(|| "AI 回复缺少原用户消息引用".to_string())?;
    if !latest.discussion_threads[thread_idx]
        .messages
        .iter()
        .any(|message| message.id == user_message_id && message.role == "user")
    {
        return Err("原用户消息已不存在，拒绝保存 AI 回复".to_string());
    }
    if latest.discussion_threads[thread_idx]
        .messages
        .iter()
        .any(|message| message.id == reply.id)
    {
        return Err(format!("AI 回复消息标识已存在: {}", reply.id));
    }
    latest.discussion_threads[thread_idx].messages.push(reply);
    latest.discussion_threads[thread_idx].revision = latest.discussion_threads[thread_idx]
        .revision
        .saturating_add(1);
    if latest.discussion_threads[thread_idx].scope == project::DiscussionScope::AdjustFuture {
        invalidate_future_milestone_draft(latest);
    }
    latest.workflow_state.data_revision = latest.workflow_state.data_revision.saturating_add(1);
    Ok(())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        id: &str,
        role: &str,
        content: &str,
        message_type: Option<&str>,
    ) -> project::Message {
        project::Message {
            id: id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            timestamp: 1,
            msg_type: message_type.map(str::to_string),
            approved: None,
            rejected: None,
            milestone_id: None,
            reply_to_message_id: None,
        }
    }

    #[test]
    fn interrupted_messages_are_excluded_from_future_context() {
        let mut project = project::Project::new("context-test");
        project.discussion_threads[0].messages = vec![
            message("u1", "user", "first", None),
            message(
                "a1",
                "产品经理",
                "partial secret",
                Some(INTERRUPTED_MESSAGE_TYPE),
            ),
            message("u2", "user", "second", None),
        ];

        let context = build_chat_context(&project, 0, None);
        assert!(context.contains("first"));
        assert!(context.contains("second"));
        assert!(!context.contains("partial secret"));
    }

    #[test]
    fn retry_context_stops_at_original_user_message() {
        let mut project = project::Project::new("retry-test");
        project.discussion_threads[0].messages = vec![
            message("u1", "user", "retry this", None),
            message("u2", "user", "later request", None),
        ];

        let context = build_chat_context(&project, 0, Some("u1"));
        assert!(context.contains("retry this"));
        assert!(!context.contains("later request"));
    }

    #[test]
    fn context_summary_uses_the_selected_thread_only() {
        let mut project = project::Project::new("thread-test");
        project.discussion_threads[0].messages =
            vec![message("u1", "user", "other thread content", None)];
        project.discussion_threads.push(project::DiscussionThread {
            id: "thread-second".to_string(),
            title: "second".to_string(),
            node_id: "node".to_string(),
            messages: vec![message("u2", "user", "selected thread content", None)],
            ..Default::default()
        });

        let context = build_chat_context(&project, 1, None);
        assert!(context.contains("selected thread content"));
        assert!(!context.contains("other thread content"));
    }

    #[test]
    fn inactive_or_wrong_scope_threads_are_rejected() {
        let mut project = project::Project::new("active-thread-test");
        project.discussion_threads.push(project::DiscussionThread {
            id: "thread-other".to_string(),
            title: "other".to_string(),
            node_id: "root".to_string(),
            ..Default::default()
        });
        let inactive = ensure_discussion_thread_is_active(&project, "thread-other")
            .expect_err("非活动线程必须被拒绝");
        assert!(inactive.contains("已切换"));

        project.workflow_state.active_discussion_thread_id = "thread-other".to_string();
        project.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
        let wrong_scope = ensure_discussion_thread_is_active(&project, "thread-other")
            .expect_err("作用域错误必须被拒绝");
        assert!(wrong_scope.contains("讨论范围不一致"));
    }

    #[test]
    fn future_discussion_keeps_but_expires_the_existing_draft() {
        let mut project = project::Project::new("future-draft-expiration");
        project.milestone_draft = Some(project::MilestoneDraft {
            draft_kind: project::MilestoneDraftKind::FutureOnly,
            ..Default::default()
        });

        invalidate_future_milestone_draft(&mut project);

        let draft = project
            .milestone_draft
            .as_ref()
            .expect("expired draft remains visible");
        assert!(draft.expired);
        assert!(draft.expiration_reason.is_some());
    }

    #[test]
    fn records_a_delta_before_reporting_channel_failure() {
        let mut partial = String::from("first");
        let result =
            record_delta_before_emit(
                &mut partial,
                " second",
                || Err("channel closed".to_string()),
            );

        assert_eq!(result, Err("channel closed".to_string()));
        assert_eq!(partial, "first second");
    }

    #[test]
    fn cancelled_partial_reply_round_trips_through_project_storage() -> Result<(), String> {
        let temp_root =
            std::env::temp_dir().join(format!("metheus-chat-cancelled-{}", uuid::Uuid::new_v4()));
        let path = temp_root.join("project.json");
        let mut value = project::Project::new("cancelled-persistence-test");
        let thread_id = value.discussion_threads[0].id.clone();
        value.discussion_threads[0]
            .messages
            .push(message("user-1", "user", "question", None));
        let reply = new_reply_message(
            "产品经理",
            "partial answer".to_string(),
            Some(CANCELLED_MESSAGE_TYPE),
            "user-1",
            "reply-1".to_string(),
            2,
        );
        append_reply_to_project(&mut value, &thread_id, reply)?;
        assert_eq!(value.discussion_threads[0].revision, 1);

        crate::save_project_to_path(&value, &path)?;
        let stored = crate::load_project_from_path(&path)?;
        let stored_reply = stored.discussion_threads[0]
            .messages
            .last()
            .ok_or_else(|| "取消回复未保存".to_string())?;
        assert_eq!(stored_reply.content, "partial answer");
        assert_eq!(
            stored_reply.msg_type.as_deref(),
            Some(CANCELLED_MESSAGE_TYPE)
        );
        assert_eq!(stored_reply.reply_to_message_id.as_deref(), Some("user-1"));

        std::fs::remove_dir_all(&temp_root)
            .map_err(|error| format!("清理聊天测试目录失败：{error}"))?;
        Ok(())
    }

    #[test]
    fn rejects_duplicate_reply_ids_without_duplicating_the_user_message() {
        let mut value = project::Project::new("retry-persistence-test");
        let thread_id = value.discussion_threads[0].id.clone();
        value.discussion_threads[0]
            .messages
            .push(message("user-1", "user", "retry this", None));
        let reply = new_reply_message(
            "产品经理",
            "first reply".to_string(),
            None,
            "user-1",
            "reply-1".to_string(),
            2,
        );
        append_reply_to_project(&mut value, &thread_id, reply.clone()).unwrap();

        let error = append_reply_to_project(&mut value, &thread_id, reply)
            .expect_err("重复回复标识必须被拒绝");
        assert!(error.contains("AI 回复消息标识已存在"));
        assert_eq!(
            value.discussion_threads[0]
                .messages
                .iter()
                .filter(|item| item.id == "user-1")
                .count(),
            1
        );
    }

    #[test]
    fn project_storage_surfaces_a_final_atomic_replace_failure() -> Result<(), String> {
        let temp_root = std::env::temp_dir().join(format!(
            "metheus-chat-save-failure-{}",
            uuid::Uuid::new_v4()
        ));
        let blocked_path = temp_root.join("project.json");
        std::fs::create_dir_all(&blocked_path)
            .map_err(|error| format!("创建聊天保存失败测试目录失败：{error}"))?;

        let value = project::Project::new("save-failure-test");
        let error = crate::save_project_to_path(&value, &blocked_path)
            .expect_err("目录目标必须导致原子替换失败");
        assert!(error.contains("替换项目文件失败"));
        assert!(!blocked_path.with_extension("json.tmp").exists());

        std::fs::remove_dir_all(&temp_root)
            .map_err(|cleanup_error| format!("清理聊天保存失败测试目录失败：{cleanup_error}"))?;
        Ok(())
    }
}
