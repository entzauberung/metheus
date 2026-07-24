use crate::settings::{
    ConnectionTestResult, DecisionModelSettings, ModelConnectionErrorKind, ModelConnectionTarget,
    StructuredOutputPolicy,
};
use std::fmt;

const MAX_ERROR_CHARS: usize = 2_000;

pub(crate) async fn call_deepseek_api(
    system_prompt: &str,
    user_message: &str,
) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false, 0.1).await
}

pub(crate) async fn call_deepseek_api_json(
    system_prompt: &str,
    user_message: &str,
) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true, 0.5).await
}

pub(crate) async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
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

    call_deepseek_api_messages(messages, force_json, temperature).await
}

pub(crate) async fn call_deepseek_api_messages(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let snapshot = crate::settings::begin_decision_request()?;
    let _settings_revision = snapshot.settings_revision;
    send_openai_compatible(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        force_json,
        temperature,
    )
    .await
    .map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
struct ApiRequestError {
    kind: ModelConnectionErrorKind,
    message: String,
}

impl ApiRequestError {
    fn new(kind: ModelConnectionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ApiRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

async fn send_openai_compatible(
    settings: &DecisionModelSettings,
    api_key: &str,
    mut messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, ApiRequestError> {
    if force_json && settings.structured_output == StructuredOutputPolicy::PromptOnly {
        messages.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": "Return only one valid JSON value. Do not wrap it in Markdown."
            }),
        );
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(settings.timeout_secs))
        .build()
        .map_err(|error| {
            ApiRequestError::new(
                ModelConnectionErrorKind::InvalidConfiguration,
                format!("构造 OpenAI Compatible HTTP 客户端失败：{error}"),
            )
        })?;

    let mut body = serde_json::json!({
        "model": settings.model,
        "messages": messages,
        "temperature": temperature,
    });
    if force_json && settings.structured_output == StructuredOutputPolicy::NativeJsonObject {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }

    let response = client
        .post(&settings.request_url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| classify_transport_error(error, settings.timeout_secs))?;

    parse_response(response, api_key).await
}

async fn parse_response(
    response: reqwest::Response,
    api_key: &str,
) -> Result<String, ApiRequestError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(|error| {
            ApiRequestError::new(
                ModelConnectionErrorKind::Protocol,
                format!("接口返回 HTTP {status}，且错误正文读取失败：{error}"),
            )
        })?;
        let sanitized = sanitize_api_error(&body, api_key);
        return Err(classify_status_error(status, &sanitized));
    }

    let response_data: serde_json::Value = response.json().await.map_err(|error| {
        ApiRequestError::new(
            ModelConnectionErrorKind::Protocol,
            format!("解析 OpenAI Compatible 响应失败：{error}"),
        )
    })?;
    extract_message_content(&response_data).ok_or_else(|| {
        ApiRequestError::new(
            ModelConnectionErrorKind::Protocol,
            "OpenAI Compatible 响应缺少有效 choices[0].message.content",
        )
    })
}

