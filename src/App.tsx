// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useCallback, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { executionPollingOwnsNextAdvance } from "./autopilotPolicy";
import { getWorkspaceAction } from "./workspacePolicy";
import "./App.css";
import { Project, ViewMode, DiscussionReason, PipelineState, TestLog, ChatMessage, Milestone, RollbackImpact, WorkflowStep, ExecutionWorkspaceStatus, AutopilotNextStep, TestResult } from "./types";
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
import { Check, GitBranch, ListTodo, Pause, Play, RefreshCw, RotateCcw, Search, Square, WandSparkles, X } from "lucide-react";
import { AutopilotControlBar } from "./components/AutopilotControlBar";
import ExecutionTree from "./ExecutionTree";
import ChatRoom from "./ChatRoom";
import TaskConsole from "./TaskConsole";
import { ConsoleWorkflowPanel } from "./ConsoleWorkflowPanel";
import { PauseDecisionPanel } from "./PauseDecisionPanel";
import { RollbackImpactDialog } from "./RollbackImpactDialog";
import { MilestoneReviewPanel } from "./MilestoneReviewPanel";
import { ExecutionEngineSettings } from "./components/ExecutionEngineSettings";
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

/** 自动驾驶两个原子动作之间的等待周期（ms） */
const AUTOPILOT_STEP_DELAY_MS = 1000;

/** 执行状态轮询周期（ms） */
const EXECUTION_POLL_INTERVAL_MS = 1500;

/** 连续轮询失败最大次数，防止界面无限静默等待 */
const EXECUTION_POLL_MAX_FAILURES = 10;

