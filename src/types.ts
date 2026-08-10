// 与 Rust 后端 project.rs 的数据结构一一对应

// ========== 工作流状态（唯一业务状态，决定前端显示和按钮权限） ==========

export type TopLevelPhase = "Before" | "FirstDiscussion" | "Console" | "Completed";

export type WorkflowStep =
  | "WaitingEntry"
  | "ExistingAnalysis"
  | "BaselineApproval"
  | "Discussion"
  | "ThreeChecks"
  | "ProjectPlanGeneration"
  | "PlanApproval"
  | "MilestoneGeneration"
  | "MilestoneCheck"
  | "MilestoneApproval"
  | "MilestoneSelection"
  | "MidStageGeneration"
  | "MidStageCheck"
  | "MidStageApproval"
  | "MidStageSelection"
  | "PlanGeneration"
  | "PlanCheck"
  | "PlanApproving"
  | "Execution"
  | "PauseDecision"
  | "RollbackPreview"
  | "BranchDiscussion"
  | "FuturePlanApproval"
  | "MilestoneReview"
  | "Completed";

export type PauseReason = "None" | "InStop" | "EDStop";

export type AutopilotRunStatus = "Running" | "Paused" | "WaitingMilestoneReview" | "ErrorStopped";

/** 自动驾驶恢复动作 — 与 Rust AutopilotRecoveryAction 一一对应 */
export type AutopilotRecoveryAction =
  | "None"
  | "RestoreExecutionBaseline"
  | "RetryAutopilotAdvance"
  | "SyncAndClose"
  | "WaitHumanDecision"
  | "RegenerateExecutionPlan"
  | "PrepareExecutionWorkspace"
  | "ResolveWorkspaceChanges"
  | "RunAutomaticRecovery"
  | "RetryGitConfirmation";

export type AutopilotJobOwner = "None" | "BackendRuntime";

export type AutopilotFailureKind =
  | "None"
  | "Network"
  | "RateLimited"
  | "ProviderUnavailable"
  | "Timeout"
  | "RevisionConflict"
  | "ProcessCrash"
  | "Authentication"
  | "Quota"
  | "WorkspaceChanged"
  | "StateConflict"
  | "ScopeViolation"
  | "ContractContradiction"
  | "GitIntegrity"
  | "Permanent";

export type RecoveryErrorKind =
  | "WorkspaceError"
  | "TransientError"
  | "ExecutionError"
  | "EngineBlocked"
  | "PlanFailure"
  | "ValidationFailure"
  | "EvidenceInsufficient"
  | "ContractContradiction"
  | "ValidationOscillation"
  | "ScopeViolation"
  | "TestFailure"
  | "ReviewFailure"
  | "AutomatedTestUnavailable"
  | "ReviewTransientFailure"
  | "ReviewProtocolFailure"
  | "ReviewServiceBlocked"
  | "TestUnavailable"
  | "StateConflict"
  | "HumanRequired";

export type RecoveryPhase = "Diagnosing" | "Repairing" | "Retesting" | "Replanning" | "WaitingEngine" | "Recovered" | "WaitingHuman";

export type EngineFailureKind =
  | "QuotaExceeded"
  | "AuthenticationError"
  | "RateLimited"
  | "ProviderUnavailable"
  | "NetworkError"
  | "Timeout"
  | "ProcessCrash"
  | "ToolRejected"
  | "ProtocolError"
  | "OutputTruncated"
  | "MaxTurnsExceeded"
  | "RuntimeError"
  | "TaskExecutionError";

export type AcceptanceStatus =
  | "Satisfied"
  | "AiProvisionallySatisfied"
  | "DeferredHumanReview"
  | "Unsatisfied"
  | "Unknown"
  | "Contradictory"
  | "AcceptedDeviation";
export type HumanReviewCadence = "PerTask" | "MilestoneBatch";
export type MilestoneHumanDecision = "Pending" | "Confirmed" | "Rejected";
export type VisualReviewStatus =
  | "Unavailable"
  | "Satisfied"
  | "Unsatisfied"
  | "EvidenceInsufficient"
  | "Conflict";

export interface VisualEvidenceReference {
  path: string;
  sha256: string;
  mime: string;
  size_bytes: number;
}

export interface MilestoneHumanReviewItem {
  id: string;
  milestone_id: string;
  task_id: string;
  criterion_index: number;
  criterion: string;
  contract_fingerprint: string;
  execution_facts_fingerprint: string;
  review_cycle: number;
  ai_status: AcceptanceStatus;
  ai_evidence: string;
  visual_status: VisualReviewStatus;
  visual_summary: string;
  visual_evidence: VisualEvidenceReference[];
  human_decision: MilestoneHumanDecision;
  human_reason: string;
  decided_at?: string;
  updated_at: string;
}

export interface MilestoneHumanReviewDecisionSubmission {
  item_id: string;
  decision: Exclude<MilestoneHumanDecision, "Pending">;
  reason: string;
}

export interface MilestoneReviewSubmission {
  milestone_id: string;
  review_cycle: number;
  expected_revision: number;
  review_fingerprint: string;
  branch: "A" | "B" | "C";
  branch_reason: string;
  decisions: MilestoneHumanReviewDecisionSubmission[];
}
export type Provability = "Deterministic" | "AutomatedTest" | "SemanticReview" | "HumanReview" | "Unprovable";
export type ProvabilitySource = "PlanningExplicit" | "SystemInferred" | "HumanCorrected";
export type EvidenceSourceType = "LocalScan" | "AutomatedTestOutput" | "CodeSnippet" | "ExpandedCodeSnippet" | "RuntimeOrHuman";
export interface EvidenceSourceFingerprint {
  fingerprint: string;
  source_types: EvidenceSourceType[];
  covered_files: string[];
  validator_type: string;
}
export interface AcceptanceCriterion {
  text: string;
  provability: Provability;
  provability_source: ProvabilitySource;
}
export type ReviewIssueSeverity = "Blocking" | "Warning" | "Suggestion";
export type EvidenceSourceKind =
  | "GitDiff"
  | "GitDiffHunk"
  | "IdentifierContext"
  | "SymbolDefinition"
  | "SymbolReference"
  | "LifecycleContext"
  | "CurrentFileSnippet";
export type CriterionReviewConclusion = "Satisfied" | "Unsatisfied" | "EvidenceInsufficient";
export type ReviewEvidenceStrategy = "Standard" | "Targeted" | "ExpandedTargeted";
export type VerificationStage =
  | "NotStarted"
  | "AutomatedTests"
  | "PreparingEvidence"
  | "RequestingReview"
  | "ParsingReview"
  | "DeterministicNormalization"
  | "ProtocolRepair"
  | "ReviewRetry"
  | "TargetedEvidence"
  | "Completed";
export type ReviewStatus = "NotRequested" | "InProgress" | "Completed" | "Failed";
export type ReviewFailureKind =
  | "Network"
  | "Timeout"
  | "RateLimited"
  | "ServiceUnavailable"
  | "Authentication"
  | "QuotaExceeded"
  | "EmptyResponse"
  | "InvalidJson"
  | "FieldTypeMismatch";
