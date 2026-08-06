use crate::project::{
    ExistingProjectBaseline, WorkloadCheckDepth, WorkloadProfile, WorkloadScale, WorkloadSignals,
};
use sha2::{Digest, Sha256};

const LARGE_BASELINE_FILE_COUNT: usize = 200;

/// Pure workload classification. The caller supplies all facts; this module performs no I/O.
pub fn classify(
    signals: WorkloadSignals,
    baseline: Option<&ExistingProjectBaseline>,
    discussion_revision: u64,
) -> Result<WorkloadProfile, String> {
    validate_signals(&signals)?;

    let large_existing_repository = baseline.is_some_and(|value| {
        value.scan_complete && value.scanned_files.len() >= LARGE_BASELINE_FILE_COUNT
    });
    let foundational_capabilities = u32::from(signals.has_backend)
        + u32::from(signals.has_persistence)
        + u32::from(signals.has_auth_or_roles)
        + u32::from(signals.external_integration_count > 0);

    let mut scale = if signals.independent_domain_count >= 3 && foundational_capabilities >= 3 {
        WorkloadScale::System
    } else if signals.independent_domain_count >= 2
        || signals.deliverable_count >= 4
        || foundational_capabilities > 0
    {
        WorkloadScale::Standard
    } else if is_micro_scope(&signals) {
        WorkloadScale::Micro
    } else {
        WorkloadScale::Small
    };

    if scale == WorkloadScale::Micro && large_existing_repository {
        scale = WorkloadScale::Small;
    }

    let (
        use_mid_stage_layer,
        max_milestones,
        max_mid_stages,
        max_subtasks,
        max_split_depth,
        base_check_depth,
        max_executor_turns,
        max_transport_retries,
        max_doom_loop_retries,
    ) = match scale {
        WorkloadScale::Micro => (false, 1, 0, 1, 0, WorkloadCheckDepth::Lean, 4, 0, 0),
        WorkloadScale::Small => (false, 1, 0, 3, 0, WorkloadCheckDepth::Lean, 8, 1, 0),
        WorkloadScale::Standard => (
            signals.has_frontend && signals.has_backend && signals.independent_domain_count >= 3,
            3,
            3,
            6,
            1,
            WorkloadCheckDepth::Standard,
            16,
            2,
            1,
        ),
        WorkloadScale::System => (true, 5, 5, 8, 1, WorkloadCheckDepth::Strict, 32, 3, 2),
    };

    let check_depth = if signals.high_risk {
        WorkloadCheckDepth::Strict
    } else {
        base_check_depth
    };
    let evidence = classification_evidence(&signals, scale, large_existing_repository);
    let mut profile = WorkloadProfile {
        signals,
        scale,
        use_mid_stage_layer,
        max_milestones,
        max_mid_stages,
        max_subtasks,
        max_split_depth,
        check_depth,
        max_executor_turns,
        max_transport_retries,
        max_doom_loop_retries,
        evidence,
        discussion_revision,
        fingerprint: String::new(),
    };
    profile.fingerprint = fingerprint(&profile, large_existing_repository);
    Ok(profile)
}

pub fn render_planning_constraints(profile: &WorkloadProfile) -> String {
    let topology = if profile.use_mid_stage_layer {
        "Milestone -> MidStage -> Subtask"
    } else {
        "Milestone -> Subtask"
    };
    format!(
        "工作负载={:?}; 拓扑={topology}; 数量上限=Milestone {}, MidStage {}, Subtask {}; \
split深度上限={}; 检查深度={:?}; 执行预算=turns {}, transport retries {}, doom-loop retries {}",
        profile.scale,
        profile.max_milestones,
        profile.max_mid_stages,
        profile.max_subtasks,
        profile.max_split_depth,
        profile.check_depth,
        profile.max_executor_turns,
        profile.max_transport_retries,
        profile.max_doom_loop_retries,
    )
}

pub fn current_profile(project: &crate::project::Project) -> Result<&WorkloadProfile, String> {
    let profile = project
        .workload_profile
        .as_ref()
        .ok_or_else(|| "工作负载画像缺失，请从目标完整性检查重新开始三项检查。".to_string())?;
    if profile.discussion_revision != project.discussion_revision {
        return Err(format!(
            "工作负载画像已过期（画像讨论修订 {}，当前讨论修订 {}），请重新完成目标完整性检查。",
            profile.discussion_revision, project.discussion_revision
        ));
    }
    if profile.fingerprint.trim().is_empty() {
        return Err("工作负载画像指纹缺失，请重新完成目标完整性检查。".to_string());
    }
    Ok(profile)
}

