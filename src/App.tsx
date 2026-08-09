// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useCallback, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import "./App.css";
import "./ConsoleWorkspace.css";
import { Project, ViewMode, DiscussionReason, PipelineState, ChatMessage, Milestone, RollbackImpact, WorkflowStep, ExecutionWorkspaceStatus, RecoveryPresentation, RecoveryResultSummary, RuntimeSnapshot, RuntimeMutationResult, ExecutionRecoveryImpact, RecoveryDecisionResolution, MilestoneReviewSubmission } from "./types";
import { ProjectEntry } from "./ProjectEntry";
import { ExistingBaselinePanel } from "./ExistingBaselinePanel";
import { PreflightPanel } from "./PreflightPanel";
import { PlanApprovalPanel } from "./PlanApprovalPanel";
import { DecisionStepHeader } from "./components/DecisionStepHeader";
import { FeedbackBanner } from "./components/FeedbackBanner";
import { ActionButton } from "./components/ActionButton";
import { ConsoleStepShell } from "./components/ConsoleStepShell";
import { WorkflowActionBar } from "./components/WorkflowActionBar";
import { ArrowLeft, Bot, GitBranch, PanelRightOpen, RotateCcw, Search, WandSparkles } from "lucide-react";
import { AutopilotControlBar } from "./components/AutopilotControlBar";
import { RecoveryResultBanner } from "./components/RecoveryResultBanner";
import { SyncStatusIndicator } from "./components/SyncStatusIndicator";
import ExecutionTree from "./ExecutionTree";
import ChatRoom from "./ChatRoom";
import TaskConsole from "./TaskConsole";
import { ConsoleWorkflowPanel } from "./ConsoleWorkflowPanel";
import { FuturePlanningWorkspace } from "./FuturePlanningWorkspace";
import { PauseDecisionPanel } from "./PauseDecisionPanel";
import { RollbackImpactDialog } from "./RollbackImpactDialog";
import { RecoveryImpactDialog } from "./RecoveryImpactDialog";
import { MilestoneReviewPanel } from "./MilestoneReviewPanel";
import { ApplicationSettings } from "./components/ApplicationSettings";
import {
  CONSOLE_LAYOUT_CONTRACT,
  ConsoleCommandBar,
  ConsoleWorkspace,
} from "./components/ConsoleWorkspace";
import { ConsoleNavigator } from "./components/ConsoleNavigator";
import { ConsoleBottomPanel } from "./components/ConsoleBottomPanel";
import FileTree from "./FileTree";
import FloatingChatBalloon from "./FloatingChatBalloon";
import TaskInspector from "./TaskInspector";
import V1ExecutionPanel from "./V1ExecutionPanel";
import { useTaskControlWorkspace } from "./hooks/useTaskControlWorkspace";
import { useProjectStateSync } from "./hooks/useProjectStateSync";
import {
  clampPanelWidth,
  DEFAULT_INSPECTOR_WIDTH,
  DEFAULT_SIDEBAR_WIDTH,
  INSPECTOR_WIDTH_STORAGE_KEY,
  MAX_INSPECTOR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  MIN_INSPECTOR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  readStoredPanelWidth,
  SIDEBAR_WIDTH_STORAGE_KEY,
} from "./panelLayoutPolicy";
import {
  executionPollDecision,
  isTerminalRuntimeSnapshot,
  shouldReconcileAfterPollFailure,
  terminalDelayedSyncDelay,
  terminalSyncDelay,
  type TerminalSyncPhase,
} from "./executionSyncPolicy";
import { resolvePlanTarget } from "./planTargetPolicy";
import { getConsoleWritePolicy, type ConsoleWritePolicy } from "./consoleWritePolicy";

const WORKFLOW_STEPS = new Set<WorkflowStep>([
  "WaitingEntry", "ExistingAnalysis", "BaselineApproval", "Discussion", "ThreeChecks",
  "ProjectPlanGeneration", "PlanApproval", "MilestoneGeneration", "MilestoneCheck", "MilestoneApproval",
  "MilestoneSelection", "MidStageGeneration", "MidStageCheck", "MidStageApproval",
  "MidStageSelection", "PlanGeneration", "PlanCheck", "PlanApproving", "Execution",
  "PauseDecision", "RollbackPreview", "BranchDiscussion", "FuturePlanApproval",
  "MilestoneReview", "Completed",
]);

/** 执行状态轮询周期（ms） */
const EXECUTION_POLL_INTERVAL_MS = 1500;

