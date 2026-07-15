// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useCallback, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import "./App.css";
import { Project, ViewMode, DiscussionReason, PipelineState, TestLog, ChatMessage, Milestone, RollbackImpact, WorkflowStep, ExecutionWorkspaceStatus, AutopilotNextStep } from "./types";
import { ProjectEntry } from "./ProjectEntry";
import { ExistingBaselinePanel } from "./ExistingBaselinePanel";
import { PreflightPanel } from "./PreflightPanel";
import { PlanApprovalPanel } from "./PlanApprovalPanel";
import { DecisionStepHeader } from "./components/DecisionStepHeader";
import { FeedbackBanner } from "./components/FeedbackBanner";
import { ActionButton } from "./components/ActionButton";
import { Modal } from "./components/Modal";
import { ConsoleStepShell } from "./components/ConsoleStepShell";
import { WorkflowActionBar } from "./components/WorkflowActionBar";
import { Bot, Check, GitBranch, ListTodo, Pause, Play, RotateCcw, Search, Square, WandSparkles, X } from "lucide-react";
import ExecutionTree from "./ExecutionTree";
import ChatRoom from "./ChatRoom";
import TaskConsole from "./TaskConsole";
import { ConsoleWorkflowPanel } from "./ConsoleWorkflowPanel";
import { PauseDecisionPanel } from "./PauseDecisionPanel";
import { RollbackImpactDialog } from "./RollbackImpactDialog";
import { MilestoneReviewPanel } from "./MilestoneReviewPanel";
import FileTree from "./FileTree";
import FloatingChatBalloon from "./FloatingChatBalloon";

const DEFAULT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 800;

const WORKFLOW_STEPS = new Set<WorkflowStep>([
  "WaitingEntry", "ExistingAnalysis", "BaselineApproval", "Discussion", "ThreeChecks",
  "PlanApproval", "MilestoneGeneration", "MilestoneCheck", "MilestoneApproval",
  "MilestoneSelection", "MidStageGeneration", "MidStageCheck", "MidStageApproval",
  "MidStageSelection", "PlanGeneration", "PlanCheck", "PlanApproving", "Execution",
  "PauseDecision", "RollbackPreview", "BranchDiscussion", "FuturePlanApproval",
  "MilestoneReview", "Completed",
]);

// ============================================================
// App.tsx — 「弥」的前端总指挥
//
// 职责：
// 1. 管理所有核心状态（项目数据、模式切换、执行状态）
// 2. 协调“讨论模式”和“执行模式”的动态切换（带动画过渡）
// 3. 与 Rust 后端通信（通过 Tauri invoke）
// 4. 轮询执行状态，实时更新界面
// 5. 提供测试面板，方便开发阶段验证后端命令
//
// 子组件分工：
// - ExecutionTree → 任务树展示与交互
// - ChatRoom → AI 角色对话
// - TaskConsole → 执行进度与日志
// - FileTree → 项目文件树
// - FloatingChatBalloon → 执行模式下的快捷聊天入口
// ============================================================

