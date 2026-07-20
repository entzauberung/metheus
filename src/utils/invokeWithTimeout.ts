// Copyright (C) 2026 Bruce Long
// 为所有 Tauri invoke 调用提供统一超时保护，防止网络/后端故障导致前端永久卡死

import { invoke } from "@tauri-apps/api/core";

/// 各 Tauri 命令的超时秒数映射表
/// Key: invoke 命令名，Value: 超时秒数
/// 未在此表中的命令使用默认值 DEFAULT_TIMEOUT_SECS（30 秒）
const INVOKE_TIMEOUT_MAP: Record<string, number> = {
  // 聊天/讨论类 — 用户可感知，不宜过长
  send_message: 60,
  chat_with_role: 60,

  // 方案生成类 — 大模型计算密集，给充裕时间
  generate_version_plan: 180,

  // 执行控制类 — 后端轻量操作
  approve_version_plan: 30,

  // 测试/检查类 — 涉及文件读写和子进程调用
  execute_subtask: 120,
  check_subtask: 120,

  // 状态查询类 — 高频轮询，短超时
  get_execution_status: 10,

  // 持久化类 — 文件 I/O

  // 数据获取类
  get_project: 10,
  get_project_files: 15,
  get_current_diff: 10,
  get_change_history: 10,
  get_git_tags_summary: 10,
  read_constitution: 10,
  get_constitution_change_history: 10,
  validate_project_path: 10,

  // Git 操作类

  // 宪法管理（AI 调用 + 文件 I/O）
  update_constitution: 120,

  // 快照持久化
  save_snapshot_event: 15,
  restore_snapshot: 15,

  // 旧兼容命令（仍注册，仅用于旧数据迁移）
  approve_mid_stage: 15,
  reject_mid_stage: 15,

  // V1 新增命令 - 项目入口
  initialize_project_entry: 30,
  scan_existing_project: 60,
  generate_existing_baseline: 120,
  approve_existing_baseline: 15,

  // V1 新增命令 - 三项检查
  run_preflight_check: 120,

  // V1 新增命令 - 执行计划
  generate_execution_plan: 180,
  check_stage_plan: 150,
  approve_stage_plan: 15,

  // V1 Console 规划闭环 - AI 命令必须长于后端 120 秒 HTTP 超时
  generate_milestone_draft: 150,
  regenerate_milestone_draft: 150,
  check_milestone_draft: 150,
  approve_milestone_draft: 15,
  select_milestone: 15,
  generate_mid_stage_draft: 150,
  regenerate_mid_stage_draft: 150,
  check_mid_stage_draft: 150,
  approve_mid_stage_draft: 15,
  select_mid_stage: 15,
  regenerate_execution_plan: 180,

  // V1 新增命令 - 执行控制（autopilot 也需要这些）
  execute_current_subtask: 15,
  confirm_subtask_result: 30,
  reject_subtask_result: 15,
  get_execution_workspace_status: 15,
  prepare_execution_workspace: 60,
  request_in_stop: 30,
  request_ed_stop: 15,
  resolve_pause_decision: 15,
  preview_rollback_impact: 15,
  confirm_rollback: 30,

  // V1 新增命令 - 工作流
  transition_workflow: 15,
  migrate_project_workflow: 15,
  toggle_autopilot: 15,
  autopilot_pause: 120,  // 可能涉及 In Stop kill 子进程
  autopilot_next_step: 15,
  start_preflight_check: 15,
  return_to_discussion: 15,
  resume_plan_approval: 15,
  restart_discussion_from_approved: 15,
  restart_checks: 15,

  // V1 新增命令 - 暂停决策
  suggest_rollback_checkpoint: 120,
  approve_milestone_outcome: 15,

  // V1 新增命令 - 大阶段审阅与未来规划
  enter_milestone_review: 15,
  approve_future_milestones: 15,
  generate_future_milestone_draft: 150,
  summarize_milestone: 120,

  // V1 新增命令 - 宪法
  get_constitution_summary: 15,
  compact_constitution: 60,

  // V1 新增命令 - 方案和控制台
  reject_version_plan: 15,
  enter_console: 15,
  analyze_existing_project: 180,
};

/// 未显式配置的命令超时秒数
const DEFAULT_TIMEOUT_SECS = 30;

export class InvokeTimeoutError extends Error {
  readonly command: string;
  readonly timeoutMs: number;

  constructor(command: string, timeoutMs: number) {
    super(`请求等待超时（${command}，超过 ${timeoutMs / 1000} 秒）`);
    this.name = "InvokeTimeoutError";
    this.command = command;
    this.timeoutMs = timeoutMs;
  }
}

export function isInvokeTimeoutError(error: unknown): error is InvokeTimeoutError {
  return error instanceof InvokeTimeoutError;
}

/**
 * 带超时的 invoke 包装器。
 *
 * 用法与原 invoke 完全一致，仅增加超时保护：
 *   const result = await invokeWithTimeout<MyType>("command_name", { arg: val });
 *
 * @param cmd   Tauri 命令名称
 * @param args  命令参数对象（可选）
 * @param timeoutMs 可选的显式超时毫秒数，不传则从 INVOKE_TIMEOUT_MAP 查找
 * @returns Promise<T> — 与 invoke 返回值一致
 */
export async function invokeWithTimeout<T>(
  cmd: string,
  args?: Record<string, unknown>,
  timeoutMs?: number,
): Promise<T> {
  const timeout = timeoutMs ?? (INVOKE_TIMEOUT_MAP[cmd] ?? DEFAULT_TIMEOUT_SECS) * 1000;

  // 调试：记录未配置的命令
  if (!INVOKE_TIMEOUT_MAP[cmd] && !timeoutMs) {
    console.warn(
      `[invokeWithTimeout] 命令 "${cmd}" 未在超时映射表中配置，使用默认值 ${DEFAULT_TIMEOUT_SECS}s`,
    );
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new InvokeTimeoutError(cmd, timeout));
    }, timeout);
  });

  try {
    return await Promise.race([invoke<T>(cmd, args), timeoutPromise]);
  } finally {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  }
}

export { INVOKE_TIMEOUT_MAP, DEFAULT_TIMEOUT_SECS };
