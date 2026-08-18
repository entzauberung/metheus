// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::AppState;

/// 快照文件后缀，拼接到 project_id 后形成文件名
const SNAPSHOT_FILE_SUFFIX: &str = "_snapshot.json";

/// 当前快照格式版本，用于向前兼容
const SNAPSHOT_VERSION: u32 = 1;

// ============================================================
// 数据结构
// ============================================================

/// 前端 UI 状态快照，由前端序列化后传给后端保存
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UISnapshot {
    /// 仅用于恢复视觉布局，不参与业务阶段裁决。
    pub view_phase: String,
    #[serde(default)]
    pub sidebar_width: Option<u32>,
    #[serde(default)]
    pub active_tab: Option<String>,
    pub saved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum StartupProcessObservationKind {
    Killed,
    AlreadyExited,
    TerminationUnknown,
    IdentityUnverified,
    IntentionalExit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StartupProcessObservation {
    pub pid: u32,
    pub kind: StartupProcessObservationKind,
    pub observed_at: String,
}

/// Identity captured immediately after spawning an execution child.
/// `execution_id` is the business owner; the other fields protect against PID reuse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RunningProcessIdentity {
    pub pid: u32,
    #[serde(default)]
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_start_id: Option<String>,
}

/// 应用完整快照，持久化到 ~/.metheus/{project_id}_snapshot.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppSnapshot {
    pub ui: UISnapshot,
    pub project_id: String,
    pub snapshot_version: u32,
    /// 孤儿进程保护：当前正在运行的子进程 PID（无则为 None）
    pub running_pid: Option<u32>,
    /// Conservative evidence from the next startup; this does not claim OOM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_process_observation: Option<StartupProcessObservation>,
    /// Optional identity for safe cleanup; old snapshots deserialize without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_process_identity: Option<RunningProcessIdentity>,
    pub saved_at: String,
}

// ============================================================
// 路径辅助函数
// ============================================================

/// 获取快照文件的完整路径：~/.metheus/{project_id}_snapshot.json
fn snapshot_data_path(project_id: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户家目录路径".to_string())?;
    Ok(home
        .join(".metheus")
        .join(format!("{}{}", project_id, SNAPSHOT_FILE_SUFFIX)))
}

// ============================================================
// 核心 I/O 函数
// ============================================================

/// 将 UI 快照持久化到磁盘
///
/// # 参数
/// - `project_id`: 项目标识，用于构造文件路径
/// - `ui`: 前端传来的 UI 状态
/// - `running_pid`: 当前后端流水线中正在运行的子进程 PID（无则为 None）
pub(crate) fn save_snapshot(
    project_id: &str,
    ui: &UISnapshot,
    running_pid: Option<u32>,
) -> Result<(), String> {
    let (prior_observation, running_process_identity) = load_snapshot(project_id)
        .ok()
        .flatten()
        .map(|snapshot| {
            let identity = running_pid
                .filter(|pid| {
                    snapshot
                        .running_process_identity
                        .as_ref()
                        .is_some_and(|value| value.pid == *pid)
                })
                .and(snapshot.running_process_identity);
            (snapshot.startup_process_observation, identity)
        })
        .unwrap_or((None, None));
    save_snapshot_with_observation(
        project_id,
        ui,
        running_pid,
        running_process_identity,
        prior_observation,
    )
}

fn save_snapshot_with_observation(
    project_id: &str,
    ui: &UISnapshot,
    running_pid: Option<u32>,
    running_process_identity: Option<RunningProcessIdentity>,
    startup_process_observation: Option<StartupProcessObservation>,
) -> Result<(), String> {
    let path = snapshot_data_path(project_id)?;

    // 确保 .metheus/ 目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建快照目录失败：{}", e))?;
    }

    let snapshot = AppSnapshot {
        ui: ui.clone(),
        project_id: project_id.to_string(),
        snapshot_version: SNAPSHOT_VERSION,
        running_pid,
        startup_process_observation,
        running_process_identity,
        saved_at: chrono_now(),
    };

    let json =
        serde_json::to_string_pretty(&snapshot).map_err(|e| format!("序列化快照失败: {}", e))?;

    fs::write(&path, json).map_err(|e| format!("写入快照文件失败: {}", e))?;

    Ok(())
}

