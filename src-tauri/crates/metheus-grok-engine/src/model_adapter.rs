use crate::MAX_MODEL_OUTPUT_TOKENS;
use crate::config::{GrokBuildApiBackend, GrokBuildExecutionConfig};
use crate::error::{GrokBuildRuntimeError, GrokBuildRuntimeErrorKind};
use std::time::Instant;
use tokio::sync::mpsc;
use xai_grok_sampler::{AuthScheme, RetryPolicy, SamplerActor, SamplerConfig};
use xai_grok_sampling_types::{
    ApiBackend, ConversationItem, ConversationRequest, ConversationResponse,
};

#[derive(Debug, Clone)]
pub struct GrokBuildConnectionTestResult {
    pub success: bool,
    pub model: String,
    pub latency_ms: u64,
    pub error_kind: Option<GrokBuildRuntimeErrorKind>,
    pub message: String,
}

pub(crate) fn retry_policy(config: &GrokBuildExecutionConfig) -> RetryPolicy {
    RetryPolicy {
        max_retries: config.max_transport_retries,
        rate_limit_retry_threshold: 1,
    }
}

pub(crate) fn sampling_config(config: &GrokBuildExecutionConfig) -> SamplerConfig {
    SamplerConfig {
        api_key: Some(config.api_key.clone()),
        base_url: config.api_base_url.trim_end_matches('/').to_string(),
        model: config.model.clone(),
        max_completion_tokens: Some(MAX_MODEL_OUTPUT_TOKENS),
        temperature: Some(0.0),
        api_backend: match config.api_backend {
            GrokBuildApiBackend::ChatCompletions => ApiBackend::ChatCompletions,
            GrokBuildApiBackend::Responses => ApiBackend::Responses,
            GrokBuildApiBackend::Messages => ApiBackend::Messages,
        },
        auth_scheme: if config.api_backend == GrokBuildApiBackend::Messages {
            AuthScheme::XApiKey
        } else {
            AuthScheme::Bearer
        },
        max_retries: Some(config.max_transport_retries),
        stream_tool_calls: false,
        idle_timeout_secs: Some(config.timeout_secs),
        origin_client: Some(xai_grok_sampler::OriginClientInfo {
            product: "metheus".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
        client_identifier: Some("metheus-built-in-grok-build".to_string()),
        ..Default::default()
    }
}

pub(crate) async fn sample(
    config: &GrokBuildExecutionConfig,
    request: ConversationRequest,
) -> Result<ConversationResponse, GrokBuildRuntimeError> {
    config.validate()?;
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let sampler = SamplerActor::spawn(sampling_config(config), retry_policy(config), event_tx);
    sampler
        .submit_and_collect(xai_grok_sampler::RequestId::random(), request)
        .await
        .map(|(response, _)| response)
        .map_err(|error| GrokBuildRuntimeError::from_sampling(error, &config.api_key))
}

pub async fn test_model_connection(
    config: GrokBuildExecutionConfig,
) -> GrokBuildConnectionTestResult {
    let started = Instant::now();
    let model = config.model.clone();
    let request = ConversationRequest::from_items(vec![ConversationItem::user(
        "Reply with exactly OK and no other text.",
    )])
    .with_model(model.clone())
    .with_max_output_tokens(32);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(config.timeout_secs),
        sample(&config, request),
    )
    .await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(Ok(response)) if !response.assistant_text().trim().is_empty() => {
            GrokBuildConnectionTestResult {
                success: true,
                model,
                latency_ms,
                error_kind: None,
                message: "Grok Build model connection succeeded".to_string(),
            }
        }
        Ok(Ok(_)) => GrokBuildConnectionTestResult {
            success: false,
            model,
            latency_ms,
            error_kind: Some(GrokBuildRuntimeErrorKind::Protocol),
            message: "Grok Build model returned an empty response".to_string(),
        },
        Ok(Err(error)) => GrokBuildConnectionTestResult {
            success: false,
            model,
            latency_ms,
            error_kind: Some(error.kind),
            message: error.message().to_string(),
        },
        Err(_) => GrokBuildConnectionTestResult {
            success: false,
            model,
            latency_ms,
            error_kind: Some(GrokBuildRuntimeErrorKind::Timeout),
            message: format!(
                "Grok Build model connection timed out after {} seconds",
                config.timeout_secs
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(api_backend: GrokBuildApiBackend) -> GrokBuildExecutionConfig {
        GrokBuildExecutionConfig {
            api_backend,
            api_base_url: "https://example.invalid/v1/".to_string(),
            model: "test-model".to_string(),
            api_key: "test-secret".to_string(),
            timeout_secs: 30,
            max_turns: 4,
            max_transport_retries: 0,
            max_doom_loop_retries: 0,
        }
    }

    #[test]
    fn messages_uses_x_api_key_authentication() {
        let sampler = sampling_config(&config(GrokBuildApiBackend::Messages));
        assert!(matches!(sampler.auth_scheme, AuthScheme::XApiKey));
        assert_eq!(sampler.base_url, "https://example.invalid/v1");
    }

    #[test]
    fn openai_backends_use_bearer_authentication() {
        for backend in [
            GrokBuildApiBackend::ChatCompletions,
            GrokBuildApiBackend::Responses,
        ] {
            let sampler = sampling_config(&config(backend));
            assert!(matches!(sampler.auth_scheme, AuthScheme::Bearer));
        }
    }

    #[test]
    fn adaptive_grok_contract_zero_transport_budget_disables_sampler_retries() {
        let config = config(GrokBuildApiBackend::Responses);

        assert_eq!(sampling_config(&config).max_retries, Some(0));
        assert_eq!(retry_policy(&config).max_retries, 0);
        assert_eq!(retry_policy(&config).rate_limit_retry_threshold, 1);
    }

    #[test]
    fn adaptive_grok_contract_nonzero_transport_budget_reaches_sampler_config() {
        let mut config = config(GrokBuildApiBackend::Responses);
        config.max_transport_retries = 3;

        assert_eq!(sampling_config(&config).max_retries, Some(3));
        assert_eq!(retry_policy(&config).max_retries, 3);
        assert_eq!(retry_policy(&config).rate_limit_retry_threshold, 1);
    }
}
