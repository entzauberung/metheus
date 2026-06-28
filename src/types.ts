// 与 Rust 后端 project.rs 的数据结构一一对应

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
  status: "Pending" | "Executing" | "Passed" | "Rejected" | "RolledBack";
  test_report: string;
  execution_result?: ExecutionResult;
  test_result?: TestResult;
  retry_count: number;
  auto_tag?: string;  // 小阶段 auto tag，格式 metheus/auto/v0.1.1/task-0
}

export type MidStageStatus = "Pending" | "Ready" | "InProgress" | "Completed" | "Rejected" | "Approved" | "RolledBack";

export interface MidStage {
  id: string;
  title: string;
  version: string;
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
  git_tag?: string;  // ← 4.1.1b
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
  qa_result?: QAResult;  // ← 需求质检
  git_commit_hash: string;
}

export interface ChatMessage {
  id: string;
  role: string;
  content: string;
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
  mode: ProjectMode;
  current_milestone_id: string;
  current_mid_stage_id: string;
  version_plan: string;
  milestones: Milestone[];
  discussion_threads: DiscussionThread[];
  project_path: string;
}

// ========== Phase 3 新增 ==========

export type PipelineStatus = "Idle" | "Running" | "Paused" | "Completed" | "Failed";

export interface SubtaskStatusItem {
  subtask_id: string;
  title: string;
  status: "waiting" | "executing" | "testing" | "passed" | "retrying";
  test_result?: TestResult;
  retry_count: number;
}

export interface PipelineState {
  mid_stage_id: string;
  status: PipelineStatus;
  current_subtask_index: number;
  total_subtasks: number;
  subtask_statuses: SubtaskStatusItem[];
  current_log: string;
  last_error?: string;
}

// ========== 阶段一新增：DiffSummary ==========

export interface DiffSummary {
  new_files: string[];
  modified_files: string[];
  deleted_files: string[];
  new_functions: string[];
  modified_functions: string[];
  deleted_functions: string[];
  changed_dependencies: string[];
}

// ========== 阶段三新增：小阶段回退 ==========

// ========== Phase A 新增：后端命令返回值 ==========

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

export interface FileEntry {
  path: string;
  is_dir: boolean;
  file_type: string;
}

// ========== Phase C 新增：测试日志 + 视图模式 ==========

export interface TestLog {
  subtask_title: string;
  status: 'passed' | 'rejected' | 'retried';
  reason?: string;
  files?: string[];
  full_report?: string;
}

export type ViewPhase = 'discussion' | 'execution';
export type DiscussionReason = 'idle' | 'active' | 'review' | 'paused';

export interface ViewMode {
  phase: ViewPhase;
  reason?: DiscussionReason;
}

// ========== 阶段三新增：小阶段回退 ==========

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