function App() {
  // Phase 5: startupPhase prevents flashing to entry page during recovery
  type StartupPhase = "recovering" | "ready";
  const [startupPhase, setStartupPhase] = useState<StartupPhase>("recovering");
  const [project, setProject] = useState<Project | null>(null);
  const projectRef = useRef<Project | null>(null);
  const [projectPath, setProjectPath] = useState<string>("");

  // === Phase B：视图模式控制 ===
  const [viewMode, setViewMode] = useState<ViewMode>({ phase: 'discussion', reason: 'idle' });

  // Phase D: 动画控制（保留用于视觉过渡，不决定业务阶段）
  const animationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 测试日志去重：记录已处理过的子任务 ID
  const processedSubtaskIdsRef = useRef<Set<string>>(new Set());

  // 大阶段完成总结去重：记录已发送过总结消息的大阶段 ID
  const completedMilestonesRef = useRef<Set<string>>(new Set());

  // === 侧边栏拖拽缩放 ===
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [isDragging, setIsDragging] = useState(false);
  const dragStartX = useRef(0);
  const dragStartWidth = useRef(0);

  const enterDiscussionMode = useCallback((reason: DiscussionReason) => {
    // 仅保留视觉过渡职责，不再决定业务阶段
    if (viewMode.phase === 'discussion' && viewMode.reason === reason) return;
    if (animationTimerRef.current) { clearTimeout(animationTimerRef.current); animationTimerRef.current = null; }
    setViewMode({ phase: 'discussion', reason });
    animationTimerRef.current = setTimeout(() => {
      animationTimerRef.current = null;
    }, 250);
  }, [viewMode.phase, viewMode.reason]);

  // 后端持久化后的完整 Project 是唯一事实；异步旧结果不得覆盖较新修订。
  const handleChatComplete = useCallback((updatedProject: Project) => {
    const current = projectRef.current;
    if (!updatedProject.workflow_state || !WORKFLOW_STEPS.has(updatedProject.workflow_state.current_step)) {
      console.error("拒绝应用缺少合法工作流状态的 Project", updatedProject);
      return false;
    }
    if (current) {
      if (updatedProject.name !== current.name) {
        console.warn("拒绝应用其他项目的异步结果", updatedProject.name);
        return false;
      }
      if (updatedProject.project_path !== current.project_path) {
        console.warn("拒绝应用项目路径不一致的异步结果", updatedProject.project_path);
        return false;
      }
      if (updatedProject.workflow_state.data_revision < current.workflow_state.data_revision) {
        console.warn("拒绝应用较旧的 Project 修订", updatedProject.workflow_state.data_revision);
        return false;
      }

      // Phase 5: Protect execution session from stale overwrites.
      // Don't let an older/inactive session overwrite a newer active one.
      const curSess = current.execution_session;
      const newSess = updatedProject.execution_session;
      if (curSess?.active && (!newSess || !newSess.active)) {
        console.warn("拒绝应用：当前有活跃执行会话，但新数据无活跃会话",
          { curSubtaskId: curSess.subtask_id, curStatus: curSess.status });
        return false;
      }

      // Phase 5: Protect autopilot state from racing backward.
      // If currently Running and incoming says Paused/ErrorStopped, accept it (backend authority).
      // If currently ErrorStopped/Paused and incoming says Running, verify the data_revision
      // actually advanced (the backend intentionally restarted autopilot).
      const curAp = current.workflow_state.autopilot_state;
      const newAp = updatedProject.workflow_state.autopilot_state;
      if (curAp && newAp && curAp.run_status === "Running" && newAp.run_status !== "Running"
          && updatedProject.workflow_state.data_revision === current.workflow_state.data_revision) {
        console.warn("暂停应用 autopilot 状态变更：data_revision 未递增",
          { curStatus: curAp.run_status, newStatus: newAp.run_status });
        // Accept it anyway — the backend is authoritative. Just log for debugging.
      }
    }

    projectRef.current = updatedProject;
    setProject(() => updatedProject);
    setProjectPath(updatedProject.project_path);
    return true;
  }, []);

  // handleAddMessage: 添加系统消息等不需要后端持久化的非对话消息
  // 系统消息不递增 discussion_revision（只有用户需求消息才递增，且由后端 chat_with_role 控制）
  const handleAddMessage = useCallback((msg: any) => {
    setProject((prev) => {
      if (!prev) return null;
      if (prev.discussion_threads.length === 0) return prev;
      const updated = { ...prev };
      updated.discussion_threads = prev.discussion_threads.map((thread, i) => {
        if (i === 0) {
          return { ...thread, messages: [...thread.messages, msg] };
        }
        return thread;
      });
      return updated;
    });
  }, []);

  // === 侧边栏拖拽事件处理 ===
  const handleResizeMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    dragStartX.current = e.clientX;
    dragStartWidth.current = sidebarWidth;
  };

  const handleResizeMouseMove = useCallback((e: MouseEvent) => {
    const newWidth = dragStartWidth.current + (e.clientX - dragStartX.current);
    setSidebarWidth(Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, newWidth)));
    // 安全网：鼠标释放但 mouseup 事件丢失（如鼠标移出窗口）
    if (e.buttons === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleResizeMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  useEffect(() => {
    if (!isDragging) return;
    document.addEventListener('mousemove', handleResizeMouseMove);
    document.addEventListener('mouseup', handleResizeMouseUp);
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'col-resize';
    return () => {
      document.removeEventListener('mousemove', handleResizeMouseMove);
      document.removeEventListener('mouseup', handleResizeMouseUp);
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
    };
  }, [isDragging, handleResizeMouseMove, handleResizeMouseUp]);

  const [isExecuting, setIsExecuting] = useState(false);
  const [feedbackMsg, setFeedbackMsg] = useState<{ type: "error" | "success" | "warning" | "info"; message: string } | null>(null);
  const [executionStatus, setExecutionStatus] = useState<PipelineState | null>(null);
  const [_autoAdvance, _setAutoAdvance] = useState(false);
  // === 决策层统一提交锁（同一时间只能执行一个关键动作） ===
  const [decisionAction, setDecisionAction] = useState<string | null>(null);
  const isDecisionSubmitting = decisionAction !== null;

  const [testLogs, setTestLogs] = useState<TestLog[]>([]);
  // === 执行工作区状态（供 V1ExecutionPanel 和 TaskConsole 共用） ===
  const [workspaceStatus, setWorkspaceStatus] = useState<ExecutionWorkspaceStatus | null>(null);

  useEffect(() => {
    projectRef.current = project;
  }, [project]);
  // V1: 回退后手动触发生成（不再自动触发）

  // === 阶段一关键修复：启动时从磁盘 Project 恢复执行状态 ===
  // 解决刷新后执行状态丢失的问题。
  useEffect(() => {
    if (!project) return;
    const session = project.execution_session;
    if (!session || !session.active) {
      // No active session — clear any stale execution state
      setExecutionStatus(null);
      setIsExecuting(false);
      // Phase 2: Signal recovery done so autopilot can start
      startupRecoveryDoneRef.current = true;
      return;
    }

    const markRecoveryDone = () => {
      startupRecoveryDoneRef.current = true;
      setRecoveryEpoch((e) => e + 1);
    };

    // Recover based on disk session status
    if (session.status === "executing") {
      // Was executing when page was closed/refreshed.
      // Check if backend memory still has the pipeline running.
      invokeWithTimeout<PipelineState | null>("get_execution_status")
        .then((memStatus) => {
          if (memStatus && memStatus.status === "Running") {
            // Backend still running — restore and poll
            setExecutionStatus(memStatus);
            setIsExecuting(true);
          } else if (memStatus && memStatus.awaiting_confirmation) {
            // Already finished while we were away
            setExecutionStatus(memStatus);
            setIsExecuting(false);
            // Reload project to get latest disk state
            invokeWithTimeout<Project>("get_project", { projectName: project.name })
              .then((p) => handleChatComplete(p))
              .catch(() => {});
          } else {
            // Memory state lost but disk says executing.
            // Show the executing state from disk data so user isn't confused.
            setExecutionStatus({
              mid_stage_id: session.mid_stage_id,
              status: "Running",
              current_subtask_index: session.subtask_index,
              total_subtasks: session.total_subtasks,
              subtask_statuses: [],
              current_log: `▶ 执行中 (${session.subtask_index + 1}/${session.total_subtasks})：${session.subtask_title}`,
              last_error: undefined,
              child_pid: undefined,
              project_name: project.name,
              milestone_id: session.milestone_id,
              plan_revision: session.plan_revision,
              current_subtask_id: session.subtask_id,
              awaiting_confirmation: false,
              log_history: [],
            });
            setIsExecuting(true);
            // Poll in case backend recovers
          }
          markRecoveryDone();
        })
        .catch(() => {
          // Can't reach backend — show disk-based state
          setExecutionStatus({
            mid_stage_id: session.mid_stage_id,
            status: "Running",
            current_subtask_index: session.subtask_index,
            total_subtasks: session.total_subtasks,
            subtask_statuses: [],
            current_log: "⏳ 执行状态恢复中…请稍候或刷新页面。",
            last_error: undefined,
            child_pid: undefined,
            project_name: project.name,
            milestone_id: session.milestone_id,
            plan_revision: session.plan_revision,
            current_subtask_id: session.subtask_id,
            awaiting_confirmation: false,
            log_history: [],
          });
          setIsExecuting(true);
          markRecoveryDone();
        });
    } else if (session.status === "awaiting_confirmation") {
      // Was waiting for confirmation — restore that state directly
      setExecutionStatus({
        mid_stage_id: session.mid_stage_id,
        status: "Paused",
        current_subtask_index: session.subtask_index,
        total_subtasks: session.total_subtasks,
        subtask_statuses: [],
        current_log: `⏳ 待确认 (${session.subtask_index + 1}/${session.total_subtasks})：${session.subtask_title}`,
        last_error: undefined,
        child_pid: undefined,
        project_name: project.name,
        milestone_id: session.milestone_id,
        plan_revision: session.plan_revision,
        current_subtask_id: session.subtask_id,
        awaiting_confirmation: true,
        log_history: [],
      });
      setIsExecuting(false);
      markRecoveryDone();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.name, project?.execution_session?.active, project?.execution_session?.status, project?.execution_session?.subtask_id]);

  // Fetch workspace status when entering Execution
  useEffect(() => {
    if (!project || project.workflow_state.current_step !== "Execution") return;
    invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", { projectName: project.name })
      .then(setWorkspaceStatus)
      .catch(() => setWorkspaceStatus(null));
  }, [project?.name, project?.workflow_state.current_step, project?.workflow_state.data_revision]);

  // === 自动驾驶驱动循环 — 使用 autopilot_next_step 逐步推进 ===
  // 覆盖范围：大阶段内部所有步骤（中阶段规划、执行计划、执行、确认）
  // 停止条件：大阶段边界（MilestoneReview）、出错、暂停
  const autopilotLoopRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const autopilotActiveRef = useRef(false);
  const autopilotDrivingRef = useRef(false); // 防重入：true 表示当前正在执行一个循环步
  const startupRecoveryDoneRef = useRef(false); // Phase 2: 启动恢复完成前不允许 autopilot 启动
  const lastAutopilotErrorRef = useRef<{ message: string; count: number } | null>(null); // Phase 5: 相同错误连续检测
  const [recoveryEpoch, setRecoveryEpoch] = useState(0); // Phase 2: 递增以触发 autopilot 重启
  const [autopilotProgress, setAutopilotProgress] = useState<AutopilotNextStep | null>(null);

  // Phase 7 (2026-07-15): Autopilot sanity check on startup recovery.
  // If autopilot is Running but Execution is dead (no active session), reset to Paused.
  // If autopilot target milestone is completed, redirect to next incomplete.
  useEffect(() => {
    if (!project || !startupRecoveryDoneRef.current) return;
    const ap = project.workflow_state.autopilot_state;
    if (!ap || ap.run_status !== "Running") return;

    const step = project.workflow_state.current_step;
    const execSession = project.execution_session;

    // Case 1: Autopilot says Running but we're at Execution with no active session
    if (step === "Execution" && (!execSession || !execSession.active)) {
      console.warn("[recovery] Autopilot Running but execution is dead — resetting to Paused");
      invokeWithTimeout<Project>("autopilot_pause", { projectName: project.name })
        .then((p) => handleChatComplete(p))
        .catch(() => {});
      return;
    }

    // Case 2: Autopilot target milestone is completed — redirect via toggle
    const targetMsId = project.workflow_state.autopilot_target_milestone_id;
    if (targetMsId) {
      const targetMs = project.milestones.find((m) => m.id === targetMsId);
      if (targetMs && targetMs.status === "Completed") {
        console.warn("[recovery] Autopilot target milestone completed — re-activating");
        invokeWithTimeout<Project>("toggle_autopilot", { projectName: project.name, active: false })
          .then(() => invokeWithTimeout<Project>("toggle_autopilot", { projectName: project.name, active: true }))
          .then((p) => handleChatComplete(p))
          .catch(() => {});
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.name, recoveryEpoch]);

  const runAutopilotCycle = useCallback(async (proj: Project) => {
    // 防重入：已有循环步在执行中，跳过本次调度
    if (autopilotDrivingRef.current) return;
    if (!proj.workflow_state.autopilot_active) return;
    if (proj.workflow_state.top_level_phase !== "Console") return;

    const autopilotState = proj.workflow_state.autopilot_state;
    // Check if autopilot is paused, at boundary, or errored
    if (autopilotState) {
      if (autopilotState.run_status === "Paused") return;
      if (autopilotState.run_status === "WaitingMilestoneReview") return;
      if (autopilotState.run_status === "ErrorStopped") return;
    }

    autopilotDrivingRef.current = true;
    try {
      const next = await invokeWithTimeout<AutopilotNextStep>("autopilot_next_step", { projectName: proj.name });
      setAutopilotProgress(next);

      // Stop conditions
      if (next.at_milestone_boundary) {
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
        return;
      }

      if (next.is_error) {
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
        return;
      }

      if (!next.command) {
        // Phase 2: Distinguish "legal no-command" (autopilot paused/stopped by backend)
        // from "abnormal no-command" (backend returned empty but should be running).
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        const apState = latest.workflow_state.autopilot_state;
        const isLegalStop = !apState
          || apState.run_status === "Paused"
          || apState.run_status === "WaitingMilestoneReview"
          || apState.run_status === "ErrorStopped";

        if (!isLegalStop && latest.workflow_state.autopilot_active) {
          // Abnormal: autopilot is active and running but backend gave no command
          console.warn("[autopilot] Abnormal no-command: autopilot reported Running but returned empty command");
          try {
            await invokeWithTimeout<Project>("autopilot_mark_error", {
              projectName: proj.name,
              message: "自动驾驶异常：后端未返回下一步命令但状态为运行中",
            });
          } catch (_) {}
        }
        handleChatComplete(latest);
        return;
      }

      // Phase 2: Action completion assertion — after executing a command,
      // verify the project state actually changed (data_revision should have advanced
      // or step should have changed).
      const beforeRevision = proj.workflow_state.data_revision;
      const beforeStep = proj.workflow_state.current_step;

      const updated = await invokeWithTimeout<Project>(next.command, {
        ...next.args,
        projectName: proj.name,
      });

      const applied = handleChatComplete(updated);
      if (!applied) {
        // handleChatComplete rejected the result — stop the loop
        console.warn("[autopilot] handleChatComplete rejected result, stopping loop");
        return;
      }

      // Action completion assertion: if the command didn't change data_revision
      // or step, it may have been silently skipped by the backend
      const afterStep = updated.workflow_state.current_step;
      const afterRevision = updated.workflow_state.data_revision;
      if (beforeRevision === afterRevision && beforeStep === afterStep) {
        console.warn("[autopilot] Command executed but no state change detected",
          { command: next.command, beforeStep, afterStep, beforeRevision, afterRevision });
        // Not necessarily an error — some commands like select_milestone may
        // not change revision if already selected. But log for debugging.
      }

      // Schedule next cycle (1 second delay to let UI update)
      if (autopilotActiveRef.current && updated.workflow_state.autopilot_active) {
        autopilotLoopRef.current = setTimeout(() => {
          runAutopilotCycle(updated);
        }, 1000);
      }
    } catch (error) {
      console.warn("[autopilot] Cycle error:", error);
      const errMsg = error instanceof Error ? error.message : String(error);
      const errStep = proj.workflow_state.current_step;
      const errMsId = proj.current_milestone_id || "(none)";
      const errMidId = proj.current_mid_stage_id || "(none)";

      // Phase 7 (2026-07-15): Error categorization
      const isRecoverable = errMsg.includes("timeout")
        || errMsg.includes("fetch failed")
        || errMsg.includes("network")
        || errMsg.includes("allowed_file_paths")
        || errMsg.includes("acceptance_criteria")
        || errMsg.includes("execution_prompt");
      const isFatal = errMsg.includes("JSON") && errMsg.includes("parse")
        || errMsg.includes("corrupt")
        || errMsg.includes("disk");
      const category = isFatal ? "[致命]" : isRecoverable ? "[可自动补救]" : "[需人工介入]";

      // Phase 5: same-error guard — keyed on step+message for precise dedup
      const errKey = `${errStep}:${errMsg}`;
      const prevErr = lastAutopilotErrorRef.current;
      if (prevErr && prevErr.message === errKey) {
        prevErr.count += 1;
      } else {
        lastAutopilotErrorRef.current = { message: errKey, count: 1 };
      }
      const consecutiveCount = lastAutopilotErrorRef.current?.count ?? 1;
      const finalMsg = consecutiveCount >= 3
        ? `${category} [连续失败 ${consecutiveCount} 次] step=${errStep} ms=${errMsId} mid=${errMidId} | ${errMsg}`
        : `${category} step=${errStep} ms=${errMsId} mid=${errMidId} | ${errMsg}`;

      try {
        await invokeWithTimeout<Project>("autopilot_mark_error", {
          projectName: proj.name,
          message: finalMsg,
        });
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
      } catch (_) {
        // Last resort: sync project state
        try {
          const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
          handleChatComplete(latest);
        } catch (__) {}
      }
    } finally {
      autopilotDrivingRef.current = false;
    }
  }, []);

  // Start/stop autopilot loop when autopilot_active changes
  useEffect(() => {
    if (!project) return;
    const active = project.workflow_state.autopilot_active === true;
    autopilotActiveRef.current = active;

    // Clear any pending loop
    if (autopilotLoopRef.current) {
      clearTimeout(autopilotLoopRef.current);
      autopilotLoopRef.current = null;
    }

    if (!active) return;
    if (project.workflow_state.top_level_phase !== "Console") return;

    // Check if should start
    const apState = project.workflow_state.autopilot_state;
    if (apState) {
      if (apState.run_status === "Paused") return;
      if (apState.run_status === "WaitingMilestoneReview") return;
      if (apState.run_status === "ErrorStopped") return;
    }

    // Phase 2: Defer autopilot start if execution recovery is still pending.
    // The execution recovery effect (above) will restart autopilot when done.
    const execSession = project.execution_session;
    if (execSession?.active && !startupRecoveryDoneRef.current) {
      console.log("[autopilot] Deferring start: execution recovery pending",
        { sessionStatus: execSession.status });
      return;
    }

    // Start the drive loop
    autopilotLoopRef.current = setTimeout(() => {
      runAutopilotCycle(project);
    }, 500);
  }, [project?.workflow_state?.autopilot_active, project?.workflow_state?.autopilot_state?.run_status, project?.workflow_state?.top_level_phase, recoveryEpoch]);

  // === 快照：保存 UI 状态到后端，用于刷新恢复和孤儿进程保护 ===
  const takeSnapshot = () => {
    if (!project) return;
    const snapshotUi = {
      view_phase: viewMode.phase,
      sidebar_width: sidebarWidth,
      active_tab: null,
      saved_at: new Date().toISOString(),
    };
    invokeWithTimeout("save_snapshot_event", {
      projectId: project.name,
      uiJson: JSON.stringify(snapshotUi),
    }).catch(err => console.warn("快照保存失败:", err));
  };

  // 自动快照：关键 UI 状态变更后持久化（React 18 自动批处理，一次用户操作只触发一次）
  useEffect(() => {
    if (!project) return;
    takeSnapshot();
    // takeSnapshot 通过闭包读取最新 state，不放入 deps 以避免循环
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, viewMode.phase, sidebarWidth]);

  // === 阶段一关键修复：执行状态轮询 ===
  // 覆盖 V1 恢复场景（刷新时后端仍在执行）和旧流水线场景。
  // 轮询条件：isExecuting 为真（由恢复 effect 或旧流水线设置）。
  // 轮询内部额外检查当前工作流步骤，非 Execution 时停止。
  useEffect(() => {
    if (!isExecuting) return;

    const interval = setInterval(async () => {
      try {
        // 若工作流步骤已离开 Execution，停止轮询
        const current = projectRef.current;
        if (current && current.workflow_state.current_step !== "Execution") {
          setIsExecuting(false);
          return;
        }

        const status = await invokeWithTimeout<PipelineState | null>("get_execution_status");
        if (!status) {
          // No memory state — if we're polling from recovery, the backend may have exited.
          // Keep polling for a bit; the recovery effect handles the fallback.
          return;
        }
        setExecutionStatus(status);

        // 执行中定期刷新快照，确保 running_pid（孤儿进程保护用）保持最新
        takeSnapshot();

        // 从执行状态中提取测试日志（去重）
        if (status.subtask_statuses) {
          const newLogs: TestLog[] = [];
          for (const item of status.subtask_statuses) {
            if (processedSubtaskIdsRef.current.has(item.subtask_id)) continue;
            if (item.test_result && (item.status === "passed" || item.status === "retrying")) {
              processedSubtaskIdsRef.current.add(item.subtask_id);
              const tr = item.test_result;
              let reason: string;
              if (tr.passed && (!tr.issues || tr.issues.length === 0)) {
                reason = "通过测试";
              } else if (!tr.passed) {
                reason = "不通过: " + (tr.suggestion || "未提供建议");
              } else {
                reason = (tr.issues || []).join("\n");
              }
              newLogs.push({
                subtask_title: item.title,
                status: item.status === "retrying" ? "retried" : "passed",
                reason,
                full_report: tr.suggestion || undefined,
              });
            }
          }
          if (newLogs.length > 0) {
            setTestLogs((prev) => [...prev, ...newLogs]);
          }
        }

        // V1: awaiting_confirmation — stop polling, let V1ExecutionPanel handle it
        if (status.awaiting_confirmation) {
          setIsExecuting(false);
          // Refresh project to get latest disk state
          if (project) {
            invokeWithTimeout<Project>("get_project", { projectName: project.name })
              .then((updatedProject) => {
                handleChatComplete(updatedProject);
              })
              .catch((err) => {
                console.error("刷新项目数据失败:", err);
              });
          }
          clearInterval(interval);
          return;
        }

        if (status.status === "Paused") {
          // Old pipeline pause — switch to discussion
          setIsExecuting(false);
          handleAddMessage({
            id: `sys-${Date.now()}`,
            role: 'system',
            content: '⏸️ 执行已暂停。讨论修改方向后，点击恢复执行继续。',
            timestamp: Date.now(),
          });
          enterDiscussionMode('paused');
          clearInterval(interval);
        } else if (status.status === "Completed") {
          setIsExecuting(false);
          clearInterval(interval);
          invokeWithTimeout<Project>("get_project", { projectName: project?.name ?? "" })
            .then((updatedProject) => {
              handleChatComplete(updatedProject);
            })
            .catch((err) => {
              console.error("刷新项目数据失败:", err);
            });
        } else if (status.status === "Failed") {
          setIsExecuting(false);
          clearInterval(interval);
        }
      } catch (e) {
        console.error("轮询状态失败:", e);
      }
    }, 2000);

    return () => clearInterval(interval);
  }, [isExecuting, project, enterDiscussionMode, handleAddMessage, handleChatComplete, takeSnapshot]);

  // 大阶段完成检测：当所有中阶段执行完成后，自动插入总结消息
  useEffect(() => {
    if (!project || project.mode === "Quick") return;
    for (const ms of project.milestones) {
      if (isMilestoneFullyCompleted(ms) && !completedMilestonesRef.current.has(ms.id)) {
        // 收集统计数据
        const midStages = ms.mid_stages || [];
        const totalCount = midStages.length;
        const completedCount = midStages.filter(m => m.status === "Completed").length;
        const failedCount = midStages.filter(m => m.status === "Rejected").length;
        // 收集 Git tag
        const tags: string[] = [];
        for (const mid of midStages) {
          if (mid.git_tag) tags.push(mid.git_tag);
        }
        const tagsLine = tags.length > 0 ? tags.join("、") : "无";
        // 统计子任务测试通过率
        let totalSubtasks = 0;
        let passedSubtasks = 0;
        for (const mid of midStages) {
          for (const st of (mid.subtasks || [])) {
            totalSubtasks++;
            if (st.test_result?.passed) passedSubtasks++;
          }
        }
        const passRate = totalSubtasks > 0 ? `${Math.round(passedSubtasks / totalSubtasks * 100)}%` : "N/A";

        const markdown = `### 📋 大阶段「${ms.title}」执行完成

| 项目 | 数据 |
|------|------|
| 中阶段总数 | ${totalCount} |
| 已完成 | ${completedCount} |
| 失败 | ${failedCount} |
| 子任务测试通过率 | ${passRate} |
| Git 标签 | ${tagsLine} |

所有中阶段已执行完成，请审阅后决定下一步。`;

        const summaryMsg: ChatMessage = {
          id: crypto.randomUUID(),
          role: "assistant",
          content: markdown,
          timestamp: Date.now(),
          msg_type: "milestone_summary",
          milestone_id: ms.id,
        };
        handleAddMessage(summaryMsg);
        completedMilestonesRef.current = new Set([...completedMilestonesRef.current, ms.id]);

        // 任务 2.5：调用后端 AI 命令生成自然语言总结（第二层消息）
        invokeWithTimeout<string>('summarize_milestone', {
          projectName: project.name,
          milestone_id: ms.id,
        })
          .then((aiSummary) => {
            const aiMsg: ChatMessage = {
              id: crypto.randomUUID(),
              role: 'assistant',
              content: aiSummary,
              timestamp: Date.now(),
              msg_type: 'milestone_summary',
              milestone_id: ms.id,
            };
            handleAddMessage(aiMsg);
          })
          .catch((err) => {
            console.error('AI 大阶段总结生成失败（第一层统计表格仍可用）:', err);
          });
      }
    }
  }, [project, handleAddMessage]);

  // 启动恢复逻辑：从存储的项目名称恢复，没有则进入 Before 页面
  useEffect(() => {
    const storedName = localStorage.getItem("metheus_last_project");
    if (!storedName) {
      // 没有存储的项目，直接进入就绪状态（停留在 Before 页面）
      setStartupPhase("ready");
      return;
    }

    invokeWithTimeout<Project>("get_project", { projectName: storedName })
      .then((project) => {
        // 检查项目是否有效且处于正确的阶段
        if (!project || !project.name) {
          setProject(null);
          setStartupPhase("ready");
          return;
        }

        setProject(project);
        // 检查是否需要迁移旧项目（workflow_state 仍为 WaitingEntry/Before）
        const needsMigration = project.workflow_state.current_step === "WaitingEntry"
          && project.workflow_state.top_level_phase === "Before";
        if (needsMigration) {
          invokeWithTimeout<Project>("migrate_project_workflow", {
            projectName: project.name,
          }).then((migrated) => {
            handleChatComplete(migrated);
          }).catch((err) => {
            console.error("迁移旧项目工作流失败:", err);
          });
        }
        // 重建已发送总结的大阶段 Set
        if (project?.discussion_threads?.[0]?.messages) {
          const summaryIds = new Set<string>();
          for (const msg of project.discussion_threads[0].messages) {
            if (msg.msg_type === "milestone_summary" && msg.milestone_id) {
              summaryIds.add(msg.milestone_id);
            }
          }
          completedMilestonesRef.current = summaryIds;
        }
        if (project && project.project_path) {
          setProjectPath(project.project_path);
        }
        return invokeWithTimeout<any>("restore_snapshot", { projectId: project.name });
      })
      .then((snapshot) => {
        if (snapshot && snapshot.ui) {
          const ui = snapshot.ui;
          if (ui.view_phase === 'execution') {
            setViewMode({ phase: 'execution', reason: 'active' });
          }
          if (typeof ui.sidebar_width === "number") {
            setSidebarWidth(Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, ui.sidebar_width)));
          }
        }
        // 恢复后进入默认讨论模式
        if (project) {
          enterDiscussionMode('idle');
        }
        // Phase 5: Mark startup recovery as complete
        setStartupPhase("ready");
      })
      .catch((err) => {
        console.error("获取项目失败:", err);
        setProject(null);
        localStorage.removeItem("metheus_last_project");
        setStartupPhase("ready");
      });
  }, []);

  // 项目创建后的处理：使用后端返回的完整 Project（已含正确的 workflow_state）
  const handleProjectCreated = useCallback((project: Project) => {
    projectRef.current = project;
    setProject(project);
    setProjectPath(project.project_path);
    localStorage.setItem("metheus_last_project", project.name);
    // 不额外调用 enterDiscussionMode — workflow_state 已经由后端设置为 Discussion
  }, []);

  const handleSelectMilestone = async (id: string) => {
    if (!project || project.current_milestone_id === id) return;
    const updated = await invokeWithTimeout<Project>("select_milestone", {
      projectName: project.name,
      milestoneId: id,
    });
    handleChatComplete(updated);
  };

  const handleSelectMidStage = async (id: string) => {
    if (!project || project.current_mid_stage_id === id) return;
    const updated = await invokeWithTimeout<Project>("select_mid_stage", {
      projectName: project.name,
      midStageId: id,
    });
    handleChatComplete(updated);
  };
  // 生成版本方案（V1: 后端校验三项检查 → 返回完整 Project → PlanApproval 步骤）
  const handleGeneratePlan = async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("generate_plan");
    try {
      const updatedProject = await invokeWithTimeout<Project>("generate_version_plan", {
        projectName: project.name,
        expectedDiscussionRevision: project.discussion_revision,
        expectedDataRevision: project.workflow_state.data_revision,
      });
      handleChatComplete(updatedProject);
    } catch (err) {
      console.error("生成方案失败", err);
      setFeedbackMsg({ type: "error", message: "生成方案失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  };
  // 批准方案（传入 draft_id 和 generation_revision）
  const handleApproveWithDraft = useCallback(async (draftId: string, generationRevision: number) => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("approve_plan");
    try {
      const updated = await invokeWithTimeout<Project>("approve_version_plan", {
        projectName: project.name,
        draftId: draftId,
        generationRevision: generationRevision,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "项目方案已批准。宪法第一部分已写入项目目录。" });
    } catch (err) {
      console.error("批准失败:", err);
      setFeedbackMsg({ type: "error", message: "批准失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 驳回方案（传入 draft_id 和反馈）
  const handleRejectWithDraft = useCallback(async (draftId: string, feedback: string) => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("reject_plan");
    try {
      const updated = await invokeWithTimeout<Project>("reject_version_plan", {
        projectName: project.name,
        draftId: draftId,
        feedback: feedback,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "方案已驳回，已返回讨论模式。" });
    } catch (err) {
      console.error("驳回失败:", err);
      setFeedbackMsg({ type: "error", message: "驳回失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从 ThreeChecks 或 PlanApproval 返回 Discussion
  const handleReturnToDiscussion = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    const currentStep = project.workflow_state.current_step;
    if (currentStep !== "ThreeChecks" && currentStep !== "PlanApproval") return;
    setDecisionAction("return_to_discussion");
    try {
      const updated = await invokeWithTimeout<Project>("return_to_discussion", {
        projectName: project.name,
        sourceStep: currentStep,
        reason: "用户返回继续讨论",
      });
      handleChatComplete(updated);
    } catch (err) {
      console.error("返回讨论失败:", err);
      setFeedbackMsg({ type: "error", message: "返回讨论失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从 Discussion 恢复方案审批
  const handleResumePlanApproval = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("resume_plan_approval");
    try {
      const updated = await invokeWithTimeout<Project>("resume_plan_approval", {
        projectName: project.name,
      });
      handleChatComplete(updated);
    } catch (err) {
      console.error("恢复方案审批失败:", err);
      setFeedbackMsg({ type: "error", message: "恢复方案审批失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 重新讨论已批准方案
  const handleReDiscussApprovedPlan = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("rediscuss_approved");
    try {
      const updated = await invokeWithTimeout<Project>("restart_discussion_from_approved", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "已返回讨论模式，旧方案已保留在历史记录中。" });
    } catch (err) {
      console.error("重新讨论失败:", err);
      setFeedbackMsg({ type: "error", message: "重新讨论失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从 Discussion 进入三项检查
  const handleStartChecks = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("start_checks");
    try {
      const updated = await invokeWithTimeout<Project>("start_preflight_check", {
        projectName: project.name,
      });
      handleChatComplete(updated);
    } catch (err) {
      console.error("进入检查模式失败:", err);
      setFeedbackMsg({ type: "error", message: "进入检查模式失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从 ThreeChecks 重新开始全部检查
  const handleRestartChecks = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("restart_checks");
    try {
      const updated = await invokeWithTimeout<Project>("restart_checks", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "检查结果已重置，请从第一项重新开始。" });
    } catch (err) {
      console.error("重新开始检查失败:", err);
      setFeedbackMsg({ type: "error", message: "重新开始检查失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从 PlanApproval 进入 Console
  const handleEnterConsole = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("enter_console");
    try {
      const updatedProject = await invokeWithTimeout<Project>("enter_console", {
        projectName: project.name,
      });
      handleChatComplete(updatedProject);
    } catch (err) {
      console.error("进入控制台失败:", err);
      setFeedbackMsg({ type: "error", message: "进入控制台失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 判断一个大阶段的所有中阶段是否都已执行完成
  const isMilestoneFullyCompleted = (milestone: Milestone): boolean => {
    if (!milestone.mid_stages || milestone.mid_stages.length === 0) return false;
    return milestone.mid_stages.every(m => m.status === "Completed");
  };

  // === V1 暂停：In Stop ===
  const handleInStop = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("request_in_stop", { projectName: project.name });
      handleChatComplete(updated);
      setIsExecuting(false);
      setExecutionStatus(null);
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "暂停失败：" + String(err) });
    }
  };

  // === V1 暂停：ED Stop ===
  const handleEdStop = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("request_ed_stop", { projectName: project.name });
      handleChatComplete(updated);
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "ED Stop 请求失败：" + String(err) });
    }
  };

  // === V1 暂停决策：继续/调整/回退 ===
  const handleResolvePause = async (action: string) => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("resolve_pause_decision", {
        projectName: project.name,
        action,
      });
      handleChatComplete(updated);
      if (action === "continue") {
        setFeedbackMsg({ type: "info", message: "已恢复执行模式，可继续执行下一个小阶段。" });
      }
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "决策失败：" + String(err) });
    }
  };

  // === V1 autopilot 暂停恢复 ===
  const handleResumeAutopilot = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("resume_autopilot", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "自动驾驶已恢复，正在继续推进。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复自动驾驶失败：" + String(err) });
    }
  };

  const handleEnterAutopilotDiscussion = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("enter_autopilot_discussion", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "已进入阶段讨论，完成后可恢复自动驾驶。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "进入讨论失败：" + String(err) });
    }
  };

  const handleExitAutopilotKeepState = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("exit_autopilot_keep_state", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "已退出自动驾驶，保留当前阶段进度。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "退出自动驾驶失败：" + String(err) });
    }
  };

  // === V1 回退预览 ===
  const handlePreviewRollback = async (checkpointSubtaskId: string): Promise<RollbackImpact | null> => {
    if (!project) return null;
    try {
      const impact = await invokeWithTimeout<RollbackImpact>("preview_rollback_impact", {
        projectName: project.name,
        checkpointSubtaskId,
      });
      return impact ?? null;
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "预览失败：" + String(err) });
      return null;
    }
  };

  // === V1 确认回退 ===
  const handleConfirmRollback = async (checkpointSubtaskId: string) => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("confirm_rollback", {
        projectName: project.name,
        checkpointSubtaskId,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "回退已完成。请重新生成执行计划。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "回退失败：" + String(err) });
    }
  };

  // V1: 回退后不自动触发生成。pendingRollbackGenerate 已移除。

  // === V1 人工执行：执行当前小阶段 ===
  const handleExecuteCurrentSubtask = async () => {
    if (!project) return;
    try {
      const status = await invokeWithTimeout<PipelineState>("execute_current_subtask", {
        projectName: project.name,
      });
      setExecutionStatus(status);
      if (status.awaiting_confirmation) {
        // Refresh project to get updated subtask status
        const updated = await invokeWithTimeout<Project>("get_project", { projectName: project.name });
        handleChatComplete(updated);
      }
    } catch (err) {
      console.error("执行失败:", err);
      setFeedbackMsg({ type: "error", message: "执行失败：" + String(err) });
    }
  };

  // === V1 人工执行：确认通过 ===
  const handleConfirmSubtask = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("confirm_subtask_result", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setExecutionStatus(null);
      setFeedbackMsg({ type: "success", message: "小阶段已确认通过，Git 标签已创建。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "确认失败：" + String(err) });
    }
  };

  // === V1 人工执行：驳回 ===
  const handleRejectSubtask = async (reason: string) => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("reject_subtask_result", {
        projectName: project.name,
        reason,
      });
      handleChatComplete(updated);
      setExecutionStatus(null);
      setFeedbackMsg({ type: "warning", message: "小阶段已驳回：" + reason });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "驳回失败：" + String(err) });
    }
  };

  // === V1 手动同步项目状态（不依赖浏览器 reload） ===
  const handleSyncProject = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("get_project", { projectName: project.name });
      handleChatComplete(updated);
      // Also refresh pipeline state if available
      const pipelineStatus = await invokeWithTimeout<PipelineState | null>("get_execution_status");
      if (pipelineStatus) {
        setExecutionStatus(pipelineStatus);
        if (pipelineStatus.awaiting_confirmation) {
          setIsExecuting(false);
        }
      }
      setFeedbackMsg({ type: "info", message: "项目状态已同步。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "同步项目状态失败：" + String(err) });
    }
  };

  // === V1 A/B/C 大阶段审阅 ===
  // V1: enter_milestone_review is called via invokeWithTimeout directly when needed

  const handleApproveMilestoneOutcome = async (branch: string) => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("approve_milestone_outcome", {
        projectName: project.name,
        branch,
      });
      handleChatComplete(updated);
      if (branch === "A") {
        setFeedbackMsg({ type: "success", message: "大阶段已批准。" });
      }
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "决策失败：" + String(err) });
    }
  };

  const handleSuggestRollback = async () => {
    if (!project) return;
    try {
      const suggestion = await invokeWithTimeout<string>("suggest_rollback_checkpoint", { projectName: project.name });
      handleAddMessage({ id: `sys-${Date.now()}`, role: "assistant", content: suggestion, timestamp: Date.now() });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "建议生成失败：" + String(err) });
    }
  };

  const handleGenerateFutureMilestones = async () => {
    if (!project) return;
    try {
      const updated = await invokeWithTimeout<Project>("generate_future_milestone_draft", { projectName: project.name });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "未来大阶段草稿已生成，请在 Console 中检查和批准。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "生成失败：" + String(err) });
    }
  };

  // approve_future_milestones is called via ConsoleWorkflowPanel when step === FuturePlanApproval

  /// 查看详细报告（切换到执行模式）
  const handleViewDetailedReport = useCallback(() => {
    if (!project) return;
    setViewMode({ phase: 'execution', reason: 'view_report' });
  }, [project]);

  // 根据工作流步骤返回默认对话角色（不再依赖 project.status）
  const getDefaultRole = (step: string): string => {
    switch (step) {
      case "Discussion":
        return "策略产品经理";
      default:
        return "策略产品经理";
    }
  };

  // Phase 5: Show recovery shell during startup to prevent flash to entry page
  if (startupPhase === "recovering") {
    return (
      <div className="startup-recovery-shell" style={{
        display: "flex", flexDirection: "column", alignItems: "center",
        justifyContent: "center", height: "100vh", background: "#0d1117", color: "#c9d1d9",
      }}>
        <div style={{ fontSize: "24px", marginBottom: "16px" }}>⟳</div>
        <h2 style={{ margin: 0, fontSize: "18px" }}>Metheus 恢复中</h2>
        <p style={{ color: "#8b949e", fontSize: "14px", marginTop: "8px" }}>
          正在加载项目状态与执行上下文...
        </p>
      </div>
    );
  }

  if (!project) {
    return <ProjectEntry onProjectCreated={handleProjectCreated} />;
  }

  const currentThread = project.discussion_threads[0];
  if (!currentThread) {
    return <ProjectEntry onProjectCreated={handleProjectCreated} />;
  }

  // Determine which main panel to show based on workflow_state
  const phase = project.workflow_state.top_level_phase;
  const step = project.workflow_state.current_step;

  // Before phase: show ExistingBaselinePanel for Half Project analysis
  if (phase === "Before" && (step === "ExistingAnalysis" || step === "BaselineApproval")) {
    return <ExistingBaselinePanel
      projectName={project.name}
      projectPath={project.project_path}
      onBaselineApproved={(updated) => {
        handleChatComplete(updated);
        setProjectPath(updated.project_path);
      }}
      onReject={() => {
        localStorage.removeItem("metheus_last_project");
        setProject(null);
      }}
    />;
  }

  return (
    <div className="app-layout">
      {project.milestones.length > 0 && (
        <aside className="sidebar" style={{ width: sidebarWidth + 'px' }}>
          <ExecutionTree
            project={project}
            onSelectMilestone={handleSelectMilestone}
            projectPath={projectPath}
            onSelectMidStage={handleSelectMidStage}
          />
          <div
            className={`resize-handle${isDragging ? ' dragging' : ''}`}
            onMouseDown={handleResizeMouseDown}
            onDoubleClick={() => setSidebarWidth(DEFAULT_SIDEBAR_WIDTH)}
          />
        </aside>
      )}

      <main className="main-content">

        {/* ===== Phase-dependent main content ===== */}
        {(phase === "FirstDiscussion" || phase === "Before") && (
          <div className="transition-wrapper">
            {/* 决策层步骤导航 */}
            {step !== "WaitingEntry" && (
              <DecisionStepHeader currentStep={step} />
            )}

            {/* 决策层错误/成功反馈（替代浏览器 alert） */}
            {feedbackMsg && (
              <FeedbackBanner
                type={feedbackMsg.type}
                message={feedbackMsg.message}
                onRetry={() => setFeedbackMsg(null)}
                style={{ margin: "8px 16px" }}
              />
            )}

            {/* ThreeChecks step: render PreflightPanel */}
            {step === "ThreeChecks" && (
              <PreflightPanel
                projectName={project.name}
                preflightResults={project.preflight_results}
                discussionRevision={project.discussion_revision}
                dataRevision={project.workflow_state.data_revision}
                onProjectUpdated={handleChatComplete}
                onReturnToDiscussion={handleReturnToDiscussion}
                onAllPassed={handleGeneratePlan}
                onRestartChecks={handleRestartChecks}
                isSubmitting={isDecisionSubmitting}
              />
            )}

            {/* PlanApproval step: render PlanApprovalPanel (根据 draft_status 分发视图) */}
            {step === "PlanApproval" && (
              <PlanApprovalPanel
                project={project}
                onReturnToDiscussion={handleReturnToDiscussion}
                onApprove={handleApproveWithDraft}
                onReject={handleRejectWithDraft}
                onEnterConsole={handleEnterConsole}
                onReDiscuss={handleReDiscussApprovedPlan}
                isSubmitting={isDecisionSubmitting}
              />
            )}

            {/* Discussion step: show action buttons + ChatRoom */}
            {step === "Discussion" && (
              <>
                {/* 在讨论中，如果没有方案，提供生成方案和进入检查的入口 */}
                {!project.version_plan && (
                  <div className="discussion-actions" style={{
                    display: "flex", gap: "12px", justifyContent: "center",
                    padding: "12px", marginBottom: "12px", flexWrap: "wrap",
                  }}>
                    {/* 存在待审批草稿时，提供"继续审阅草稿"入口 */}
                    {project.plan_draft?.draft_status === "Pending" && (
                      <button
                        className="btn-start-checks"
                        onClick={handleResumePlanApproval}
                        style={{
                          padding: "8px 20px",
                          fontSize: "14px",
                          background: "#1a7f37",
                          color: "#fff",
                          border: "none",
                          borderRadius: "6px",
                          cursor: "pointer",
                        }}
                      >
                        📝 继续审阅当前草稿
                      </button>
                    )}
                    <button
                      className="btn-start-checks"
                      onClick={handleStartChecks}
                      style={{
                        padding: "8px 20px",
                        fontSize: "14px",
                        background: "#0969da",
                        color: "#fff",
                        border: "none",
                        borderRadius: "6px",
                        cursor: "pointer",
                      }}
                    >
                      📋 进行三项检查
                    </button>
                  </div>
                )}
              </>
            )}

            {/* ChatRoom visible only during Discussion step (not during ThreeChecks or PlanApproval) */}
            {step === "Discussion" && (
              <ChatRoom
                messages={currentThread.messages || []}
                onAddMessage={handleAddMessage}
                projectName={project.name}
                currentRole={getDefaultRole(step)}
                threadId={currentThread.id}
                onViewDetailedReport={handleViewDetailedReport}
                onProjectUpdated={handleChatComplete}
              />
            )}
          </div>
        )}

        {(phase === "Console") && (
          <div className="execution-layout">
            <FileTree projectPath={projectPath} />
            <div className="execution-main">
              {/* Phase 3: Autopilot progress bar — visible when autopilot is actively running */}
              {project.workflow_state.autopilot_active && (
                <AutopilotProgressBar
                  progress={autopilotProgress}
                  project={project}
                  autopilotState={project.workflow_state.autopilot_state}
                  onPause={async () => {
                    try {
                      const updated = await invokeWithTimeout<Project>("autopilot_pause", { projectName: project.name });
                      handleChatComplete(updated);
                      setAutopilotProgress(null);
                    } catch (err) {
                      console.warn("暂停自动驾驶失败:", err);
                    }
                  }}
                  onRetry={async () => {
                    try {
                      // Re-activate autopilot from ErrorStopped
                      await invokeWithTimeout<Project>("toggle_autopilot", { projectName: project.name, active: false });
                      const updated = await invokeWithTimeout<Project>("toggle_autopilot", { projectName: project.name, active: true });
                      handleChatComplete(updated);
                      setAutopilotProgress(null);
                      setFeedbackMsg({ type: "info", message: "已重新激活自动驾驶。" });
                    } catch (err) { console.warn("重试失败:", err); }
                  }}
                  onEnterDiscussion={async () => {
                    try {
                      const updated = await invokeWithTimeout<Project>("enter_autopilot_discussion", { projectName: project.name });
                      handleChatComplete(updated);
                      setAutopilotProgress(null);
                    } catch (err) { console.warn("进入讨论失败:", err); }
                  }}
                  onExitAutopilot={async () => {
                    try {
                      const updated = await invokeWithTimeout<Project>("exit_autopilot_keep_state", { projectName: project.name });
                      handleChatComplete(updated);
                      setAutopilotProgress(null);
                      setFeedbackMsg({ type: "info", message: "已退出自动驾驶。" });
                    } catch (err) { console.warn("退出失败:", err); }
                  }}
                />
              )}
              {/* V1 Console 规划闭环：大阶段 → 中阶段 → 执行计划 */}
              {(step === "MilestoneGeneration" || step === "MilestoneCheck" ||
                step === "MilestoneApproval" || step === "MilestoneSelection" ||
                step === "MidStageGeneration" || step === "MidStageCheck" ||
                step === "MidStageApproval" || step === "MidStageSelection" ||
                step === "PlanGeneration" || step === "PlanCheck" || step === "PlanApproving") && (
                <ConsoleWorkflowPanel
                  project={project}
                  onProjectUpdated={handleChatComplete}
                />
              )}
              {/* V1 执行阶段 UI — 仅在 Execution 步骤渲染 */}
              {step === "Execution" && (
                <>
                  <V1ExecutionPanel
                    project={project}
                    executionStatus={executionStatus}
                    workspaceStatus={workspaceStatus}
                    onWorkspaceStatusChange={setWorkspaceStatus}
                    onExecute={handleExecuteCurrentSubtask}
                    onConfirm={handleConfirmSubtask}
                    onReject={handleRejectSubtask}
                    onInStop={handleInStop}
                    onEdStop={handleEdStop}
                    onSyncProject={handleSyncProject}
                    autopilotRunning={project.workflow_state.autopilot_active === true
                      && project.workflow_state.autopilot_state?.run_status === "Running"}
                  />
                  <TaskConsole
                    projectPath={projectPath}
                    projectName={project.name}
                    executionStatus={executionStatus}
                    testLogs={testLogs}
                    workspaceReady={workspaceStatus?.ready === true}
                    executionHistory={project.execution_history}
                  />
                </>
              )}
              {/* V1 暂停决策 */}
              {step === "PauseDecision" && (
                <PauseDecisionPanel
                  pauseType={project.pause_context?.pause_type === "ed_stop" ? "ed_stop" : "in_stop"}
                  onContinue={() => handleResolvePause("continue")}
                  onAdjustOnly={() => handleResolvePause("adjust")}
                  onRollback={() => handleResolvePause("rollback")}
                  isAutopilot={project.workflow_state.autopilot_active}
                  onResumeAutopilot={handleResumeAutopilot}
                  onEnterAutopilotDiscussion={handleEnterAutopilotDiscussion}
                  onExitAutopilot={handleExitAutopilotKeepState}
                />
              )}
              {/* V1 回退预览 */}
              {step === "RollbackPreview" && (
                <RollbackImpactDialog
                  project={project}
                  onPreview={handlePreviewRollback}
                  onConfirm={handleConfirmRollback}
                />
              )}
              {/* V1 大阶段审阅 A/B/C */}
              {step === "MilestoneReview" && (
                <MilestoneReviewPanel
                  milestoneTitle={project.milestones.find(m => m.id === project.current_milestone_id)?.title ?? "?"}
                  onContinue={() => handleApproveMilestoneOutcome("A")}
                  onFixPast={() => handleApproveMilestoneOutcome("B")}
                  onAdjustFuture={() => handleApproveMilestoneOutcome("C")}
                />
              )}
              {/* V1 分支讨论 (B/C) */}
              {step === "BranchDiscussion" && (
                <BranchDiscussionPanel
                  project={project}
                  onSuggestRollback={handleSuggestRollback}
                  onGenerateFuture={handleGenerateFutureMilestones}
                  onChatComplete={handleChatComplete}
                  onAddMessage={handleAddMessage}
                />
              )}
              {/* V1 未来计划审批 (C) */}
              {step === "FuturePlanApproval" && (
                <ConsoleWorkflowPanel
                  project={project}
                  onProjectUpdated={handleChatComplete}
                />
              )}
              {/* 未识别步骤只显示错误，不回退到旧业务控制台。 */}
              {step !== "MilestoneGeneration" && step !== "MilestoneCheck" &&
                step !== "MilestoneApproval" && step !== "MilestoneSelection" &&
                step !== "MidStageGeneration" && step !== "MidStageCheck" &&
                step !== "MidStageApproval" && step !== "MidStageSelection" &&
                step !== "PlanGeneration" && step !== "PlanCheck" &&
                step !== "PlanApproving" && step !== "Execution" &&
                step !== "PauseDecision" && step !== "RollbackPreview" &&
                step !== "MilestoneReview" && step !== "BranchDiscussion" &&
                step !== "FuturePlanApproval" && (
                <div className="unsupported-console-step">
                  <h2>不支持的 Console 步骤</h2>
                  <p>当前步骤：{step}。请同步项目状态后重试。</p>
                  <button onClick={() => invokeWithTimeout<Project>("get_project", { projectName: project.name }).then(handleChatComplete)}>
                    同步项目状态
                  </button>
                </div>
              )}
            </div>
          </div>
        )}

        {phase === "Completed" && (
          <div className="completed-view" style={{ padding: "40px", textAlign: "center" }}>
            <h2>✅ 项目已完成</h2>
            <p style={{ color: "#656d76" }}>所有大阶段已执行完毕。</p>
          </div>
        )}

        {/* ===== Floating chat balloon in console mode ===== */}
        {phase === "Console" && (
          <FloatingChatBalloon messages={currentThread.messages || []} />
        )}
      </main>

    </div>
  );
}

