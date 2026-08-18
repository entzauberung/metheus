use crate::project::{
    AcceptanceLedgerItem, AcceptanceStatus, AutopilotJobOwner, AutopilotState, Milestone, Project,
    ResourceObservationState, ResourceObservationSummary, Subtask,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const RUN_ID: &str = "phase1-six-task-run-001";
pub(crate) const VISUAL_CRITERION: &str = "主题切换背景渐变具有平滑过渡";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VirtualClock {
    tick: u64,
}

impl VirtualClock {
    fn new() -> Self {
        Self { tick: 0 }
    }

    pub(crate) fn now(&self) -> String {
        format!("virtual-tick-{:04}", self.tick)
    }

    pub(crate) fn advance(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixtureOwner {
    pub owner_type: AutopilotJobOwner,
    pub claim_at: String,
    pub last_heartbeat_at: String,
    pub last_business_progress_at: String,
    pub terminal_state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FixtureTask {
    pub run_id: String,
    pub task_id: String,
    pub execution_id: String,
    pub generation: u64,
    pub dispatch_count: u32,
    pub continuation_count: u32,
    pub replan_count: u32,
    pub owner: FixtureOwner,
    pub ledger: Vec<AcceptanceLedgerItem>,
    pub attempts: Vec<FixtureAttempt>,
    pub resource_observation: ResourceObservationSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixtureExecutionOutcome {
    Running,
    Succeeded,
    OutputTruncated,
    QualityFailed,
    ResourceHardStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixtureAttempt {
    pub execution_id: String,
    pub generation: u64,
    pub outcome: FixtureExecutionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FixtureEventKind {
    ExecutionStarted,
    ExecutionSucceeded,
    OutputTruncated,
    ContinuationStarted,
    ReplanStarted,
    ReviewFailed,
    ResourceWarning,
    ResourceHardStop,
    LedgerUpdated {
        criterion_index: u32,
        status: AcceptanceStatus,
    },
    OwnerReleased,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixtureEvent {
    pub sequence: u64,
    pub task_id: String,
    pub execution_id: String,
    pub generation: u64,
    pub occurred_at: String,
    pub kind: FixtureEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FixtureFaultError {
    DuplicateSignature,
    ContinuationLimitReached,
    ReplanLimitReached,
    AttemptAlreadyTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixtureViolation {
    TaskCount,
    DuplicateExecutionGeneration(String, u64),
    DispatchAttemptMismatch(String),
    MissingTerminal(String),
    RunningAttempt(String),
    EmptyLedger(String),
    ActiveOwnerAfterTerminal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FixtureMutationError {
    StaleOwner,
    MissingCriterion,
}

#[derive(Debug, Clone)]
pub(crate) struct SixTaskFixture {
    pub project: Project,
    pub tasks: Vec<FixtureTask>,
    pub clock: VirtualClock,
    pub events: Vec<FixtureEvent>,
    terminal_by_execution: BTreeMap<String, FixtureExecutionOutcome>,
    fault_signatures: BTreeSet<String>,
}

impl SixTaskFixture {
    fn push_event(&mut self, task_index: usize, kind: FixtureEventKind) {
        let task_id = self.tasks[task_index].task_id.clone();
        let execution_id = self.tasks[task_index].execution_id.clone();
        let generation = self.tasks[task_index].generation;
        let occurred_at = self.clock.now();
        self.events.push(FixtureEvent {
            sequence: self.events.len() as u64 + 1,
            task_id,
            execution_id,
            generation,
            occurred_at,
            kind,
        });
        self.clock.advance();
    }

    fn dispatch(&mut self, task_index: usize) {
        let task = &mut self.tasks[task_index];
        task.dispatch_count = task.dispatch_count.saturating_add(1);
        task.attempts.push(FixtureAttempt {
            execution_id: task.execution_id.clone(),
            generation: task.generation,
            outcome: FixtureExecutionOutcome::Running,
        });
        self.push_event(task_index, FixtureEventKind::ExecutionStarted);
    }

    fn finish_attempt(
        &mut self,
        task_index: usize,
        outcome: FixtureExecutionOutcome,
    ) -> Result<(), FixtureFaultError> {
        let Some(execution_id) = self.tasks[task_index]
            .attempts
            .last()
            .map(|attempt| attempt.execution_id.clone())
        else {
            return Err(FixtureFaultError::AttemptAlreadyTerminal);
        };
        if self.tasks[task_index]
            .attempts
            .last()
            .is_some_and(|attempt| attempt.outcome != FixtureExecutionOutcome::Running)
            || self.terminal_by_execution.contains_key(&execution_id)
        {
            return Err(FixtureFaultError::AttemptAlreadyTerminal);
        }
        self.tasks[task_index]
            .attempts
            .last_mut()
            .expect("attempt checked above")
            .outcome = outcome;
        self.terminal_by_execution.insert(execution_id, outcome);
        Ok(())
    }

    fn set_ledger_status(
        &mut self,
        task_index: usize,
        criterion_index: usize,
        status: AcceptanceStatus,
        evidence: &str,
    ) {
        let updated_at = self.clock.now();
        if let Some(item) = self.tasks[task_index].ledger.get_mut(criterion_index) {
            item.status = status.clone();
            item.evidence = evidence.to_string();
            item.updated_at = updated_at.clone();
        }
        if let Some(item) = self.project.milestones[0].subtasks[task_index]
            .acceptance_ledger
            .get_mut(criterion_index)
        {
            item.status = status.clone();
            item.evidence = evidence.to_string();
            item.updated_at = updated_at;
        }
        self.push_event(
            task_index,
            FixtureEventKind::LedgerUpdated {
                criterion_index: criterion_index as u32 + 1,
                status,
            },
        );
    }

    fn release_owner(&mut self, task_index: usize, terminal_state: &str) {
        let now = self.clock.now();
        self.tasks[task_index].owner.terminal_state = terminal_state.to_string();
        self.tasks[task_index].owner.last_heartbeat_at = now;
        self.push_event(task_index, FixtureEventKind::OwnerReleased);
    }

    fn begin_next_generation(&mut self, task_index: usize) {
        let now = self.clock.now();
        let task = &mut self.tasks[task_index];
        task.generation = task.generation.saturating_add(1);
        task.execution_id = format!(
            "{}:{}:generation-{}",
            task.run_id, task.task_id, task.generation
        );
        task.owner.claim_at = now;
        task.owner.last_heartbeat_at = task.owner.claim_at.clone();
        task.owner.last_business_progress_at = task.owner.claim_at.clone();
        task.owner.terminal_state = "claimed".to_string();
    }

    pub(crate) fn complete_first_five(&mut self) -> Result<(), FixtureFaultError> {
        for task_index in 0..5 {
            self.dispatch(task_index);
            self.finish_attempt(task_index, FixtureExecutionOutcome::Succeeded)?;
            self.push_event(task_index, FixtureEventKind::ExecutionSucceeded);
            let ledger_len = self.tasks[task_index].ledger.len();
            for criterion_index in 0..ledger_len {
                self.set_ledger_status(
                    task_index,
                    criterion_index,
                    AcceptanceStatus::Satisfied,
                    "fixture execution, quality and confirmation verified",
                );
            }
            self.release_owner(task_index, "succeeded");
        }
        Ok(())
    }

    fn record_truncation(
        &mut self,
        task_index: usize,
        signature: &str,
    ) -> Result<(), FixtureFaultError> {
        if !self.fault_signatures.insert(signature.to_string()) {
            return Err(FixtureFaultError::DuplicateSignature);
        }
        self.finish_attempt(task_index, FixtureExecutionOutcome::OutputTruncated)?;
        self.push_event(task_index, FixtureEventKind::OutputTruncated);
        Ok(())
    }

    pub(crate) fn inject_t6_fault_chain(&mut self) -> Result<(), FixtureFaultError> {
        let task_index = 5;
        self.dispatch(task_index);
        self.record_truncation(task_index, "T6:output-truncated")?;

        if self.tasks[task_index].continuation_count >= 1 {
            return Err(FixtureFaultError::ContinuationLimitReached);
        }
        self.tasks[task_index].continuation_count += 1;
        self.begin_next_generation(task_index);
        self.push_event(task_index, FixtureEventKind::ContinuationStarted);
        self.dispatch(task_index);
        self.finish_attempt(task_index, FixtureExecutionOutcome::QualityFailed)?;
        self.set_ledger_status(
            task_index,
            1,
            AcceptanceStatus::Unsatisfied,
            "visual retest failed",
        );
        self.push_event(task_index, FixtureEventKind::ReviewFailed);

        if self.tasks[task_index].replan_count >= 1 {
            return Err(FixtureFaultError::ReplanLimitReached);
        }
        self.tasks[task_index].replan_count += 1;
        self.begin_next_generation(task_index);
        self.push_event(task_index, FixtureEventKind::ReplanStarted);
        self.dispatch(task_index);
        self.tasks[task_index].resource_observation = ResourceObservationSummary {
            state: ResourceObservationState::Warning,
            current_rss_bytes: Some(700),
            peak_rss_bytes: Some(700),
            headroom_bytes: Some(200),
            warning_reserve_bytes: Some(200),
            hard_stop_reserve_bytes: Some(100),
            ..Default::default()
        };
        self.push_event(task_index, FixtureEventKind::ResourceWarning);
        self.tasks[task_index].resource_observation.state = ResourceObservationState::HardStop;
        self.tasks[task_index]
            .resource_observation
            .current_rss_bytes = Some(800);
        self.tasks[task_index].resource_observation.peak_rss_bytes = Some(800);
        self.tasks[task_index].resource_observation.headroom_bytes = Some(100);
        self.push_event(task_index, FixtureEventKind::ResourceHardStop);
        self.finish_attempt(task_index, FixtureExecutionOutcome::ResourceHardStop)?;
        self.release_owner(task_index, "resource_hard_stop");
        Ok(())
    }

    pub(crate) fn reject_duplicate_truncation(&mut self) -> Result<(), FixtureFaultError> {
        self.record_truncation(5, "T6:output-truncated")
    }

    pub(crate) fn validate_closeout(&self) -> Vec<FixtureViolation> {
        let mut violations = BTreeSet::new();
        let mut execution_generations = BTreeSet::new();
        if self.tasks.len() != 6 {
            violations.insert(FixtureViolation::TaskCount);
        }
        for task in &self.tasks {
            if task.dispatch_count as usize != task.attempts.len() {
                violations.insert(FixtureViolation::DispatchAttemptMismatch(
                    task.task_id.clone(),
                ));
            }
            if task.ledger.is_empty() {
                violations.insert(FixtureViolation::EmptyLedger(task.task_id.clone()));
            }
            if task.owner.terminal_state == "claimed" {
                violations.insert(FixtureViolation::ActiveOwnerAfterTerminal(
                    task.task_id.clone(),
                ));
            }
            for attempt in &task.attempts {
                let identity = (attempt.execution_id.clone(), attempt.generation);
                if !execution_generations.insert(identity.clone()) {
                    violations.insert(FixtureViolation::DuplicateExecutionGeneration(
                        identity.0, identity.1,
                    ));
                }
                if attempt.outcome == FixtureExecutionOutcome::Running {
                    violations.insert(FixtureViolation::RunningAttempt(
                        attempt.execution_id.clone(),
                    ));
                }
                if self.terminal_by_execution.get(&attempt.execution_id) != Some(&attempt.outcome) {
                    violations.insert(FixtureViolation::MissingTerminal(
                        attempt.execution_id.clone(),
                    ));
                }
            }
        }
        violations.into_iter().collect()
    }

    pub(crate) fn reopen(&self) -> Self {
        let encoded = serde_json::to_string(&self.project).expect("serialize fixture project");
        let mut reopened = self.clone();
        reopened.project = serde_json::from_str(&encoded).expect("reopen fixture project");
        reopened
    }

    pub(crate) fn try_owner_ledger_write(
        &mut self,
        task_index: usize,
        generation: u64,
        criterion_index: usize,
        status: AcceptanceStatus,
    ) -> Result<(), FixtureMutationError> {
        let task = &self.tasks[task_index];
        if generation != task.generation || task.owner.terminal_state != "claimed" {
            return Err(FixtureMutationError::StaleOwner);
        }
        if criterion_index >= task.ledger.len() {
            return Err(FixtureMutationError::MissingCriterion);
        }
        self.set_ledger_status(task_index, criterion_index, status, "owner mutation");
        Ok(())
    }
}

fn criteria_for(index: usize) -> Vec<String> {
    if index == 6 {
        vec![
            "第六项执行结果保持可追溯".to_string(),
            VISUAL_CRITERION.to_string(),
        ]
    } else {
        vec![format!("任务 T{index} 执行、质量和确认事实一致")]
    }
}

fn ledger_for(criteria: &[String], updated_at: &str) -> Vec<AcceptanceLedgerItem> {
    criteria
        .iter()
        .enumerate()
        .map(|(index, criterion)| AcceptanceLedgerItem {
            criterion_index: index as u32 + 1,
            criterion: criterion.clone(),
            status: AcceptanceStatus::Unknown,
            updated_at: updated_at.to_string(),
            ..Default::default()
        })
        .collect()
}

pub(crate) fn six_task_fixture() -> SixTaskFixture {
    let mut clock = VirtualClock::new();
    let mut project = Project::new("phase1-six-task-fixture");
    let mut fixture_tasks = Vec::with_capacity(6);
    let mut project_tasks = Vec::with_capacity(6);

    for index in 1..=6 {
        let task_id = format!("T{index}");
        let execution_id = format!("{RUN_ID}:{task_id}:generation-1");
        let criteria = criteria_for(index);
        let updated_at = clock.now();
        let ledger = ledger_for(&criteria, &updated_at);
        let owner = FixtureOwner {
            owner_type: AutopilotJobOwner::BackendRuntime,
            claim_at: updated_at.clone(),
            last_heartbeat_at: updated_at.clone(),
            last_business_progress_at: updated_at.clone(),
            terminal_state: "claimed".to_string(),
        };
        project_tasks.push(Subtask {
            id: task_id.clone(),
            title: format!("Fixture task {task_id}"),
            goal: format!("Complete fixture contract for {task_id}"),
            order: index as u32,
            acceptance_criteria: criteria,
            acceptance_ledger: ledger.clone(),
            depends_on: (index > 1)
                .then(|| format!("T{}", index - 1))
                .into_iter()
                .collect(),
            ..Default::default()
        });
        fixture_tasks.push(FixtureTask {
            run_id: RUN_ID.to_string(),
            task_id,
            execution_id,
            generation: 1,
            dispatch_count: 0,
            continuation_count: 0,
            replan_count: 0,
            owner,
            ledger,
            attempts: Vec::new(),
            resource_observation: ResourceObservationSummary::default(),
        });
        clock.advance();
    }

    project.current_milestone_id = "fixture-milestone".to_string();
    project.milestones = vec![Milestone {
        id: "fixture-milestone".to_string(),
        title: "Phase1 six-task fixture".to_string(),
        subtasks: project_tasks,
        ..Default::default()
    }];
    project.workflow_state.autopilot_active = true;
    project.workflow_state.autopilot_target_milestone_id = "fixture-milestone".to_string();
    project.workflow_state.autopilot_state = Some(AutopilotState {
        active: true,
        target_milestone_id: "fixture-milestone".to_string(),
        job_id: RUN_ID.to_string(),
        job_generation: 1,
        job_owner: AutopilotJobOwner::BackendRuntime,
        heartbeat_at: clock.now(),
        ..Default::default()
    });

    SixTaskFixture {
        project,
        tasks: fixture_tasks,
        clock,
        events: Vec::new(),
        terminal_by_execution: BTreeMap::new(),
        fault_signatures: BTreeSet::new(),
    }
}

#[test]
fn t6_fault_chain_preserves_attempts_ledger_owner_and_resource_facts() {
    let mut fixture = six_task_fixture();
    fixture.complete_first_five().expect("complete T1-T5");
    fixture.inject_t6_fault_chain().expect("inject T6 faults");

    for task in &fixture.tasks[..5] {
        assert_eq!(task.dispatch_count, 1);
        assert_eq!(task.attempts[0].outcome, FixtureExecutionOutcome::Succeeded);
        assert!(task
            .ledger
            .iter()
            .all(|item| item.status == AcceptanceStatus::Satisfied));
        assert_eq!(task.owner.terminal_state, "succeeded");
    }

    let t6 = &fixture.tasks[5];
    assert_eq!(t6.dispatch_count, 3);
    assert_eq!(t6.continuation_count, 1);
    assert_eq!(t6.replan_count, 1);
    assert_eq!(t6.attempts.len(), 3);
    assert_eq!(
        t6.attempts
            .iter()
            .map(|attempt| attempt.outcome)
            .collect::<Vec<_>>(),
        vec![
            FixtureExecutionOutcome::OutputTruncated,
            FixtureExecutionOutcome::QualityFailed,
            FixtureExecutionOutcome::ResourceHardStop,
        ]
    );
    assert_eq!(t6.ledger[1].criterion, VISUAL_CRITERION);
    assert_eq!(t6.ledger[1].status, AcceptanceStatus::Unsatisfied);
    assert_eq!(
        t6.resource_observation.state,
        ResourceObservationState::HardStop
    );
    assert_eq!(t6.resource_observation.peak_rss_bytes, Some(800));
    assert_eq!(t6.owner.terminal_state, "resource_hard_stop");
}

#[test]
fn duplicate_truncation_signature_is_rejected_without_new_event() {
    let mut fixture = six_task_fixture();
    fixture.inject_t6_fault_chain().expect("inject T6 faults");
    let event_count = fixture.events.len();
    let attempt_count = fixture.tasks[5].attempts.len();

    assert_eq!(
        fixture.reject_duplicate_truncation(),
        Err(FixtureFaultError::DuplicateSignature)
    );
    assert_eq!(fixture.events.len(), event_count);
    assert_eq!(fixture.tasks[5].attempts.len(), attempt_count);
}

#[test]
fn closeout_has_unique_terminals_no_running_owner_and_survives_reopen() {
    let mut fixture = six_task_fixture();
    fixture.complete_first_five().expect("complete T1-T5");
    fixture.inject_t6_fault_chain().expect("inject T6 faults");
    let violations = fixture.validate_closeout();
    assert!(
        violations.is_empty(),
        "structured closeout violations: {violations:#?}"
    );

    let mut reopened = fixture.reopen();
    let reopened_violations = reopened.validate_closeout();
    assert!(
        reopened_violations.is_empty(),
        "structured reopen violations: {reopened_violations:#?}"
    );
    assert_eq!(
        reopened.project.milestones[0].subtasks[5].acceptance_ledger,
        fixture.project.milestones[0].subtasks[5].acceptance_ledger
    );
    assert_eq!(reopened.tasks[5].attempts, fixture.tasks[5].attempts);

    let ledger_before = reopened.tasks[5].ledger.clone();
    assert_eq!(
        reopened.try_owner_ledger_write(5, 2, 1, AcceptanceStatus::Satisfied),
        Err(FixtureMutationError::StaleOwner)
    );
    assert_eq!(reopened.tasks[5].ledger, ledger_before);
}

#[test]
fn closeout_validator_detects_structural_regressions() {
    let mut fixture = six_task_fixture();
    fixture.complete_first_five().expect("complete T1-T5");
    fixture.inject_t6_fault_chain().expect("inject T6 faults");

    fixture.tasks[0].ledger.clear();
    fixture.tasks[1].owner.terminal_state = "claimed".to_string();
    fixture.tasks[2].attempts[0].outcome = FixtureExecutionOutcome::Running;
    fixture.tasks[3].dispatch_count += 1;
    let duplicate = fixture.tasks[4].attempts[0].clone();
    fixture.tasks[5].attempts.push(duplicate.clone());
    fixture.tasks[5].dispatch_count += 1;

    let violations = fixture.validate_closeout();
    assert!(violations.contains(&FixtureViolation::EmptyLedger("T1".to_string())));
    assert!(
        violations.contains(&FixtureViolation::ActiveOwnerAfterTerminal(
            "T2".to_string()
        ))
    );
    assert!(violations.contains(&FixtureViolation::RunningAttempt(
        fixture.tasks[2].attempts[0].execution_id.clone()
    )));
    assert!(violations.contains(&FixtureViolation::DispatchAttemptMismatch("T4".to_string())));
    assert!(
        violations.contains(&FixtureViolation::DuplicateExecutionGeneration(
            duplicate.execution_id,
            duplicate.generation
        ))
    );
}

#[test]
fn runtime_long_chain_closeout_emits_structured_task_facts() {
    let mut fixture = six_task_fixture();
    fixture.complete_first_five().expect("complete T1-T5");
    fixture.inject_t6_fault_chain().expect("inject T6 faults");
    let violations = fixture.validate_closeout();
    assert!(
        violations.is_empty(),
        "structured closeout violations: {violations:#?}"
    );

    for task in &fixture.tasks {
        let ledger = task
            .ledger
            .iter()
            .map(|item| {
                serde_json::json!({
                    "criterion_index": item.criterion_index,
                    "criterion": item.criterion.clone(),
                    "status": format!("{:?}", item.status),
                })
            })
            .collect::<Vec<_>>();
        let attempts = task
            .attempts
            .iter()
            .map(|attempt| {
                serde_json::json!({
                    "execution_id": attempt.execution_id.clone(),
                    "generation": attempt.generation,
                    "outcome": format!("{:?}", attempt.outcome),
                })
            })
            .collect::<Vec<_>>();
        let fact = serde_json::json!({
            "run_id": task.run_id.clone(),
            "task_id": task.task_id.clone(),
            "current_execution_id": task.execution_id.clone(),
            "current_generation": task.generation,
            "dispatch_count": task.dispatch_count,
            "continuation_count": task.continuation_count,
            "replan_count": task.replan_count,
            "owner": {
                "type": format!("{:?}", task.owner.owner_type),
                "claim_at": task.owner.claim_at.clone(),
                "last_heartbeat_at": task.owner.last_heartbeat_at.clone(),
                "last_business_progress_at": task.owner.last_business_progress_at.clone(),
                "terminal_state": task.owner.terminal_state.clone(),
            },
            "ledger": ledger,
            "attempts": attempts,
            "resource": {
                "state": format!("{:?}", task.resource_observation.state),
                "peak_rss_bytes": task.resource_observation.peak_rss_bytes,
                "headroom_bytes": task.resource_observation.headroom_bytes,
            },
        });
        println!(
            "LONG_CHAIN_TASK_FACT {}",
            serde_json::to_string(&fact).expect("serialize task fact")
        );
    }
    println!(
        "LONG_CHAIN_SUMMARY {}",
        serde_json::to_string(&serde_json::json!({
            "run_id": RUN_ID,
            "task_count": fixture.tasks.len(),
            "event_count": fixture.events.len(),
            "violation_count": violations.len(),
            "reopen_violation_count": fixture.reopen().validate_closeout().len(),
        }))
        .expect("serialize summary")
    );
}

#[test]
fn six_task_fixture_has_stable_identity_owner_and_nonempty_ledgers() {
    let fixture = six_task_fixture();
    assert_eq!(fixture.tasks.len(), 6);
    assert_eq!(fixture.project.milestones.len(), 1);
    assert_eq!(fixture.project.milestones[0].subtasks.len(), 6);

    let task_ids = fixture
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let execution_ids = fixture
        .tasks
        .iter()
        .map(|task| task.execution_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(task_ids.len(), 6);
    assert_eq!(execution_ids.len(), 6);

    for (index, task) in fixture.tasks.iter().enumerate() {
        assert_eq!(task.run_id, RUN_ID);
        assert_eq!(task.task_id, format!("T{}", index + 1));
        assert_eq!(task.generation, 1);
        assert_eq!(task.dispatch_count, 0);
        assert_eq!(task.owner.owner_type, AutopilotJobOwner::BackendRuntime);
        assert!(!task.owner.claim_at.is_empty());
        assert!(!task.ledger.is_empty());
        assert!(task
            .ledger
            .iter()
            .enumerate()
            .all(|(ledger_index, item)| item.criterion_index == ledger_index as u32 + 1));
    }

    assert!(fixture.tasks[5]
        .ledger
        .iter()
        .any(|item| item.criterion == VISUAL_CRITERION));
}

#[test]
fn six_task_project_round_trip_preserves_ledger_order() {
    let fixture = six_task_fixture();
    let encoded = serde_json::to_string(&fixture.project).expect("serialize fixture project");
    let restored: Project = serde_json::from_str(&encoded).expect("restore fixture project");
    let tasks = &restored.milestones[0].subtasks;

    assert_eq!(tasks.len(), 6);
    for (index, task) in tasks.iter().enumerate() {
        assert_eq!(task.id, format!("T{}", index + 1));
        assert!(!task.acceptance_ledger.is_empty());
        assert_eq!(task.acceptance_ledger[0].criterion_index, 1);
    }
    assert_eq!(tasks[5].acceptance_ledger[1].criterion, VISUAL_CRITERION);
}