function verificationLabel(result: TestResult): string {
  if (result.automated_test_status === "NotConfigured") {
    return result.passed ? "未配置自动化测试，代码审查通过" : "未配置自动化测试，代码审查未通过";
  }
  if (result.automated_test_status === "Unavailable") {
    return "测试环境不可用";
  }
  if (result.verification_kind === "CodeReviewOnly") {
    return result.passed ? "仅代码审查通过" : "代码审查未通过";
  }
  return result.passed ? "自动化测试与代码审查通过" : "未通过";
}

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
  // === 启动恢复完成标记（防止 UI 在恢复完成前渲染） ===
  const [startupRecoveryDone, setStartupRecoveryDone] = useState(false);
  // === 决策层统一提交锁（同一时间只能执行一个关键动作） ===
  const [decisionAction, setDecisionAction] = useState<string | null>(null);
  const isDecisionSubmitting = decisionAction !== null;
  const [consoleAction, setConsoleAction] = useState<string | null>(null);
  const consoleActionRef = useRef<string | null>(null);
  const beginConsoleAction = useCallback((action: string) => {
    if (consoleActionRef.current !== null) return false;
    consoleActionRef.current = action;
    setConsoleAction(action);
    return true;
  }, []);
  const endConsoleAction = useCallback(() => {
    consoleActionRef.current = null;
    setConsoleAction(null);
  }, []);
  const isConsoleBusy = consoleAction !== null;

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
    // Guard: don't recover execution until startup recovery is complete.
    // reconcile_on_startup must finish first to clean stale sessions.
    if (!startupRecoveryDone) return;

    const session = project.execution_session;
    if (!session || !session.active) {
      // No active session — clear any stale execution state
      setExecutionStatus(null);
      setIsExecuting(false);
      return;
    }

    // Recover based on disk session status
    if (session.status === "executing" || session.status === "recovering") {
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
            // Memory state lost. reconcile_on_startup already ran and
            // should have marked the session as "session_lost".
            // Reload from disk to get the reconciled state.
            invokeWithTimeout<Project>("get_project", { projectName: project.name })
              .then((p) => handleChatComplete(p))
              .catch(() => {
                // Last resort: show recovery message from session data
                setFeedbackMsg({
                  type: "warning",
                  message: `执行状态已丢失 (${session.subtask_title})，请手动继续。`,
                });
              });
          }
        })
        .catch(() => {
          // Can't reach backend — reload project from disk
          invokeWithTimeout<Project>("get_project", { projectName: project.name })
            .then((p) => handleChatComplete(p))
            .catch(() => {
              setFeedbackMsg({
                type: "warning",
                message: "无法连接后端，请重启应用。",
              });
            });
        });
    } else if (session.status === "awaiting_confirmation") {
      // Try backend memory first — may have richer subtask_statuses/log_history.
      // Fall back to a minimal display state from session data if backend memory is gone.
      invokeWithTimeout<PipelineState | null>("get_execution_status")
        .then((memStatus) => {
          if (memStatus && memStatus.awaiting_confirmation) {
            // Backend memory has the full state — use it
            setExecutionStatus(memStatus);
          } else {
            // Backend memory lost. Build minimal display state from disk session.
            // subtask_statuses/log_history are lost (only in memory), but UI can still
            // show the confirmation prompt from session data.
            setExecutionStatus({
              execution_id: session.execution_id,
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
          }
          setIsExecuting(false);
        })
        .catch(() => {
          // Can't reach backend — minimal fallback
          setExecutionStatus({
            execution_id: session.execution_id,
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
        });
    } else if (
      session.status === "session_lost"
      || session.status === "execution_failed"
      || session.status === "stop_failed"
    ) {
      // 失败/失联会话：仅提示需要恢复基线，不得谎称已恢复到安全状态
      setExecutionStatus(null);
      setIsExecuting(false);
      setFeedbackMsg({
        type: "warning",
        message: `执行中断 (${session.subtask_title})：${session.failure_message || "请先恢复执行基线后再继续。"}`,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.name, project?.execution_session?.active, project?.execution_session?.status, project?.execution_session?.subtask_id, startupRecoveryDone]);

  // Fetch workspace status before plan approval and throughout execution.
  useEffect(() => {
    if (!project || !["PlanApproving", "Execution"].includes(project.workflow_state.current_step)) return;
    invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", { projectName: project.name })
      .then(setWorkspaceStatus)
      .catch(() => setWorkspaceStatus(null));
  }, [project?.name, project?.workflow_state.current_step, project?.workflow_state.data_revision]);

  // === 自动驾驶驱动循环 — 使用 autopilot_next_step 逐步推进 ===
  // 覆盖范围：大阶段内部所有步骤（中阶段规划、执行计划、执行、确认）
  // 停止条件：大阶段边界（MilestoneReview）、出错、暂停
  const autopilotLoopRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const autopilotActiveRef = useRef(false);
  // 单飞锁：同一项目同一时刻只允许一个自动动作在途
  const autopilotGenerationRef = useRef(0);
  // 执行状态轮询失败计数器
  const executionPollFailuresRef = useRef(0);
  // 执行状态轮询定时器
  const executionPollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const executionPollingActiveRef = useRef(false);

  // === 托管层驱动循环 ===
  // 覆盖范围：ThreeChecks → PlanApproval → Console → MilestoneGeneration → MilestoneCheck → MilestoneApproval
  // 停止条件：reached_target、needs_human、is_error
  const managedLoopRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const managedActiveRef = useRef(false);

  /** 自动驾驶唯一驱动入口：建立代次和单飞锁，阻止重复循环和旧循环回写 */
  const driveAutopilot = useCallback(async (proj: Project) => {
    // 递增代次，使旧循环失效
    autopilotGenerationRef.current += 1;
    const myGen = autopilotGenerationRef.current;

    // 检查是否已有人在飞
    if (autopilotLoopRef.current) {
      clearTimeout(autopilotLoopRef.current);
      autopilotLoopRef.current = null;
    }

    const cycle = async (p: Project): Promise<void> => {
      // 代次失效检查
      if (autopilotGenerationRef.current !== myGen) return;
      if (!autopilotActiveRef.current) return;

      await runAutopilotCycle(p, myGen);
    };

    await cycle(proj);
  }, []);

  const runAutopilotCycle = useCallback(async (proj: Project, generation: number) => {
    if (!proj.workflow_state.autopilot_active) return;
    if (proj.workflow_state.top_level_phase !== "Console") return;

    const autopilotState = proj.workflow_state.autopilot_state;
    if (autopilotState) {
      if (autopilotState.run_status === "Paused") return;
      if (autopilotState.run_status === "WaitingMilestoneReview") return;
      if (autopilotState.run_status === "ErrorStopped") return;
    }

    const reschedule = (nextProj: Project) => {
      if (
        autopilotGenerationRef.current === generation &&
        autopilotActiveRef.current &&
        nextProj.workflow_state.autopilot_active &&
        nextProj.workflow_state.autopilot_state?.run_status === "Running" &&
        nextProj.workflow_state.top_level_phase === "Console"
      ) {
        autopilotLoopRef.current = setTimeout(() => {
          runAutopilotCycle(nextProj, generation);
        }, AUTOPILOT_STEP_DELAY_MS);
      }
    };

    try {
      const next = await invokeWithTimeout<AutopilotNextStep>("autopilot_next_step", { projectName: proj.name });

      // 代次失效检查
      if (autopilotGenerationRef.current !== generation) return;

      if (next.waiting_for_execution) {
        setIsExecuting(true);
        startExecutionPolling(proj.name, generation);
        return;
      }

      // 终止字段是后端强制契约，必须先于 result_kind 处理。
      if (next.is_error || !next.command) {
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        if (autopilotGenerationRef.current !== generation) return;
        handleChatComplete(latest);
        if (next.is_error) {
          setFeedbackMsg({
            type: "error",
            message: next.error_message || next.description || "自动驾驶已因未知错误停止。",
          });
        } else if (next.at_milestone_boundary) {
          setFeedbackMsg({
            type: "warning",
            message: next.description || "已到达大阶段边界，请完成人工审阅。",
          });
        } else {
          setFeedbackMsg({
            type: "info",
            message: next.description || "自动驾驶已暂停。",
          });
        }
        return;
      }

      // 与人工确认/执行/驳回共用 consoleAction 在途锁，禁止并发提交
      if (!beginConsoleAction(`autopilot:${next.command}`)) {
        reschedule(proj);
        return;
      }

      try {
        if (next.command === "run_error_recovery") {
          setFeedbackMsg({ type: "info", message: next.description || "正在执行自动修复。" });
        }
        switch (next.result_kind) {
          case "ProjectState": {
            if (!next.command) {
              const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
              handleChatComplete(latest);
              return;
            }
            const updated = await invokeWithTimeout<Project>(next.command, {
              ...next.args,
              projectName: proj.name,
            });
            if (autopilotGenerationRef.current !== generation) return;
            if (updated.workflow_state.data_revision >= (proj.workflow_state.data_revision ?? 0)) {
              handleChatComplete(updated);
            } else {
              const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
              handleChatComplete(latest);
            }
            break;
          }

          case "PipelineState": {
            if (!next.command) return;
            const pipelineState = await invokeWithTimeout<PipelineState>(next.command, {
              ...next.args,
              projectName: proj.name,
            });
            setExecutionStatus(pipelineState);
            setIsExecuting(pipelineState.status === "Running");
            if (
              executionPollingOwnsNextAdvance(pipelineState)
              && autopilotGenerationRef.current === generation
            ) {
              startExecutionPolling(proj.name, generation);
              return;
            }
            if (pipelineState.status !== "Running") {
              const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
              handleChatComplete(latest);
            }
            break;
          }

          case "WorkspaceState": {
            if (!next.command) return;
            const wsStatus = await invokeWithTimeout<ExecutionWorkspaceStatus>(next.command, {
              ...next.args,
              projectName: proj.name,
            });
            setWorkspaceStatus(wsStatus);
            const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
            handleChatComplete(latest);
            break;
          }

          case "NoResult": {
            const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
            handleChatComplete(latest);
            return;
          }
        }

        if (
          autopilotGenerationRef.current === generation &&
          autopilotActiveRef.current
        ) {
          const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
          if (
            latest.workflow_state.autopilot_active &&
            latest.workflow_state.autopilot_state?.run_status === "Running" &&
            latest.workflow_state.top_level_phase === "Console"
          ) {
            reschedule(latest);
          } else {
            handleChatComplete(latest);
          }
        }
      } finally {
        endConsoleAction();
      }
    } catch (error) {
      console.warn("[autopilot] Cycle error:", error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      try {
        const updated = await invokeWithTimeout<Project>("autopilot_mark_error", {
          projectName: proj.name,
          actionDescription: "自动驾驶循环异常",
          errorDetail: errorMsg.slice(0, 2048),
        });
        handleChatComplete(updated);
        setFeedbackMsg({ type: "error", message: `自动驾驶已停止：${errorMsg}` });
      } catch (markError) {
        console.error("[autopilot] Failed to persist error state:", markError);
        try {
          const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
          handleChatComplete(latest);
          setFeedbackMsg({ type: "error", message: "自动驾驶错误状态可能未落盘，请手动同步项目。" });
        } catch (_) {
          setFeedbackMsg({ type: "error", message: "自动驾驶异常且无法同步项目，请检查后端连接。" });
        }
      }
    }
  }, [beginConsoleAction, endConsoleAction]);

  /** 手动执行和自动驾驶共用的唯一执行状态轮询入口。 */
  const startExecutionPolling = useCallback(async (projectName: string, generation?: number) => {
    if (executionPollingActiveRef.current) return;
    executionPollingActiveRef.current = true;
    executionPollFailuresRef.current = 0;

    const poll = async () => {
      if (generation !== undefined && autopilotGenerationRef.current !== generation) {
        executionPollingActiveRef.current = false;
        return;
      }
      try {
        const status = await invokeWithTimeout<PipelineState | null>("get_execution_status", {});
        if (generation !== undefined && autopilotGenerationRef.current !== generation) {
          executionPollingActiveRef.current = false;
          return;
        }
        if (!status) {
          executionPollingActiveRef.current = false;
          setIsExecuting(false);
          setFeedbackMsg({
            type: "error",
            message: "执行状态已丢失。请同步项目状态后重试，避免重复启动任务。",
          });
          return;
        }

        executionPollFailuresRef.current = 0; // 重置失败计数
        setExecutionStatus(status);
        setIsExecuting(status.status === "Running");

        const newLogs: TestLog[] = [];
        for (const item of status.subtask_statuses ?? []) {
          if (processedSubtaskIdsRef.current.has(item.subtask_id)) continue;
          if (item.test_result && (item.status === "passed" || item.status === "retrying")) {
            processedSubtaskIdsRef.current.add(item.subtask_id);
            const testResult = item.test_result;
            const reason = testResult.passed
              ? ((testResult.issues ?? []).join("\n") || verificationLabel(testResult))
              : `不通过: ${testResult.suggestion || "未提供建议"}`;
            newLogs.push({
              subtask_title: item.title,
              status: item.status === "retrying" ? "retried" : "passed",
              reason,
              full_report: testResult.suggestion || undefined,
            });
          }
        }
        if (newLogs.length > 0) setTestLogs((previous) => [...previous, ...newLogs]);

        if (status.status === "Running") {
          executionPollTimerRef.current = setTimeout(poll, EXECUTION_POLL_INTERVAL_MS);
        } else {
          executionPollingActiveRef.current = false;
          executionPollTimerRef.current = null;
          const latest = await invokeWithTimeout<Project>("get_project", { projectName });
          handleChatComplete(latest);
          if (status.status === "Failed") {
            setFeedbackMsg({
              type: "error",
              message: status.last_error || "后台执行失败，请查看阶段日志后重试。",
            });
          }
          if (
            generation !== undefined &&
            autopilotGenerationRef.current === generation &&
            autopilotActiveRef.current &&
            latest.workflow_state.autopilot_active &&
            latest.workflow_state.autopilot_state?.run_status === "Running"
          ) {
            autopilotLoopRef.current = setTimeout(() => {
              runAutopilotCycle(latest, generation);
            }, AUTOPILOT_STEP_DELAY_MS);
          }
        }
      } catch (error) {
        executionPollFailuresRef.current += 1;
        if (executionPollFailuresRef.current >= EXECUTION_POLL_MAX_FAILURES) {
          executionPollingActiveRef.current = false;
          executionPollTimerRef.current = null;
          setIsExecuting(false);
          const pollError = error instanceof Error ? error.message : String(error);
          setFeedbackMsg({
            type: "error",
            message: `执行状态连续同步失败：${pollError}。请检查后端连接并手动同步项目。`,
          });
          try {
            const latest = await invokeWithTimeout<Project>("get_project", { projectName });
            handleChatComplete(latest);
          } catch (syncError) {
            console.error("执行轮询失败后同步项目失败:", syncError);
          }
          return;
        }
        executionPollTimerRef.current = setTimeout(poll, EXECUTION_POLL_INTERVAL_MS);
      }
    };

    poll();
  }, []);

  // 启动恢复只需恢复 isExecuting，此 effect 会接入同一轮询入口。
  useEffect(() => {
    if (!isExecuting || !project || executionPollingActiveRef.current) return;
    startExecutionPolling(project.name);
  }, [isExecuting, project?.name, startExecutionPolling]);

  // === 托管层驱动循环 ===
  // 调用 managed_next_step 获取下一步建议，执行返回的原子命令，循环推进
  const runManagedCycle = useCallback(async (proj: Project) => {
    if (!managedActiveRef.current) return;
    const mf = proj.workflow_state.managed_flow_state;
    if (!mf || !mf.active) return;
    if (mf.run_status === "Paused" || mf.run_status === "ErrorStopped" || mf.run_status === "WaitingHuman") return;

    try {
      const next = await invokeWithTimeout<{
        command: string;
        args: Record<string, unknown>;
        description: string;
        reached_target: boolean;
        needs_human: boolean;
        is_error: boolean;
        error_message: string;
      }>("managed_next_step", { projectName: proj.name });

      // Stop conditions
      if (next.reached_target) {
        // Stop managed flow to release the autopilot mutual exclusion lock
        try {
          const stopped = await invokeWithTimeout<Project>("stop_managed_flow", {
            projectName: proj.name,
          });
          handleChatComplete(stopped);
          setFeedbackMsg({
            type: "success",
            message: "托管完成：大阶段已批准。可启动自动驾驶继续推进中阶段和子任务。",
          });
        } catch (_) {
          // Fallback: sync and stop driving loop
          const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
          handleChatComplete(latest);
          setFeedbackMsg({
            type: "success",
            message: `托管完成：${next.description}`,
          });
        }
        return;
      }

      if (next.is_error) {
        setFeedbackMsg({
          type: "error",
          message: `托管错误：${next.error_message}`,
        });
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
        return;
      }

      if (next.needs_human) {
        // Persist a distinct WaitingHuman state and keep the actual blocker visible.
        try {
          await invokeWithTimeout<Project>("wait_managed_flow_for_human", {
            projectName: proj.name,
            reason: next.description,
          });
        } catch { /* best effort */ }
        setFeedbackMsg({
          type: "info",
          message: `托管暂停：${next.description}`,
        });
        // Sync project to reflect the paused state
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
        return;
      }

      if (!next.command) {
        // No action — sync and retry
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
        if (managedActiveRef.current) {
          managedLoopRef.current = setTimeout(() => runManagedCycle(latest), 2000);
        }
        return;
      }

      // Execute the suggested command
      const updated = await invokeWithTimeout<Project>(next.command, {
        ...next.args,
        projectName: proj.name,
      });
      handleChatComplete(updated);

      if (next.command === "approve_milestone_draft"
          && updated.workflow_state.current_step === "MilestoneSelection"
          && !updated.workflow_state.managed_flow_state) {
        setFeedbackMsg({
          type: "success",
          message: "托管完成：大阶段已批准。可启动自动驾驶继续推进中阶段和子任务。",
        });
        return;
      }

      // Schedule next cycle
      if (managedActiveRef.current && updated.workflow_state.managed_flow_state?.active) {
        managedLoopRef.current = setTimeout(() => {
          runManagedCycle(updated);
        }, 1500);
      }
    } catch (error) {
      console.warn("[managed-flow] Cycle error:", error);
      // Sync and stop on error
      try {
        const latest = await invokeWithTimeout<Project>("get_project", { projectName: proj.name });
        handleChatComplete(latest);
      } catch (_) {}
    }
  }, []);

  // Start/stop autopilot loop when autopilot state changes
  useEffect(() => {
    if (!project) return;
    const active = project.workflow_state.autopilot_active === true;
    autopilotActiveRef.current = active;

    // Clear any pending loop and poll timer
    if (autopilotLoopRef.current) {
      clearTimeout(autopilotLoopRef.current);
      autopilotLoopRef.current = null;
    }
    if (executionPollTimerRef.current) {
      clearTimeout(executionPollTimerRef.current);
      executionPollTimerRef.current = null;
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

    // Start the drive loop via the unique entry point
    autopilotLoopRef.current = setTimeout(() => {
      driveAutopilot(project);
    }, 500);
  }, [
    project?.workflow_state?.autopilot_active,
    project?.workflow_state?.autopilot_state?.run_status,
    project?.workflow_state?.top_level_phase,
    project?.workflow_state?.current_step, // Re-check when step changes (e.g., PauseDecision → Execution)
  ]);

  // Start/stop managed flow loop when managed_flow_state changes
  useEffect(() => {
    if (!project) return;
    const mf = project.workflow_state.managed_flow_state;
    const active = mf?.active === true && mf?.run_status === "Running";
    managedActiveRef.current = active;

    // Clear any pending loop
    if (managedLoopRef.current) {
      clearTimeout(managedLoopRef.current);
      managedLoopRef.current = null;
    }

    if (!active) return;

    // Start the drive loop
    managedLoopRef.current = setTimeout(() => {
      runManagedCycle(project);
    }, 500);
  }, [
    project?.workflow_state?.managed_flow_state?.active,
    project?.workflow_state?.managed_flow_state?.run_status,
    project?.workflow_state?.current_step, // Re-check when step changes
  ]);

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
            invokeWithTimeout<Project>("migrate_project_workflow", {
              projectName: p.name,
            }).then((migrated) => {
              handleChatComplete(migrated);
              return migrated;
            }).catch((err) => {
              console.error("迁移旧项目工作流失败:", err);
              return p;
            })
          );
        }

        // 独立修复旧版本留下的大阶段检查/托管矛盾状态。
        chain = chain.then((p: Project) =>
          invokeWithTimeout<Project>("reconcile_managed_milestone_state", {
            projectName: p.name,
          }).then((reconciled) => {
            handleChatComplete(reconciled);
            return reconciled;
          }).catch((err) => {
            console.error("大阶段托管状态对账失败:", err);
            return p;
          })
        );

        // 启动时对账执行状态：清理 stale session、修复工作流状态
        chain = chain.then((p: Project) =>
          invokeWithTimeout<Project>("reconcile_on_startup", {
            projectName: p.name,
          }).then((reconciled) => {
            handleChatComplete(reconciled);
            return reconciled;
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
          if (typeof ui.sidebar_width === "number") {
            setSidebarWidth(Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, ui.sidebar_width)));
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
  // 启动托管层（ThreeChecks 后自动推进到大阶段批准）
  const handleStartManagedFlow = useCallback(async () => {
    if (!project || isDecisionSubmitting) return;
    setDecisionAction("start_managed");
    try {
      const updated = await invokeWithTimeout<Project>("start_managed_flow", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "托管层已激活。将自动推进到 Console 并完成大阶段审批。" });
      // The useEffect on managed_flow_state will automatically start the drive loop
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
    if (!project || !beginConsoleAction("in_stop")) return;
    try {
      const updated = await invokeWithTimeout<Project>("request_in_stop", { projectName: project.name });
      handleChatComplete(updated);
      setIsExecuting(false);
      setExecutionStatus(null);
      setFeedbackMsg({ type: "warning", message: "执行已暂停并恢复到安全基线。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "暂停失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 暂停：ED Stop ===
  const handleEdStop = async () => {
    if (!project || !beginConsoleAction("ed_stop")) return;
    try {
      const updated = await invokeWithTimeout<Project>("request_ed_stop", { projectName: project.name });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "将在当前任务完成并确认后暂停。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "ED Stop 请求失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 暂停决策：继续/调整/回退 ===
  const handleResolvePause = async (action: string) => {
    if (!project || !beginConsoleAction(`pause_${action}`)) return;
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
      const updated = await invokeWithTimeout<Project>("confirm_rollback", {
        projectName: project.name,
        checkpointSubtaskId,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "回退已完成。请重新生成执行计划。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "回退失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // V1: 回退后不自动触发生成。pendingRollbackGenerate 已移除。

  const refreshExecutionContext = async (projectName: string) => {
    const [projectResult, workspaceResult, pipelineResult] = await Promise.allSettled([
      invokeWithTimeout<Project>("get_project", { projectName }),
      invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", { projectName }),
      invokeWithTimeout<PipelineState | null>("get_execution_status", {}),
    ]);
    if (projectResult.status === "rejected") throw projectResult.reason;
    handleChatComplete(projectResult.value);
    if (workspaceResult.status === "fulfilled") setWorkspaceStatus(workspaceResult.value);
    if (pipelineResult.status === "fulfilled") {
      setExecutionStatus(pipelineResult.value);
      setIsExecuting(pipelineResult.value?.status === "Running");
    }
    return {
      project: projectResult.value,
      workspace: workspaceResult.status === "fulfilled" ? workspaceResult.value : null,
      pipeline: pipelineResult.status === "fulfilled" ? pipelineResult.value : null,
    };
  };

  const handlePrepareExecutionWorkspace = async () => {
    if (!project || !beginConsoleAction("prepare_workspace")) return;
    try {
      const status = await invokeWithTimeout<ExecutionWorkspaceStatus>("prepare_execution_workspace", {
        projectName: project.name,
      });
      await refreshExecutionContext(project.name);
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
      const status = await invokeWithTimeout<ExecutionWorkspaceStatus>("refresh_execution_workspace", {
        projectName: project.name,
      });
      await refreshExecutionContext(project.name);
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
      const status = await invokeWithTimeout<PipelineState>("execute_current_subtask", {
        projectName: project.name,
      });
      setExecutionStatus(status);
      setIsExecuting(status.status === "Running");
      if (status.status === "Running") {
        startExecutionPolling(project.name);
      }
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
      const updated = await invokeWithTimeout<Project>("confirm_subtask_result", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setExecutionStatus(null);
      setFeedbackMsg({ type: "success", message: "小阶段已确认通过，Git 标签已创建。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "确认失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 人工执行：驳回 ===
  const handleRejectSubtask = async (reason: string) => {
    if (!project || !beginConsoleAction("reject_subtask")) return;
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
    } finally {
      endConsoleAction();
    }
  };

  // === V1 人工执行：恢复基线并重试 ===
  const handleRetryCurrentSubtask = async () => {
    if (!project || !beginConsoleAction("retry_subtask")) return;
    try {
      const updated = await invokeWithTimeout<Project>("retry_current_subtask", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setExecutionStatus(null);
      // 只有后端返回后才刷新工作区；失败时不在前端清空失败状态
      try {
        const ws = await invokeWithTimeout<ExecutionWorkspaceStatus>("get_execution_workspace_status", {
          projectName: project.name,
        });
        setWorkspaceStatus(ws);
      } catch {
        /* 工作区探测失败不掩盖重试成功 */
      }
      setFeedbackMsg({ type: "info", message: "已恢复执行基线，可重新执行小阶段。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "重试失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === 自动驾驶控制 ===
  const handleToggleAutopilot = async (active: boolean) => {
    if (!project || !beginConsoleAction(active ? "autopilot_start" : "autopilot_stop")) return;
    try {
      const updated = await invokeWithTimeout<Project>("toggle_autopilot", {
        projectName: project.name,
        active,
      });
      handleChatComplete(updated);
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
      const updated = await invokeWithTimeout<Project>("stop_managed_flow", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "托管层已停止，当前步骤已交给手动处理。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "停止托管失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleAutopilotPauseNow = async () => {
    if (!project || !beginConsoleAction("autopilot_pause")) return;
    try {
      const updated = await invokeWithTimeout<Project>("autopilot_pause", {
        projectName: project.name,
      });
      handleChatComplete(updated);
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
      const updated = await invokeWithTimeout<Project>("request_ed_stop", {
        projectName: project.name,
      });
      handleChatComplete(updated);
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
      const updated = await invokeWithTimeout<Project>("autopilot_resume", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "info", message: "自动驾驶已恢复。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleRegenerateInvalidPlan = async () => {
    if (!project || !beginConsoleAction("regenerate_invalid_plan")) return;
    try {
      const milestone = project.milestones.find(item => item.id === project.current_milestone_id);
      const midStage = milestone?.mid_stages.find(item => item.id === project.current_mid_stage_id);
      if (!midStage) throw new Error("当前中阶段不存在。");
      const source = project.workflow_state.current_step === "PlanApproving"
        ? "approval_rejected"
        : "check_failed";
      const updated = await invokeWithTimeout<Project>("regenerate_execution_plan", {
        projectName: project.name,
        expectedDataRevision: project.workflow_state.data_revision,
        expectedPlanDraftRevision: midStage.plan_draft_revision,
        feedback: "补全并校正每个小阶段的精确文件范围。",
        source,
      });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "执行计划已重新生成，请重新检查。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "重新生成计划失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === 恢复执行基线：实际 Git 回退；失败时保留恢复面板与后端原始错误 ===
  const handleAcknowledgeExecutionRecovery = async () => {
    if (!project || !beginConsoleAction("acknowledge_recovery")) return;
    try {
      const updated = await invokeWithTimeout<Project>("acknowledge_execution_recovery", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      const refreshed = await refreshExecutionContext(project.name);
      setFeedbackMsg({
        type: refreshed.workspace?.working_tree_clean ? "success" : "warning",
        message: refreshed.workspace?.working_tree_clean
          ? "执行基线已恢复，自动驾驶将继续执行。"
          : "基线已恢复，但工作区仍有残留，请检查后再继续。",
      });
    } catch (err) {
      // 不得先在前端清空失败状态；保持恢复面板并显示后端原始错误
      setFeedbackMsg({ type: "error", message: "恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  const handleResolveHumanRecovery = async (
    resolution: "retest" | "restore_and_retry" | "regenerate_plan" | "human_override",
  ) => {
    if (!project || !beginConsoleAction(`human_recovery:${resolution}`)) return;
    try {
      let reason = "";
      if (resolution === "human_override") {
        reason = window.prompt("请填写人工核验通过的依据")?.trim() ?? "";
        if (!reason) return;
      }
      const updated = await invokeWithTimeout<Project>("resolve_human_recovery", {
        projectName: project.name,
        resolution,
        reason,
      });
      handleChatComplete(updated);
      await refreshExecutionContext(project.name);
      const messages = {
        retest: updated.workflow_state.recovery_state
          ? "重新测试仍未通过，继续等待人工处理。"
          : "重新测试通过，自动驾驶将继续执行。",
        restore_and_retry: "已恢复执行基线，将重新执行当前小阶段。",
        regenerate_plan: "已安排重新规划当前任务，自动驾驶将继续处理。",
        human_override: "人工核验已单独记录，自动驾驶将继续执行。",
      };
      setFeedbackMsg({
        type: updated.workflow_state.recovery_state ? "warning" : "success",
        message: messages[resolution],
      });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "人工恢复失败：" + String(err) });
    } finally {
      endConsoleAction();
    }
  };

  // === V1 手动同步项目状态（不依赖浏览器 reload） ===
  const handleSyncProject = async () => {
    if (!project || !beginConsoleAction("sync_project")) return;
    try {
      const updated = await invokeWithTimeout<Project>("reconcile_managed_milestone_state", {
        projectName: project.name,
      });
      handleChatComplete(updated);
      // Also refresh pipeline state if available
      const pipelineStatus = await invokeWithTimeout<PipelineState | null>("get_execution_status");
      if (pipelineStatus) {
        setExecutionStatus(pipelineStatus);
        if (pipelineStatus.awaiting_confirmation) {
          setIsExecuting(false);
        }
      }
      if (["PlanApproving", "Execution"].includes(updated.workflow_state.current_step)) {
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

  const handleApproveMilestoneOutcome = async (branch: string) => {
    if (!project || !beginConsoleAction(`milestone_review_${branch}`)) return;
    try {
      const updated = await invokeWithTimeout<Project>("approve_milestone_outcome", {
        projectName: project.name,
        branch,
      });
      handleChatComplete(updated);
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
      const updated = await invokeWithTimeout<Project>("generate_future_milestone_draft", { projectName: project.name });
      handleChatComplete(updated);
      setFeedbackMsg({ type: "success", message: "未来大阶段草稿已生成，请在 Console 中检查和批准。" });
    } catch (err) {
      setFeedbackMsg({ type: "error", message: "生成失败：" + String(err) });
    } finally {
      endConsoleAction();
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

  // 启动恢复尚未完成且存在可能恢复的项目记录时，短暂等待以避免闪回 ProjectEntry
  const hasStoredProject = !!localStorage.getItem("metheus_last_project");
  if (!startupRecoveryDone && hasStoredProject) {
    return <div className="app-shell"><div className="loading-hint">正在恢复项目状态…</div></div>;
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
        <div className="project-utility-bar">
          <ExecutionEngineSettings
            project={project}
            pipeline={executionStatus}
            onProjectUpdated={handleChatComplete}
          />
        </div>

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
                onStartManagedFlow={handleStartManagedFlow}
                managedFlowActive={project.workflow_state.managed_flow_state?.active === true}
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
              {/* 全局自动驾驶控制条 */}
              <AutopilotControlBar
                project={project}
                executionStatus={executionStatus}
                busy={isConsoleBusy}
                onToggle={handleToggleAutopilot}
                onStopManagedFlow={handleStopManagedFlow}
                onPauseNow={handleAutopilotPauseNow}
                onPauseAfterCurrent={handleAutopilotPauseAfterCurrent}
                onResume={handleAutopilotResume}
                onSync={handleSyncProject}
                onRetryCurrent={handleRetryCurrentSubtask}
                onAcknowledgeRecovery={handleAcknowledgeExecutionRecovery}
                onRegeneratePlan={handleRegenerateInvalidPlan}
                onPrepareWorkspace={handlePrepareExecutionWorkspace}
                onRefreshWorkspace={handleRefreshExecutionWorkspace}
                onResolveHumanRecovery={handleResolveHumanRecovery}
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
                  onProjectUpdated={handleChatComplete}
                  externalBusy={isConsoleBusy}
                  onActionStart={beginConsoleAction}
                  onActionEnd={endConsoleAction}
                  onFeedback={setFeedbackMsg}
                  workspaceStatus={workspaceStatus}
                  onPrepareWorkspace={handlePrepareExecutionWorkspace}
                  onRefreshWorkspace={handleSyncProject}
                />
              )}
              {/* V1 执行阶段 UI — 仅在 Execution 步骤渲染 */}
              {step === "Execution" && (
                <>
                  <V1ExecutionPanel
                    project={project}
                    executionStatus={executionStatus}
                    workspaceStatus={workspaceStatus}
                    busy={
                      isConsoleBusy
                      || (project.workflow_state.autopilot_active === true
                        && project.workflow_state.autopilot_state?.run_status === "Running")
                    }
                    onPrepareWorkspace={handlePrepareExecutionWorkspace}
                    onExecute={handleExecuteCurrentSubtask}
                    onConfirm={handleConfirmSubtask}
                    onReject={handleRejectSubtask}
                    onRetry={handleRetryCurrentSubtask}
                    onInStop={handleInStop}
                    onEdStop={handleEdStop}
                    onSyncProject={handleSyncProject}
                    onAcknowledgeRecovery={handleAcknowledgeExecutionRecovery}
                  />
                  <TaskConsole
                    projectPath={projectPath}
                    projectName={project.name}
                    executionStatus={executionStatus}
                    testLogs={testLogs}
                    workspaceReady={workspaceStatus?.git_metadata_ready === true}
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
                  milestoneTitle={project.milestones.find(m => m.id === project.current_milestone_id)?.title ?? "?"}
                  onContinue={() => handleApproveMilestoneOutcome("A")}
                  onFixPast={() => handleApproveMilestoneOutcome("B")}
                  onAdjustFuture={() => handleApproveMilestoneOutcome("C")}
                  busy={isConsoleBusy}
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
                  externalBusy={isConsoleBusy}
                  onActionStart={beginConsoleAction}
                  onActionEnd={endConsoleAction}
                  onFeedback={setFeedbackMsg}
                  workspaceStatus={workspaceStatus}
                  onPrepareWorkspace={handlePrepareExecutionWorkspace}
                  onRefreshWorkspace={handleSyncProject}
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
  project, executionStatus, workspaceStatus, busy: externalBusy,
  onPrepareWorkspace, onExecute, onConfirm, onReject, onRetry, onInStop, onEdStop, onSyncProject,
  onAcknowledgeRecovery,
}: {
  project: Project; executionStatus: PipelineState | null;
  workspaceStatus: ExecutionWorkspaceStatus | null;
  busy: boolean;
  onPrepareWorkspace: () => Promise<void>;
  onExecute: () => Promise<void>; onConfirm: () => Promise<void>;
  onReject: (reason: string) => Promise<void>;
  onRetry: () => Promise<void>;
  onInStop: () => Promise<void>; onEdStop: () => Promise<void>;
  onSyncProject: () => Promise<void>;
  onAcknowledgeRecovery?: () => Promise<void>;
}) {
  const [rejectReason, setRejectReason] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const [showReject, setShowReject] = useState(false);
  const busy = externalBusy || localBusy;

  const ms = project.milestones.find(m => m.id === project.current_milestone_id);
  const mid = ms?.mid_stages.find(m => m.id === project.current_mid_stage_id);
  const planApproved = mid?.plan_approved_at != null && (mid?.plan_revision ?? 0) > 0;

  // Find next pending subtask or one awaiting confirmation
  const pendingSubtasks = mid?.subtasks.filter(s => s.status === "Pending") ?? [];
  const nextSubtask = pendingSubtasks[0] ?? null;
  const awaitingSubtask = mid?.subtasks.find(s => s.status === "AwaitingConfirmation") ?? null;

  const isAwaiting = executionStatus?.awaiting_confirmation === true || awaitingSubtask != null;
  const isExecuting = executionStatus?.status === "Running";

  // 精确失败会话：优先显示失败面板，暂时隐藏普通执行和准备环境
  const failedSession = project.execution_session;
  const failedStatus = (failedSession?.status ?? "").toLowerCase();
  const hasExecutionFailure =
    failedStatus === "execution_failed"
    || failedStatus === "session_lost"
    || failedStatus === "stop_failed";
  const recoveryActive = project.workflow_state.recovery_state != null;

  const handlePrepareWorkspace = async () => {
    if (!project || busy) return;
    setLocalBusy(true);
    try {
      await onPrepareWorkspace();
    } finally {
      setLocalBusy(false);
    }
  };

  const handleConfirm = async () => {
    setLocalBusy(true);
    await onConfirm();
    setLocalBusy(false);
  };

  const handleReject = async () => {
    if (!rejectReason.trim()) return;
    setLocalBusy(true);
    await onReject(rejectReason.trim());
    setRejectReason("");
    setShowReject(false);
    setLocalBusy(false);
  };

  const handleRetry = async () => {
    setLocalBusy(true);
    await onRetry();
    setLocalBusy(false);
  };

  // 质量判定：判断当前待确认任务是否可以确认通过
  const execOk = awaitingSubtask?.execution_result?.success === true;
  const humanOverride = awaitingSubtask?.human_verification?.verification_kind === "HumanOverride"
    && Boolean(awaitingSubtask.human_verification.verification_reason.trim());
  const testOk = awaitingSubtask?.test_result?.passed === true || humanOverride;
  const canConfirm = execOk && testOk && isAwaiting;
  const failureReason = !canConfirm && isAwaiting
    ? (!execOk ? "执行未成功" : !testOk ? "核验未通过" : null)
    : null;

  const workspaceReady = workspaceStatus?.ready_for_new_execution === true;
  const workspaceAction = getWorkspaceAction(workspaceStatus);
  const managedTaskChanges = workspaceStatus?.has_managed_task_changes === true
    && workspaceStatus.has_external_changes === false;

  return (
    <div className="v1-execution-panel" style={{ padding: "24px" }}>
      <h2 className="execution-panel-title"><ListTodo size={20} />执行</h2>

      {/* 执行失败专用面板：优先显示，隐藏普通执行/准备环境 */}
      {hasExecutionFailure && failedSession && (
        <div className="execution-failure-panel" style={{
          marginBottom: "20px", padding: "16px",
          background: "#ffebe9", borderRadius: "8px", border: "1px solid #cf222e",
        }}>
          <div style={{ fontWeight: 600, fontSize: "14px", marginBottom: "8px", color: "#cf222e" }}>
            {failedStatus === "session_lost" ? "执行中断（进程失联）"
              : failedStatus === "stop_failed" ? "暂停失败"
              : "执行失败"}
          </div>
          <div style={{ fontSize: "13px", color: "#24292f", marginBottom: "8px", overflowWrap: "anywhere" }}>
            <div>受影响任务：{failedSession.subtask_title || failedSession.subtask_id}</div>
            {failedSession.failure_message && (
              <div style={{ marginTop: "6px", whiteSpace: "pre-wrap" }}>
                失败原因：{failedSession.failure_message}
              </div>
            )}
            {failedSession.base_commit && (
              <div style={{ marginTop: "4px", color: "#656d76", fontFamily: "monospace", fontSize: "12px" }}>
                基线：{failedSession.base_commit.slice(0, 12)}
              </div>
            )}
          </div>
          <WorkflowActionBar>
            <ActionButton
              icon={<RotateCcw size={16} />}
              loading={busy}
              loadingLabel="恢复中"
              onClick={async () => {
                setLocalBusy(true);
                try {
                  if (onAcknowledgeRecovery) {
                    await onAcknowledgeRecovery();
                  } else {
                    await onRetry();
                  }
                } finally {
                  setLocalBusy(false);
                }
              }}
            >
              恢复执行基线
            </ActionButton>
            <ActionButton icon={<RotateCcw size={16} />} disabled={busy} onClick={onSyncProject}>
              同步状态
            </ActionButton>
          </WorkflowActionBar>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            恢复成功前不会显示“已恢复到安全状态”。请先完成基线恢复，再重新执行。
          </p>
        </div>
      )}

      {/* Workspace status banner — 失败会话期间隐藏准备环境 */}
      {!hasExecutionFailure && !recoveryActive && planApproved && workspaceStatus && !workspaceReady && (
        <FeedbackBanner
          type={managedTaskChanges ? "info" : "warning"}
          message={workspaceStatus.status_message}
          details={workspaceStatus.changes.map(change =>
            `${change.tracked ? `${change.index_status}${change.worktree_status}` : "??"} ${change.path}${change.managed ? "（当前任务）" : ""}`
          )}
        />
      )}

      {/* Workspace preparation is only valid before repository metadata exists. */}
      {!hasExecutionFailure && !recoveryActive && planApproved && workspaceAction === "prepare" && (
        <div style={{ marginBottom: "20px" }}>
          <ActionButton icon={<GitBranch size={16} />} loading={busy} loadingLabel="准备中"
            onClick={handlePrepareWorkspace}>准备执行环境</ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            执行小阶段前需要初始化 Git 仓库并创建首次提交。
          </p>
        </div>
      )}

      {!hasExecutionFailure && !recoveryActive && planApproved && workspaceStatus &&
        workspaceAction !== "none" && workspaceAction !== "prepare"
        && workspaceAction !== "managed_task_changes" && (
        <div style={{ marginBottom: "20px" }}>
          <ActionButton icon={<RefreshCw size={16} />} disabled={busy} onClick={onSyncProject}>
            刷新工作区
          </ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            {workspaceAction === "resolve_changes"
              ? "请先处理上方列出的工作区变更，再刷新状态。"
              : workspaceAction === "configure_identity"
                ? "请先配置 Git user.name 和 user.email，再刷新状态。"
                : "请修复项目路径后刷新状态。"}
          </p>
        </div>
      )}

      {/* Awaiting confirmation */}
      {!hasExecutionFailure && !recoveryActive && isAwaiting && awaitingSubtask && (
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
                  核验：{verificationLabel(awaitingSubtask.test_result)}
                  {awaitingSubtask.test_result.suggestion && ` — ${awaitingSubtask.test_result.suggestion}`}
                </div>
              )}
              {awaitingSubtask.human_verification && (
                <div style={{ marginTop: "4px", color: "#1a7f37" }}>
                  人工核验：{awaitingSubtask.human_verification.verification_reason}
                </div>
              )}
              <div style={{ marginTop: "4px" }}>验收标准：{awaitingSubtask.acceptance_criteria?.join("；") || "（无）"}</div>
            </div>
          </div>
          <WorkflowActionBar>
            {canConfirm ? (
              <ActionButton icon={<Check size={16} />} loading={busy} loadingLabel="确认中" onClick={handleConfirm}>确认通过</ActionButton>
            ) : (
              <ActionButton icon={<RotateCcw size={16} />} loading={busy} loadingLabel="恢复中" onClick={handleRetry}>恢复基线并重试</ActionButton>
            )}
            <ActionButton icon={<X size={16} />} variant="danger" disabled={busy} onClick={() => setShowReject(true)}>发现问题</ActionButton>
          </WorkflowActionBar>
          {failureReason && (
            <div style={{ padding: "10px 14px", background: "#fff8c5", borderRadius: "6px", border: "1px solid #d4a72c", marginTop: "12px", fontSize: "13px", color: "#9a6700" }}>
              ⚠️ 质量门禁阻断：{failureReason}。请先恢复基线并重试，或驳回后人工处理。
            </div>
          )}
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

      {/* Next pending subtask — only when workspace is ready and no failure session */}
      {!hasExecutionFailure && !recoveryActive && !isAwaiting && planApproved && workspaceReady && nextSubtask && (
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
            onClick={async () => { setLocalBusy(true); await onExecute(); setLocalBusy(false); }}>执行当前小阶段</ActionButton>
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
                onClick={async () => { setLocalBusy(true); await onInStop(); setLocalBusy(false); }}
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
                onClick={async () => { setLocalBusy(true); await onEdStop(); setLocalBusy(false); }}
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
            onClick={onSyncProject}
          >
            同步项目状态
          </ActionButton>
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

export default App;