// ============================================================
// V1 执行面板：单小阶段执行 + 人工确认
// ============================================================
function V1ExecutionPanel({
  project, executionStatus, workspaceStatus, onWorkspaceStatusChange, onExecute, onConfirm, onReject, onInStop, onEdStop, onSyncProject, autopilotRunning,
}: {
  project: Project; executionStatus: PipelineState | null;
  workspaceStatus: ExecutionWorkspaceStatus | null;
  onWorkspaceStatusChange: (status: ExecutionWorkspaceStatus) => void;
  onExecute: () => Promise<void>; onConfirm: () => Promise<void>;
  onReject: (reason: string) => Promise<void>;
  onInStop: () => Promise<void>; onEdStop: () => Promise<void>;
  onSyncProject: () => Promise<void>;
  autopilotRunning: boolean;
}) {
  const [rejectReason, setRejectReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [showReject, setShowReject] = useState(false);

  const ms = project.milestones.find(m => m.id === project.current_milestone_id);
  const mid = ms?.mid_stages.find(m => m.id === project.current_mid_stage_id);
  const planApproved = mid?.plan_approved_at != null && (mid?.plan_revision ?? 0) > 0;

  // Find next pending subtask or one awaiting confirmation
  const pendingSubtasks = mid?.subtasks.filter(s => s.status === "Pending") ?? [];
  const nextSubtask = pendingSubtasks[0] ?? null;
  const awaitingSubtask = mid?.subtasks.find(s => s.status === "AwaitingConfirmation") ?? null;

  const isAwaiting = executionStatus?.awaiting_confirmation === true || awaitingSubtask != null;
  const isExecuting = executionStatus?.status === "Running";

  const handlePrepareWorkspace = async () => {
    if (!project || busy) return;
    setBusy(true);
    try {
      const status = await invokeWithTimeout<ExecutionWorkspaceStatus>("prepare_execution_workspace", { projectName: project.name });
      onWorkspaceStatusChange(status);
    } catch (err) {
      console.error("准备执行工作区失败:", err);
    } finally {
      setBusy(false);
    }
  };

  const handleConfirm = async () => {
    setBusy(true);
    await onConfirm();
    setBusy(false);
  };

  const handleReject = async () => {
    if (!rejectReason.trim()) return;
    setBusy(true);
    await onReject(rejectReason.trim());
    setRejectReason("");
    setShowReject(false);
    setBusy(false);
  };

  const workspaceReady = workspaceStatus?.ready === true;

  return (
    <div className="v1-execution-panel" style={{ padding: "24px" }}>
      <h2 className="execution-panel-title"><ListTodo size={20} />执行</h2>

      {/* Workspace status banner */}
      {planApproved && workspaceStatus && !workspaceReady && (
        <FeedbackBanner type="warning" message={workspaceStatus.status_message} />
      )}

      {/* Workspace preparation */}
      {planApproved && !workspaceReady && (
        <div style={{ marginBottom: "20px" }}>
          <ActionButton icon={<GitBranch size={16} />} loading={busy} loadingLabel="准备中"
            disabled={autopilotRunning} onClick={handlePrepareWorkspace}>准备执行环境</ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            执行小阶段前需要初始化 Git 仓库并创建首次提交。
          </p>
        </div>
      )}

      {/* Awaiting confirmation */}
      {isAwaiting && awaitingSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <div style={{ padding: "14px", background: "#ddf4ff", borderRadius: "8px", border: "1px solid #0969da", marginBottom: "16px" }}>
            <strong>待确认：{awaitingSubtask.title}</strong>
            <div style={{ fontSize: "13px", color: "#656d76", marginTop: "8px" }}>
              <div>目标：{awaitingSubtask.goal || awaitingSubtask.title}</div>
              {awaitingSubtask.execution_result && (
                <>
                  <div style={{ marginTop: "4px" }}>变更文件：{awaitingSubtask.execution_result.file_changes?.join(", ") || "无"}</div>
                  <div style={{ marginTop: "4px", maxHeight: "150px", overflowY: "auto", background: "#f6f8fa", padding: "8px", borderRadius: "4px", fontFamily: "monospace", fontSize: "11px" }}>
                    {awaitingSubtask.execution_result.output?.slice(-1000)}
                  </div>
                </>
              )}
              {awaitingSubtask.test_result && (
                <div style={{ marginTop: "4px", color: awaitingSubtask.test_result.passed ? "#1a7f37" : "#cf222e" }}>
                  测试：{awaitingSubtask.test_result.passed ? "通过" : "未通过"}
                  {awaitingSubtask.test_result.suggestion && ` — ${awaitingSubtask.test_result.suggestion}`}
                </div>
              )}
              <div style={{ marginTop: "4px" }}>验收标准：{awaitingSubtask.acceptance_criteria?.join("；") || "（无）"}</div>
            </div>
          </div>
          <WorkflowActionBar>
            <ActionButton icon={<Check size={16} />} loading={busy} loadingLabel="确认中"
              disabled={autopilotRunning} onClick={handleConfirm}>确认通过</ActionButton>
            <ActionButton icon={<X size={16} />} variant="danger" disabled={busy || autopilotRunning}
              onClick={() => setShowReject(true)}>发现问题</ActionButton>
          </WorkflowActionBar>
          <Modal isOpen={showReject} onClose={() => setShowReject(false)} title="驳回执行结果"
            description="请记录需要修正的问题。" isDanger lockClose={busy} isSubmitting={busy}
            actions={[
              { label: "取消", onClick: () => setShowReject(false), variant: "secondary", disabled: busy },
              { label: busy ? "提交中..." : "确认驳回", onClick: handleReject, variant: "danger", disabled: busy || !rejectReason.trim() },
            ]}>
            <textarea className="console-feedback-input" value={rejectReason} onChange={e => setRejectReason(e.target.value)} placeholder="请说明发现的问题" disabled={busy} />
          </Modal>
        </div>
      )}

      {/* Next pending subtask — only when workspace is ready */}
      {!isAwaiting && planApproved && workspaceReady && nextSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <div style={subtaskCardStyle}>
            <strong>下一个任务：{nextSubtask.title}</strong>
            <div style={{ fontSize: "13px", color: "#656d76", marginTop: "4px" }}>
              目标：{nextSubtask.goal || nextSubtask.title}
            </div>
            <div style={{ fontSize: "12px", color: "#656d76", marginTop: "2px" }}>
              允许修改：{nextSubtask.allowed_file_paths?.join(", ") || "—"} |
              允许新建：{nextSubtask.new_file_paths?.join(", ") || "—"}
            </div>
            <div style={{ fontSize: "12px", color: "#656d76", marginTop: "2px" }}>
              验收标准：{nextSubtask.acceptance_criteria?.join("；") || "（无）"}
            </div>
          </div>
          <ActionButton icon={<Play size={16} />} loading={busy || isExecuting} loadingLabel={isExecuting ? "执行中" : "启动中"}
            disabled={busy || autopilotRunning}
            onClick={async () => { setBusy(true); await onExecute(); setBusy(false); }}>执行当前小阶段</ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            一次只执行一个已批准小阶段。执行完成后需要人工确认结果。
          </p>
        </div>
      )}

      {/* Pause controls — only visible when execution is actively running */}
      {isExecuting && !isAwaiting && (
        <div style={{
          marginBottom: "20px", padding: "16px",
          background: "#fff8f0", borderRadius: "8px", border: "1px solid #e6a23c",
        }}>
          <div style={{ fontWeight: 600, fontSize: "14px", marginBottom: "12px", color: "#9a6700" }}>
            ⏸ 暂停执行
          </div>
          <div style={{ display: "flex", gap: "16px", flexWrap: "wrap" }}>
            <div style={{ flex: 1, minWidth: "180px" }}>
              <ActionButton
                icon={<Square size={16} />}
                variant="danger"
                disabled={busy}
                onClick={async () => { setBusy(true); await onInStop(); setBusy(false); }}
                fullWidth
              >
                立即暂停 (In Stop)
              </ActionButton>
              <p style={{ color: "#656d76", fontSize: "12px", marginTop: "4px" }}>
                立即终止当前任务，回到上一个稳定检查点。未完成的任务不保留部分结果。
              </p>
            </div>
            <div style={{ flex: 1, minWidth: "180px" }}>
              <ActionButton
                icon={<Pause size={16} />}
                variant="secondary"
                disabled={busy}
                onClick={async () => { setBusy(true); await onEdStop(); setBusy(false); }}
                fullWidth
              >
                完成后暂停 (ED Stop)
              </ActionButton>
              <p style={{ color: "#656d76", fontSize: "12px", marginTop: "4px" }}>
                当前任务执行完成并确认后再暂停，已完成的任务得到保留。
              </p>
            </div>
          </div>
        </div>
      )}

      {/* All done — workflow should have auto-advanced; this is a safety net */}
      {!isAwaiting && planApproved && workspaceReady && !nextSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <FeedbackBanner type="success" message="当前中阶段所有小阶段已执行完成。" />
          <p style={{ color: "#656d76", fontSize: "13px", marginTop: "12px" }}>
            如果页面未自动跳转，请手动同步项目状态。
          </p>
          <ActionButton
            icon={<RotateCcw size={16} />}
            variant="secondary"
            disabled={autopilotRunning}
            onClick={onSyncProject}
          >
            同步项目状态
          </ActionButton>
          {autopilotRunning && (
            <p style={{ color: "#e6a23c", fontSize: "12px", marginTop: "8px" }}>
              🤖 自动驾驶运行中，手动操作已锁定。请使用上方进度条的暂停按钮。
            </p>
          )}
        </div>
      )}

      {/* Execution log */}
      {executionStatus && (
        <div style={{ marginTop: "20px", padding: "10px", background: "#f6f8fa", borderRadius: "6px", fontSize: "12px", fontFamily: "monospace", color: "#656d76" }}>
          {executionStatus.current_log}
        </div>
      )}
    </div>
  );
}