/// 从磁盘读取快照
///
/// 返回 `Ok(None)` 表示快照文件不存在（首次启动），或文件损坏/版本不兼容（静默处理）。
/// 返回 `Ok(Some(snapshot))` 表示成功读取。
pub(crate) fn load_snapshot(project_id: &str) -> Result<Option<AppSnapshot>, String> {
    let path = snapshot_data_path(project_id)?;

    if !path.exists() {
        return Ok(None);
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            // 文件存在但不可读 → 静默删除损坏文件
            eprintln!(
                "[snapshot] 快照文件读取失败，将删除损坏文件 ({}): {}",
                path.display(),
                e
            );
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    };

    let snapshot: AppSnapshot = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            // JSON 解析失败 → 静默删除损坏文件
            eprintln!(
                "[snapshot] 快照文件 JSON 解析失败，将删除损坏文件 ({}): {}",
                path.display(),
                e
            );
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    };

    // 版本兼容检查
    if snapshot.snapshot_version != SNAPSHOT_VERSION {
        eprintln!(
            "[snapshot] 快照版本不兼容 (文件版本={}, 当前版本={})，将忽略快照",
            snapshot.snapshot_version, SNAPSHOT_VERSION
        );
        return Ok(None);
    }

    Ok(Some(snapshot))
}

/// 仅更新快照中的 running_pid，保留 UI 部分不变
///
/// 用于执行引擎侧（executor/pipeline）在 PID 变更时同步到快照，
/// 无需前端重新传递 UI 状态。
pub(crate) fn update_snapshot_pid(
    project_id: &str,
    execution_id: &str,
    running_pid: Option<u32>,
) -> Result<(), String> {
    if running_pid.is_some() && execution_id.is_empty() {
        return Err("拒绝在缺少 execution_id 时写入子进程 PID".to_string());
    }
    // 读取现有快照（如果存在），并先验证当前执行是否仍拥有该 PID 槽位。
    let (ui, startup_process_observation) = match load_snapshot(project_id)? {
        Some(existing) => {
            validate_running_process_update(&existing, execution_id, running_pid)?;
            (existing.ui, existing.startup_process_observation)
        }
        None => {
            // 无现有快照 → 不创建新快照（前端未初始化过 UI 状态）
            return Ok(());
        }
    };
    let startup_process_observation = running_pid
        .is_none()
        .then_some(startup_process_observation)
        .flatten();
    let running_process_identity = running_pid.map(|pid| {
        let (executable_path, process_start_id) = read_process_identity(pid);
        RunningProcessIdentity {
            pid,
            execution_id: execution_id.to_string(),
            executable_path,
            process_start_id,
        }
    });
    save_snapshot_with_observation(
        project_id,
        &ui,
        running_pid,
        running_process_identity,
        startup_process_observation,
    )
}

fn validate_running_process_update(
    snapshot: &AppSnapshot,
    execution_id: &str,
    running_pid: Option<u32>,
) -> Result<(), String> {
    let Some(existing_pid) = snapshot.running_pid else {
        if snapshot.running_process_identity.is_some() {
            return Err("快照中的进程身份与 running_pid 不一致".to_string());
        }
        return Ok(());
    };
    let Some(identity) = snapshot.running_process_identity.as_ref() else {
        return Err(format!(
            "拒绝覆盖没有可验证身份的运行中 PID={}，请先人工对账",
            existing_pid
        ));
    };
    if identity.pid != existing_pid
        || identity.execution_id.is_empty()
        || execution_id.is_empty()
        || identity.execution_id != execution_id
    {
        return Err(format!(
            "拒绝修改不属于当前 execution 的运行中 PID={}，请先人工对账",
            existing_pid
        ));
    }
    if running_pid.is_some_and(|pid| pid != existing_pid) {
        return Err(format!(
            "拒绝用 PID={:?} 覆盖当前 execution 的 PID={}",
            running_pid, existing_pid
        ));
    }
    Ok(())
}

pub(crate) fn clear_startup_process_observation(project_id: &str) -> Result<(), String> {
    let Some(mut snapshot) = load_snapshot(project_id)? else {
        return Ok(());
    };
    if snapshot.startup_process_observation.is_none() {
        return Ok(());
    }
    snapshot.startup_process_observation = None;
    save_snapshot_record(&snapshot)
}