export type ValidationRetryStrategy =
  | "DeterministicNormalization"
  | "ProtocolRepair"
  | "ReviewRequestRetry"
  | "TargetedEvidence";

export interface ReviewEvidenceReference {
  block_id: string;
  source_kind: EvidenceSourceKind;
  file: string;
  start_line?: number;
  end_line?: number;
}

export interface CriterionReviewResult {
  criterion_index: number;
  criterion: string;
  conclusion: CriterionReviewConclusion;
  confidence: number;
  evidence_references: ReviewEvidenceReference[];
}

export interface AcceptanceLedgerItem {
  criterion_index: number;
  criterion: string;
  status: AcceptanceStatus;
  evidence: string;
  evidence_references: ReviewEvidenceReference[];
  confidence: number;
  updated_at: string;
}

export interface ProjectFactSnapshot {
  git_head: string;
  file_hashes: Record<string, string>;
  symbols: string[];
  storage_keys: string[];
  dom_ids: string[];
  event_bindings: string[];
  relevant_snippets: string[];
  identifier_contexts: Record<string, string[]>;
  accepted_deviations: string[];
  structural_fingerprint: string;
  captured_at: string;
}

export interface RecoveryLearningRecord {
  failure_signature: string;
  failure_domain: string;
  strategy: string;
  succeeded: boolean;
  related_paths: string[];
  required_identifiers: string[];
  acceptance_fingerprint: string;
  stable_constraint: string;
  recorded_at: string;
}

export interface ReviewIssue {
  criterion_index?: number;
  criterion: string;
  file: string;
  expected: string;
  actual: string;
  suggested_change: string;
  confidence: number;
  severity?: ReviewIssueSeverity;
  evidence_references?: ReviewEvidenceReference[];
}

export interface RecoveryIssue extends ReviewIssue {
  id: string;
}

export interface RecoveryAttemptRecord {
  attempt: number;
  issue_ids: string[];
  resolved_issue_ids: string[];
  remaining_issue_ids: string[];
  regressed_issue_ids: string[];
  changed_files: string[];
  made_progress: boolean;
  summary: string;
  recorded_at: string;
}

export interface RecoveryState {
  error_kind: RecoveryErrorKind;
  phase: RecoveryPhase;
  attempt: number;
  max_attempts: number;
  error_signature: string;
  repeated_signature_count: number;
  subtask_id: string;
  execution_id: string;
  baseline_commit: string;
  last_diagnosis: string;
  last_repair_summary: string;
  original_test_failure: string;
  replan_attempted: boolean;
  failure_history: string[];
  active_issues: RecoveryIssue[];
  attempt_history: RecoveryAttemptRecord[];
  replan_execution_attempted: boolean;
  started_at: string;
  updated_at: string;
  engine_failure_kind?: EngineFailureKind;
  checkpoint_id: string;
  rollback_retest_pending: boolean;
  evidence_rebuild_attempted: boolean;
  evidence_rebuild_attempts: number;
  pending_evidence_criteria: number[];
  evidence_strategies: ReviewEvidenceStrategy[];
  evidence_source_history: EvidenceSourceFingerprint[];
  validation_retry_count: number;
  max_validation_retries: number;
  next_validation_retry_at?: string;
  validation_strategies: ValidationRetryStrategy[];
  pending_execution_result?: ExecutionResult;
}

export interface AutopilotState {
  active: boolean;
  target_milestone_id: string;
  run_status: AutopilotRunStatus;
  last_action: string;
  last_action_at: string;
  error_message: string;
  /** 出错后的恢复动作；旧项目默认 None */
  recovery_action?: AutopilotRecoveryAction;
  job_id?: string;
  job_generation?: number;
  job_owner?: AutopilotJobOwner;
  current_action_id?: string;
  current_action_kind?: string;
  action_started_at?: string;
  heartbeat_at?: string;
  transient_retry_count?: number;
  next_retry_at?: string;
  last_failure_kind?: AutopilotFailureKind;
  last_failure_fingerprint?: string;
  consecutive_no_progress?: number;
}

// ========== V2 托管层 ==========

export type ManagedRunStatus = "Running" | "Paused" | "WaitingHuman" | "ErrorStopped";

export interface ManagedFlowState {
  active: boolean;
  managed_state: string;
  managed_target: string;
  last_action: string;
  last_action_at: string;
  run_status: ManagedRunStatus;
  error_message: string;
  job_id: string;
  job_generation: number;
  current_action: string;
  current_action_id: string;
  heartbeat_at: string;
  retry_count: number;
  last_completed_action: string;
}

export type DiscussionScope = "FirstDiscussion" | "PauseAdjustment" | "FixPast" | "AdjustFuture";

export interface WorkflowState {
  top_level_phase: TopLevelPhase;
  current_step: WorkflowStep;
  pause_reason: PauseReason;
  data_revision: number;
  discussion_scope: DiscussionScope;
  active_discussion_thread_id: string;
  review_node_id: string;
  last_transition_at: string;
  autopilot_active: boolean;
  autopilot_target_milestone_id: string;
  autopilot_state?: AutopilotState;
  managed_flow_state?: ManagedFlowState;
  recovery_state?: RecoveryState;
}

/** 后端事实变化通知。通知只负责失效提示，完整事实必须通过运行时快照读取。 */
export interface ProjectStateChangedEvent {
  project_name: string;
  process_start_id: string;
  event_sequence: number;
  data_revision: number;
  current_step: WorkflowStep;
  execution_session_status: string | null;
  autopilot_status: AutopilotRunStatus | null;
  recovery_action: AutopilotRecoveryAction;
  task_control_tree_revision: number;
  task_control_snapshot_version: string;
  control_action_id: string | null;
  control_mode: TaskControlMode;
  task_control_dirty: boolean;
  runtime_dirty: boolean;
  occurred_at: string;
}

export interface ProjectStateSubscription {
  subscription_id: string;
  process_start_id: string;
  event_sequence: number;
}

export type RecoveryPresentationKind =
  | "None"
  | "ControlActionOccupied"
  | "BaselineRecovery"
  | "GitReconfirmation"
  | "EngineBlocked"
  | "ValidationRetry"
  | "EvidenceInsufficient"
  | "HumanDecision"
  | "AutomaticRecovery"
  | "RetryAdvance"
  | "RegeneratePlan"
  | "PrepareWorkspace"
  | "ResolveWorkspaceChanges"
  | "SyncAndClose";

export type RecoverySeverity = "Info" | "Warning" | "Error";

export type RecoveryCapability =
  | "SyncProject"
  | "ClearStaleControlLock"
  | "AcknowledgeExecutionRecovery"
  | "RetryGitConfirmation"
  | "RetryAutopilotAdvance"
  | "RegenerateExecutionPlan"
  | "PrepareExecutionWorkspace"
  | "RefreshExecutionWorkspace"
  | "RunAutomaticRecovery"
  | "ResolveHumanRecovery"
  | "ResumeAutopilot"
  | "CloseAutopilot";

export interface RecoveryActionPresentation {
  capability: RecoveryCapability;
  label: string;
  enabled: boolean;
  disabled_reason: string | null;
}