const subtaskCardStyle: React.CSSProperties = {
  padding: "14px", background: "#f6f8fa", borderRadius: "8px",
  border: "1px solid #d0d7de", marginBottom: "12px",
};

// ============================================================
// V1 分支讨论面板 (B/C 分支)
// ============================================================
function BranchDiscussionPanel({
  project, onSuggestRollback, onGenerateFuture, onChatComplete, onAddMessage,
}: {
  project: Project;
  onSuggestRollback: () => Promise<void>;
  onGenerateFuture: () => Promise<void>;
  onChatComplete: (p: Project) => void;
  onAddMessage: (msg: ChatMessage) => void;
}) {
  const scope = project.workflow_state.discussion_scope;
  const isFixPast = scope === "FixPast";
  const isPauseAdjustment = scope === "PauseAdjustment";

  return (
    <ConsoleStepShell icon={isPauseAdjustment ? <Pause /> : (isFixPast ? <RotateCcw /> : <GitBranch />)}
      title={isPauseAdjustment ? "暂停调整讨论" : (isFixPast ? "B 分支：修正过去" : "C 分支：调整未来")}
      description={isPauseAdjustment
        ? "在阶段中讨论当前问题，讨论完成后可恢复自动驾驶"
        : (isFixPast ? "分析执行证据并建议稳定回退点" : "保留已完成大阶段并调整后续")}
      status="pending" statusLabel="讨论中"
      actions={<WorkflowActionBar>{
      isPauseAdjustment ? (
        <>
          <ActionButton icon={<Bot size={16} />}
            onClick={async () => {
              try {
                const updated = await invokeWithTimeout<Project>("resume_autopilot_after_discussion", {
                  projectName: project.name,
                });
                onChatComplete(updated);
              } catch (err) { console.error("恢复自动驾驶失败:", err); }
            }}>
            恢复自动驾驶
          </ActionButton>
          <ActionButton icon={<Square size={16} />} variant="secondary"
            onClick={async () => {
              try {
                const updated = await invokeWithTimeout<Project>("exit_autopilot_keep_state", {
                  projectName: project.name,
                });
                onChatComplete(updated);
              } catch (err) { console.error("退出自动驾驶失败:", err); }
            }}>
            退出自动驾驶改手动
          </ActionButton>
        </>
      ) : isFixPast ? (
        <ActionButton icon={<Search size={16} />} variant="danger" onClick={onSuggestRollback}>诊断并建议回退点</ActionButton>
      ) : (
        <ActionButton icon={<WandSparkles size={16} />} onClick={onGenerateFuture}>生成后续大阶段草稿</ActionButton>
      )}</WorkflowActionBar>}>
      <ChatRoom
        messages={project.discussion_threads[0]?.messages || []}
        onAddMessage={onAddMessage}
        projectName={project.name}
        currentRole="产品经理"
        threadId={project.discussion_threads[0]?.id || "thread-init"}
        onProjectUpdated={onChatComplete}
      />
    </ConsoleStepShell>
  );
}

