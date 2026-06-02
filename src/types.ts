// 与 Rust 后端 project.rs 的数据结构一一对应

export type ProjectStatus = "Idle" | "Discussing" | "Planning" | "MilestoneReady" | "Executing" | "Paused";

export type MilestoneStatus = "Pending" | "InProgress" | "Completed" | "Paused";

export interface ExecutionResult {
  success: boolean;
  output: string;
  error_log: string;
  file_changes: string[];
}

export interface TestResult {
  passed: boolean;
  issues: string[];
  suggestion: string;
}

export interface GeneratedSubtask {
  title: string;
  prompt: string;
}

export interface Subtask {
  id: string;
  title: string;
  prompt: string;
  status: "Pending" | "Executing" | "Passed" | "Rejected";
  test_report: string;
  execution_result?: ExecutionResult;
  test_result?: TestResult;
  retry_count: number;
}

export type MidStageStatus = "Pending" | "Ready" | "InProgress" | "Completed" | "Rejected" | "Approved";

export interface MidStage {
  id: string;
  version: string;
  title: string;
  description: string;
  tech_focus: string;
  status: MidStageStatus;
  subtasks: Subtask[];
  test_report: string;
}

export type StageMode = "Quick" | "Professional";
export type ProjectMode = "Quick" | "Professional";

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
}
