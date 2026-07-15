// 与 Rust 后端 project.rs 的数据结构一一对应

// ========== 工作流状态（唯一业务状态，决定前端显示和按钮权限） ==========

export type TopLevelPhase = "Before" | "FirstDiscussion" | "Console" | "Completed";

export type WorkflowStep =
  | "WaitingEntry"
  | "ExistingAnalysis"
  | "BaselineApproval"
  | "Discussion"
  | "ThreeChecks"
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

/** 自动驾驶下一步建议（与 Rust AutopilotNextStep 一一对应） */
export interface AutopilotNextStep {
  command: string;
  args: Record<string, unknown>;
  description: string;
  at_milestone_boundary: boolean;
  is_error: boolean;
  error_message: string;
  // Phase 3: progress fields
  completed_subtasks: number;
  total_subtasks: number;
  current_mid_stage_index: number;
  total_mid_stages: number;
  current_action: string;
}

export interface AutopilotState {
  active: boolean;
  target_milestone_id: string;
  run_status: AutopilotRunStatus;
  last_action: string;
  last_action_at: string;
  error_message: string;
  /** 最近一次自动补救原因（空字符串表示无补救） */
  last_recovery_reason: string;
  /** 累计自动补救次数 */
  recovery_count: number;
  /** 最近一次自动补救时间（ISO 8601） */
  last_recovery_at: string;
  /** 暂停时的步骤（用于恢复上下文） */
  pause_step: string;
  /** 暂停时的大阶段 ID（用于恢复上下文） */
  pause_milestone_id: string;
  /** 暂停时的中阶段 ID（用于恢复上下文） */
  pause_mid_stage_id: string;
}

export type DiscussionScope = "FirstDiscussion" | "PauseAdjustment" | "FixPast" | "AdjustFuture";

export interface WorkflowState {
  top_level_phase: TopLevelPhase;
  current_step: WorkflowStep;
  pause_reason: PauseReason;
  data_revision: number;
  discussion_scope: DiscussionScope;
  review_node_id: string;
  last_transition_at: string;
  autopilot_active: boolean;
  autopilot_target_milestone_id: string;
  autopilot_state?: AutopilotState;
}

// ========== 项目来源 ==========

export type ProjectEntryKind = "NoProject" | "HalfProject";

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

export type MilestoneDraftStatus = "Pending" | "CheckFailed" | "Approved";

export type MilestoneDraftKind = "Normal" | "FutureOnly";

export interface MilestoneDraft {
  draft_id: string;
  status: MilestoneDraftStatus;
  draft_kind: MilestoneDraftKind;
  candidate_milestones: Milestone[];
  check_result?: string;
  generation_revision: number;
  source_plan_revision: number;
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
}

// ========== 方案草稿 ==========

export interface PlanDraft {
  draft_id: string;
  draft_status: DraftStatus;
  plan_content: string;
  constitution_part1_draft: string;
  generation_revision: number;
  data_revision_at_generation: number;
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
  /** 是否由 autopilot 暂停触发 */
  paused_by_autopilot: boolean;
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

// ========== 项目状态（保留用于旧数据兼容） ==========

export type ProjectStatus = "Idle" | "Discussing" | "Planning" | "MilestoneReady" | "Executing" | "Paused" | "Completed";

export type MilestoneStatus = "Pending" | "InProgress" | "Completed" | "Paused";

export interface ExecutionResult {
  success: boolean;
  output: string;
  error_log: string;
  file_changes: string[];
}

// 子任务执行错误类型（与 Rust SubTaskError 枚举对应）
export type SubTaskError =
  | { type: "UserPaused" }
  | { type: "ExecutionFailed"; message: string }
  | { type: "Timeout" };

export interface TestResult {
  passed: boolean;
  issues: string[];
  suggestion: string;
  warnings?: string[];
}

export interface GeneratedSubtask {
  title: string;
  prompt: string;
}

export interface Subtask {
  id: string;
  title: string;
  prompt: string;
  status: "Pending" | "Executing" | "AwaitingConfirmation" | "Passed" | "Rejected" | "RolledBack";
  test_report: string;
  execution_result?: ExecutionResult;
  test_result?: TestResult;
  retry_count: number;
  auto_tag?: string;  // 小阶段 auto tag，格式 metheus/auto/v0.1.1/task-0
  // V1 结构化字段
  order: number;
  goal: string;
  allowed_file_paths: string[];
  new_file_paths: string[];
  evidence_files: string[];
  context_summary: string;
  acceptance_criteria: string[];
  stop_rules: string[];
  execution_prompt: string;
  confirmed_by_user?: boolean;
  confirmed_at?: string;
  confirmation_notes?: string;
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
}

export type StageMode = "Quick" | "Professional";
export type ProjectMode = "Quick" | "Professional";

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
}

export interface ChatMessage {
  id: string;
  role: string;
  content: string;
  msg_type?: string;       // 与 Rust Message.msg_type 一致
  approved?: boolean;
  rejected?: boolean;
  milestone_id?: string;   // 与 Rust Message.milestone_id 一致
  timestamp: number;
}

export interface DiscussionThread {
  id: string;
  title: string;
  node_id: string;
  messages: ChatMessage[];
}

export interface Project {
  name: string;
  status: ProjectStatus;
  entry_kind: ProjectEntryKind;
  workflow_state: WorkflowState;
  mode: ProjectMode;
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
  project_path: string;
}

/** 执行会话 — 记录当前正在执行或待确认的小阶段 */
export interface ExecutionSession {
  active: boolean;
  milestone_id: string;
  mid_stage_id: string;
  subtask_id: string;
  subtask_title: string;
  status: string;        // "executing" | "awaiting_confirmation"
  started_at: string;
  state_entered_at: string;
  plan_revision: number;
  subtask_index: number;
  total_subtasks: number;
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
  | "SubtaskExecuting"
  | "ExecutorComplete"
  | "TestComplete"
  | "AwaitingConfirmation"
  | "UserConfirm"
  | "UserReject"
  | "UserInStop"
  | "UserEdStop"
  | "UserContinue"
  | "UserAdjust"
  | "UserRollback"
  | "MidStageComplete"
  | "AdvanceNextMidStage"
  | "AdvanceMilestoneReview"
  | "SystemAdvance";

/** 执行历史条目 — 持久化到 Project 中，刷新不丢 */
export interface ExecutionHistoryEntry {
  timestamp: string;
  level: string;           // "info" | "success" | "error" | "pause"
  event_type: ExecutionEventType;
  text: string;
  milestone_id?: string;
  mid_stage_id?: string;
  subtask_id?: string;
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
  status: "waiting" | "executing" | "testing" | "passed" | "retrying";
  test_result?: TestResult;
  retry_count: number;
}

export interface LogEntry {
  timestamp: string;  // ISO 8601
  level: string;      // "info" | "success" | "error" | "pause"
  text: string;
}

export interface PipelineState {
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
  ready: boolean;
  status_message: string;
}

export interface RollbackCheckpoint {
  milestoneId: string;
  midStageId: string;
  subtaskId: string;
}