export type RecoveryDecisionResolution =
  | "retest"
  | "revalidate"
  | "restore_and_retry"
  | "regenerate_plan"
  | "confirm_actual_pass"
  | "accept_deviation"
  | "skip_task";

export interface RecoveryDecisionOption {
  resolution: RecoveryDecisionResolution;
  label: string;
  enabled: boolean;
  disabled_reason: string | null;
  requires_reason: boolean;
  requires_acceptance_selection: boolean;
  requires_baseline_preview: boolean;
  preview_message?: string;
}

export interface RecoveryPresentation {
  presentation_version?: string;
  kind: RecoveryPresentationKind;
  title: string;
  reason: string;
  severity: RecoverySeverity;
  primary_action: RecoveryActionPresentation | null;
  secondary_actions: RecoveryActionPresentation[];
  preserve_current_code: boolean;
  requires_baseline_restore: boolean;
  supports_preview: boolean;
  automatic_retry: boolean;
  capabilities: RecoveryCapability[];
  decision_options: RecoveryDecisionOption[];
  state_fingerprint: string;
  phase_label?: string;
  background_retry_active?: boolean;
  background_retry_summary?: string;
  post_action_expectation?: string;
  stale_risk?: boolean;
  sync_risk_summary?: string;
  sync_needed?: boolean;
  code_impact_summary?: string;
  affected_task_label?: string;
  baseline_reference?: string;
  validation_phase_label?: string;
  retry_count?: number;
  retry_limit?: number;
  next_retry_at?: string | null;
  validation_retry_count?: number;
  validation_retry_limit?: number;
  next_validation_retry_at?: string | null;
  heartbeat_status?: string;
  automated_test_status?: string;
  code_review_status?: string;
  review_protocol_status?: string;
  acceptance_evidence_status?: string;
  control_lock_valid?: boolean | null;
  control_action_description?: string;
  control_action_elapsed_seconds?: number;
  control_lock_last_heartbeat_at?: string | null;
  control_lock_failure_reason?: string;
  control_lock_cleanup_available?: boolean;
}

export interface RecoveryResultSummary {
  title: string;
  message: string;
  baseline: string | null;
  baseline_summary: string;
  discarded_files: string[];
  discarded_files_summary: string;
  background_job_started: boolean;
  background_job_summary: string;
  next_step: string;
  next_step_summary: string;
}

export interface RuntimeActionSummary {
  action: string;
  message: string;
  notify_user: boolean;
  recovery_result: RecoveryResultSummary | null;
}

export interface TaskControlSnapshotSummary {
  available: boolean;
  snapshot_version: string;
  tree_revision: number;
  event_sequence: number;
  control_action_id: string | null;
  control_mode: TaskControlMode;
}

// ========== 项目来源 ==========

export type ProjectEntryKind = "NoProject" | "HalfProject";

export type ExecutionRuntime = "BuiltIn" | "Plugin";
export type ExecutionProvider = "GrokBuild" | "ClaudeCode" | "Codex" | "KimiCli";
export type PermissionProfile = "Interactive" | "Unattended";

export interface ExecutionProfile {
  runtime: ExecutionRuntime;
  provider: ExecutionProvider;
  permission_profile: PermissionProfile;
  profile_revision: number;
}

export type EngineHealthStatus =
  | "Available"
  | "NotInstalled"
  | "Unauthenticated"
  | "UnsupportedVersion"
  | "Disabled"
  | "VerificationRequired"
  | "VerificationFailed"
  | "Unknown";

export type EngineAuthState = "Authenticated" | "Unauthenticated" | "Unknown";
export type EngineLocalAuthState = "ConfiguredEvidence" | "Missing" | "Unknown";
export type EngineOnlineAuthState = "NotVerified" | "Verified" | "Failed";
export type EngineAuthVerificationMethod =
  | "None"
  | "PassiveConfiguration"
  | "OnlineMinimalRequest"
  | "OnlineModelList";

export type EngineConfigurationEvidenceSource =
  | "Confirmed"
  | "ProviderDefault"
  | "Unknown";

export interface EngineRuntimeConfigurationEvidence {
  model?: string;
  model_source: EngineConfigurationEvidenceSource;
  reasoning_effort?: string;
  reasoning_effort_source: EngineConfigurationEvidenceSource;
}

export interface EngineAuthenticationResult {
  local_state: EngineLocalAuthState;
  online_state: EngineOnlineAuthState;
  method: EngineAuthVerificationMethod;
  verified_at?: string;
  expires_at?: string;
  failure_kind?: EngineFailureKind;
  runtime_configuration?: EngineRuntimeConfigurationEvidence;
  message: string;
}

export interface EngineHealth {
  runtime: ExecutionRuntime;
  provider: ExecutionProvider;
  status: EngineHealthStatus;
  executable_path?: string;
  version?: string;
  auth_state: EngineAuthState;
  authentication: EngineAuthenticationResult;
  supports_unattended: boolean;
  configuration_valid: boolean;
  capabilities: string[];
  source_revision?: string;
  runtime_self_test: EngineRuntimeSelfTestState;
  message: string;
}

export type EngineRuntimeSelfTestState = "NotRun" | "Passed" | "Failed";

export interface EngineRuntimeSelfTestResult {
  success: boolean;
  state: EngineRuntimeSelfTestState;
  source_revision: string;
  verified_at: string;
  message: string;
}

export type ApiInterface = "OpenAiCompatible";
export type GrokBuildApiBackend = "ChatCompletions" | "Responses" | "Messages";
export type StructuredOutputPolicy = "NativeJsonObject" | "PromptOnly";

export interface DecisionModelSettingsView {
  api_interface: ApiInterface;
  request_url: string;
  model: string;
  timeout_secs: number;
  structured_output: StructuredOutputPolicy;
}

export interface BuiltInGrokBuildSettingsView {
  api_backend: GrokBuildApiBackend;
  api_base_url: string;
  model: string;
  timeout_secs: number;
  max_turns: number;
}

export interface PluginCliSettingsView {
  claude_code_path?: string;
  codex_path?: string;
  kimi_path?: string;
  grok_path?: string;
}

export interface VisionModelSettingsView {
  enabled: boolean;
  request_url: string;
  model: string;
  timeout_secs: number;
  max_image_bytes: number;
  max_total_bytes: number;
  max_images: number;
}

export interface AppSettingsData {
  schema_version: number;
  revision: number;
  decision_model: DecisionModelSettingsView;
  built_in_grok_build: BuiltInGrokBuildSettingsView;
  plugin_cli: PluginCliSettingsView;
  vision_model: VisionModelSettingsView;
}

export type SecretPersistence = "SecureStore" | "SessionOnly";
export type SecretSource =
  | "Session"
  | "SystemCredentialStore"
  | "Environment"
  | "LegacyEnvironment"
  | "Missing";

export interface SecretStatus {
  configured: boolean;
  source: SecretSource;
  hint: string;
  persistent_available: boolean;
  persisted: boolean;
  persistence_error?: string;
}

export interface AppSettingsView {
  settings: AppSettingsData;
  decision_secret: SecretStatus;
  built_in_grok_build_secret: SecretStatus;
  vision_model_secret: SecretStatus;
  load_warning?: string;
}

