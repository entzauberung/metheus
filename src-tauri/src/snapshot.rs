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

/// 应用完整快照，持久化到 ~/.metheus/{project_id}_snapshot.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppSnapshot {
    pub ui: UISnapshot,
    pub project_id: String,
    pub snapshot_version: u32,
    /// 孤儿进程保护：当前正在运行的子进程 PID（无则为 None）
    pub running_pid: Option<u32>,
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
pub(crate) fn update_snapshot_pid(project_id: &str, running_pid: Option<u32>) -> Result<(), String> {
    // 读取现有快照（如果存在）
    let ui = match load_snapshot(project_id)? {
        Some(existing) => existing.ui,
        None => {
            // 无现有快照 → 不创建新快照（前端未初始化过 UI 状态）
            return Ok(());
        }
    };
    save_snapshot(project_id, &ui, running_pid)
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
pub(crate) async fn restore_snapshot(
    project_id: String,
) -> Result<Option<AppSnapshot>, String> {
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

/// 应用启动时调用：扫描所有快照文件，终止孤儿进程
///
/// 遍历 `~/.metheus/*_snapshot.json`，检测每个快照中的 `running_pid`。
/// 如果 PID 存活 → 判定为上次异常退出遗留的孤儿进程 → kill -9 终止 → 清除快照中的 PID。
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

        let snapshot: AppSnapshot = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Some(pid) = snapshot.running_pid {
            if is_pid_alive(pid) {
                eprintln!(
                    "[snapshot] 发现孤儿进程 PID={} (项目={})，正在终止...",
                    pid, snapshot.project_id
                );
                if kill_pid(pid) {
                    eprintln!("[snapshot] 孤儿进程 PID={} 已终止", pid);
                } else {
                    eprintln!(
                        "[snapshot] 警告: 无法终止孤儿进程 PID={}（权限不足或进程已退出）",
                        pid
                    );
                }
                // 清除快照中的 running_pid
                if let Err(e) = update_snapshot_pid(&snapshot.project_id, None) {
                    eprintln!("[snapshot] 清除快照 PID 失败: {}", e);
                }
            } else {
                // PID 不存活 → 进程已自然结束，清理快照中的残留 PID
                if let Err(e) = update_snapshot_pid(&snapshot.project_id, None) {
                    eprintln!("[snapshot] 清除残留 PID 失败: {}", e);
                }
            }
        }
    }
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
