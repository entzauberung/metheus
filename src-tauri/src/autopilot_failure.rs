use crate::project;

pub(crate) const MAX_TRANSIENT_RETRIES: u32 = 3;
const RETRY_DELAYS_SECS: [u64; 3] = [5, 15, 45];

pub(crate) fn retry_delay_secs(attempt: u32) -> Option<u64> {
    if attempt == 0 || attempt > MAX_TRANSIENT_RETRIES {
        None
    } else {
        Some(RETRY_DELAYS_SECS[(attempt - 1) as usize])
    }
}

pub(crate) fn from_engine_failure(
    kind: &project::EngineFailureKind,
) -> project::AutopilotFailureKind {
    match kind {
        project::EngineFailureKind::QuotaExceeded => project::AutopilotFailureKind::Quota,
        project::EngineFailureKind::AuthenticationError => {
            project::AutopilotFailureKind::Authentication
        }
        project::EngineFailureKind::RateLimited => project::AutopilotFailureKind::RateLimited,
        project::EngineFailureKind::ProviderUnavailable => {
            project::AutopilotFailureKind::ProviderUnavailable
        }
        project::EngineFailureKind::NetworkError => project::AutopilotFailureKind::Network,
        project::EngineFailureKind::Timeout => project::AutopilotFailureKind::Timeout,
        project::EngineFailureKind::ProcessCrash => project::AutopilotFailureKind::ProcessCrash,
        project::EngineFailureKind::ToolRejected
        | project::EngineFailureKind::ProtocolError
        | project::EngineFailureKind::OutputTruncated
        | project::EngineFailureKind::MaxTurnsExceeded
        | project::EngineFailureKind::RuntimeError
        | project::EngineFailureKind::TaskExecutionError => {
            project::AutopilotFailureKind::Permanent
        }
    }
}

pub(crate) fn classify_message(message: &str) -> project::AutopilotFailureKind {
    let value = message.to_lowercase();
    if value.contains("401")
        || value.contains("403")
        || value.contains("认证")
        || value.contains("unauthorized")
    {
        project::AutopilotFailureKind::Authentication
    } else if value.contains("402")
        || value.contains("额度")
        || value.contains("quota")
        || value.contains("balance")
    {
        project::AutopilotFailureKind::Quota
    } else if value.contains("429") || value.contains("限流") || value.contains("rate limit") {
        project::AutopilotFailureKind::RateLimited
    } else if value.contains("503")
        || value.contains("502")
        || value.contains("504")
        || value.contains("服务暂")
    {
        project::AutopilotFailureKind::ProviderUnavailable
    } else if value.contains("timeout") || value.contains("超时") {
        project::AutopilotFailureKind::Timeout
    } else if value.contains("network") || value.contains("连接") || value.contains("dns") {
        project::AutopilotFailureKind::Network
    } else if value.contains("修订号已变化") || value.contains("事实已变化") {
        project::AutopilotFailureKind::RevisionConflict
    } else if value.contains("工作区") || value.contains("外部修改") {
        project::AutopilotFailureKind::WorkspaceChanged
    } else if value.contains("契约") || value.contains("矛盾") {
        project::AutopilotFailureKind::ContractContradiction
    } else {
        project::AutopilotFailureKind::Permanent
    }
}

pub(crate) fn is_transient(kind: &project::AutopilotFailureKind) -> bool {
    matches!(
        kind,
        project::AutopilotFailureKind::Network
            | project::AutopilotFailureKind::RateLimited
            | project::AutopilotFailureKind::ProviderUnavailable
            | project::AutopilotFailureKind::Timeout
            | project::AutopilotFailureKind::RevisionConflict
            | project::AutopilotFailureKind::ProcessCrash
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_is_finite() {
        assert_eq!(retry_delay_secs(1), Some(5));
        assert_eq!(retry_delay_secs(2), Some(15));
        assert_eq!(retry_delay_secs(3), Some(45));
        assert_eq!(retry_delay_secs(4), None);
    }

    #[test]
    fn auth_and_quota_are_not_transient() {
        assert!(!is_transient(&classify_message("401 Unauthorized")));
        assert!(!is_transient(&classify_message("quota exceeded")));
        assert!(is_transient(&classify_message("429 rate limit")));
    }

    #[test]
    fn adaptive_execution_contract_runtime_errors_are_human_boundaries() {
        for kind in [
            project::EngineFailureKind::ToolRejected,
            project::EngineFailureKind::ProtocolError,
            project::EngineFailureKind::MaxTurnsExceeded,
            project::EngineFailureKind::RuntimeError,
        ] {
            let failure = from_engine_failure(&kind);
            assert_eq!(failure, project::AutopilotFailureKind::Permanent);
            assert!(!is_transient(&failure));
        }
    }

    #[test]
    fn output_truncated_is_permanent_not_transport_retry() {
        // Permanent means no 1/3 transport/transient backoff. Automatic recovery still
        // proceeds via RunAutomaticRecovery + Replanning (current-subtask replan only).
        let failure = from_engine_failure(&project::EngineFailureKind::OutputTruncated);
        assert_eq!(failure, project::AutopilotFailureKind::Permanent);
        assert!(!is_transient(&failure));
        assert_eq!(retry_delay_secs(1), Some(5)); // schedule exists for true transients only
    }
}