export type ModelConnectionTarget = "DecisionModel" | "BuiltInGrokBuild" | "VisionModel";
export type ModelConnectionErrorKind =
  | "MissingSecret"
  | "InvalidConfiguration"
  | "Authentication"
  | "QuotaExceeded"
  | "RateLimited"
  | "Timeout"
  | "Network"
  | "ProviderUnavailable"
  | "Protocol"
  | "HttpStatus";

export interface ConnectionTestResult {
  success: boolean;
  target: ModelConnectionTarget;
  model: string;
  latency_ms: number;
  error_kind?: ModelConnectionErrorKind;
  message: string;
}

// ========== 已有项目基线 ==========

export interface ExistingProjectBaseline {
  project_summary: string;
  tech_stack: string;
  architecture_evidence: string;
  completed_capabilities: string[];
  pending_capabilities: string[];
  risks: string[];
  uncertainties: string[];
  scanned_files: string[];
  scan_complete: boolean;
  evidence_summary: string;
  generated_at: string;
  approved: boolean;
  approved_at?: string;
  already_constitution_path: string;
  already_constitution_summary: string;
  readme_full: string;
  manifest_details: [string, string][];
  source_abstracts: [string, string][];
}

// ========== 三项检查结果 ==========

export interface PreflightCheckResult {
  check_type: "goal_completeness" | "reality_consistency" | "task_executability";
  passed: boolean;
  summary: string;
  issues: string[];
  suggestions: string[];
  discussion_revision: number;
  checked_at: string;
  stale: boolean;
  expired_at?: string;
}

// ========== 草稿生命周期状态 ==========

export type DraftStatus = "Pending" | "Approved" | "Rejected" | "Expired" | "Superseded";

// ========== 大阶段草稿 ==========

export type MilestoneDraftStatus = "Pending" | "CheckFailed" | "CheckPassed" | "Approved";

export type MilestoneDraftKind = "Normal" | "FutureOnly";

export interface MilestoneDraft {
  draft_id: string;
  status: MilestoneDraftStatus;
  draft_kind: MilestoneDraftKind;
  candidate_milestones: Milestone[];
  check_result?: string;
  generation_revision: number;
  source_plan_revision: number;
  source_thread_id: string;
  source_thread_revision: number;
  source_data_revision: number;
  expired: boolean;
  expiration_reason?: string;
  generated_at: string;
  approved_at?: string;
  regeneration_count: number;
  previous_draft_id?: string;
  last_regeneration_reason?: string;
  last_regenerated_at?: string;
  // C 分支"只改未来"元数据
  split_after_milestone_id?: string;
  retained_milestone_ids: string[];
  future_candidate_ids: string[];
  original_ai_versions: string[];
  normalized_versions: string[];
  versions_normalized: boolean;
  // 数量与粒度校验（阶段六）
  original_remaining_count?: number;
  new_future_count?: number;
  count_expansion_warning: boolean;
  granularity_check_passed: boolean;
  granularity_issues: string[];
}

// ========== 中阶段草稿 ==========

export type MidStageDraftStatus = "Pending" | "CheckFailed" | "Approved";
export type MidStageDraftPurpose = "InitialFullList" | "FuturePendingPatch";

export interface MidStageDraft {
  draft_id: string;
  milestone_id: string;
  status: MidStageDraftStatus;
  candidate_mid_stages: MidStage[];
  check_result?: string;
  generation_revision: number;
  generated_at: string;
  approved_at?: string;
  regeneration_count: number;
  previous_draft_id?: string;
  last_regeneration_reason?: string;
  source_data_revision: number;
  last_check_failure_fingerprint?: string;
  last_candidate_fingerprint?: string;
  no_progress_count?: number;
  purpose: MidStageDraftPurpose;
  base_mid_stage_revision: number;
  retained_mid_stage_ids: string[];
  source_step: WorkflowStep;
  allow_full_replacement: boolean;
}

// ========== 方案草稿 ==========

export interface PlanDraft {
  draft_id: string;
  draft_status: DraftStatus;
  plan_content: string;
  constitution_part1_draft: string;
  generation_revision: number;
  data_revision_at_generation: number;
  workload_profile_fingerprint: string;
  self_check_result: string;
  generated_at: string;
  /** @deprecated 使用 draft_status 代替 */
  approved: boolean;
  approved_at?: string;
  approved_at_discussion_revision?: number;
  rejection_feedback?: string;
  rejected_at?: string;
  expired_at?: string;
  superseded_at?: string;
}

// ========== 执行计划检查结果 ==========

export interface StagePlanCheckResult {
  passed: boolean;
  omissions: string[];
  out_of_scope: string[];
  not_executable: string[];
  suggestions: string[];
  checked_at: string;
}

// ========== 暂停上下文 ==========

export interface PauseContext {
  pause_type: "in_stop" | "ed_stop";
  current_subtask_id: string;
  last_passed_subtask_id: string;
  stable_tag: string;
  paused_at: string;
  discussion_start_revision: number;
  pending_action: string;
  resume_step?: WorkflowStep;
  autopilot_was_active: boolean;
}

// ========== 回退影响范围 ==========

export interface RollbackImpact {
  target_checkpoint: string;
  retained_nodes: string[];
  discarded_nodes: string[];
  deleted_tags: string[];
  regeneration_scope: string;
  includes_code_rollback: boolean;
}

// ========== 分支决策 ==========

// 过渡期兼容：包含新旧枚举值。
// 新代码应只使用 "Continue" | "FixPast" | "AdjustFuture"。
export type DiscussionBranchType = "Continue" | "FixPast" | "AdjustFuture";

export interface BranchDecision {
  branch_type?: DiscussionBranchType;
  discussion_start_revision: number;
  user_feedback: string;
  suggested_checkpoint: string;
  impact_scope: string;
  confirmed: boolean;
}

// ========== 项目状态（已退役，新界面使用 workflow_state） ==========

/// @deprecated 使用 WorkflowState 替代。保留用于旧项目文件反序列化兼容。
export type ProjectStatus = "Idle" | "Discussing" | "Planning" | "MilestoneReady" | "Executing" | "Paused" | "Completed";

export type MilestoneStatus = "Pending" | "InProgress" | "Completed" | "Paused";

export interface ProviderUsage {
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  cached_input_tokens?: number;
}

export interface ExecutionResult {
  success: boolean;
  output: string;
  error_log: string;
  file_changes: string[];
  exit_code?: number;
  engine_provider?: ExecutionProvider;
  engine_runtime: ExecutionRuntime;
  engine_settings_revision: number;
  engine_source_revision: string;
  engine_api_backend: string;
  stdout: string;
  stderr: string;
  engine_failure_kind?: EngineFailureKind;
  token_usage?: ProviderUsage;
}

