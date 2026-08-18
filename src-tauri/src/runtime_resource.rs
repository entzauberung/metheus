use crate::project::{
    ResourceObservationSource, ResourceObservationState, ResourceObservationSummary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResourceThresholds {
    pub warning_reserve_bytes: u64,
    pub hard_stop_reserve_bytes: u64,
}

impl ResourceThresholds {
    pub(crate) fn new(
        warning_reserve_bytes: u64,
        hard_stop_reserve_bytes: u64,
    ) -> Result<Self, &'static str> {
        if hard_stop_reserve_bytes > warning_reserve_bytes {
            return Err("hard-stop 保留空间不能大于 warning 保留空间");
        }
        Ok(Self {
            warning_reserve_bytes,
            hard_stop_reserve_bytes,
        })
    }

    pub(crate) fn for_finite_limit(limit: u64) -> Self {
        let warning_reserve_bytes = limit / 10;
        let hard_stop_reserve_bytes = limit / 20;
        Self {
            warning_reserve_bytes,
            hard_stop_reserve_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceDecision {
    /// No finite limit or authoritative sample was available.
    Unknown,
    /// A finite limit was observed and remaining headroom is above warning.
    Continue,
    Warning,
    HardStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceObservation {
    pub summary: ResourceObservationSummary,
    pub decision: ResourceDecision,
}

fn parse_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    (!value.is_empty() && value != "max")
        .then(|| value.parse::<u64>().ok())
        .flatten()
}

pub(crate) fn parse_cgroup_memory(current: &str, limit: &str) -> Option<(u64, u64)> {
    let current = parse_bytes(current)?;
    let limit = parse_bytes(limit)?;
    Some((current, limit))
}

fn parse_rss_value(value: &str, unit: &str) -> Option<u64> {
    let value = value.parse::<u64>().ok()?;
    match unit.to_ascii_lowercase().as_str() {
        "b" | "bytes" => Some(value),
        "kb" | "kib" => value.checked_mul(1024),
        "mb" | "mib" => value.checked_mul(1024 * 1024),
        "gb" | "gib" => value.checked_mul(1024 * 1024 * 1024),
        _ => None,
    }
}

/// Parse `/proc/<pid>/status`-style VmRSS without assuming a platform page size.
pub(crate) fn parse_proc_rss_bytes(status: &str) -> Option<u64> {
    let parsed = status.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        if label.trim() != "VmRSS" {
            return None;
        }
        let mut parts = value.split_whitespace();
        let amount = parts.next()?;
        let unit = parts.next().unwrap_or("bytes");
        parse_rss_value(amount, unit)
    });
    parsed.or_else(|| {
        let mut parts = status.split_whitespace();
        let amount = parts.next()?;
        let unit = parts.next().unwrap_or("bytes");
        parse_rss_value(amount, unit)
    })
}

fn base_summary(sampled_at: Option<&str>) -> ResourceObservationSummary {
    ResourceObservationSummary {
        sampled_at: sampled_at
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string),
        ..Default::default()
    }
}

fn decision_for_headroom(
    current: u64,
    limit: u64,
    thresholds: ResourceThresholds,
) -> ResourceDecision {
    let Some(headroom) = limit.checked_sub(current) else {
        return ResourceDecision::HardStop;
    };
    if headroom <= thresholds.hard_stop_reserve_bytes {
        ResourceDecision::HardStop
    } else if headroom <= thresholds.warning_reserve_bytes {
        ResourceDecision::Warning
    } else {
        ResourceDecision::Continue
    }
}

fn apply_decision(summary: &mut ResourceObservationSummary, decision: ResourceDecision) {
    summary.state = match decision {
        ResourceDecision::Warning => ResourceObservationState::Warning,
        ResourceDecision::HardStop => ResourceObservationState::HardStop,
        ResourceDecision::Unknown => ResourceObservationState::Unknown,
        ResourceDecision::Continue => ResourceObservationState::MeasuredSafe,
    };
}

/// Evaluate one low-frequency sample. cgroup is authoritative when it has a finite limit;
/// proc RSS is retained as evidence but cannot produce a threshold decision on its own.
pub(crate) fn observe(
    cgroup_current: Option<&str>,
    cgroup_limit: Option<&str>,
    proc_status: Option<&str>,
    thresholds: ResourceThresholds,
    sampled_at: Option<&str>,
) -> ResourceObservation {
    let mut summary = base_summary(sampled_at);
    let parsed_cgroup_current = cgroup_current.and_then(parse_bytes);
    let parsed_cgroup_limit = cgroup_limit.and_then(parse_bytes);
    if let Some(current) = parsed_cgroup_current {
        summary.source = ResourceObservationSource::Cgroup;
        summary.cgroup_current_bytes = Some(current);
        if let Some(limit) = parsed_cgroup_limit {
            summary.cgroup_limit_bytes = Some(limit);
            summary.headroom_bytes = Some(limit.saturating_sub(current));
            summary.warning_reserve_bytes = Some(thresholds.warning_reserve_bytes);
            summary.hard_stop_reserve_bytes = Some(thresholds.hard_stop_reserve_bytes);
            let decision = decision_for_headroom(current, limit, thresholds);
            apply_decision(&mut summary, decision);
            return ResourceObservation { summary, decision };
        }
    }

    if let Some(status) = proc_status.and_then(parse_proc_rss_bytes) {
        if summary.source == ResourceObservationSource::Unknown {
            summary.source = ResourceObservationSource::Proc;
        }
        summary.current_rss_bytes = Some(status);
    }
    ResourceObservation {
        summary,
        decision: ResourceDecision::Unknown,
    }
}