fn extract_message_content(response: &serde_json::Value) -> Option<String> {
    let content = response
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?;
    if let Some(text) = content.as_str().filter(|value| !value.trim().is_empty()) {
        return Some(text.to_string());
    }
    let parts = content.as_array()?;
    let combined = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| part.get("content").and_then(serde_json::Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("");
    (!combined.trim().is_empty()).then_some(combined)
}

fn classify_transport_error(error: reqwest::Error, timeout_secs: u64) -> ApiRequestError {
    if error.is_timeout() {
        ApiRequestError::new(
            ModelConnectionErrorKind::Timeout,
            format!("模型接口请求超时（超过 {timeout_secs} 秒）"),
        )
    } else {
        ApiRequestError::new(
            ModelConnectionErrorKind::Network,
            format!("模型接口网络请求失败：{error}"),
        )
    }
}

fn classify_status_error(status: reqwest::StatusCode, body: &str) -> ApiRequestError {
    let body_lower = body.to_ascii_lowercase();
    let kind = match status.as_u16() {
        401 | 403 => ModelConnectionErrorKind::Authentication,
        429 if ["quota", "credit", "balance", "额度"]
            .iter()
            .any(|marker| body_lower.contains(marker)) =>
        {
            ModelConnectionErrorKind::QuotaExceeded
        }
        429 => ModelConnectionErrorKind::RateLimited,
        500..=599 => ModelConnectionErrorKind::ProviderUnavailable,
        _ => ModelConnectionErrorKind::HttpStatus,
    };
    ApiRequestError::new(kind, format!("模型接口返回 HTTP {status}：{body}"))
}

fn sanitize_api_error(value: &str, api_key: &str) -> String {
    let exact_redacted = if api_key.is_empty() {
        value.to_string()
    } else {
        value.replace(api_key, "[REDACTED]")
    };
    let bearer_redacted = redact_bearer_tokens(&exact_redacted);
    truncate_chars(&bearer_redacted, MAX_ERROR_CHARS)
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    loop {
        let lower = remaining.to_ascii_lowercase();
        let Some(index) = lower.find("bearer ") else {
            result.push_str(remaining);
            break;
        };
        result.push_str(&remaining[..index]);
        result.push_str("Bearer [REDACTED]");
        let token_start = index + "bearer ".len();
        let token_end = remaining[token_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '"' | '\'' | ',' | '，' | ';' | '；' | '}' | ']')
            })
            .map(|offset| token_start + offset)
            .unwrap_or(remaining.len());
        remaining = &remaining[token_end..];
    }
    result
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut output: String = value.chars().take(limit).collect();
    output.push_str("…[截断]");
    output
}

pub(crate) async fn test_model_connection(target: ModelConnectionTarget) -> ConnectionTestResult {
    let started = std::time::Instant::now();
    if target == ModelConnectionTarget::BuiltInGrokBuild {
        let snapshot = match crate::settings::begin_built_in_grok_build_request() {
            Ok(snapshot) => snapshot,
            Err(message) => {
                return ConnectionTestResult {
                    success: false,
                    target,
                    model: String::new(),
                    latency_ms: elapsed_millis(started),
                    error_kind: Some(ModelConnectionErrorKind::MissingSecret),
                    message,
                }
            }
        };
        let _settings_revision = snapshot.settings_revision;
        let config = metheus_grok_engine::GrokBuildExecutionConfig {
            api_backend: match snapshot.settings.api_backend {
                crate::settings::GrokBuildApiBackend::ChatCompletions => {
                    metheus_grok_engine::GrokBuildApiBackend::ChatCompletions
                }
                crate::settings::GrokBuildApiBackend::Responses => {
                    metheus_grok_engine::GrokBuildApiBackend::Responses
                }
                crate::settings::GrokBuildApiBackend::Messages => {
                    metheus_grok_engine::GrokBuildApiBackend::Messages
                }
            },
            api_base_url: snapshot.settings.api_base_url.clone(),
            model: snapshot.settings.model.clone(),
            api_key: snapshot.api_key.clone(),
            timeout_secs: snapshot.settings.timeout_secs,
            max_turns: snapshot.settings.max_turns,
        };
        let result = metheus_grok_engine::test_model_connection(config).await;
        return ConnectionTestResult {
            success: result.success,
            target,
            model: result.model,
            latency_ms: result.latency_ms,
            error_kind: result.error_kind.map(map_grok_connection_error),
            message: result.message,
        };
    }
    let request = crate::settings::begin_decision_request()
        .map(|snapshot| (snapshot.settings, snapshot.api_key, snapshot._activity));

    let (settings, api_key, activity) = match request {
        Ok(request) => request,
        Err(message) => {
            return ConnectionTestResult {
                success: false,
                target,
                model: String::new(),
                latency_ms: elapsed_millis(started),
                error_kind: Some(ModelConnectionErrorKind::MissingSecret),
                message,
            }
        }
    };
    let model = settings.model.clone();
    let result = send_openai_compatible(
        &settings,
        &api_key,
        vec![serde_json::json!({
            "role": "user",
            "content": "Reply with OK."
        })],
        false,
        0.0,
    )
    .await;
    drop(activity);

    match result {
        Ok(_) => ConnectionTestResult {
            success: true,
            target,
            model,
            latency_ms: elapsed_millis(started),
            error_kind: None,
            message: "连接成功".to_string(),
        },
        Err(error) => ConnectionTestResult {
            success: false,
            target,
            model,
            latency_ms: elapsed_millis(started),
            error_kind: Some(error.kind),
            message: error.message,
        },
    }
}