export interface TestResult {
  passed: boolean;
  issues: string[];
  suggestion: string;
  review_issues?: ReviewIssue[];
  criterion_reviews?: CriterionReviewResult[];
  warnings?: string[];
  test_command?: string;
  test_exit_code?: number;
  test_output_summary?: string;
  automated_test_status?: "Unknown" | "Passed" | "Failed" | "NotConfigured" | "Unavailable";
  review_passed?: boolean;
  verification_kind?: "Legacy" | "DeterministicLocal" | "AutomatedTestOnly" | "AutomatedTestAndReview" | "CodeReviewOnly" | "HumanOverride";
  review_evidence_status?: ReviewEvidenceStatus;
  review_evidence_summary?: string;
  acceptance_results?: AcceptanceLedgerItem[];
  verification_stage?: VerificationStage;
  review_status?: ReviewStatus;
  review_failure_kind?: ReviewFailureKind;
  review_protocol_attempts?: number;
  review_diagnostic_summary?: string;
}

export type ReviewEvidenceStatus = "Complete" | "Partial" | "Unavailable";

export interface HumanVerification {
  verification_kind: "HumanOverride";
  verification_reason: string;
  verified_at: string;
  original_test_failure: string;
  resolution?: "ConfirmActualPass" | "AcceptDeviation" | "SkipTask";
  accepted_criteria?: number[];
  dependency_check?: string;
}

export interface GeneratedSubtask {
  title: string;
  prompt: string;
}

export interface Subtask {
  id: string;
  title: string;
  prompt: string;
  status: "Pending" | "Executing" | "AwaitingConfirmation" | "Passed" | "AcceptedDeviation" | "Skipped" | "Rejected" | "RolledBack";
  test_report: string;
  execution_result?: ExecutionResult;
  test_result?: TestResult;
  retry_count: number;
  auto_tag?: string;  // 实际 Git tag 名；兼容 V1 与实体身份驱动的 V2
  // V1 结构化字段
  order: number;
  goal: string;
  allowed_file_paths: string[];
  new_file_paths: string[];
  evidence_files: string[];
  context_summary: string;
  acceptance_criteria: string[];
  acceptance_criteria_meta?: AcceptanceCriterion[];
  stop_rules: string[];
  execution_prompt: string;
  confirmed_by_user?: boolean;
  confirmed_at?: string;
  confirmation_notes?: string;
  human_verification?: HumanVerification;
  required_identifiers: string[];
  acceptance_ledger: AcceptanceLedgerItem[];
  fact_snapshot?: ProjectFactSnapshot;
  plan_patch_revision: number;
  depends_on: string[];
  dependency_notes: string;
  contract_snapshot?: TaskContract;
  child_tasks: Subtask[];
}

export type MidStageStatus = "Pending" | "Ready" | "InProgress" | "Completed" | "Rejected" | "Approved" | "RolledBack";

export interface MidStage {
  id: string;
  title: string;
  version: string;
  order?: number;
  status: MidStageStatus;
  subtasks: Subtask[];
  domain?: string;
  test_log?: string;
  created_at: string;
  completed_at?: string;
  approved_at?: string;
  description: string;
  tech_focus: string;
  test_report: string;
  git_tag?: string;
  plan_check_result?: StagePlanCheckResult;
  plan_approved_at?: string;
  plan_revision: number;
  plan_draft_revision: number;
  plan_generated_at?: string;
  plan_regeneration_count: number;
  last_plan_failure_fingerprint?: string;
  last_plan_issue_count?: number;
  plan_no_progress_count?: number;
}

export type StageMode = "Quick" | "Professional";

export type WorkloadScale = "Micro" | "Small" | "Standard" | "System";
export type WorkloadCheckDepth = "Lean" | "Standard" | "Strict";

export interface WorkloadSignals {
  has_frontend: boolean;
  has_backend: boolean;
  has_persistence: boolean;
  has_auth_or_roles: boolean;
  external_integration_count: number;
  independent_domain_count: number;
  deliverable_count: number;
  high_risk: boolean;
}

export interface WorkloadProfile {
  signals: WorkloadSignals;
  scale: WorkloadScale;
  use_mid_stage_layer: boolean;
  max_milestones: number;
  max_mid_stages: number;
  max_subtasks: number;
  max_split_depth: number;
  check_depth: WorkloadCheckDepth;
  max_executor_turns: number;
  max_transport_retries: number;
  max_doom_loop_retries: number;
  evidence: string[];
  discussion_revision: number;
  fingerprint: string;
}

// ========== 需求质检相关 ==========

export interface QADetail {
  issue_type: "遗漏" | "多余" | "偏离";
  description: string;
  related_requirement: string;
}

export interface QAResult {
  passed: boolean;
  reason: string;
  details: QADetail[];
  attention_points: string[];
  checked_at: string;
  warnings?: string[];
}

export interface Milestone {
  id: string;
  version: string;
  title: string;
  description: string;
  tech_stack: string;
  status: MilestoneStatus;
  mode: StageMode;
  mid_stages: MidStage[];
  subtasks: Subtask[];
  qa_result?: QAResult;
  git_commit_hash: string;
  decomposition_check?: string;
  review_status?: string;  // "pending_review" | "approved" | "needs_fix" | "future_adjusted"
  review_conclusion?: string;  // A/B/C 分支选择结果
  approved_at?: string;
  goal: string;
  scope: string;
  dependencies: string[];
  expected_output: string;
  acceptance_criteria: string[];
  plan_check_result?: StagePlanCheckResult;
  plan_approved_at?: string;
  plan_revision: number;
  plan_draft_revision: number;
  plan_generated_at?: string;
  plan_regeneration_count: number;
  last_plan_failure_fingerprint: string;
  last_plan_issue_count: number;
  plan_no_progress_count: number;
  human_review_items: MilestoneHumanReviewItem[];
  human_review_cycle: number;
  human_review_fingerprint: string;
}

export interface ChatMessage {
  id: string;
  role: string;
  content: string;
  msg_type?: string;       // 与 Rust Message.msg_type 一致
  approved?: boolean;
  rejected?: boolean;
  milestone_id?: string;   // 与 Rust Message.milestone_id 一致
  reply_to_message_id?: string;
  timestamp: number;
}

export type ChatStreamEvent =
  | { event: "started"; request_id: string; thread_id: string; role: string }
  | { event: "user_saved"; request_id: string; thread_id: string; role: string; message: ChatMessage }
  | { event: "reply_started"; request_id: string; thread_id: string; role: string; message_id: string; timestamp: number }
  | { event: "delta"; request_id: string; thread_id: string; role: string; text: string }
  | { event: "completed"; request_id: string; thread_id: string; role: string; message_id: string }
  | { event: "cancelled"; request_id: string; thread_id: string; role: string; message_id: string | null }
  | { event: "failed"; request_id: string; thread_id: string; role: string; message_id: string | null; error: string; retryable: boolean };

export interface DiscussionThread {
  id: string;
  title: string;
  node_id: string;
  messages: ChatMessage[];
  scope: DiscussionScope;
  milestone_id: string;
  review_cycle_id: string;
  revision: number;
  opened_at: string;
  closed_at?: string;
  status: "Open" | "Closed";
}