fn read_first(paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        std::fs::read_to_string(path)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

/// Take one bounded sample for the current application process. cgroup is authoritative when
/// available; proc RSS remains evidence only when no finite cgroup limit can be read.
pub(crate) fn observe_current_process(sampled_at: Option<&str>) -> ResourceObservation {
    observe_process_status(read_first(&["/proc/self/status"]), sampled_at)
}

/// Take one bounded sample for a known child PID. The child RSS is read from its own proc
/// status; termination ownership remains with the caller's child handle.
pub(crate) fn observe_process(pid: u32, sampled_at: Option<&str>) -> ResourceObservation {
    let proc_path = format!("/proc/{pid}/status");
    observe_process_status(read_first(&[proc_path.as_str()]), sampled_at)
}

fn observe_process_status(
    proc_status: Option<String>,
    sampled_at: Option<&str>,
) -> ResourceObservation {
    let cgroup_current = read_first(&[
        "/sys/fs/cgroup/memory.current",
        "/sys/fs/cgroup/memory/memory.usage_in_bytes",
    ]);
    let cgroup_limit = read_first(&[
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ]);
    let thresholds = cgroup_limit
        .as_deref()
        .and_then(parse_bytes)
        .map(ResourceThresholds::for_finite_limit)
        .unwrap_or_else(|| ResourceThresholds::new(0, 0).expect("zero thresholds are ordered"));
    observe(
        cgroup_current.as_deref(),
        cgroup_limit.as_deref(),
        proc_status.as_deref(),
        thresholds,
        sampled_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thresholds() -> ResourceThresholds {
        ResourceThresholds::new(200, 100).expect("test thresholds are ordered")
    }

    #[test]
    fn parses_cgroup_values_without_treating_max_as_a_finite_limit() {
        assert_eq!(parse_cgroup_memory("900", "1000"), Some((900, 1000)));
        assert_eq!(parse_cgroup_memory("900", "max"), None);
        assert_eq!(parse_cgroup_memory("not-a-number", "1000"), None);
    }

    #[test]
    fn parses_proc_rss_in_kib_and_rejects_malformed_input() {
        assert_eq!(
            parse_proc_rss_bytes("Name:\tworker\nVmRSS:\t12 kB\n"),
            Some(12 * 1024)
        );
        assert_eq!(parse_proc_rss_bytes("VmRSS: not-a-number kB"), None);
        assert_eq!(parse_proc_rss_bytes("VmSize: 12 kB"), None);
    }

    #[test]
    fn finite_cgroup_headroom_routes_continue_warning_and_hard_stop() {
        let continue_sample = observe(Some("700"), Some("1000"), None, thresholds(), None);
        assert_eq!(continue_sample.decision, ResourceDecision::Continue);
        assert_eq!(
            continue_sample.summary.source,
            ResourceObservationSource::Cgroup
        );
        assert_eq!(
            continue_sample.summary.state,
            ResourceObservationState::MeasuredSafe
        );
        assert_eq!(continue_sample.summary.headroom_bytes, Some(300));
        assert_eq!(continue_sample.summary.warning_reserve_bytes, Some(200));
        assert_eq!(continue_sample.summary.hard_stop_reserve_bytes, Some(100));

        let warning = observe(Some("850"), Some("1000"), None, thresholds(), None);
        assert_eq!(warning.decision, ResourceDecision::Warning);
        assert_eq!(warning.summary.state, ResourceObservationState::Warning);

        let hard_stop = observe(Some("950"), Some("1000"), None, thresholds(), None);
        assert_eq!(hard_stop.decision, ResourceDecision::HardStop);
        assert_eq!(hard_stop.summary.state, ResourceObservationState::HardStop);

        let over_limit = observe(Some("1001"), Some("1000"), None, thresholds(), None);
        assert_eq!(over_limit.decision, ResourceDecision::HardStop);
    }

    #[test]
    fn proc_only_and_unlimited_cgroup_remain_unknown() {
        let proc_only = observe(
            None,
            None,
            Some("VmRSS: 12 kB"),
            thresholds(),
            Some("2026-08-15T00:00:00Z"),
        );
        assert_eq!(proc_only.decision, ResourceDecision::Unknown);
        assert_eq!(proc_only.summary.source, ResourceObservationSource::Proc);
        assert_eq!(proc_only.summary.current_rss_bytes, Some(12 * 1024));
        assert!(proc_only.summary.headroom_bytes.is_none());
        assert!(proc_only.summary.warning_reserve_bytes.is_none());
        assert!(proc_only.summary.hard_stop_reserve_bytes.is_none());
        assert_eq!(
            proc_only.summary.sampled_at.as_deref(),
            Some("2026-08-15T00:00:00Z")
        );

        let unlimited = observe(
            Some("12"),
            Some("max"),
            Some("VmRSS: 12 kB"),
            thresholds(),
            None,
        );
        assert_eq!(unlimited.decision, ResourceDecision::Unknown);
        assert_eq!(unlimited.summary.source, ResourceObservationSource::Cgroup);
        assert_eq!(unlimited.summary.cgroup_current_bytes, Some(12));
        assert_eq!(unlimited.summary.current_rss_bytes, Some(12 * 1024));
    }

    #[test]
    fn invalid_thresholds_are_rejected_without_machine_defaults() {
        assert!(ResourceThresholds::new(100, 200).is_err());
    }

    #[test]
    fn runtime_thresholds_scale_from_finite_limit_without_fixed_machine_size() {
        let thresholds = ResourceThresholds::for_finite_limit(1_000);
        assert_eq!(thresholds.warning_reserve_bytes, 100);
        assert_eq!(thresholds.hard_stop_reserve_bytes, 50);
    }
}
