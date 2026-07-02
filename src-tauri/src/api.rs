use std::env;
use crate::constants::DEEPSEEK_API_TIMEOUT_SECS;

pub(crate) async fn call_deepseek_api(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false, 0.1).await
}

// ===== 结构化输出用（强制 JSON） =====
pub(crate) async fn call_deepseek_api_json(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true, 0.5).await
}

// ===== 内部实现 =====
pub(crate) async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[call_deepseek_api_inner] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });

    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message
    }));

    let mut body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": messages,
        "temperature": temperature,
    });

    if force_json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }

    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
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