export interface Project {
  name: string;
  /// @deprecated 使用 workflow_state 替代。仅在旧项目文件加载时存在。
  status?: ProjectStatus;
  entry_kind: ProjectEntryKind;
  workflow_state: WorkflowState;
  workload_profile?: WorkloadProfile;
  execution_profile: ExecutionProfile;
  human_review_cadence: HumanReviewCadence;
  vision_review_enabled: boolean;
  current_milestone_id: string;
  current_mid_stage_id: string;
  version_plan: string;
  existing_baseline?: ExistingProjectBaseline;
  preflight_results: PreflightCheckResult[];
  discussion_revision: number;
  plan_draft?: PlanDraft;
  draft_history: PlanDraft[];
  milestones: Milestone[];
  discussion_threads: DiscussionThread[];
  milestone_draft?: MilestoneDraft;
  mid_stage_draft?: MidStageDraft;
  pause_context?: PauseContext;
  branch_decision?: BranchDecision;
  change_history: ChangeHistoryEntry[];
  constitution_change_history: ConstitutionChangeEntry[];
  /** 当前执行会话（用于刷新恢复与状态同步） */
  execution_session?: ExecutionSession;
  /** 执行操作历史（持久化，刷新不丢） */
  execution_history: ExecutionHistoryEntry[];
  recovery_learning: RecoveryLearningRecord[];
  project_path: string;
  task_control?: TaskControlState;
  cost_ledger?: CostLedger;
}

export type TaskControlMode = "Legacy" | "Shadow" | "SerialTakeover";
export type TaskControlDetailStatus = "idle" | "syncing" | "ready" | "unavailable";
export type TakeoverCapabilityStatus = "Unknown" | "Ready" | "Unavailable";

export interface TaskControlModeChangeRecord {
  from: TaskControlMode;
  to: TaskControlMode;
  source: string;
  reason: string;
  changed_at: string;
  project_revision: number;
}

export interface ControlActionLease {
  action_id: string;
  owner_process_start_id: string;
  action_kind: string;
  task_id: string;
  started_at: string;
  heartbeat_at: string;
  expected_max_duration_secs: number;
}

export interface TaskControlState {
  mode: TaskControlMode;
  algorithm_version: string;
  snapshot_version: string;
  takeover_version: string;
  takeover_capability_status: TakeoverCapabilityStatus;
  last_takeover_check_result: string;
  takeover_unavailable_reason: string;
  takeover_checked_at?: string;
  mode_change_history: TaskControlModeChangeRecord[];
  last_shadow_decision_at?: string;
  last_shadow_decision_summary: string;
  shadow_comparison: ShadowComparisonMetrics;
  last_decision_id: string;
  last_decision_fingerprint: string;
  last_decision?: TaskControlDecision;
  control_source: string;
  tree_revision?: number;
  active_action_lease?: ControlActionLease;
  active_action_id?: string;
  active_action_kind?: string;
  active_action_task_id?: string;
  last_completed_action_id?: string;
  last_completed_action_kind?: string;
  last_completed_action_task_id?: string;
  last_action_result?: string;
  last_action_made_progress?: boolean;
  last_action_clear_reason?: string;
  last_action_cleared_at?: string;
}

export type TaskActionFamily = "Execute" | "Confirm" | "Repair" | "Wait" | "Human";
export type ShadowComparisonOutcome = "Match" | "Difference" | "Uncomparable";

export interface ShadowDecisionComparison {
  compared_at: string;
  shadow_decision_id: string;
  shadow_action: string;
  legacy_command: string;
  shadow_family?: TaskActionFamily;
  legacy_family?: TaskActionFamily;
  outcome: ShadowComparisonOutcome;
  reason: string;
}

export interface ShadowComparisonMetrics {
  evaluated: number;
  comparable_matches: number;
  comparable_differences: number;
  uncomparable: number;
  latest?: ShadowDecisionComparison;
}

export type TaskNodeType = "Milestone" | "MidStage" | "Subtask";
export type TaskComplexity = "Small" | "Medium" | "Large";
export type TaskRiskLevel = "Low" | "Medium" | "High" | "Critical";
export type VerificationMode = "Deterministic" | "AutomatedTest" | "SemanticReview" | "HumanReview";

export interface TaskContract {
  version: string;
  task_id: string;
  parent_task_id?: string;
  depth: number;
  node_type: TaskNodeType;
  workload_scale: WorkloadScale;
  workload_profile_fingerprint: string;
  max_split_depth: number;
  title: string;
  goal: string;
  allowed_file_paths: string[];
  new_file_paths: string[];
  evidence_files: string[];
  acceptance_criteria: string[];
  acceptance_criteria_meta?: AcceptanceCriterion[];
  verification_modes: VerificationMode[];
  stop_rules: string[];
  dependencies: string[];
  complexity: TaskComplexity;
  risk: TaskRiskLevel;
  artifacts: {
    expected_files: string[];
    expected_identifiers: string[];
    completion_facts: string[];
    expected_artifacts: string[];
    related_symbols: string[];
    read_file_paths: string[];
    write_file_paths: string[];
  };
  budget: {
    level: string;
    estimated_model_calls: number;
    estimated_input_tokens: number;
    estimated_output_tokens: number;
    max_executor_turns: number;
    max_transport_retries: number;
    max_doom_loop_retries: number;
  };
  recommended_executor: string;
  plan_source: string;
  split_basis: string;
  estimated_complexity_reduction: number;
  independently_verifiable: boolean;
  future_parallel_safe: boolean;
  compiled_at: string;
  fingerprint: string;
}

export interface TaskTreeNodeView {
  id: string;
  title: string;
  node_type: TaskNodeType;
  status: string;
  depth: number;
  complexity: string;
  risk: string;
  contract_fingerprint: string;
  contract?: TaskContract;
  dependencies: string[];
  acceptance: AcceptanceLedgerItem[];
  capabilities: string[];
  disabled_reasons: Record<string, string>;
  is_currently_actionable: boolean;
  actionable_acceptance_criteria: number[];
  children: TaskTreeNodeView[];
}

export type ControlActionKind = "Split" | "Execute" | "LocalValidate" | "AutomatedValidate" | "TargetedValidate" | "Repair" | "Recompile" | "AcceptDeviation" | "GitConfirm" | "Wait" | "Human";

export interface TaskControlDecision {
  decision_id: string;
  task_id: string;
  contract_fingerprint: string;
  facts_fingerprint: string;
  acceptance: {
    satisfied: number;
    unsatisfied: number;
    unknown: number;
    contradictory: number;
    accepted_deviation: number;
  };
  action: {
    kind: ControlActionKind;
    priority: number;
    risk: string;
    reason: string;
    retryable: boolean;
  };
  expected_cost: string;
  expected_risk: string;
  cache_hit: boolean;
  shadow: boolean;
  reason: string;
}

export interface TokenCostSummary {
  calls: number;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  known_input_tokens: number;
  known_output_tokens: number;
  known_total_tokens: number;
  usage_known_calls: number;
  usage_unknown_calls: number;
  effective_calls: number;
  no_progress_calls: number;
}

export interface CostGroupSummary {
  key: string;
  summary: TokenCostSummary;
}

export interface CostLedger {
  calls: ModelCallRecord[];
  archived_calls?: ArchivedModelCallRecord[];
  project_summary: TokenCostSummary;
  soft_budget_level: string;
}

