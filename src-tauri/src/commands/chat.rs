use crate::project;
use std::env;

#[tauri::command]
pub(crate) fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 把前端发的消息，转交给api，之后把ai的原回复原封不动的拿回来
#[tauri::command]
pub(crate) async fn send_message(message: String) -> Result<String, String> {
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            crate::constants::DEEPSEEK_API_TIMEOUT_SECS,
        ))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [
            {"role": "user", "content": message}
        ]
    });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let reply = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();
    Ok(reply)
}
// ===新增: 多角色对话命令 ===
#[tauri::command]
pub(crate) async fn chat_with_role(
    message: String,
    role: String,
    _thread_id: String,
) -> Result<project::Message, String> {
    //1. 根据角色选择system prompt
    let system_prompt = match role.as_str() {
        "策略产品经理" => crate::prompts::STRATEGY_PROMPT,
        "产品经理" => crate::prompts::PM_PROMPT,
        "域负责人" => crate::prompts::DOMAIN_LEAD_PROMPT,
        "全栈技术顾问" => crate::prompts::TECH_PROMPT,
        "测试工程师" => crate::prompts::TEST_PROMPT,
        _ => return Err(format!("未知角色: {}", role)),
    };
    //4.发送请求 -> 调用ai  3.4.1b改动: 换为封装函数
    let reply = crate::api::call_deepseek_api(&system_prompt, &message).await?;
    //5.返回结构化Message对象（非纯字符串）
    let ai_message = project::Message {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: reply.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        msg_type: None,
        approved: None,
        rejected: None,
        milestone_id: None,
    };
    Ok(ai_message)
}
