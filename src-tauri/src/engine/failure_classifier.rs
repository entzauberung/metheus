use crate::project::EngineFailureKind;

/// Classify provider failures from both output streams. Provider CLIs do not
/// consistently choose stderr for API failures, so stream-specific parsing is unsafe.
pub(crate) fn classify_process_failure(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> EngineFailureKind {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();

    if combined.contains("insufficient balance")
        || combined.contains("insufficient_balance")
        || combined.contains("quota exceeded")
        || combined.contains("credit balance")
        || combined.contains("billing hard limit")
        || combined.contains("api error: 402")
        || combined.contains("status 402")
    {
        return EngineFailureKind::QuotaExceeded;
    }
    if combined.contains("unauthorized")
        || combined.contains("authentication failed")
        || combined.contains("invalid api key")
        || combined.contains("invalid token")
        || combined.contains("api error: 401")
        || combined.contains("api error: 403")
    {
        return EngineFailureKind::AuthenticationError;
    }
    if combined.contains("rate limit")
        || combined.contains("too many requests")
        || combined.contains("api error: 429")
        || combined.contains("status 429")
    {
        return EngineFailureKind::RateLimited;
    }
    if combined.contains("service unavailable")
        || combined.contains("bad gateway")
        || combined.contains("gateway timeout")
        || combined.contains("api error: 500")
        || combined.contains("api error: 502")
        || combined.contains("api error: 503")
        || combined.contains("api error: 504")
    {
        return EngineFailureKind::ProviderUnavailable;
    }
    if combined.contains("network error")
        || combined.contains("connection refused")
        || combined.contains("connection reset")
        || combined.contains("dns")
        || combined.contains("timed out while connecting")
    {
        return EngineFailureKind::NetworkError;
    }
    if exit_code.is_none() || matches!(exit_code, Some(126 | 127 | 134 | 137 | 139)) {
        return EngineFailureKind::ProcessCrash;
    }
    EngineFailureKind::TaskExecutionError
}

pub(crate) fn blocks_code_recovery(kind: &EngineFailureKind) -> bool {
    matches!(
        kind,
        EngineFailureKind::QuotaExceeded
            | EngineFailureKind::AuthenticationError
            | EngineFailureKind::RateLimited
            | EngineFailureKind::ProviderUnavailable
            | EngineFailureKind::NetworkError
            | EngineFailureKind::Timeout
            | EngineFailureKind::ProcessCrash
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_quota_from_stdout() {
        assert_eq!(
            classify_process_failure(Some(1), "API Error: 402 Insufficient Balance", ""),
            EngineFailureKind::QuotaExceeded
        );
    }

    #[test]
    fn classifies_common_provider_failures() {
        assert_eq!(
            classify_process_failure(Some(1), "", "401 Unauthorized: invalid API key"),
            EngineFailureKind::AuthenticationError
        );
        assert_eq!(
            classify_process_failure(Some(1), "API Error: 429 Too Many Requests", ""),
            EngineFailureKind::RateLimited
        );
        assert_eq!(
            classify_process_failure(Some(1), "", "503 Service Unavailable"),
            EngineFailureKind::ProviderUnavailable
        );
    }
}