export type ModelCallPurpose =
  | "Decision"
  | "Review"
  | "Execution"
  | "Recovery"
  | "Replan"
  | "Constitution"
  | "HumanTriggered"
  | "MilestoneGeneration"
  | "MilestoneCheck"
  | "MidStageGeneration"
  | "MidStageCheck"
  | "ExecutionPlanGeneration"
  | "ExecutionPlanCheck"
  | "TaskCalibration"
  | "EvidenceSupplement"
  | "SchemaRepair"
  | "ConstitutionSummary"
  | "ConstitutionCompression"
  | "PreflightCheck"
  | "VersionPlanGeneration"
  | "ExistingProjectAnalysis"
  | "Discussion";

export interface ModelCallRecord {
  call_id: string;
  task_id: string;
  stage_id: string;
  purpose?: ModelCallPurpose;
  model: string;
  provider?: string;
  started_at: string;
  ended_at: string;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  elapsed_ms?: number;
  cache_hit: boolean;
  produced_change: boolean;
  produced_evidence: boolean;
  produced_plan: boolean;
  no_progress: boolean;
  failure_kind: string;
  decision_id?: string;
  action_id?: string;
  provider_response_id?: string;
  produced_contract?: boolean;
  produced_fact?: boolean;
  duplicate_reason?: string;
}

export interface ArchivedModelCallRecord {
  call_id: string;
  task_id: string;
  stage_id: string;
  milestone_id?: string;
  purpose?: ModelCallPurpose;
  provider: string;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  no_progress: boolean;
}

export interface ControlActionStateView {
  action_id: string;
  kind: string;
  task_id: string;
  result: string;
  made_progress: boolean;
  at?: string;
}

export interface TaskControlSnapshot {
  snapshot_version: string;
  project_name: string;
  project_revision: number;
  control_algorithm_version: string;
  control_mode: TaskControlMode;
  control_mode_label: string;
  current_milestone_id: string;
  current_mid_stage_id: string;
  current_task_id: string;
  task_tree_revision: number;
  source_process_start_id: string;
  source_event_sequence: number;
  source_control_action_id: string | null;
  nodes: TaskTreeNodeView[];
  selected_contract?: TaskContract;
  selected_acceptance: AcceptanceLedgerItem[];
  decision?: TaskControlDecision;
  shadow_comparison: ShadowComparisonMetrics;
  current_action?: ControlActionStateView;
  recent_action?: ControlActionStateView;
  control_capabilities: string[];
  cost: TokenCostSummary;
  stage_cost: TokenCostSummary;
  task_cost: TokenCostSummary;
  provider_costs: CostGroupSummary[];
  purpose_costs: CostGroupSummary[];
  cost_calls: ModelCallRecord[];
  events: Array<{
    timestamp: string;
    level: string;
    source: string;
    text: string;
    task_id?: string;
    criterion_index?: number;
    decision_id?: string;
    action_id?: string;
    validator_id?: string;
    model_call_id?: string;
  }>;
  heartbeat_at: string;
}

export interface TaskControlActionResult {
  snapshot: TaskControlSnapshot;
  job_started: boolean;
  queued: boolean;
  action_id: string;
  project_revision: number;
  snapshot_version: string;
}

export type ExecutionSessionStatus =
  | "Executing"
  | "AwaitingConfirmation"
  | "QualityBlocked"
  | "ConfirmationBlocked"
  | "SessionLost"
  | "ExecutionFailed"
  | "StopFailed";

export type ConfirmationPhase =
  | "NotStarted"
  | "Preparing"
  | "CommitCreated"
  | "TagCreated"
  | "ProjectFinalizing";

export type GitConfirmationFailureKind =
  | "TagIdentityConflict"
  | "LegacyV1TagConflict"
  | "V2TagIntegrityConflict"
  | "ScopeViolation"
  | "CommitFailed"
  | "TagFailed"
  | "ProjectFinalizationFailed"
  | "GitMetadataUnavailable";

/** 执行会话 — 记录当前正在执行或待确认的小阶段 */
export interface ExecutionSession {
  execution_id: string;
  active: boolean;
  milestone_id: string;
  mid_stage_id: string;
  subtask_id: string;
  subtask_title: string;
  status: string;        // "executing" | "awaiting_confirmation" | "execution_failed" | ...
  base_commit: string;   // 执行前的 Git commit，用于回退基线
  /** 失败原因；旧项目默认空 */
  failure_message?: string;
  verification_stage?: VerificationStage;
  confirmation_transaction_id?: string;
  confirmation_phase?: ConfirmationPhase;
  confirmation_candidate_tag?: string;
  confirmation_commit?: string;
  confirmation_failure_kind?: GitConfirmationFailureKind;
  started_at: string;
  state_entered_at: string;
  plan_revision: number;
  subtask_index: number;
  total_subtasks: number;
  engine_snapshot: ExecutionProfile;
  engine_settings_revision: number;
  engine_source_revision: string;
  engine_api_backend: string;
  engine_model: string;
  endpoint_fingerprint: string;
  engine_executable_path: string;
  human_review_cadence: HumanReviewCadence;
}

// ========== 宪法变更历史 ==========

export interface ConstitutionChangeEntry {
  timestamp: string;
  subtask_id: string;
  subtask_title: string;
  change_summary: string;
  token_estimate: number;
}

export interface ConstitutionChangeHistory {
  entries: ConstitutionChangeEntry[];
  current_token_estimate: number;
  compaction_threshold: number;
  needs_compaction: boolean;
}

// ========== 代码变更历史 ==========

/** 执行事件类型 */
export type ExecutionEventType =
  | "WorkspacePrepare"
  | "WorkspaceReady"
  | "WorkspacePrepareFailed"
  | "UserExecute"
  | "AutopilotExecute"
  | "SubtaskExecuting"
  | "ExecutorComplete"
  | "TestComplete"
  | "AwaitingConfirmation"
  | "UserConfirm"
  | "AutopilotConfirm"
  | "UserReject"
  | "UserInStop"
  | "UserEdStop"
  | "UserContinue"
  | "UserAdjust"
  | "UserRollback"
  | "MidStageComplete"
  | "AdvanceNextMidStage"
  | "AdvanceMilestoneReview"
  | "SystemAdvance"
  | "QualityGateBlocked"
  | "GitConfirmationStarted"
  | "GitConfirmationCommitCreated"
  | "GitConfirmationBlocked"
  | "GitConfirmationCompleted"
  | "StaleControlLockCleared"
  | "StaleControlActionNeedsHumanConfirmation"
  | "RetryScheduled"
  | "ExecutionFailed"
  | "RecoveryStarted"
  | "ErrorDiagnosed"
  | "RepairAttemptStarted"
  | "RepairAttemptCompleted"
  | "RetestCompleted"
  | "EvidenceRebuildStarted"
  | "EvidenceRebuildCompleted"
  | "EvidenceStillInsufficient"
  | "RecoverySucceeded"
  | "RecoveryExhausted"
  | "ReviewRequested"
  | "ProtocolNormalized"
  | "ProtocolRepairAttempted"
  | "ValidationRetryScheduled"
  | "ValidationRecoverySucceeded"
  | "HumanVerificationAccepted"
  | "ReplanStarted"
  | "ReplanCompleted"
  | "ReplanExecutionStarted"
  | "PlanCalibrationApplied"
  | "TaskSkipped"
  | "EngineProfileChanged";