fn map_grok_connection_error(
    kind: metheus_grok_engine::GrokBuildRuntimeErrorKind,
) -> ModelConnectionErrorKind {
    match kind {
        metheus_grok_engine::GrokBuildRuntimeErrorKind::InvalidConfiguration => {
            ModelConnectionErrorKind::InvalidConfiguration
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::Authentication => {
            ModelConnectionErrorKind::Authentication
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::QuotaExceeded => {
            ModelConnectionErrorKind::QuotaExceeded
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::RateLimited => {
            ModelConnectionErrorKind::RateLimited
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::Network => {
            ModelConnectionErrorKind::Network
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::ProviderUnavailable => {
            ModelConnectionErrorKind::ProviderUnavailable
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::Timeout => {
            ModelConnectionErrorKind::Timeout
        }
        metheus_grok_engine::GrokBuildRuntimeErrorKind::Cancelled
        | metheus_grok_engine::GrokBuildRuntimeErrorKind::ToolRejected
        | metheus_grok_engine::GrokBuildRuntimeErrorKind::ToolFailed
        | metheus_grok_engine::GrokBuildRuntimeErrorKind::Protocol
        | metheus_grok_engine::GrokBuildRuntimeErrorKind::MaxTurns
        | metheus_grok_engine::GrokBuildRuntimeErrorKind::Runtime => {
            ModelConnectionErrorKind::Protocol
        }
    }
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn one_shot_server(
        status: &str,
        body: &str,
    ) -> Result<(String, tokio::task::JoinHandle<Result<String, String>>), String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; 16 * 1024];
            let size = socket
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            socket
                .write_all(response.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            Ok(String::from_utf8_lossy(&buffer[..size]).to_string())
        });
        Ok((format!("http://{address}/custom/chat"), handle))
    }

    fn test_settings(request_url: String) -> DecisionModelSettings {
        DecisionModelSettings {
            request_url,
            timeout_secs: 5,
            ..DecisionModelSettings::default()
        }
    }

    #[tokio::test]
    async fn uses_the_configured_full_url_and_parses_text_parts() -> Result<(), String> {
        let body = r#"{"choices":[{"message":{"content":[{"text":"O"},{"text":"K"}]}}]}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let reply = send_openai_compatible(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![serde_json::json!({"role":"user","content":"hello"})],
            false,
            0.0,
        )
        .await
        .map_err(|error| error.to_string())?;
        assert_eq!(reply, "OK");
        let raw_request = request.await.map_err(|error| error.to_string())??;
        assert!(raw_request.starts_with("POST /custom/chat HTTP/1.1"));
        assert!(raw_request.contains("authorization: Bearer metheus-secret-sentinel"));
        Ok(())
    }

    #[tokio::test]
    async fn classifies_and_redacts_authentication_errors() -> Result<(), String> {
        let body = r#"{"error":"Bearer metheus-secret-sentinel is invalid"}"#;
        let (url, request) = one_shot_server("401 Unauthorized", body).await?;
        let error = send_openai_compatible(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![],
            false,
            0.0,
        )
        .await
        .err()
        .ok_or_else(|| "请求应失败".to_string())?;
        assert_eq!(error.kind, ModelConnectionErrorKind::Authentication);
        assert!(!error.message.contains("metheus-secret-sentinel"));
        assert!(error.message.contains("[REDACTED]"));
        request
            .await
            .map_err(|join_error| join_error.to_string())??;
        Ok(())
    }

    #[test]
    fn prompt_only_policy_does_not_require_native_json_support() {
        let settings = DecisionModelSettings {
            structured_output: StructuredOutputPolicy::PromptOnly,
            ..DecisionModelSettings::default()
        };
        assert_eq!(
            settings.structured_output,
            StructuredOutputPolicy::PromptOnly
        );
    }

    #[test]
    fn bearer_redaction_is_case_insensitive_and_unicode_safe() {
        let value = redact_bearer_tokens("错误 bearer secret-token，稍后重试");
        assert_eq!(value, "错误 Bearer [REDACTED]，稍后重试");
    }
}