// ============================================================
// Phase 3: Autopilot progress bar
// ============================================================
function AutopilotProgressBar({
  progress, project, autopilotState, onPause, onRetry, onEnterDiscussion, onExitAutopilot,
}: {
  progress: AutopilotNextStep | null;
  project: Project;
  autopilotState?: { run_status: string; last_action: string; error_message: string; last_recovery_reason?: string; recovery_count?: number };
  onPause: () => Promise<void>;
  onRetry?: () => void;
  onEnterDiscussion?: () => void;
  onExitAutopilot?: () => void;
}) {
  const [busy, setBusy] = useState(false);

  // Phase 3: Dual-source progress data.
  // Primary: autopilot_next_step response (real-time, from backend advisor).
  // Fallback: compute from Project when autopilot_next_step hasn't returned yet
  // (e.g., first render after activating autopilot, or mid-step execution).
  const targetMs = project.milestones.find(
    (m) => m.id === project.workflow_state.autopilot_target_milestone_id
  );
  const completedSubs = progress?.completed_subtasks
    ?? targetMs?.mid_stages.reduce((sum, mid) =>
      sum + mid.subtasks.filter((st) => st.status === "Passed").length, 0)
    ?? 0;
  const totalSubs = progress?.total_subtasks
    ?? targetMs?.mid_stages.reduce((sum, mid) => sum + mid.subtasks.length, 0)
    ?? 0;
  const currentMidIdx = progress?.current_mid_stage_index
    ?? (targetMs ? (targetMs.mid_stages.findIndex((m) => m.status !== "Completed") + 1) || targetMs.mid_stages.length : 0);
  const totalMids = progress?.total_mid_stages ?? targetMs?.mid_stages.length ?? 0;
  const currentAction = progress?.current_action
    ?? (targetMs ? `大阶段「${targetMs.title}」` : "");

  const pct = totalSubs > 0 ? Math.round((completedSubs / totalSubs) * 100) : 0;

  const statusLabel = autopilotState?.run_status === "ErrorStopped" ? "已停止"
    : autopilotState?.run_status === "Paused" ? "已暂停"
    : autopilotState?.run_status === "WaitingMilestoneReview" ? "等待审阅"
    : "自动驾驶中";

  const isErrorStopped = autopilotState?.run_status === "ErrorStopped";
  const hasRecovery = !!(autopilotState?.last_recovery_reason);

  return (
    <div className="autopilot-progress-bar" style={{
      padding: "12px 24px", background: "linear-gradient(135deg, #1a1e2b 0%, #1f2940 100%)",
      borderRadius: "10px", marginBottom: "16px", border: "1px solid #2d3a50",
    }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "8px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <span style={{
            display: "inline-flex", alignItems: "center", gap: "4px",
            background: isErrorStopped ? "#cf222e20" : "#0969da20",
            color: isErrorStopped ? "#f85149" : "#58a6ff",
            padding: "2px 10px",
            borderRadius: "12px", fontSize: "12px", fontWeight: 600,
          }}>
            🤖 {statusLabel}
          </span>
          <span style={{ color: "#8b949e", fontSize: "13px" }}>
            {progress?.description || currentAction}
          </span>
        </div>
        {!isErrorStopped && (
          <button
            className="btn-autopilot-pause"
            onClick={async () => { setBusy(true); await onPause(); setBusy(false); }}
            disabled={busy}
            style={{
              padding: "4px 14px", fontSize: "12px", borderRadius: "6px",
              border: "1px solid #e6a23c", background: "#e6a23c20",
              color: "#e6a23c", cursor: "pointer", fontWeight: 600,
            }}
          >
            {busy ? "暂停中..." : "⏸ 暂停"}
          </button>
        )}
      </div>

      {/* Progress bar */}
      <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
        <div style={{
          flex: 1, height: "6px", background: "#2d3a50", borderRadius: "3px", overflow: "hidden",
        }}>
          <div style={{
            width: `${pct}%`, height: "100%",
            background: isErrorStopped ? "#cf222e" : (pct >= 100 ? "#1a7f37" : "linear-gradient(90deg, #0969da, #58a6ff)"),
            borderRadius: "3px", transition: "width 0.5s ease",
          }} />
        </div>
        <span style={{ color: "#8b949e", fontSize: "12px", minWidth: "80px", textAlign: "right" }}>
          {completedSubs}/{totalSubs} 小阶段
          {totalMids > 0 && ` · 中阶段 ${currentMidIdx}/${totalMids}`}
        </span>
        <span style={{ color: isErrorStopped ? "#f85149" : "#58a6ff", fontSize: "12px", fontWeight: 600, minWidth: "36px" }}>
          {pct}%
        </span>
      </div>

      {/* Recovery info (Phase 5: auto-recovery notification) */}
      {hasRecovery && !isErrorStopped && (
        <div style={{ marginTop: "8px", padding: "6px 10px", background: "#0969da15", borderRadius: "6px", color: "#58a6ff", fontSize: "12px" }}>
          🔄 自动驾驶已自动修复一次结构化输出异常，正在继续
          {autopilotState?.recovery_count ? `（累计补救 ${autopilotState.recovery_count} 次）` : ""}
        </div>
      )}

      {/* Error message + actions (Phase 5: ErrorStopped recovery) */}
      {isErrorStopped && autopilotState?.error_message && (
        <div style={{ marginTop: "8px" }}>
          <div style={{ padding: "6px 10px", background: "#cf222e15", borderRadius: "6px", color: "#f85149", fontSize: "12px", marginBottom: "8px" }}>
            ⚠ {autopilotState.error_message}
          </div>
          <div style={{ display: "flex", gap: "8px" }}>
            {onRetry && (
              <button onClick={onRetry} style={{
                padding: "4px 12px", fontSize: "12px", borderRadius: "6px",
                border: "1px solid #58a6ff", background: "#0969da20",
                color: "#58a6ff", cursor: "pointer",
              }}>
                🔄 重试当前步骤
              </button>
            )}
            {onEnterDiscussion && (
              <button onClick={onEnterDiscussion} style={{
                padding: "4px 12px", fontSize: "12px", borderRadius: "6px",
                border: "1px solid #8b949e", background: "#30363d30",
                color: "#8b949e", cursor: "pointer",
              }}>
                💬 进入阶段讨论
              </button>
            )}
            {onExitAutopilot && (
              <button onClick={onExitAutopilot} style={{
                padding: "4px 12px", fontSize: "12px", borderRadius: "6px",
                border: "1px solid #f85149", background: "#cf222e15",
                color: "#f85149", cursor: "pointer",
              }}>
                ⏹ 退出自动驾驶
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