/// Record a normal application exit without clearing the child evidence.
/// Reopen reconciliation may still terminate a verified child, but it must
/// preserve this cause instead of presenting it as a resource kill.
pub(crate) fn mark_intentional_exit(
    project_id: &str,
    execution_id: &str,
    running_pid: Option<u32>,
) -> Result<bool, String> {
    let Some(mut snapshot) = load_snapshot(project_id)? else {
        return Ok(false);
    };
    let pid = running_pid.or(snapshot.running_pid);
    let Some(pid) = pid else {
        return Ok(false);
    };
    if snapshot.running_pid.is_some_and(|existing| existing != pid) {
        return Ok(false);
    }
    if let Some(identity) = snapshot.running_process_identity.as_ref() {
        if identity.pid != pid
            || (!identity.execution_id.is_empty()
                && !execution_id.is_empty()
                && identity.execution_id != execution_id)
        {
            return Ok(false);
        }
    }

    snapshot.running_pid = Some(pid);
    if snapshot.running_process_identity.is_none() {
        let (executable_path, process_start_id) = read_process_identity(pid);
        snapshot.running_process_identity = Some(RunningProcessIdentity {
            pid,
            execution_id: execution_id.to_string(),
            executable_path,
            process_start_id,
        });
    } else if let Some(identity) = snapshot.running_process_identity.as_mut() {
        if identity.execution_id.is_empty() && !execution_id.is_empty() {
            identity.execution_id = execution_id.to_string();
        }
    }
    snapshot.startup_process_observation = Some(StartupProcessObservation {
        pid,
        kind: StartupProcessObservationKind::IntentionalExit,
        observed_at: chrono_now(),
    });
    save_snapshot_record(&snapshot)?;
    Ok(true)
}

// ============================================================
// Tauri 命令
// ============================================================

/// 前端保存 UI 状态快照（fire-and-forget 调用）
///
/// 前端将当前 UI 状态序列化为 JSON 传给后端，后端合并 running_pid 后写盘。
#[tauri::command]
pub(crate) async fn save_snapshot_event(
    state: tauri::State<'_, AppState>,
    project_id: String,
    ui_json: String,
) -> Result<(), String> {
    let ui: UISnapshot =
        serde_json::from_str(&ui_json).map_err(|e| format!("解析 UI 快照 JSON 失败: {}", e))?;

    // 从当前流水线状态中取 child_pid
    let running_pid = {
        let guard = state.pipeline_state.lock().await;
        guard.as_ref().and_then(|ps| ps.child_pid)
    };

    save_snapshot(&project_id, &ui, running_pid)
}

/// 前端加载快照（项目首次加载时调用）
///
/// 返回 `Ok(None)` 表示无快照或快照不可用，前端沿用默认状态。
#[tauri::command]
pub(crate) async fn restore_snapshot(project_id: String) -> Result<Option<AppSnapshot>, String> {
    load_snapshot(&project_id)
}

// ============================================================
// 孤儿进程保护（任务 L）
// ============================================================

/// 检查指定 PID 是否对应一个存活的进程
///
/// Unix: 使用 `kill -0` 检测（不发送信号，仅检查存在性）
/// Windows: 使用 `tasklist /FI "PID eq {pid}"` 检测
pub(crate) fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // tasklist 无匹配时输出 "INFO: No tasks are running..."
                !stdout.contains("No tasks") && !stdout.trim().is_empty()
            })
            .unwrap_or(false)
    }
}

