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
  generate_milestones: 180,
  regenerate_milestones_with_feedback: 180,
  generate_mid_stages: 180,
  generate_next_prompt: 120,

  // 执行控制类 — 后端轻量操作
  start_execution: 30,
  pause_execution: 15,
  resume_execution: 15,
  stop_execution: 15,
  approve_version_plan: 30,

  // 测试/检查类 — 涉及文件读写和子进程调用
  execute_subtask: 120,
  check_subtask: 120,

  // 状态查询类 — 高频轮询，短超时
  get_execution_status: 10,

  // 持久化类 — 文件 I/O
  persist_project: 15,

  // 数据获取类
  get_project: 10,
  get_project_files: 15,
  get_current_diff: 10,
  get_git_tags_summary: 10,
  read_constitution: 10,
  validate_project_path: 10,

  // Git 操作类
  git_rollback_to_mid_stage: 60,
  git_rollback_to_subtask: 60,
};

/// 未显式配置的命令超时秒数
const DEFAULT_TIMEOUT_SECS = 30;

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

  const timeoutPromise = new Promise<never>((_, reject) => {
    setTimeout(() => {
      reject(new Error(`请求超时（${cmd}，超过 ${timeout / 1000} 秒），请检查网络或重试`));
    }, timeout);
  });

  return Promise.race([invoke<T>(cmd, args), timeoutPromise]);
}

export { INVOKE_TIMEOUT_MAP, DEFAULT_TIMEOUT_SECS };