fn validate_signals(signals: &WorkloadSignals) -> Result<(), String> {
    if signals.deliverable_count == 0 {
        return Err("工作负载信号缺少交付物数量".to_string());
    }
    if signals.independent_domain_count == 0 {
        return Err("工作负载信号缺少独立领域数量".to_string());
    }
    Ok(())
}

fn is_micro_scope(signals: &WorkloadSignals) -> bool {
    signals.deliverable_count == 1
        && signals.independent_domain_count == 1
        && !signals.has_backend
        && !signals.has_persistence
        && !signals.has_auth_or_roles
        && signals.external_integration_count == 0
}

fn classification_evidence(
    signals: &WorkloadSignals,
    scale: WorkloadScale,
    large_existing_repository: bool,
) -> Vec<String> {
    let mut evidence = vec![format!(
        "范围事实：{} 个独立领域，{} 个交付物，{} 个外部集成",
        signals.independent_domain_count,
        signals.deliverable_count,
        signals.external_integration_count
    )];
    let capabilities = [
        (signals.has_frontend, "前端"),
        (signals.has_backend, "后端"),
        (signals.has_persistence, "持久化"),
        (signals.has_auth_or_roles, "认证或角色"),
    ]
    .into_iter()
    .filter_map(|(present, label)| present.then_some(label))
    .collect::<Vec<_>>();
    evidence.push(format!(
        "基础能力：{}",
        if capabilities.is_empty() {
            "无".to_string()
        } else {
            capabilities.join("、")
        }
    ));
    if large_existing_repository && scale == WorkloadScale::Small {
        evidence.push("已有大仓库只将 Micro 下限提升为 Small".to_string());
    }
    if signals.high_risk {
        evidence.push("存在高风险范围事实：保持当前树深度并使用 Strict 检查".to_string());
    }
    evidence
}