/// 终止指定 PID 的进程
fn kill_pid(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn read_process_identity(pid: u32) -> (Option<String>, Option<String>) {
    #[cfg(target_os = "linux")]
    {
        let executable_path = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
        let process_start_id = fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')?
                    .1
                    .split_whitespace()
                    .nth(19)
                    .map(str::to_string)
            });
        (executable_path, process_start_id)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        (None, None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessIdentityCheck {
    Match,
    Unverified,
}

fn process_identity_check(identity: &RunningProcessIdentity) -> ProcessIdentityCheck {
    if identity.execution_id.is_empty()
        || identity.executable_path.is_none()
        || identity.process_start_id.is_none()
    {
        return ProcessIdentityCheck::Unverified;
    }
    let (executable_path, process_start_id) = read_process_identity(identity.pid);
    if executable_path == identity.executable_path && process_start_id == identity.process_start_id
    {
        ProcessIdentityCheck::Match
    } else {
        ProcessIdentityCheck::Unverified
    }
}

fn startup_process_cleanup_decision(
    snapshot: &AppSnapshot,
    pid_alive: bool,
) -> (bool, StartupProcessObservationKind) {
    if !pid_alive {
        return (true, StartupProcessObservationKind::AlreadyExited);
    }
    match snapshot.running_process_identity.as_ref() {
        Some(identity)
            if identity.pid == snapshot.running_pid.unwrap_or_default()
                && process_identity_check(identity) == ProcessIdentityCheck::Match =>
        {
            (true, StartupProcessObservationKind::Killed)
        }
        _ => (false, StartupProcessObservationKind::IdentityUnverified),
    }
}

fn preserve_intentional_exit_observation(
    snapshot: &AppSnapshot,
    observed: StartupProcessObservationKind,
) -> StartupProcessObservationKind {
    let prior_intentional_exit =
        snapshot
            .startup_process_observation
            .as_ref()
            .is_some_and(|observation| {
                observation.kind == StartupProcessObservationKind::IntentionalExit
            });
    if prior_intentional_exit
        && matches!(
            observed,
            StartupProcessObservationKind::Killed | StartupProcessObservationKind::AlreadyExited
        )
    {
        StartupProcessObservationKind::IntentionalExit
    } else {
        observed
    }
}

/// 应用启动时调用：扫描所有快照文件，安全处理遗留执行进程
///
/// 遍历 `~/.metheus/*_snapshot.json`，检测每个快照中的 `running_pid`。
/// 只有身份可验证且匹配时才终止存活 PID；旧快照或无法验证身份的存活 PID
/// 保留并记录人工阻断。已退出 PID 可直接清除快照残留。
pub(crate) fn cleanup_orphan_processes_at_startup() {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("[snapshot] 无法获取家目录，跳过孤儿进程清理");
            return;
        }
    };

    let metheus_dir = home.join(".metheus");
    if !metheus_dir.exists() || !metheus_dir.is_dir() {
        // 目录不存在 → 首次启动，无快照可清理
        return;
    }

    let entries = match fs::read_dir(&metheus_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[snapshot] 无法读取 .metheus 目录: {}", e);
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // 仅处理 *_snapshot.json 文件
        if !file_name.ends_with("_snapshot.json") {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut snapshot: AppSnapshot = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Some(pid) = snapshot.running_pid {
            let (should_attempt_kill, observation_kind) =
                startup_process_cleanup_decision(&snapshot, is_pid_alive(pid));
            let (should_clear_pid, observation_kind) = if should_attempt_kill
                && observation_kind == StartupProcessObservationKind::Killed
            {
                eprintln!(
                    "[snapshot] 已验证孤儿进程 PID={} (项目={})，正在终止...",
                    pid, snapshot.project_id
                );
                if kill_pid(pid) {
                    eprintln!("[snapshot] 孤儿进程 PID={} 已终止", pid);
                    (
                        true,
                        preserve_intentional_exit_observation(
                            &snapshot,
                            StartupProcessObservationKind::Killed,
                        ),
                    )
                } else {
                    eprintln!(
                        "[snapshot] 警告: 无法终止孤儿进程 PID={}（权限不足或进程已退出）",
                        pid
                    );
                    (false, StartupProcessObservationKind::TerminationUnknown)
                }
            } else {
                if observation_kind == StartupProcessObservationKind::IdentityUnverified {
                    eprintln!(
                        "[snapshot] 人工阻断：无法证明 PID={} (项目={}) 仍属于当前 execution，保留 PID 不终止",
                        pid, snapshot.project_id
                    );
                }
                (
                    observation_kind == StartupProcessObservationKind::AlreadyExited,
                    preserve_intentional_exit_observation(&snapshot, observation_kind),
                )
            };

            snapshot.startup_process_observation = Some(StartupProcessObservation {
                pid,
                kind: observation_kind,
                observed_at: chrono_now(),
            });
            if should_clear_pid {
                snapshot.running_pid = None;
                snapshot.running_process_identity = None;
            }
            if let Err(e) = save_snapshot_record(&snapshot) {
                eprintln!("[snapshot] 清除快照 PID 失败，保留阻断证据: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with_identity(identity: Option<RunningProcessIdentity>) -> AppSnapshot {
        AppSnapshot {
            ui: UISnapshot {
                view_phase: "Console".to_string(),
                sidebar_width: None,
                active_tab: None,
                saved_at: "0".to_string(),
            },
            project_id: "identity-test".to_string(),
            snapshot_version: SNAPSHOT_VERSION,
            running_pid: Some(42),
            startup_process_observation: None,
            running_process_identity: identity,
            saved_at: "0".to_string(),
        }
    }

    #[test]
    fn old_snapshot_alive_pid_is_human_blocked_without_identity() {
        let snapshot = snapshot_with_identity(None);
        assert_eq!(
            startup_process_cleanup_decision(&snapshot, true),
            (false, StartupProcessObservationKind::IdentityUnverified)
        );
    }

    #[test]
    fn dead_pid_can_be_cleared_without_identity() {
        let snapshot = snapshot_with_identity(None);
        assert_eq!(
            startup_process_cleanup_decision(&snapshot, false),
            (true, StartupProcessObservationKind::AlreadyExited)
        );
    }

    #[test]
    fn incomplete_identity_never_authorizes_kill() {
        let snapshot = snapshot_with_identity(Some(RunningProcessIdentity {
            pid: 42,
            execution_id: "execution-1".to_string(),
            executable_path: None,
            process_start_id: None,
        }));
        assert_eq!(
            startup_process_cleanup_decision(&snapshot, true),
            (false, StartupProcessObservationKind::IdentityUnverified)
        );
    }

    #[test]
    fn pid_reuse_identity_mismatch_never_authorizes_kill() {
        let pid = std::process::id();
        let mut snapshot = snapshot_with_identity(Some(RunningProcessIdentity {
            pid,
            execution_id: "execution-1".to_string(),
            executable_path: Some("/definitely-not-the-current-executable".to_string()),
            process_start_id: Some("definitely-not-the-current-start".to_string()),
        }));
        snapshot.running_pid = Some(pid);
        assert_eq!(
            startup_process_cleanup_decision(&snapshot, true),
            (false, StartupProcessObservationKind::IdentityUnverified)
        );
    }

    #[test]
    fn intentional_exit_observation_survives_verified_reopen_cleanup() {
        let mut snapshot = snapshot_with_identity(None);
        snapshot.startup_process_observation = Some(StartupProcessObservation {
            pid: 42,
            kind: StartupProcessObservationKind::IntentionalExit,
            observed_at: "0".to_string(),
        });
        assert_eq!(
            preserve_intentional_exit_observation(&snapshot, StartupProcessObservationKind::Killed),
            StartupProcessObservationKind::IntentionalExit
        );
        assert_eq!(
            preserve_intentional_exit_observation(
                &snapshot,
                StartupProcessObservationKind::AlreadyExited
            ),
            StartupProcessObservationKind::IntentionalExit
        );
        assert_eq!(
            preserve_intentional_exit_observation(
                &snapshot,
                StartupProcessObservationKind::IdentityUnverified
            ),
            StartupProcessObservationKind::IdentityUnverified
        );
    }

    #[test]
    fn snapshot_pid_updates_require_the_current_execution_identity() {
        let current = snapshot_with_identity(Some(RunningProcessIdentity {
            pid: 42,
            execution_id: "execution-1".to_string(),
            executable_path: Some("/bin/worker".to_string()),
            process_start_id: Some("start-1".to_string()),
        }));
        assert!(validate_running_process_update(&current, "execution-1", None).is_ok());
        assert!(validate_running_process_update(&current, "execution-2", None).is_err());
        assert!(validate_running_process_update(&current, "execution-1", Some(43)).is_err());
    }

    #[test]
    fn snapshot_pid_updates_preserve_unknown_pid_as_a_human_boundary() {
        let unknown = snapshot_with_identity(None);
        assert!(validate_running_process_update(&unknown, "execution-1", None).is_err());
        assert!(validate_running_process_update(&unknown, "execution-1", Some(43)).is_err());
    }
}

fn save_snapshot_record(snapshot: &AppSnapshot) -> Result<(), String> {
    let path = snapshot_data_path(&snapshot.project_id)?;
    let json =
        serde_json::to_string_pretty(snapshot).map_err(|e| format!("序列化快照失败: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("写入快照文件失败: {}", e))
}

// ============================================================
// 辅助函数
// ============================================================

/// 返回当前 UTC 时间的 ISO 8601 字符串，用于快照时间戳
fn chrono_now() -> String {
    // 不引入 chrono crate，使用标准库构造简易时间戳
    // std 无直接 ISO 格式化 → 用 UNIX 时间戳代替
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_secs())
}