/** 连续轮询失败最大次数，防止界面无限静默等待 */
const EXECUTION_POLL_MAX_FAILURES = 10;

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
  const [project, setProject] = useState<Project | null>(null);
  const projectRef = useRef<Project | null>(null);
  const [projectPath, setProjectPath] = useState<string>("");

  // === Phase B：视图模式控制 ===
  const [viewMode, setViewMode] = useState<ViewMode>({ phase: 'discussion', reason: 'idle' });

  // Phase D: 动画控制（保留用于视觉过渡，不决定业务阶段）
  const animationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 大阶段完成总结去重：记录已发送过总结消息的大阶段 ID
  const completedMilestonesRef = useRef<Set<string>>(new Set());

  // === 侧边栏拖拽缩放 ===
  const [sidebarWidth, setSidebarWidth] = useState(() => readStoredPanelWidth(
    typeof window === "undefined" ? null : window.localStorage,
    SIDEBAR_WIDTH_STORAGE_KEY,
    DEFAULT_SIDEBAR_WIDTH,
    MIN_SIDEBAR_WIDTH,
    MAX_SIDEBAR_WIDTH,
  ));
  const sidebarWidthWasStored = useRef(
    typeof window !== "undefined" && window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY) !== null,
  );
  const [isDragging, setIsDragging] = useState(false);
  const dragStartX = useRef(0);
  const dragStartWidth = useRef(0);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [inspectorWidth, setInspectorWidth] = useState(() => readStoredPanelWidth(
    typeof window === "undefined" ? null : window.localStorage,
    INSPECTOR_WIDTH_STORAGE_KEY,
    DEFAULT_INSPECTOR_WIDTH,
    MIN_INSPECTOR_WIDTH,
    MAX_INSPECTOR_WIDTH,
  ));
  const [isInspectorDragging, setIsInspectorDragging] = useState(false);
  const inspectorDragStartX = useRef(0);
  const inspectorDragStartWidth = useRef(0);

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
        console.warn("拒绝应用较旧的 Project 修订",
          `incoming=${updatedProject.workflow_state.data_revision} current=${current.workflow_state.data_revision}`);
        return false;
      }
      // 同修订但子状态不一致：记录警告但不拒绝（可能只是不同字段的合法更新）
      if (updatedProject.workflow_state.data_revision === current.workflow_state.data_revision) {
        if (updatedProject.execution_session?.status !== current.execution_session?.status
            || updatedProject.workflow_state.autopilot_active !== current.workflow_state.autopilot_active
            || updatedProject.workflow_state.managed_flow_state?.active !== current.workflow_state.managed_flow_state?.active) {
          console.warn("同修订子状态变化",
            { exec: updatedProject.execution_session?.status, ap: updatedProject.workflow_state.autopilot_active, mf: updatedProject.workflow_state.managed_flow_state?.active });
        }
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
      updated.discussion_threads = prev.discussion_threads.map((thread) => {
        if (thread.id === prev.workflow_state.active_discussion_thread_id) {
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

  const handleSidebarSeparatorKeyDown = (event: React.KeyboardEvent) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (event.key === "Home") setSidebarWidth(MIN_SIDEBAR_WIDTH);
    else if (event.key === "End") setSidebarWidth(MAX_SIDEBAR_WIDTH);
    else setSidebarWidth(width => clampPanelWidth(
      width + (event.key === "ArrowRight" ? 16 : -16),
      MIN_SIDEBAR_WIDTH,
      MAX_SIDEBAR_WIDTH,
    ));
  };

  const handleInspectorPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    inspectorDragStartX.current = event.clientX;
    inspectorDragStartWidth.current = inspectorWidth;
    setIsInspectorDragging(true);
  };

  const handleInspectorPointerMove = useCallback((event: PointerEvent) => {
    const delta = event.clientX - inspectorDragStartX.current;
    setInspectorWidth(clampPanelWidth(
      inspectorDragStartWidth.current - delta,
      MIN_INSPECTOR_WIDTH,
      MAX_INSPECTOR_WIDTH,
    ));
    if (event.buttons === 0) setIsInspectorDragging(false);
  }, []);

  const handleInspectorPointerUp = useCallback(() => {
    setIsInspectorDragging(false);
  }, []);

  const handleInspectorSeparatorKeyDown = (event: React.KeyboardEvent) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (event.key === "Home") setInspectorWidth(MIN_INSPECTOR_WIDTH);
    else if (event.key === "End") setInspectorWidth(MAX_INSPECTOR_WIDTH);
    else setInspectorWidth(width => clampPanelWidth(
      width + (event.key === "ArrowLeft" ? 16 : -16),
      MIN_INSPECTOR_WIDTH,
      MAX_INSPECTOR_WIDTH,
    ));
  };

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

  useEffect(() => {
    if (!isInspectorDragging) return;
    window.addEventListener("pointermove", handleInspectorPointerMove);
    window.addEventListener("pointerup", handleInspectorPointerUp);
    window.addEventListener("pointercancel", handleInspectorPointerUp);
    window.addEventListener("blur", handleInspectorPointerUp);
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    return () => {
      window.removeEventListener("pointermove", handleInspectorPointerMove);
      window.removeEventListener("pointerup", handleInspectorPointerUp);
      window.removeEventListener("pointercancel", handleInspectorPointerUp);
      window.removeEventListener("blur", handleInspectorPointerUp);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [handleInspectorPointerMove, handleInspectorPointerUp, isInspectorDragging]);

  useEffect(() => {
    localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    localStorage.setItem(INSPECTOR_WIDTH_STORAGE_KEY, String(inspectorWidth));
  }, [inspectorWidth]);

  const [isExecuting, setIsExecuting] = useState(false);
  const [feedbackMsg, setFeedbackMsg] = useState<{ type: "error" | "success" | "warning" | "info"; message: string } | null>(null);
  const [executionStatus, setExecutionStatus] = useState<PipelineState | null>(null);
  const [recoveryPresentation, setRecoveryPresentation] = useState<RecoveryPresentation | null>(null);
  const [recoveryResult, setRecoveryResult] = useState<RecoveryResultSummary | null>(null);
  const [runtimeTaskControlSnapshot, setRuntimeTaskControlSnapshot] = useState<import("./types").TaskControlSnapshot | null>(null);
  const dismissRecoveryResult = useCallback(() => setRecoveryResult(null), []);
  const [terminalSyncPhase, setTerminalSyncPhase] = useState<TerminalSyncPhase>("idle");
  const [recoveryImpact, setRecoveryImpact] = useState<ExecutionRecoveryImpact | null>(null);
  const [pendingRecoveryDecision, setPendingRecoveryDecision] = useState<{
    resolution: RecoveryDecisionResolution;
    reason: string;
    acceptedCriteria: number[];
  } | null>(null);
  // === 启动恢复完成标记（防止 UI 在恢复完成前渲染） ===
  const [startupRecoveryDone, setStartupRecoveryDone] = useState(false);
  // === 决策层统一提交锁（同一时间只能执行一个关键动作） ===
  const [decisionAction, setDecisionAction] = useState<string | null>(null);
  const isDecisionSubmitting = decisionAction !== null;
  const [consoleAction, setConsoleAction] = useState<string | null>(null);
  const consoleActionRef = useRef<string | null>(null);
  const consoleWritePolicyRef = useRef<ConsoleWritePolicy>({ writable: true, reason: "" });
  const beginConsoleAction = useCallback((action: string) => {
    if (consoleActionRef.current !== null) return false;
    if (action !== "sync_project" && !consoleWritePolicyRef.current.writable) {
      setFeedbackMsg({ type: "warning", message: consoleWritePolicyRef.current.reason });
      return false;
    }
    consoleActionRef.current = action;
    setConsoleAction(action);
    return true;
  }, []);
  const endConsoleAction = useCallback(() => {
    consoleActionRef.current = null;
    setConsoleAction(null);
  }, []);
  const isConsoleBusy = consoleAction !== null;

  // === 执行工作区状态（供 V1ExecutionPanel 和 TaskConsole 共用） ===
  const [workspaceStatus, setWorkspaceStatus] = useState<ExecutionWorkspaceStatus | null>(null);
  const applyRuntimeSnapshot = useCallback((snapshot: RuntimeSnapshot) => {
    if (!handleChatComplete(snapshot.project)) return;
    setExecutionStatus(snapshot.pipeline_state);
    setIsExecuting(snapshot.pipeline_state?.status === "Running");
    setRecoveryPresentation(snapshot.recovery_presentation);
    if (snapshot.task_control_snapshot !== undefined) {
      setRuntimeTaskControlSnapshot(snapshot.task_control_snapshot ?? null);
    }
    setTerminalSyncPhase(current => (
      current !== "idle" && isTerminalRuntimeSnapshot(snapshot, snapshot.project.name)
        ? "idle"
        : current
    ));
  }, [handleChatComplete]);
  const applyRuntimeMutation = useCallback((result: RuntimeMutationResult) => {
    applyRuntimeSnapshot(result.runtime_snapshot);
    if (result.task_control_snapshot) setRuntimeTaskControlSnapshot(result.task_control_snapshot);
    if (result.action.recovery_result) setRecoveryResult(result.action.recovery_result);
  }, [applyRuntimeSnapshot]);
  const invokeRuntimeMutation = useCallback(async (
    command: string,
    args: Record<string, unknown>,
    timeoutMs?: number,
  ) => {
    const result = await invokeWithTimeout<RuntimeMutationResult>(command, args, timeoutMs);
    applyRuntimeMutation(result);
    return result;
  }, [applyRuntimeMutation]);
  const projectStateSync = useProjectStateSync({
    projectName: project?.name ?? "",
    enabled: startupRecoveryDone && Boolean(project?.name),
    includeTaskControlSnapshot: inspectorOpen,
    onSnapshot: applyRuntimeSnapshot,
  });
  const consoleWritePolicy = getConsoleWritePolicy(
    project?.workflow_state.top_level_phase === "Console",
    projectStateSync.state,
  );
  consoleWritePolicyRef.current = consoleWritePolicy;
  const forceRuntimeSync = projectStateSync.forceSync;
  const taskControlWorkspace = useTaskControlWorkspace({
    project,
    enabled: project?.workflow_state.top_level_phase === "Console" && inspectorOpen,
    invalidationSequence: projectStateSync.state.taskControlEventSequence,
    runtimeCursor: {
      processStartId: projectStateSync.state.taskControlProcessStartId,
      eventSequence: projectStateSync.state.taskControlEventSequence,
      projectRevision: projectStateSync.state.taskControlProjectRevision,
      treeRevision: projectStateSync.state.taskControlTreeRevision,
      controlActionId: projectStateSync.state.taskControlActionId,
      controlActionKnown: Boolean(projectStateSync.state.taskControlProcessStartId),
      snapshotVersion: projectStateSync.state.taskControlSnapshotVersion,
    },
    atomicSnapshot: runtimeTaskControlSnapshot,
    atomicSnapshotStatus: projectStateSync.state.taskControlDetailStatus,
    atomicSnapshotUpdatedAt: projectStateSync.state.taskControlDetailUpdatedAt,
    subscriptionStatus: projectStateSync.state.subscriptionStatus,
    runtimeSyncStatus: projectStateSync.state.status,
    runtimeSyncFailures: projectStateSync.state.consecutiveFailures,
    onRuntimeMutation: applyRuntimeMutation,
  });

  const openTaskInspector = useCallback((taskId: string) => {
    taskControlWorkspace.selectTask(taskId);
    setInspectorOpen(true);
  }, [taskControlWorkspace.selectTask]);

  const runTaskControlAction = useCallback(async (
    name: string,
    options?: { criterionIndexes?: number[]; reason?: string },
  ) => {
    if (!beginConsoleAction(`task_control:${name}`)) return;
    try {
      await taskControlWorkspace.executeAction(name, options);
    } finally {
      endConsoleAction();
    }
  }, [beginConsoleAction, endConsoleAction, taskControlWorkspace.executeAction]);

  const changeTaskControlMode = useCallback(async (
    mode: import("./types").TaskControlMode,
    reason?: string,
  ) => {
    if (!beginConsoleAction("task_control:change_mode")) return;
    try {
      await taskControlWorkspace.changeMode(mode, reason);
    } finally {
      endConsoleAction();
    }
  }, [beginConsoleAction, endConsoleAction, taskControlWorkspace.changeMode]);

  useEffect(() => {
    projectRef.current = project;
  }, [project]);
  useEffect(() => {
    setRecoveryImpact(null);
    setPendingRecoveryDecision(null);
    setRecoveryResult(null);
    setRuntimeTaskControlSnapshot(null);
    setTerminalSyncPhase("idle");
  }, [project?.name]);
  // V1: 回退后手动触发生成（不再自动触发）

  // 启动恢复也通过统一运行时快照对账，避免 Project 与 PipelineState 分头读取。
  useEffect(() => {
    if (!project) return;
    if (!startupRecoveryDone) return;

    const session = project.execution_session;
    if (!session || !session.active) {
      setExecutionStatus(null);
      setIsExecuting(false);
      return;
    }

    const sessionStatus = session.status.toLowerCase();
    if (["session_lost", "execution_failed", "stop_failed"].includes(sessionStatus)) {
      setExecutionStatus(null);
      setIsExecuting(false);
      setFeedbackMsg({
        type: "warning",
        message: `执行中断 (${session.subtask_title})：${session.failure_message || "请先恢复执行基线后再继续。"}`,
      });
      return;
    }

    let cancelled = false;
    void forceRuntimeSync().then(snapshot => {
      if (cancelled) return;
      if (!snapshot) {
        setFeedbackMsg({
          type: "warning",
          message: "启动执行状态同步失败，当前状态可能过期；系统将继续后台对账。",
        });
        return;
      }
      const latestSession = snapshot.project.execution_session ?? session;
      const pipeline = snapshot.pipeline_state;
      if (pipeline?.status === "Running") {
        setExecutionStatus(pipeline);
        setIsExecuting(true);
        return;
      }
      if (pipeline?.awaiting_confirmation) {
        setExecutionStatus(pipeline);
        setIsExecuting(false);
        return;
      }
      if (latestSession.status.toLowerCase() === "awaiting_confirmation") {
        setExecutionStatus({
          execution_id: latestSession.execution_id,
          mid_stage_id: latestSession.mid_stage_id,
          status: "Paused",
          current_subtask_index: latestSession.subtask_index,
          total_subtasks: latestSession.total_subtasks,
          subtask_statuses: [],
          current_log: `⏳ 待确认 (${latestSession.subtask_index + 1}/${latestSession.total_subtasks})：${latestSession.subtask_title}`,
          last_error: undefined,
          child_pid: undefined,
          project_name: snapshot.project.name,
          milestone_id: latestSession.milestone_id,
          plan_revision: latestSession.plan_revision,
          current_subtask_id: latestSession.subtask_id,
          awaiting_confirmation: true,
          log_history: [],
        });
        setIsExecuting(false);
      }
    });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.name, project?.execution_session?.active, project?.execution_session?.status, project?.execution_session?.subtask_id, startupRecoveryDone, forceRuntimeSync]);

  // 工作区只在进入审批/执行步骤时读取；命令和恢复完成会显式刷新。
  useEffect(() => {
    if (!project || !["PlanApproving", "Execution"].includes(project.workflow_state.current_step)) return;
    invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", { projectName: project.name })
      .then(setWorkspaceStatus)
      .catch(() => setWorkspaceStatus(null));
  }, [project?.name, project?.workflow_state.current_step]);

  const previousRecoveryKindRef = useRef<RecoveryPresentation["kind"] | null>(null);
  useEffect(() => {
    const currentKind = recoveryPresentation?.kind ?? null;
    const previousKind = previousRecoveryKindRef.current;
    previousRecoveryKindRef.current = currentKind;
    if (
      !project
      || project.workflow_state.current_step !== "Execution"
      || !previousKind
      || previousKind === "None"
      || currentKind !== "None"
    ) return;
    invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", {
      projectName: project.name,
    }).then(setWorkspaceStatus).catch(() => {});
  }, [project?.name, project?.workflow_state.current_step, recoveryPresentation?.kind]);

  // 执行状态轮询失败计数器
  const executionPollFailuresRef = useRef(0);
  // 执行状态轮询定时器
  const executionPollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const executionPollingActiveRef = useRef(false);

  const reconcileExecutionState = useCallback(async (projectName: string) => {
    setTerminalSyncPhase("terminal_reconciling");
    for (let attempt = 0; ; attempt += 1) {
      const delay = terminalSyncDelay(attempt);
      if (delay === null) break;
      if (delay > 0) {
        await new Promise<void>(resolve => setTimeout(resolve, delay));
      }
      if (projectRef.current?.name !== projectName) return null;
      const snapshot = await forceRuntimeSync();
      if (isTerminalRuntimeSnapshot(snapshot, projectName)) {
        setTerminalSyncPhase("idle");
        return snapshot;
      }
    }
    setTerminalSyncPhase("terminal_delayed");
    for (let attempt = 0; ; attempt += 1) {
      const delay = terminalDelayedSyncDelay(attempt);
      if (delay === null) return null;
      await new Promise<void>(resolve => setTimeout(resolve, delay));
      if (projectRef.current?.name !== projectName) return null;
      const snapshot = await forceRuntimeSync();
      if (isTerminalRuntimeSnapshot(snapshot, projectName)) {
        setTerminalSyncPhase("idle");
        return snapshot;
      }
    }
  }, [forceRuntimeSync]);

  /** 执行状态轮询只负责展示，不拥有自动驾驶推进权。 */
  const startExecutionPolling = useCallback(async (projectName: string) => {
    if (executionPollingActiveRef.current) return;
    executionPollingActiveRef.current = true;
    executionPollFailuresRef.current = 0;

    const poll = async () => {
      try {
        const status = await invokeWithTimeout<PipelineState | null>("get_execution_status", {});
        executionPollFailuresRef.current = 0;

        if (executionPollDecision(status, projectName) === "continue" && status) {
          setExecutionStatus(status);
          setIsExecuting(true);
          executionPollTimerRef.current = setTimeout(poll, EXECUTION_POLL_INTERVAL_MS);
        } else {
          executionPollingActiveRef.current = false;
          executionPollTimerRef.current = null;
          const snapshot = await reconcileExecutionState(projectName);
          if (!snapshot) {
            setFeedbackMsg({
              type: "warning",
              message: "执行已停止响应，但运行时对账失败；保留当前展示，状态可能过期。",
            });
            return;
          }
          if (status?.status === "Failed") {
            setFeedbackMsg({
              type: "error",
              message: status.last_error || "后台执行失败，请查看阶段日志后重试。",
            });
          }
        }
      } catch (error) {
        executionPollFailuresRef.current += 1;
        if (shouldReconcileAfterPollFailure(executionPollFailuresRef.current)) {
          const snapshot = await forceRuntimeSync();
          if (snapshot?.project.name === projectName) {
            executionPollFailuresRef.current = 0;
            if (snapshot.pipeline_state?.status === "Running") {
              executionPollTimerRef.current = setTimeout(poll, EXECUTION_POLL_INTERVAL_MS);
            } else {
              executionPollingActiveRef.current = false;
              executionPollTimerRef.current = null;
            }
            return;
          }
        }
        if (executionPollFailuresRef.current >= EXECUTION_POLL_MAX_FAILURES) {
          executionPollingActiveRef.current = false;
          executionPollTimerRef.current = null;
          const pollError = error instanceof Error ? error.message : String(error);
          setFeedbackMsg({
            type: "warning",
            message: `执行状态连续同步失败：${pollError}。保留当前展示，状态可能过期。`,
          });
          return;
        }
        executionPollTimerRef.current = setTimeout(poll, EXECUTION_POLL_INTERVAL_MS);
      }
    };

    poll();
  }, [forceRuntimeSync, reconcileExecutionState]);

  // 启动恢复只需恢复 isExecuting，此 effect 会接入同一轮询入口。
  useEffect(() => {
    if (!isExecuting || !project || executionPollingActiveRef.current) return;
    startExecutionPolling(project.name);
  }, [isExecuting, project?.name, startExecutionPolling]);

  useEffect(() => () => {
    if (executionPollTimerRef.current) clearTimeout(executionPollTimerRef.current);
    executionPollTimerRef.current = null;
    executionPollingActiveRef.current = false;
    executionPollFailuresRef.current = 0;
  }, [project?.name]);

  // Project 的实时同步由 useProjectStateSync 的 Channel 驱动；低频轮询仅作断线兜底。

  // === 快照：保存 UI 状态到后端，用于刷新恢复和孤儿进程保护 ===
  const takeSnapshot = () => {
    if (!project) return;
    const snapshotUi = {
      view_phase: viewMode.phase,
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
  }, [project, viewMode.phase]);

  // 大阶段完成检测：按大阶段自己的正式拓扑判断完成事实。
  useEffect(() => {
    if (!project) return;
    for (const ms of project.milestones) {
      if (isMilestoneFullyCompleted(ms) && !completedMilestonesRef.current.has(ms.id)) {
        const isDirect = ms.mode === "Quick";
        const midStages = ms.mid_stages || [];
        const directSubtasks = ms.subtasks || [];
        const totalCount = isDirect ? directSubtasks.length : midStages.length;
        const completedCount = isDirect
          ? directSubtasks.filter(task => isTerminalSubtask(task.status)).length
          : midStages.filter(mid => mid.status === "Completed").length;
        const failedCount = isDirect
          ? directSubtasks.filter(task => task.status === "Rejected").length
          : midStages.filter(mid => mid.status === "Rejected").length;
        // 收集 Git tag
        const tags: string[] = [];
        if (isDirect) {
          for (const task of directSubtasks) {
            if (task.auto_tag) tags.push(task.auto_tag);
          }
        } else {
          for (const mid of midStages) {
            if (mid.git_tag) tags.push(mid.git_tag);
          }
        }
        const tagsLine = tags.length > 0 ? tags.join("、") : "无";
        // 统计子任务测试通过率
        let totalSubtasks = 0;
        let passedSubtasks = 0;
        const taskGroups = isDirect ? [directSubtasks] : midStages.map(mid => mid.subtasks || []);
        for (const tasks of taskGroups) {
          for (const st of tasks) {
            totalSubtasks++;
            if (st.test_result?.passed) passedSubtasks++;
          }
        }
        const passRate = totalSubtasks > 0 ? `${Math.round(passedSubtasks / totalSubtasks * 100)}%` : "N/A";

        const markdown = `### 📋 大阶段「${ms.title}」执行完成

| 项目 | 数据 |
|------|------|
| ${isDirect ? "小阶段总数" : "中阶段总数"} | ${totalCount} |
| 已完成 | ${completedCount} |
| 失败 | ${failedCount} |
| 子任务测试通过率 | ${passRate} |
| Git 标签 | ${tagsLine} |

${isDirect ? "所有直挂小阶段" : "所有中阶段"}已执行完成，请审阅后决定下一步。`;

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
        invokeWithTimeout<RuntimeMutationResult>('summarize_milestone_runtime', {
          projectName: project.name,
          milestone_id: ms.id,
        })
          .then((result) => {
            applyRuntimeMutation(result);
            const aiMsg: ChatMessage = {
              id: crypto.randomUUID(),
              role: 'assistant',
              content: result.action.message,
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
  }, [applyRuntimeMutation, project, handleAddMessage]);

  // 启动恢复逻辑：从存储的项目名称恢复，没有则进入 Before 页面
  useEffect(() => {
    const storedName = localStorage.getItem("metheus_last_project");
    if (!storedName) {
      // 没有存储的项目，停留在 Before 页面
      setStartupRecoveryDone(true);
      return;
    }

    invokeWithTimeout<Project>("get_project", { projectName: storedName })
      .then((project) => {
        // 检查项目是否有效且处于正确的阶段
        if (!project || !project.name) {
          // 项目数据无效 — 清除失效记录，进入 Before
          setProject(null);
          localStorage.removeItem("metheus_last_project");
          setStartupRecoveryDone(true);
          return null; // 阻止后续 .then() 执行
        }

        setProject(project);

        // Build a sequential chain: migration → managed-state reconcile → execution reconcile → snapshot
        let chain: Promise<any> = Promise.resolve(project);

        const needsMigration = project.workflow_state.current_step === "WaitingEntry"
          && project.workflow_state.top_level_phase === "Before";
        if (needsMigration) {
          chain = chain.then((p: Project) =>
            invokeWithTimeout<RuntimeMutationResult>("migrate_project_workflow_runtime", {
              projectName: p.name,
            }).then((result) => {
              applyRuntimeMutation(result);
              return result.runtime_snapshot.project;
            }).catch((err) => {
              console.error("迁移旧项目工作流失败:", err);
              return p;
            })
          );
        }

        // 独立修复旧版本留下的大阶段检查/托管矛盾状态。
        chain = chain.then((p: Project) =>
          invokeWithTimeout<RuntimeMutationResult>("reconcile_managed_milestone_state_runtime", {
            projectName: p.name,
          }).then((result) => {
            applyRuntimeMutation(result);
            return result.runtime_snapshot.project;
          }).catch((err) => {
            console.error("大阶段托管状态对账失败:", err);
            return p;
          })
        );

        // 启动时对账执行状态：清理 stale session、修复工作流状态
        chain = chain.then((p: Project) =>
          invokeWithTimeout<RuntimeMutationResult>("reconcile_on_startup_runtime", {
            projectName: p.name,
          }).then((result) => {
            applyRuntimeMutation(result);
            return result.runtime_snapshot.project;
          }).catch((err) => {
            console.error("启动执行状态对账失败:", err);
            return p;
          })
        );

        return chain;
      })
      .then((project: Project | null) => {
        // null means the previous .then() bailed out — don't continue
        if (project === null) return null;

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
        // null means the previous .then() bailed out — don't continue
        if (snapshot === null) return;

        if (snapshot && snapshot.ui) {
          const ui = snapshot.ui;
          if (ui.view_phase === 'execution') {
            setViewMode({ phase: 'execution', reason: 'active' });
          }
          if (
            !sidebarWidthWasStored.current
            && typeof ui.sidebar_width === "number"
          ) {
            const migratedWidth = clampPanelWidth(
              ui.sidebar_width,
              MIN_SIDEBAR_WIDTH,
              MAX_SIDEBAR_WIDTH,
            );
            setSidebarWidth(migratedWidth);
            localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(migratedWidth));
            sidebarWidthWasStored.current = true;
          }
        }
        // 恢复后进入默认讨论模式
        if (project) {
          enterDiscussionMode('idle');
        }
        setStartupRecoveryDone(true);
      })
      .catch((err) => {
        console.error("获取项目失败:", err);
        setProject(null);
        localStorage.removeItem("metheus_last_project");
        setStartupRecoveryDone(true);
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
    await invokeRuntimeMutation("select_milestone_runtime", {
      projectName: project.name,
      milestoneId: id,
    });
  };

  const handleSelectMidStage = async (id: string) => {
    if (!project || project.current_mid_stage_id === id) return;
    await invokeRuntimeMutation("select_mid_stage_runtime", {
      projectName: project.name,
      midStageId: id,
    });
  };
  // 生成版本方案（V1: 后端校验三项检查 → 返回完整 Project → PlanApproval 步骤）
  const handleGeneratePlan = async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("generate_plan");
    try {
      await invokeRuntimeMutation("generate_version_plan_runtime", {
        projectName: project.name,
        expectedDiscussionRevision: project.discussion_revision,
        expectedDataRevision: project.workflow_state.data_revision,
      });
    } catch (err) {
      console.error("生成方案失败", err);
      setFeedbackMsg({ type: "error", message: "生成方案失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  };
  // 启动托管层（ThreeChecks 后自动推进到大阶段批准）
  const handleStartManagedFlow = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("start_managed");
    try {
      await invokeRuntimeMutation("start_managed_flow_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "托管层已激活。将自动推进到 Console 并完成大阶段审批。" });
    } catch (err) {
      console.error("启动托管失败", err);
      setFeedbackMsg({ type: "error", message: "启动托管失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);
  // 批准方案（传入 draft_id 和 generation_revision）
  const handleApproveWithDraft = useCallback(async (draftId: string, generationRevision: number) => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("approve_plan");
    try {
      await invokeRuntimeMutation("approve_version_plan_runtime", {
        projectName: project.name,
        draftId: draftId,
        generationRevision: generationRevision,
      });
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
      await invokeRuntimeMutation("reject_version_plan_runtime", {
        projectName: project.name,
        draftId: draftId,
        feedback: feedback,
      });
      setFeedbackMsg({ type: "info", message: "方案已驳回，已返回讨论模式。" });
    } catch (err) {
      console.error("驳回失败:", err);
      setFeedbackMsg({ type: "error", message: "驳回失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  // 从检查、项目方案生成或方案审批返回 Discussion
  const handleReturnToDiscussion = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    const currentStep = project.workflow_state.current_step;
    if (currentStep !== "ThreeChecks" && currentStep !== "ProjectPlanGeneration" && currentStep !== "PlanApproval") return;
    setDecisionAction("return_to_discussion");
    try {
      await invokeRuntimeMutation("return_to_discussion_runtime", {
        projectName: project.name,
        sourceStep: currentStep,
        reason: "用户返回继续讨论",
      });
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
      await invokeRuntimeMutation("resume_plan_approval_runtime", {
        projectName: project.name,
      });
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
      await invokeRuntimeMutation("restart_discussion_from_approved_runtime", {
        projectName: project.name,
      });
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
      await invokeRuntimeMutation("start_preflight_check_runtime", {
        projectName: project.name,
      });
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
      await invokeRuntimeMutation("restart_checks_runtime", {
        projectName: project.name,
      });
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
      await invokeRuntimeMutation("enter_console_runtime", {
        projectName: project.name,
      });
    } catch (err) {
      console.error("进入控制台失败:", err);
      setFeedbackMsg({ type: "error", message: "进入控制台失败：" + String(err) });
    } finally {
      setDecisionAction(null);
    }
  }, [project, isDecisionSubmitting]);

  const isTerminalSubtask = (status: Milestone["subtasks"][number]["status"]): boolean =>
    status === "Passed" || status === "AcceptedDeviation" || status === "Skipped";

  // 判断一个大阶段在自身拓扑中的执行容器是否全部完成。
  const isMilestoneFullyCompleted = (milestone: Milestone): boolean => {
    if (milestone.mode === "Quick") {
      return milestone.subtasks.length > 0
        && milestone.subtasks.every(task => isTerminalSubtask(task.status));
    }
    return milestone.mid_stages.length > 0
      && milestone.mid_stages.every(mid => mid.status === "Completed");
  };

  // === V1 暂停决策：继续/调整/回退 ===
  const handleResolvePause = async (action: string) => {
    if (!project || !beginConsoleAction(`pause_${action}`)) return;
    try {
      await invokeRuntimeMutation("resolve_pause_decision_runtime", {
        projectName: project.name,
        action,
      });
      if (action === "continue") {
        setFeedbackMsg({ type: "info", message: "已恢复执行模式，可继续执行下一个小阶段。" });
      }
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "决策失败：" + String(err) });
      throw err;
    } finally {
      endConsoleAction();
    }
  };

  // === V1 回退预览 ===
  const handlePreviewRollback = async (checkpointSubtaskId: string): Promise<RollbackImpact | null> => {
    if (!project || !beginConsoleAction("rollback_preview")) return null;
    try {
      const impact = await invokeWithTimeout<RollbackImpact>("preview_rollback_impact", {
        projectName: project.name,
        checkpointSubtaskId,
      });
      return impact ?? null;
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "预览失败：" + String(err) });
      return null;
    } finally {
      endConsoleAction();
    }
  };

  // === V1 确认回退 ===
  const handleConfirmRollback = async (checkpointSubtaskId: string) => {
    if (!project || !beginConsoleAction("rollback_confirm")) return;
    try {
      await invokeRuntimeMutation("confirm_rollback_runtime", {
        projectName: project.name,
        checkpointSubtaskId,
      });
      setFeedbackMsg({ type: "success", message: "回退已完成。请重新生成执行计划。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "回退失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // V1: 回退后不自动触发生成。pendingRollbackGenerate 已移除。

  const handlePrepareExecutionWorkspace = async () => {
    if (!project || !beginConsoleAction("prepare_workspace")) return;
    try {
      await invokeRuntimeMutation("prepare_execution_workspace_runtime", {
        projectName: project.name,
      });
      const status = await invokeWithTimeout<ExecutionWorkspaceStatus>(
        "get_execution_workspace_status",
        { projectName: project.name },
      );
      setWorkspaceStatus(status);
      setFeedbackMsg({
        type: status.ready_for_new_execution ? "success" : "warning",
        message: status.status_message,
      });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "准备执行工作区失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleRefreshExecutionWorkspace = async () => {
    if (!project || !beginConsoleAction("refresh_workspace")) return;
    try {
      await invokeRuntimeMutation("refresh_execution_workspace_runtime", {
        projectName: project.name,
      });
      const status = await invokeWithTimeout<ExecutionWorkspaceStatus>(
        "get_execution_workspace_status",
        { projectName: project.name },
      );
      setWorkspaceStatus(status);
      setFeedbackMsg({
        type: status.ready_for_new_execution ? "success" : "warning",
        message: status.status_message,
      });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "刷新执行工作区失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 人工执行：执行当前小阶段 ===
  const handleExecuteCurrentSubtask = async () => {
    if (!project || !beginConsoleAction("execute_subtask")) return;
    try {
      const result = await invokeRuntimeMutation("execute_current_subtask_runtime", {
        projectName: project.name,
      });
      const status = result.runtime_snapshot.pipeline_state;
      if (!status || status.status !== "Running") {
        throw new Error("后端未返回已启动的执行状态，请同步后重试。");
      }
      setTerminalSyncPhase("idle");
      startExecutionPolling(project.name);
      setFeedbackMsg({ type: "info", message: "小阶段已启动，正在后台执行。" });
    } catch (err) {
      console.error("执行失败:", err);
      setFeedbackMsg({ type: "error", message: "执行失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 人工执行：确认通过 ===
  const handleConfirmSubtask = async () => {
    if (!project || !beginConsoleAction("confirm_subtask")) return;
    try {
      const result = await invokeRuntimeMutation("confirm_subtask_result_runtime", {
        projectName: project.name,
      });
      setExecutionStatus(null);
      setFeedbackMsg({
        type: "success",
        message: result.action.message || "小阶段已确认通过，Git 标签已创建。",
      });
    } catch (err) {
      try {
        await forceRuntimeSync();
      } catch (syncError) {
        console.error("确认失败后的状态同步失败:", syncError);
      }
      setFeedbackMsg({ type: "error", message: "确认失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleRetryGitConfirmation = async () => {
    if (!project || !beginConsoleAction("retry_git_confirmation")) return;
    try {
      const result = await invokeRuntimeMutation("retry_git_confirmation_runtime", {
        projectName: project.name,
      });
      setExecutionStatus(null);
      setFeedbackMsg({
        type: "success",
        message: result.action.message || "Git 确认已完成，代码与质量结果保持不变。",
      });
    } catch (err) {
      try {
        await forceRuntimeSync();
      } catch (syncError) {
        console.error("重新确认提交后的状态同步失败:", syncError);
      }
      setFeedbackMsg({ type: "error", message: "重新确认提交失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 人工执行：驳回 ===
  const handleRejectSubtask = async (reason: string) => {
    if (!project || !beginConsoleAction("reject_subtask")) return;
    try {
      await invokeRuntimeMutation("reject_subtask_result_runtime", {
        projectName: project.name,
        reason,
      });
      setExecutionStatus(null);
      setFeedbackMsg({ type: "warning", message: "小阶段已驳回：" + reason });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "驳回失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === 自动驾驶控制 ===
  const handleToggleAutopilot = async (active: boolean) => {
    if (!project || !beginConsoleAction(active ? "autopilot_start" : "autopilot_stop")) return;
    try {
      await invokeRuntimeMutation("toggle_autopilot_runtime", {
        projectName: project.name,
        active,
      });
      setFeedbackMsg({
        type: active ? "info" : "info",
        message: active ? "自动驾驶已激活。" : "自动驾驶已关闭。",
      });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "切换自动驾驶失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleStopManagedFlow = async () => {
    if (!project || !beginConsoleAction("managed_stop")) return;
    try {
      await invokeRuntimeMutation("stop_managed_flow_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "托管层已停止，当前步骤已交给手动处理。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "停止托管失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handlePauseManagedFlow = async () => {
    if (!project || !beginConsoleAction("managed_pause")) return;
    try {
      await invokeRuntimeMutation("pause_managed_flow_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "托管层已暂停。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "暂停托管失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleResumeManagedFlow = async () => {
    if (!project || !beginConsoleAction("managed_resume")) return;
    try {
      await invokeRuntimeMutation("resume_managed_flow_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "托管层已恢复。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复托管失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleAutopilotPauseNow = async () => {
    if (!project || !beginConsoleAction("autopilot_pause")) return;
    try {
      await invokeRuntimeMutation("autopilot_pause_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "自动驾驶已暂停。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "暂停失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleAutopilotPauseAfterCurrent = async () => {
    if (!project || !beginConsoleAction("autopilot_ed_stop")) return;
    try {
      await invokeRuntimeMutation("request_ed_stop_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "将在当前任务完成后暂停。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "ED Stop 失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleAutopilotResume = async () => {
    if (!project || !beginConsoleAction("autopilot_resume")) return;
    try {
      await invokeRuntimeMutation("autopilot_resume_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "info", message: "自动驾驶已恢复。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleRunAutomaticRecovery = async () => {
    if (!project || !beginConsoleAction("run_error_recovery")) return;
    try {
      const result = await invokeRuntimeMutation("run_error_recovery_runtime", {
        projectName: project.name,
      });
      const recoveryPending = result.runtime_snapshot.recovery_presentation.kind !== "None";
      setFeedbackMsg({
        type: recoveryPending ? "warning" : "success",
        message: result.action.message || (recoveryPending
          ? "自动恢复已完成本轮处理，但仍需人工处理。"
          : "自动恢复已完成。"),
      });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "自动恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleRegenerateInvalidPlan = async () => {
    if (!project || !beginConsoleAction("regenerate_invalid_plan")) return;
    try {
      const planTarget = resolvePlanTarget(project);
      if (!planTarget) throw new Error("当前计划目标不存在或拓扑不一致。");
      const source = project.workflow_state.current_step === "PlanApproving"
        ? "approval_rejected"
        : "check_failed";
      await invokeRuntimeMutation("regenerate_execution_plan_runtime", {
        projectName: project.name,
        expectedDataRevision: project.workflow_state.data_revision,
        expectedPlanDraftRevision: planTarget.planDraftRevision,
        feedback: "补全并校正每个小阶段的精确文件范围。",
        source,
      });
      setFeedbackMsg({ type: "success", message: "执行计划已重新生成，请重新检查。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "重新生成计划失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const executeAcknowledgedRecovery = async (expectedStateFingerprint: string) => {
    if (!project || !beginConsoleAction("acknowledge_recovery")) return;
    try {
      const result = await invokeRuntimeMutation("acknowledge_execution_recovery_runtime", {
        projectName: project.name,
        expectedStateFingerprint,
      });
      const workspace = await invokeWithTimeout<ExecutionWorkspaceStatus>(
        "get_execution_workspace_status",
        { projectName: project.name },
      ).catch(() => null);
      if (workspace) setWorkspaceStatus(workspace);
      setRecoveryImpact(null);
      setPendingRecoveryDecision(null);
      setFeedbackMsg({
        type: "success",
        message: result.action.message,
      });
    } catch (err) {
      // 不得先在前端清空失败状态；保持恢复面板并显示后端原始错误
      if (String(err).includes("预览已过期")) {
        setRecoveryImpact(null);
        setPendingRecoveryDecision(null);
        await forceRuntimeSync();
      }
      setFeedbackMsg({ type: "error", message: "恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // 基线恢复先预览影响；无破坏性变化时直接收口。
  const handleAcknowledgeExecutionRecovery = async () => {
    if (!project) return;
    if (!recoveryPresentation?.supports_preview) {
      setFeedbackMsg({ type: "error", message: "后端未授权此恢复动作的影响预览，已拒绝执行。" });
      return;
    }
    if (!beginConsoleAction("preview_execution_recovery")) return;
    let impact: ExecutionRecoveryImpact | null = null;
    try {
      impact = await invokeWithTimeout<ExecutionRecoveryImpact>(
        "preview_execution_recovery_impact",
        { projectName: project.name, action: "acknowledge_execution_recovery" },
      );
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复影响预览失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
    if (!impact) return;
    if (impact.has_destructive_changes) {
      setRecoveryImpact(impact);
      return;
    }
    await executeAcknowledgedRecovery(impact.state_fingerprint);
  };

  const executeHumanRecovery = async (
    resolution: RecoveryDecisionResolution,
    reason: string,
    acceptedCriteria: number[],
    expectedStateFingerprint?: string,
  ) => {
    if (!project || !beginConsoleAction(`human_recovery:${resolution}`)) return;
    try {
      const result = await invokeRuntimeMutation("resolve_human_recovery_runtime", {
        projectName: project.name,
        resolution,
        reason: reason.trim(),
        acceptedCriteria: acceptedCriteria.length > 0 ? acceptedCriteria : undefined,
        expectedStateFingerprint,
      });
      const updated = result.runtime_snapshot.project;
      const workspace = await invokeWithTimeout<ExecutionWorkspaceStatus>(
        "get_execution_workspace_status",
        { projectName: project.name },
      ).catch(() => null);
      if (workspace) setWorkspaceStatus(workspace);
      setRecoveryImpact(null);
      setPendingRecoveryDecision(null);
      setFeedbackMsg({
        type: updated.workflow_state.recovery_state ? "warning" : "success",
        message: result.action.message,
      });
    } catch (err) {
      if (String(err).includes("预览已过期")) {
        setRecoveryImpact(null);
        setPendingRecoveryDecision(null);
        await forceRuntimeSync();
      }
      setFeedbackMsg({ type: "error", message: "人工恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleResolveHumanRecovery = async (
    resolution: RecoveryDecisionResolution,
    reason: string,
    acceptedCriteria: number[],
  ) => {
    if (!project) return;
    if (!(["restore_and_retry", "skip_task"] as RecoveryDecisionResolution[]).includes(resolution)) {
      await executeHumanRecovery(resolution, reason, acceptedCriteria);
      return;
    }
    if (!beginConsoleAction(`preview_human_recovery:${resolution}`)) return;
    let impact: ExecutionRecoveryImpact | null = null;
    try {
      impact = await invokeWithTimeout<ExecutionRecoveryImpact>(
        "preview_execution_recovery_impact",
        { projectName: project.name, action: resolution },
      );
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复影响预览失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
    if (!impact) return;
    if (impact.has_destructive_changes) {
      setPendingRecoveryDecision({ resolution, reason, acceptedCriteria });
      setRecoveryImpact(impact);
      return;
    }
    await executeHumanRecovery(
      resolution,
      reason,
      acceptedCriteria,
      impact.state_fingerprint,
    );
  };

  // === V1 手动同步项目状态（不依赖浏览器 reload） ===
  const handleSyncProject = async () => {
    if (!project || !beginConsoleAction("sync_project")) return;
    try {
      const snapshot = await forceRuntimeSync();
      if (!snapshot) throw new Error("无法取得最新运行时快照");
      const result = await invokeRuntimeMutation("reconcile_managed_milestone_state_runtime", {
        projectName: project.name,
      });
      const currentStep = result.runtime_snapshot.project.workflow_state.current_step;
      if (["PlanApproving", "Execution"].includes(currentStep)) {
        const status = await invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", {
          projectName: project.name,
        });
        setWorkspaceStatus(status);
      }
      setFeedbackMsg({ type: "info", message: "项目状态已同步。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "同步项目状态失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 A/B/C 大阶段审阅 ===
  // V1: enter_milestone_review is called via invokeWithTimeout directly when needed

  const handleApproveMilestoneOutcome = async (submission: MilestoneReviewSubmission) => {
    const branch = submission.branch;
    if (!project || !beginConsoleAction(`milestone_review_${branch}`)) return;
    try {
      const result = await invokeRuntimeMutation("approve_milestone_outcome_runtime", {
        projectName: project.name,
        submission,
      });
      const updated = result.runtime_snapshot.project;
      const messages: Record<string, string> = {
        A: updated.workflow_state.current_step === "Completed"
          ? "最后一个大阶段已批准，项目流程已完成。"
          : "大阶段已批准，已进入下一大阶段。",
        B: "已进入修正过去流程，自动驾驶保持暂停。",
        C: "已进入调整未来流程，自动驾驶保持暂停。",
      };
      setFeedbackMsg({ type: "success", message: messages[branch] ?? "大阶段审阅决策已提交。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "决策失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleSuggestRollback = async () => {
    if (!project || !beginConsoleAction("suggest_rollback")) return;
    try {
      const suggestion = await invokeWithTimeout<string>("suggest_rollback_checkpoint", { projectName: project.name });
      handleAddMessage({ id: `sys-${Date.now()}`, role: "assistant", content: suggestion, timestamp: Date.now() });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "建议生成失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleGenerateFutureMilestones = async () => {
    if (!project || !beginConsoleAction("generate_future_milestones")) return;
    try {
      await invokeRuntimeMutation("generate_future_milestone_draft_runtime", { projectName: project.name });
      setFeedbackMsg({ type: "success", message: "未来大阶段草稿已生成。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "生成失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleApproveFutureMilestones = async () => {
    if (!project || !beginConsoleAction("approve_future_milestones")) return;
    try {
      await invokeRuntimeMutation("approve_future_milestones_runtime", {
        projectName: project.name,
      });
      setFeedbackMsg({ type: "success", message: "未来规划已批准。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "批准失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

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

  // 启动恢复尚未完成且存在可能恢复的项目记录时，短暂等待以避免闪回 ProjectEntry
  const hasStoredProject = !!localStorage.getItem("metheus_last_project");
  if (!startupRecoveryDone && hasStoredProject) {
    return <div className="app-shell"><div className="loading-hint">正在恢复项目状态…</div></div>;
  }

  if (!project) {
    return <ProjectEntry onProjectCreated={handleProjectCreated} />;
  }

  const currentThread = project.discussion_threads.find(
    thread => thread.id === project.workflow_state.active_discussion_thread_id
  );
  if (!currentThread) {
    return <div className="app-shell"><div className="loading-hint">讨论线程状态需要同步</div></div>;
  }

  // Determine which main panel to show based on workflow_state
  const phase = project.workflow_state.top_level_phase;
  const step = project.workflow_state.current_step;

  // Before phase: show ExistingBaselinePanel for Half Project analysis
  if (phase === "Before" && (step === "ExistingAnalysis" || step === "BaselineApproval")) {
    return <ExistingBaselinePanel
      projectName={project.name}
      projectPath={project.project_path}
      onBaselineApproved={(result) => {
        applyRuntimeMutation(result);
        setProjectPath(result.runtime_snapshot.project.project_path);
      }}
      onReject={() => {
        localStorage.removeItem("metheus_last_project");
        setProject(null);
      }}
    />;
  }

  return (
    <div
      className="app-layout"
      data-console-inspector-open={phase === "Console" && inspectorOpen ? "true" : "false"}
    >
      {project.milestones.length > 0 && phase !== "Console" && (
        <aside className="sidebar" style={{ width: sidebarWidth + 'px' }}>
          <ExecutionTree
            project={project}
            onSelectMilestone={handleSelectMilestone}
            projectPath={projectPath}
            onSelectMidStage={handleSelectMidStage}
            selectedTaskId={taskControlWorkspace.selectedTaskId}
            currentTaskId={taskControlWorkspace.snapshot?.current_task_id}
            onOpenTask={openTaskInspector}
          />
          <div
            className={`resize-handle${isDragging ? ' dragging' : ''}`}
            onMouseDown={handleResizeMouseDown}
            onDoubleClick={() => setSidebarWidth(DEFAULT_SIDEBAR_WIDTH)}
            onKeyDown={handleSidebarSeparatorKeyDown}
            role="separator"
            aria-label="调整任务树宽度"
            aria-orientation="vertical"
            aria-valuemin={MIN_SIDEBAR_WIDTH}
            aria-valuemax={MAX_SIDEBAR_WIDTH}
            aria-valuenow={sidebarWidth}
            tabIndex={0}
          />
        </aside>
      )}

      <main className="main-content">
        {phase !== "Console" && <div className="project-utility-bar">
          <SyncStatusIndicator
            state={projectStateSync.state}
            onRetry={forceRuntimeSync}
            terminalPhase={terminalSyncPhase}
          />
          <ApplicationSettings
            project={project}
            pipeline={executionStatus}
            onRuntimeMutation={applyRuntimeMutation}
          />
        </div>}

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
                onRuntimeMutation={applyRuntimeMutation}
                onReturnToDiscussion={handleReturnToDiscussion}
                onAllPassed={handleGeneratePlan}
                onRestartChecks={handleRestartChecks}
                isSubmitting={isDecisionSubmitting}
                onStartManagedFlow={handleStartManagedFlow}
                managedFlowActive={project.workflow_state.managed_flow_state?.active === true}
              />
            )}

            {step === "ProjectPlanGeneration" && (
              <div className="preflight-panel project-plan-generation-panel">
                <h2>项目方案生成</h2>
                <div className="workflow-action-row">
                  <ActionButton
                    icon={<WandSparkles size={16} />}
                    onClick={handleGeneratePlan}
                    disabled={isDecisionSubmitting}
                  >
                    生成项目方案草稿
                  </ActionButton>
                  <ActionButton
                    icon={<ArrowLeft size={16} />}
                    variant="ghost"
                    onClick={handleReturnToDiscussion}
                    disabled={isDecisionSubmitting}
                  >
                    返回讨论
                  </ActionButton>
                  {!project.workflow_state.managed_flow_state?.active && (
                    <ActionButton
                      icon={<Bot size={16} />}
                      variant="secondary"
                      onClick={handleStartManagedFlow}
                      disabled={isDecisionSubmitting}
                    >
                      启动托管
                    </ActionButton>
                  )}
                </div>
              </div>
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
                onRuntimeMutation={applyRuntimeMutation}
              />
            )}
          </div>
        )}

        {(phase === "Console") && (
          <ConsoleWorkspace
            commandBar={(
              <ConsoleCommandBar>
                <AutopilotControlBar
                  project={project}
                  recoveryPresentation={recoveryPresentation}
                  executionStatus={executionStatus}
                  busy={isConsoleBusy}
                  writeDisabled={!consoleWritePolicy.writable}
                  writeDisabledReason={consoleWritePolicy.reason}
                  onToggle={handleToggleAutopilot}
                  onPauseManagedFlow={handlePauseManagedFlow}
                  onResumeManagedFlow={handleResumeManagedFlow}
                  onStopManagedFlow={handleStopManagedFlow}
                  onPauseNow={handleAutopilotPauseNow}
                  onPauseAfterCurrent={handleAutopilotPauseAfterCurrent}
                  onResume={handleAutopilotResume}
                  onSync={handleSyncProject}
                  onAcknowledgeRecovery={handleAcknowledgeExecutionRecovery}
                  onRegeneratePlan={handleRegenerateInvalidPlan}
                  onPrepareWorkspace={handlePrepareExecutionWorkspace}
                  onRefreshWorkspace={handleRefreshExecutionWorkspace}
                  onRetryGitConfirmation={handleRetryGitConfirmation}
                  onRunAutomaticRecovery={handleRunAutomaticRecovery}
                  onResolveHumanRecovery={handleResolveHumanRecovery}
                />
                <div className="console-command-tools">
                  <span className={`console-sync-state ${projectStateSync.state.status}`}>
                    {consoleWritePolicy.writable ? "状态已同步" : consoleWritePolicy.reason}
                  </span>
                  <button
                    type="button"
                    className={`icon-button${inspectorOpen ? " active" : ""}`}
                    onClick={() => setInspectorOpen(open => !open)}
                    title={inspectorOpen ? "关闭任务检查器" : "打开任务检查器"}
                    aria-label={inspectorOpen ? "关闭任务检查器" : "打开任务检查器"}
                    aria-expanded={inspectorOpen}
                    aria-controls="task-inspector"
                  >
                    <PanelRightOpen size={16} />
                  </button>
                  <ApplicationSettings
                    project={project}
                    pipeline={executionStatus}
                    writeBlockedReason={consoleWritePolicy.writable ? "" : consoleWritePolicy.reason}
                    onRuntimeMutation={applyRuntimeMutation}
                  />
                </div>
              </ConsoleCommandBar>
            )}
            navigator={(
              <ConsoleNavigator
                taskTree={(
                  <ExecutionTree
                    project={project}
                    onSelectMilestone={handleSelectMilestone}
                    projectPath={projectPath}
                    onSelectMidStage={handleSelectMidStage}
                    selectedTaskId={taskControlWorkspace.selectedTaskId}
                    currentTaskId={taskControlWorkspace.snapshot?.current_task_id}
                    onOpenTask={openTaskInspector}
                  />
                )}
                fileTree={<FileTree projectPath={projectPath} />}
              />
            )}
            bottom={step === "Execution" ? (
              <ConsoleBottomPanel>
                <TaskConsole
                  projectPath={projectPath}
                  projectName={project.name}
                  executionStatus={executionStatus}
                  testLogs={[]}
                  workspaceReady={workspaceStatus?.git_metadata_ready === true}
                  executionHistory={project.execution_history}
                  verificationStage={project.execution_session?.verification_stage}
                  validationRetryCount={project.workflow_state.recovery_state?.validation_retry_count}
                  validationRetryLimit={project.workflow_state.recovery_state?.max_validation_retries}
                  nextValidationRetryAt={project.workflow_state.recovery_state?.next_validation_retry_at}
                  recoveryPresentation={recoveryPresentation}
                  selectedTaskId={taskControlWorkspace.selectedTaskId}
                  onOpenTask={openTaskInspector}
                />
              </ConsoleBottomPanel>
            ) : undefined}
          >
            <div className="execution-main">
              <RecoveryResultBanner
                result={recoveryResult}
                onDismiss={dismissRecoveryResult}
              />
              <RecoveryImpactDialog
                impact={recoveryImpact}
                busy={isConsoleBusy}
                onCancel={() => {
                  setRecoveryImpact(null);
                  setPendingRecoveryDecision(null);
                }}
                onConfirm={() => {
                  if (!recoveryImpact) return;
                  if (pendingRecoveryDecision) {
                    void executeHumanRecovery(
                      pendingRecoveryDecision.resolution,
                      pendingRecoveryDecision.reason,
                      pendingRecoveryDecision.acceptedCriteria,
                      recoveryImpact.state_fingerprint,
                    );
                  } else {
                    void executeAcknowledgedRecovery(recoveryImpact.state_fingerprint);
                  }
                }}
              />
              {feedbackMsg && (
                <FeedbackBanner
                  type={feedbackMsg.type}
                  message={feedbackMsg.message}
                  style={{ marginBottom: "16px", flexShrink: 0 }}
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
                  onRuntimeMutation={applyRuntimeMutation}
                  externalBusy={isConsoleBusy || !consoleWritePolicy.writable}
                  onActionStart={beginConsoleAction}
                  onActionEnd={endConsoleAction}
                  onFeedback={setFeedbackMsg}
                  workspaceStatus={workspaceStatus}
                  onPrepareWorkspace={handlePrepareExecutionWorkspace}
                />
              )}
              {/* V1 执行阶段 UI — 仅在 Execution 步骤渲染 */}
              {step === "Execution" && (
                <V1ExecutionPanel
                    project={project}
                    recoveryPresentation={recoveryPresentation}
                    executionStatus={executionStatus}
                    workspaceStatus={workspaceStatus}
                    busy={
                      isConsoleBusy
                      || !consoleWritePolicy.writable
                      || (project.workflow_state.autopilot_active === true
                        && project.workflow_state.autopilot_state?.run_status === "Running")
                    }
                    onPrepareWorkspace={handlePrepareExecutionWorkspace}
                    onExecute={handleExecuteCurrentSubtask}
                    onConfirm={handleConfirmSubtask}
                    onReject={handleRejectSubtask}
                  />
              )}
              {/* V1 暂停决策 */}
              {step === "PauseDecision" && (
                <PauseDecisionPanel
                  pauseType={project.pause_context?.pause_type === "ed_stop" ? "ed_stop" : "in_stop"}
                  onContinue={() => handleResolvePause("continue")}
                  onAdjustOnly={() => handleResolvePause("adjust")}
                  onRollback={() => handleResolvePause("rollback")}
                  busy={isConsoleBusy}
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
                  milestone={project.milestones.find(m => m.id === project.current_milestone_id)!}
                  projectRevision={project.workflow_state.data_revision}
                  onSubmit={handleApproveMilestoneOutcome}
                  busy={isConsoleBusy}
                />
              )}
              {/* V1 分支讨论 (B/C) */}
              {step === "BranchDiscussion" && project.workflow_state.discussion_scope !== "AdjustFuture" && (
                <BranchDiscussionPanel
                  project={project}
                  onSuggestRollback={handleSuggestRollback}
                  onGenerateFuture={handleGenerateFutureMilestones}
                  onChatComplete={handleChatComplete}
                  onAddMessage={handleAddMessage}
                />
              )}
              {(step === "FuturePlanApproval" || (step === "BranchDiscussion" && project.workflow_state.discussion_scope === "AdjustFuture")) && (
                <FuturePlanningWorkspace
                  project={project}
                  busy={isConsoleBusy}
                  onGenerate={handleGenerateFutureMilestones}
                  onApprove={handleApproveFutureMilestones}
                  onProjectUpdated={handleChatComplete}
                  onAddMessage={handleAddMessage}
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
                  <p>当前步骤：{step}。请使用顶部命令栏同步项目状态后重试。</p>
                </div>
              )}
            </div>
          </ConsoleWorkspace>
        )}

        {phase === "Completed" && (
          <div className="completed-view" style={{ padding: "40px", textAlign: "center" }}>
            <h2>✅ 项目已完成</h2>
            <p style={{ color: "#656d76" }}>所有大阶段已执行完毕。</p>
          </div>
        )}

        {/* ===== Floating chat balloon in console mode ===== */}
        {phase === "Console" && (
          <div
            className="console-floating-chat-layer"
            style={{ position: "relative", zIndex: CONSOLE_LAYOUT_CONTRACT.floatingLayerMaximum }}
          >
            <FloatingChatBalloon messages={currentThread.messages || []} />
          </div>
        )}
      </main>
      {phase === "Console" && inspectorOpen && (
        <>
          <button
            type="button"
            className="task-inspector-backdrop"
            style={{ zIndex: CONSOLE_LAYOUT_CONTRACT.inspectorBackdropLayer }}
            onClick={() => setInspectorOpen(false)}
            aria-label="关闭任务检查器"
          />
          <div
            className={`task-inspector-resize-handle${isInspectorDragging ? " dragging" : ""}`}
            style={{ zIndex: CONSOLE_LAYOUT_CONTRACT.inspectorResizeLayer }}
            onPointerDown={handleInspectorPointerDown}
            onDoubleClick={() => setInspectorWidth(DEFAULT_INSPECTOR_WIDTH)}
            onKeyDown={handleInspectorSeparatorKeyDown}
            role="separator"
            aria-label="调整任务检查器宽度"
            aria-orientation="vertical"
            aria-valuemin={MIN_INSPECTOR_WIDTH}
            aria-valuemax={MAX_INSPECTOR_WIDTH}
            aria-valuenow={inspectorWidth}
            aria-controls="task-inspector"
            tabIndex={0}
          />
          <div
            className="task-inspector-shell"
            style={{
              width: `${inspectorWidth}px`,
              zIndex: CONSOLE_LAYOUT_CONTRACT.inspectorLayer,
            }}
          >
            <TaskInspector
              project={project}
              snapshot={taskControlWorkspace.snapshot}
              selectedNode={taskControlWorkspace.selectedNode}
              selectedTaskId={taskControlWorkspace.selectedTaskId}
              busy={isConsoleBusy || taskControlWorkspace.busy}
              error={taskControlWorkspace.error}
              recoveryPresentation={recoveryPresentation}
              expectedEventSequence={projectStateSync.state.taskControlEventSequence}
              detailsSyncing={taskControlWorkspace.detailsSyncing}
              onClose={() => setInspectorOpen(false)}
              onRefresh={() => { void taskControlWorkspace.refresh(); }}
              onAction={(name, options) => { void runTaskControlAction(name, options); }}
              onConfirmHumanReview={(criterionIndex, reason) => {
                void handleResolveHumanRecovery("confirm_actual_pass", reason, [criterionIndex]);
              }}
              onChangeMode={(mode, reason) => { void changeTaskControlMode(mode, reason); }}
            />
          </div>
        </>
      )}

    </div>
  );
}

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
  const activeThread = project.discussion_threads.find(
    thread => thread.id === project.workflow_state.active_discussion_thread_id
  );

  if (!activeThread) {
    return <div className="loading-hint">讨论线程状态需要同步</div>;
  }

  return (
    <ConsoleStepShell icon={isFixPast ? <RotateCcw /> : <GitBranch />}
      title={isFixPast ? "B 分支：修正过去" : "C 分支：调整未来"}
      description={isFixPast ? "分析执行证据并建议稳定回退点" : "保留已完成大阶段并调整后续"}
      status="pending" statusLabel="讨论中"
      actions={<WorkflowActionBar>{isFixPast ? (
        <ActionButton icon={<Search size={16} />} variant="danger" onClick={onSuggestRollback}>诊断并建议回退点</ActionButton>
      ) : (
        <ActionButton icon={<WandSparkles size={16} />} onClick={onGenerateFuture}>生成后续大阶段草稿</ActionButton>
      )}</WorkflowActionBar>}>
      <ChatRoom
        messages={activeThread.messages}
        onAddMessage={onAddMessage}
        projectName={project.name}
        currentRole="产品经理"
        threadId={activeThread.id}
        onProjectUpdated={onChatComplete}
      />
    </ConsoleStepShell>
  );
}

export default App;