fn fingerprint(profile: &WorkloadProfile, large_existing_repository: bool) -> String {
    let payload = serde_json::to_vec(&(
        &profile.signals,
        large_existing_repository,
        profile.scale,
        profile.use_mid_stage_layer,
        profile.max_milestones,
        profile.max_mid_stages,
        profile.max_subtasks,
        profile.max_split_depth,
        profile.check_depth,
        profile.max_executor_turns,
        profile.max_transport_retries,
        profile.max_doom_loop_retries,
        profile.discussion_revision,
    ))
    .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(payload);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
pub fn test_profile(scale: WorkloadScale) -> WorkloadProfile {
    let signals = match scale {
        WorkloadScale::Micro => WorkloadSignals {
            has_frontend: true,
            has_backend: false,
            has_persistence: false,
            has_auth_or_roles: false,
            external_integration_count: 0,
            independent_domain_count: 1,
            deliverable_count: 1,
            high_risk: false,
        },
        WorkloadScale::Small => WorkloadSignals {
            has_frontend: true,
            has_backend: false,
            has_persistence: false,
            has_auth_or_roles: false,
            external_integration_count: 0,
            independent_domain_count: 1,
            deliverable_count: 2,
            high_risk: false,
        },
        WorkloadScale::Standard => WorkloadSignals {
            has_frontend: true,
            has_backend: false,
            has_persistence: false,
            has_auth_or_roles: false,
            external_integration_count: 0,
            independent_domain_count: 3,
            deliverable_count: 4,
            high_risk: false,
        },
        WorkloadScale::System => WorkloadSignals {
            has_frontend: true,
            has_backend: true,
            has_persistence: true,
            has_auth_or_roles: true,
            external_integration_count: 0,
            independent_domain_count: 3,
            deliverable_count: 4,
            high_risk: false,
        },
    };
    classify(signals, None, 0).expect("test workload profile must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(domains: u32, deliverables: u32) -> WorkloadSignals {
        WorkloadSignals {
            has_frontend: true,
            has_backend: false,
            has_persistence: false,
            has_auth_or_roles: false,
            external_integration_count: 0,
            independent_domain_count: domains,
            deliverable_count: deliverables,
            high_risk: false,
        }
    }

    #[test]
    fn adaptive_execution_contract_static_single_page_is_micro() {
        let profile = classify(signals(1, 1), None, 7).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Micro);
        assert!(!profile.use_mid_stage_layer);
        assert_eq!(profile.max_subtasks, 1);
        assert_eq!(profile.max_split_depth, 0);
        assert_eq!(profile.max_executor_turns, 4);
    }

    #[test]
    fn static_multi_deliverable_scope_is_small() {
        let profile = classify(signals(1, 2), None, 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Small);
        assert!(!profile.use_mid_stage_layer);
        assert_eq!(profile.max_subtasks, 3);
    }

    #[test]
    fn adaptive_execution_contract_frontend_is_standard_without_mid_stages() {
        let profile = classify(signals(3, 4), None, 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Standard);
        assert!(!profile.use_mid_stage_layer);
        assert_eq!(profile.max_milestones, 3);
        assert_eq!(profile.max_subtasks, 6);
    }

    #[test]
    fn adaptive_execution_contract_full_stack_multi_role_is_system() {
        let mut scope = signals(3, 4);
        scope.has_backend = true;
        scope.has_persistence = true;
        scope.has_auth_or_roles = true;
        let profile = classify(scope, None, 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::System);
        assert!(profile.use_mid_stage_layer);
        assert_eq!(profile.max_milestones, 5);
        assert_eq!(profile.max_mid_stages, 5);
        assert_eq!(profile.max_subtasks, 8);
        assert_eq!(profile.max_executor_turns, 32);
    }

    #[test]
    fn standard_uses_mid_stages_only_for_three_domain_full_stack_scope() {
        let mut scope = signals(3, 3);
        scope.has_backend = true;
        let profile = classify(scope, None, 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Standard);
        assert!(profile.use_mid_stage_layer);
    }

    #[test]
    fn large_repository_only_lifts_micro_to_small() {
        let baseline = ExistingProjectBaseline {
            scan_complete: true,
            scanned_files: (0..LARGE_BASELINE_FILE_COUNT)
                .map(|index| format!("src/{index}.rs"))
                .collect(),
            ..Default::default()
        };
        let profile = classify(signals(1, 1), Some(&baseline), 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Small);
        assert!(!profile.use_mid_stage_layer);
    }

    #[test]
    fn adaptive_execution_contract_high_risk_does_not_deepen_tree() {
        let mut scope = signals(1, 2);
        scope.high_risk = true;
        let profile = classify(scope, None, 1).unwrap();
        assert_eq!(profile.scale, WorkloadScale::Small);
        assert_eq!(profile.max_split_depth, 0);
        assert_eq!(profile.check_depth, WorkloadCheckDepth::Strict);
    }

    #[test]
    fn fingerprint_is_stable_and_revision_sensitive() {
        let first = classify(signals(2, 3), None, 4).unwrap();
        let same = classify(signals(2, 3), None, 4).unwrap();
        let revised = classify(signals(2, 3), None, 5).unwrap();
        assert_eq!(first.fingerprint, same.fingerprint);
        assert_ne!(first.fingerprint, revised.fingerprint);
        assert!(first.fingerprint.starts_with("sha256:"));
    }

    #[test]
    fn profile_serializes_roundtrip_and_new_project_has_no_implicit_profile() {
        let profile = classify(signals(2, 3), None, 4).unwrap();
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: WorkloadProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);

        let project = crate::project::Project::new("unclassified");
        assert!(project.workload_profile.is_none());
        let project_value = serde_json::to_value(project).unwrap();
        assert!(project_value.get("mode").is_none());
        assert!(project_value
            .get("workload_profile")
            .is_some_and(serde_json::Value::is_null));
    }

    #[test]
    fn incomplete_counts_are_rejected() {
        assert!(classify(signals(0, 1), None, 1).is_err());
        assert!(classify(signals(1, 0), None, 1).is_err());
    }

    #[test]
    fn planning_constraints_render_frozen_profile_values() {
        let profile = classify(signals(1, 2), None, 1).unwrap();
        let rendered = render_planning_constraints(&profile);
        assert!(rendered.contains("Milestone -> Subtask"));
        assert!(rendered.contains("Subtask 3"));
        assert!(rendered.contains("turns 8"));
    }
}