export type OperationSource = "User" | "Autopilot" | "Recovery" | "System";

/** 执行历史条目 — 持久化到 Project 中，刷新不丢 */
export interface ExecutionHistoryEntry {
  timestamp: string;
  level: string;           // "info" | "success" | "error" | "pause"
  event_type: ExecutionEventType;
  source?: OperationSource;
  text: string;
  milestone_id?: string;
  mid_stage_id?: string;
  subtask_id?: string;
  criterion_index?: number;
  decision_id?: string;
  action_id?: string;
  validator_id?: string;
  model_call_id?: string;
  control_lock_owner_process_start_id?: string;
  control_lock_heartbeat_at?: string;
  control_lock_clear_reason?: string;
  control_lock_post_task_state?: string;
}

export interface ChangeHistoryEntry {
  subtask_id: string;
  subtask_title: string;
  recorded_at: string;
  files_changed: string[];
  diff_text: string;
  diff_truncated: boolean;
}

// ========== Phase 3 遗存 ==========

export type PipelineStatus = "Idle" | "Running" | "Paused" | "Completed" | "Failed";

export interface SubtaskStatusItem {
  subtask_id: string;
  title: string;
  status: "waiting" | "executing" | "repairing" | "testing" | "passed" | "retrying";
  test_result?: TestResult;
  retry_count: number;
}

export interface LogEntry {
  timestamp: string;  // ISO 8601
  level: string;      // "info" | "success" | "error" | "pause" | "debug"
  text: string;
  source?: string;
  correlation_id?: string;
}

export interface PipelineState {
  execution_id: string;
  mid_stage_id: string;
  status: PipelineStatus;
  current_subtask_index: number;
  total_subtasks: number;
  subtask_statuses: SubtaskStatusItem[];
  current_log: string;
  last_error?: string;
  child_pid?: number;
  // V1
  project_name: string;
  milestone_id: string;
  plan_revision: number;
  current_subtask_id: string;
  awaiting_confirmation: boolean;
  log_history: LogEntry[];
}

export interface RuntimeSnapshot {
  project: Project;
  pipeline_state: PipelineState | null;
  process_start_id: string;
  event_sequence: number;
  recovery_presentation: RecoveryPresentation;
  task_control_snapshot_version: string;
  task_control_tree_revision: number;
  task_control_event_sequence: number;
  task_control_action_id: string | null;
  task_control_mode: TaskControlMode;
  task_control_snapshot?: TaskControlSnapshot | null;
}

export interface RuntimeMutationResult {
  result_version: string;
  runtime_snapshot: RuntimeSnapshot;
  task_control: TaskControlSnapshotSummary;
  action: RuntimeActionSummary;
  task_control_snapshot: TaskControlSnapshot | null;
}

// ========== DiffSummary ==========

export interface DiffSummary {
  new_files: string[];
  modified_files: string[];
  deleted_files: string[];
  new_functions: string[];
  modified_functions: string[];
  deleted_functions: string[];
  changed_dependencies: string[];
}

// ========== 后端命令返回值 ==========

export interface ConstitutionSummary {
  structure_description: string;
  function_count: number;
  recent_changes: string[];
  total_tokens: number;
}

export interface GitTagInfo {
  name: string;
  date: string;
  subject: string;
}

// ========== Git 标签树 ==========

export interface GitTagTree {
  milestones: MilestoneTagNode[];
}

export interface MilestoneTagNode {
  milestone_id: string;
  milestone_title: string;
  milestone_version: string;
  milestone_status: string;
  mid_stages: MidStageTagNode[];
  subtasks: SubtaskTagNode[];
}

export interface MidStageTagNode {
  mid_stage_id: string;
  mid_stage_title: string;
  mid_stage_version: string;
  mid_stage_tag: string;
  mid_stage_status: string;
  subtasks: SubtaskTagNode[];
}

export interface SubtaskTagNode {
  subtask_id: string;
  subtask_title: string;
  subtask_index: number;
  subtask_tag: string;
  subtask_status: string;
}

export interface FileEntry {
  path: string;
  is_dir: boolean;
  file_type: string;
}

export interface FilePreviewResult {
  path: string;
  content: string;
  file_type: string;
  truncated: boolean;
  binary: boolean;
  error?: string | null;
}

// ========== 测试日志 + 视图模式 ==========

export interface TestLog {
  subtask_title: string;
  status: 'passed' | 'rejected' | 'retried';
  reason?: string;
  files?: string[];
  full_report?: string;
}

export type ViewPhase = 'discussion' | 'execution';
export type DiscussionReason = 'idle' | 'active' | 'review' | 'paused' | 'discuss_summary' | 'view_report';

export interface ViewMode {
  phase: ViewPhase;
  reason?: DiscussionReason;
}

// ========== 小阶段回退 ==========

export interface RollbackToSubtaskPayload {
  projectPath: string;
  projectId: string;
  tagName: string;
  subtaskTitle: string;
  midStageVersion: string;
  subtaskIndex: number;
}

export interface PathValidationResult {
  is_valid: boolean;
  exists: boolean;
  is_directory: boolean;
  is_git_repo: boolean;
  error_message: string;
}

  /** 执行工作区状态 — 进入 Execution 步骤后的 Git 就绪探测结果 */
export interface ExecutionWorkspaceStatus {
  path_exists: boolean;
  is_directory: boolean;
  is_git_repo: boolean;
  has_commits: boolean;
  git_user_available: boolean;
  git_email_available: boolean;
  working_tree_clean: boolean;
  git_metadata_ready: boolean;
  ready_for_new_execution: boolean;
  has_managed_task_changes: boolean;
  has_external_changes: boolean;
  /** @deprecated 使用 ready_for_new_execution。 */
  ready: boolean;
  status_message: string;
  issues: ExecutionWorkspaceIssue[];
  changes: ExecutionWorkspaceChange[];
}

export type ExecutionWorkspaceIssue =
  | "PathMissing"
  | "NotDirectory"
  | "NotGitRepository"
  | "NoCommits"
  | "MissingGitUserName"
  | "MissingGitUserEmail"
  | "DirtyWorkingTree";

export interface ExecutionWorkspaceChange {
  path: string;
  index_status: string;
  worktree_status: string;
  tracked: boolean;
  managed: boolean;
}

export interface ExecutionRecoveryImpact {
  action_label: string;
  confirmation_title: string;
  presentation_description: string;
  safety_stash_summary: string;
  baseline_commit: string;
  current_head: string;
  affected_files: string[];
  untracked_files: string[];
  managed_changes: string[];
  external_changes: string[];
  discarded_files: string[];
  creates_safety_stash: boolean;
  has_destructive_changes: boolean;
  state_fingerprint: string;
}

export interface RollbackCheckpoint {
  milestoneId: string;
  midStageId: string;
  subtaskId: string;
}
