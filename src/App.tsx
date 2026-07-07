// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import "./App.css";
import { Project, ViewMode, DiscussionReason, Subtask, PipelineState, QAResult, GeneratedSubtask, TestLog, PathValidationResult } from "./types";
import ExecutionTree from "./ExecutionTree";
import ChatRoom from "./ChatRoom";
import TaskConsole from "./TaskConsole";
import FileTree from "./FileTree";
import FloatingChatBalloon from "./FloatingChatBalloon";
import { ExecutionEngineTestPanel } from "./features/dev-tools";

const DEFAULT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 800;

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
  const [projectPath, setProjectPath] = useState<string>("");
  const [pathSaveStatus, setPathSaveStatus] = useState<string>("");

  // === Phase B：视图模式控制 ===
  const [viewMode, setViewMode] = useState<ViewMode>({ phase: 'discussion', reason: 'idle' });

  // Phase D: 动画控制
  const [animatingComponent, setAnimatingComponent] = useState<'chatroom' | 'taskconsole' | null>(null);
  const animationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 测试日志去重：记录已处理过的子任务 ID
  const processedSubtaskIdsRef = useRef<Set<string>>(new Set());

  // === 侧边栏拖拽缩放 ===
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [isDragging, setIsDragging] = useState(false);
  const dragStartX = useRef(0);
  const dragStartWidth = useRef(0);

  const enterExecutionMode = useCallback(() => {
    // 如果已经在执行模式，不重复触发
    if (viewMode.phase === 'execution') return;
    // 清除上一个未完成的定时器，防止快速连续切换导致时序混乱
    if (animationTimerRef.current) { clearTimeout(animationTimerRef.current); animationTimerRef.current = null; }
    // 标记 ChatRoom 开始淡出
    setAnimatingComponent('chatroom');
    // 250ms 后切换状态
    animationTimerRef.current = setTimeout(() => {
      setViewMode({ phase: 'execution', reason: 'active' });
      setAnimatingComponent(null);
      animationTimerRef.current = null;
    }, 250);
  }, [viewMode.phase]);

  const enterDiscussionMode = useCallback((reason: DiscussionReason) => {
    // 如果已经在讨论模式且 reason 相同，不重复触发
    if (viewMode.phase === 'discussion' && viewMode.reason === reason) return;
    // 清除上一个定时器
    if (animationTimerRef.current) { clearTimeout(animationTimerRef.current); animationTimerRef.current = null; }
    // 标记 TaskConsole 开始淡出
    setAnimatingComponent('taskconsole');
    // 250ms 后切换状态
    animationTimerRef.current = setTimeout(() => {
      setViewMode({ phase: 'discussion', reason });
      setAnimatingComponent(null);
      animationTimerRef.current = null;
    }, 250);
  }, [viewMode.phase, viewMode.reason]);

  // handleAddMessage 必须在轮询 useEffect 之前定义（依赖数组引用）
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

  // === Phase B：从 ExecutionTree 提升的 9 个状态 ===
  const [selectedMilestoneId, setSelectedMilestoneId] = useState<string | null>(null);
  const [selectedMidStageId, setSelectedMidStageId] = useState<string | null>(null);
  const [generatedPlan, setGeneratedPlan] = useState<Map<string, Subtask[]>>(new Map());
  const [quickGeneratedPlan, setQuickGeneratedPlan] = useState<Map<string, Subtask[]>>(new Map());
  const [isExecuting, setIsExecuting] = useState(false);
  const [isGeneratingPlan, setIsGeneratingPlan] = useState(false);
  const [isGeneratingVersionPlan, setIsGeneratingVersionPlan] = useState(false);
  const [isGeneratingMilestones, setIsGeneratingMilestones] = useState(false);
  const [executionStatus, setExecutionStatus] = useState<PipelineState | null>(null);
  const [_autoAdvance, _setAutoAdvance] = useState(false);
  const [qaModalData, setQaModalData] = useState<{ milestoneId: string; qaResult: QAResult } | null>(null);
  const [isSubmitting, _setIsSubmitting] = useState(false);
  const [testLogs, setTestLogs] = useState<TestLog[]>([]);

  // === 快照：保存 UI 状态到后端，用于刷新恢复和孤儿进程保护 ===
  const takeSnapshot = () => {
    if (!project) return;
    const snapshotUi = {
      view_phase: viewMode.phase,
      selected_milestone_id: selectedMilestoneId ?? null,
      selected_mid_stage_id: selectedMidStageId ?? null,
      generated_plan_keys: Array.from(generatedPlan.keys()),
      quick_generated_plan_keys: Array.from(quickGeneratedPlan.keys()),
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
  }, [project, viewMode.phase, selectedMilestoneId, selectedMidStageId, generatedPlan, quickGeneratedPlan]);

  // Phase B: 执行状态轮询 + 自动阶段切换
  useEffect(() => {
    if (!isExecuting) return;

    const interval = setInterval(async () => {
      try {
        const status = await invokeWithTimeout<PipelineState | null>("get_execution_status");
        setExecutionStatus(status);

        // 执行中定期刷新快照，确保 running_pid（孤儿进程保护用）保持最新
        takeSnapshot();

        // 从执行状态中提取测试日志（去重）
        if (status && status.subtask_statuses) {
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

        if (status) {
          if (status.status === "Paused") {
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
          } else if (status.status === "Failed") {
            setIsExecuting(false);
            clearInterval(interval);
          }
        }
      } catch (e) {
        console.error("轮询状态失败:", e);
      }
    }, 2000);

    return () => clearInterval(interval);
  }, [isExecuting, enterDiscussionMode, handleAddMessage]);

  // 完整回调 当 App 组件第一次加载时，自动从后端（Rust）获取当前项目数据，并保存到前端的状态中
  // 页面一打开，自动从后端拉取项目数据，存到状态里，并保存项目路径，只做一次
  useEffect(() => {
    invokeWithTimeout<Project>("get_project", { projectName: "我的游戏" })
      .then((project) => {
        setProject(project);
        if (project && project.project_path) {
          setProjectPath(project.project_path);
        }
        // 从持久化数据恢复执行计划到内存 Map，使刷新后「开始执行」按钮仍可见
        if (project && project.milestones) {
          setQuickGeneratedPlan((prev) => {
            const next = new Map(prev);
            for (const ms of project.milestones) {
              if (ms.subtasks && ms.subtasks.length > 0) {
                next.set(ms.id, ms.subtasks);
              }
            }
            return next;
          });
          setGeneratedPlan((prev) => {
            const next = new Map(prev);
            for (const ms of project.milestones) {
              for (const mid of ms.mid_stages) {
                if (mid.subtasks && mid.subtasks.length > 0) {
                  next.set(mid.id, mid.subtasks);
                }
              }
            }
            return next;
          });
        }
        // 恢复 UI 状态快照（视图模式、选中阶段等）
        return invokeWithTimeout<any>("restore_snapshot", { projectId: project.name });
      })
      .then((snapshot) => {
        if (snapshot && snapshot.ui) {
          const ui = snapshot.ui;
          // 恢复视图模式（跳过动画，直接设置）
          if (ui.view_phase === 'execution') {
            setViewMode({ phase: 'execution', reason: 'active' });
          }
          // 恢复选中状态
          if (ui.selected_milestone_id) {
            setSelectedMilestoneId(ui.selected_milestone_id);
          }
          if (ui.selected_mid_stage_id) {
            setSelectedMidStageId(ui.selected_mid_stage_id);
          }
        }
      })
      .catch((err) => {
        console.error("获取项目失败:", err);
        setProject(null);
      });
    enterDiscussionMode('idle');
  }, []);

  const handleSelectMilestone = (id: string) => {
    // 点击已选中大阶段 → 幂等保持（不取消选中）
    if (selectedMilestoneId === id) return;
    setSelectedMilestoneId(id);
    // 自动选中该大阶段的第一个中阶段（专业模式）
    if (project) {
      const ms = project.milestones.find(m => m.id === id);
      if (ms && ms.mid_stages.length > 0) {
        setSelectedMidStageId(ms.mid_stages[0].id);
      }
    }
  };
  //生成版本方案
  const handleGeneratePlan = async () => {
    //无项目数据则返回
    if (!project) return;
    if (isGeneratingVersionPlan) return; // 防止重复提交
    const currentThread = project.discussion_threads[0];
    if (!currentThread) {
      console.error("没有可用的讨论线程");
      return;
    }
    setIsGeneratingVersionPlan(true);
    try {
      const plan = await invokeWithTimeout("generate_version_plan", {
        messages: currentThread.messages,
        projectPath: project.project_path,
      });
      //更新项目，加上方案
      setProject({ ...project, version_plan: plan as string });
    } catch (err) {
      console.error("生成方案失败", err);
    } finally {
      setIsGeneratingVersionPlan(false);
    }
  };
  //批准版本方案
  const handleApprove = async () => {
    //安全保护
    if (!project) return;
    try {
      await invokeWithTimeout("approve_version_plan", {
        //把项目转成 JSON 字符串传到后端
        projectJson: JSON.stringify(project),
        //再单独传一次方案（其实后端可以从projectJson里取
        versionPlan: project.version_plan,
      });
      //前端也同步状态"规划中"，并进入执行模式
      setProject({ ...project, status: "Planning" });
      enterExecutionMode();
    } catch (err) {
      console.error("批准失败：", err);
    }
  };
  //驳回版本方案（不调用后端，硬盘存储还在，临时放弃）
  const handleReject = () => {
    if (!project) return;
    //直接把方案清空
    const updatedProject = { ...project, version_plan: "", status: "Discussing" as const };
    setProject(updatedProject);
    invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) })
      .catch(err => console.error("持久化驳回方案失败:", err));
  }

  //根据版本方案拆解大阶段
  const handleGenerateMilestones = async () => {
    if (!project) return;
    if (isGeneratingMilestones) return; // 防止重复提交
    setIsGeneratingMilestones(true);
    //调用后端命令，传入版本方案和模式，等待返回Milestones数组
    try {
      const milestones = await invokeWithTimeout("generate_milestones", {
        versionPlan: project.version_plan,
        mode: project.mode,
      });
      //把Milestones数组合并到项目状态中（触发重新渲染）
      const updatedProject = { ...project, status: "MilestoneReady" as const, milestones: milestones as any[] };
      setProject(updatedProject);
      try {
        await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) });
      } catch (saveErr) {
        console.error("保存里程碑失败：", saveErr);
        alert("大阶段已生成，但保存到文件失败，请重试或检查磁盘空间。");
      }
      enterExecutionMode();
    } catch (err) {
      console.error("拆解大阶段失败：", err);
    } finally {
      setIsGeneratingMilestones(false);
    }
  };

  //切换项目的工作模式（快速/专业）
  const handleModeChange = async (mode: "Quick" | "Professional") => {
    if (!project) return;
    const updatedProject = { ...project, mode };
    setProject(updatedProject);
    //保存到文件
    try {
      await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) });
    } catch (e) {
      console.error("保存模式切换失败：", e);
      alert("模式已切换，但保存到文件失败，请重试。");
    }
  };

  // 编辑大阶段版本号
  const handleVersionEdit = (id: string, newVersion: string) => {
    if (!project) return;
    const updatedMilestones = project.milestones.map(ms =>
      ms.id === id ? { ...ms, version: newVersion } : ms
    );
    const updatedProject = { ...project, milestones: updatedMilestones };
    setProject(updatedProject);
    invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) })
      .catch(err => console.error("持久化版本编辑失败:", err));
  };

  //点拆解中阶段，调后端拆解子阶段，更新到项目状态中
  const handleGenerateMidStages = async (milestoneId: string) => {
    if (!project) return;
    const milestone = project.milestones.find(ms => ms.id === milestoneId);
    if (!milestone) return;
    try {
      const midStages = await invokeWithTimeout("generate_mid_stages", {
        milestoneId: milestoneId,
        milestoneTitle: milestone.title,
        milestoneDescription: milestone.description,
        versionPlan: project.version_plan,
        mode: project.mode,
        attentionPoints: milestone.qa_result?.attention_points ?? [],
      });
      const updatedMilestones = project.milestones.map(ms =>
        ms.id === milestoneId ? { ...ms, mid_stages: midStages as any[] } : ms
      );
      const updatedProject = { ...project, milestones: updatedMilestones };
      setProject(updatedProject);
      try {
        await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) });
      } catch (saveErr) {
        console.error("保存中阶段失败：", saveErr);
        alert("中阶段已生成，但保存到文件失败，请重试。");
      }
    } catch (err) {
      console.error("拆解中阶段失败：", err);
    }
  }

  //当质检驳回后，用户选择「采纳意见，重新拆解」时调用
  const handleRegenerateMilestones = async (feedback: string) => {
    if (!project) return;
    try {
      const newMilestones = await invokeWithTimeout("regenerate_milestones_with_feedback", {
        versionPlan: project.version_plan,
        mode: project.mode,
        feedback: feedback,
      });
      const updatedProject = { ...project, status: "MilestoneReady" as const, milestones: newMilestones as any[] };
      setProject(updatedProject);
      try {
        await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updatedProject) });
      } catch (saveErr) {
        console.error("持久化重新拆解失败:", saveErr);
      }
    } catch (err) {
      console.error("重新拆解失败：", err);
      alert("重新拆解失败：" + err);
    }
  };

  // === Phase B: 从 ExecutionTree 提升的函数 ===

  /// 快速模式：为大阶段生成执行计划（跳过中阶段）
  const handleGenerateQuickPlan = async (milestone: any) => {
    setIsGeneratingPlan(true);
    const generated: Subtask[] = [];
    let prevTitle = "";
    let prevResult = "";

    for (let i = 0; i < 3; i++) {
      try {
        const next = await invokeWithTimeout<GeneratedSubtask>("generate_next_prompt", {
          midStageTitle: milestone.title,
          midStageDescription: milestone.description || "",
          previousSubtaskTitle: prevTitle,
          previousSubtaskResult: prevResult,
          fileChanges: [],
          testResult: "",
          isRetry: false,
          retryReason: "",
        });
        generated.push({
          id: `${milestone.id}-st-${i + 1}`,
          title: next.title,
          prompt: next.prompt,
          status: "Pending" as const,
          test_report: "",
          retry_count: 0,
        });
        prevTitle = next.title;
        prevResult = "通过";
      } catch (e) {
        console.error("快速模式生成子任务失败:", e);
        break;
      }
    }

    setQuickGeneratedPlan((prev) => {
      const next = new Map(prev);
      next.set(milestone.id, generated);
      return next;
    });
    setIsGeneratingPlan(false);
  };

  /// 快速模式：开始执行大阶段的子任务
  const handleStartQuickExecution = async (milestone: any) => {
    if (!projectPath) {
      alert("请先在主界面设置项目目录");
      return;
    }
    // 验证路径有效性
    try {
      const validation = await invokeWithTimeout<PathValidationResult>("validate_project_path", { projectPath });
      if (!validation.is_valid) {
        alert(`项目目录无效：${validation.error_message}`);
        return;
      }
    } catch (e) {
      console.error("路径验证失败:", e);
      // 继续执行 — 后端也会做校验
    }
    const plan = quickGeneratedPlan.get(milestone.id);
    if (!plan || plan.length === 0) {
      alert("请先生成执行计划");
      return;
    }
    try {
      await invokeWithTimeout("start_execution", {
        projectId: project?.name ?? "",
        projectPath: projectPath,
        stageIdentifier: milestone.version,
        isQuickMode: true,
        midStageTitle: milestone.title,
        midStageDescription: milestone.description,
        subtasksJson: JSON.stringify(plan),
      });
      setIsExecuting(true);
      enterExecutionMode();
    } catch (e) {
      console.error("快速模式启动执行失败:", e);
    }
  };

  // === Phase C: TaskConsole 专用的执行控制回调 ===

  /// 专业模式：为中阶段生成执行计划
  const handleGeneratePlanForMidStage = async (midStageId: string) => {
    if (!project) return;
    // 找到对应的 midStage 数据
    const mid = project.milestones
      .flatMap((m) => m.mid_stages)
      .find((ms) => ms.id === midStageId);
    if (!mid) {
      console.error("找不到 midStage:", midStageId);
      return;
    }
    setIsGeneratingPlan(true);
    const generated: Subtask[] = [];
    let prevTitle = "";
    let prevResult = "";

    for (let i = 0; i < 3; i++) {
      try {
        const next = await invokeWithTimeout<GeneratedSubtask>("generate_next_prompt", {
          midStageTitle: mid.title,
          midStageDescription: mid.description || "",
          previousSubtaskTitle: prevTitle,
          previousSubtaskResult: prevResult,
          fileChanges: [],
          testResult: "",
          isRetry: false,
          retryReason: "",
        });
        generated.push({
          id: `${midStageId}-st-${i + 1}`,
          title: next.title,
          prompt: next.prompt,
          status: "Pending" as const,
          test_report: "",
          retry_count: 0,
        });
        prevTitle = next.title;
        prevResult = "通过";
      } catch (e) {
        console.error("生成子任务失败:", e);
        break;
      }
    }

    setGeneratedPlan((prev) => {
      const next = new Map(prev);
      next.set(midStageId, generated);
      return next;
    });
    setIsGeneratingPlan(false);
  };

  /// 专业模式：启动中阶段执行
  const handleStartExecution = async (midStageId: string) => {
    if (!projectPath) {
      alert("请先在主界面设置项目目录");
      return;
    }
    // 验证路径有效性
    try {
      const validation = await invokeWithTimeout<PathValidationResult>("validate_project_path", { projectPath });
      if (!validation.is_valid) {
        alert(`项目目录无效：${validation.error_message}`);
        return;
      }
    } catch (e) {
      console.error("路径验证失败:", e);
      // 继续执行 — 后端也会做校验
    }
    const plan = generatedPlan.get(midStageId);
    if (!plan || plan.length === 0) {
      alert("请先生成执行计划");
      return;
    }
    const mid = project?.milestones
      .flatMap((m) => m.mid_stages)
      .find((ms) => ms.id === midStageId);
    try {
      await invokeWithTimeout("start_execution", {
        projectId: project?.name ?? "",
        projectPath: projectPath,
        stageIdentifier: midStageId,
        isQuickMode: false,
        midStageTitle: mid?.title ?? midStageId,
        midStageDescription: mid?.description ?? "",
        subtasksJson: JSON.stringify(plan),
      });
      setIsExecuting(true);
      enterExecutionMode();
    } catch (e) {
      console.error("启动执行失败:", e);
    }
  };

  /// 暂停执行
  const handlePause = async () => {
    try {
      await invokeWithTimeout("pause_execution");
      setIsExecuting(false);
      handleAddMessage({
        id: Date.now().toString(),
        role: 'system',
        content: '⏸️ 执行已暂停。讨论修改方向后，点击恢复执行。',
        timestamp: Date.now(),
      });
      enterDiscussionMode('paused');
    } catch (e) {
      console.error("暂停失败:", e);
    }
  };

  /// 恢复执行
  const handleResume = async () => {
    try {
      await invokeWithTimeout("resume_execution");
      setIsExecuting(true);
      enterExecutionMode();
    } catch (e) {
      console.error("恢复失败:", e);
    }
  };

  /// 停止执行
  const handleStop = async () => {
    try {
      await invokeWithTimeout("stop_execution");
    } catch (e) {
      console.error("停止执行失败:", e);
    }
    setIsExecuting(false);
    handleAddMessage({
      id: Date.now().toString(),
      role: 'system',
      content: '⏹️ 执行已被用户停止。可重新生成执行计划或返回讨论。',
      timestamp: Date.now(),
    });
    enterDiscussionMode('paused');
  };

  /// 切换到下一个中阶段
  const handleNextMidStage = useCallback(() => {
    if (!project || !selectedMilestoneId || !selectedMidStageId) return;
    const currentMilestone = project.milestones?.find(m => m.id === selectedMilestoneId);
    if (!currentMilestone) return;
    const midStages = currentMilestone.mid_stages || [];
    const sortedMidStages = [...midStages].sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
    const currentIndex = sortedMidStages.findIndex(ms => ms.id === selectedMidStageId);
    if (currentIndex >= 0 && currentIndex < sortedMidStages.length - 1) {
      setSelectedMidStageId(sortedMidStages[currentIndex + 1].id);
    }
  }, [project, selectedMilestoneId, selectedMidStageId]);

  /// 切换到下一个中阶段并重置执行状态
  const handleNextMidStageAndReset = useCallback(() => {
    handleNextMidStage();
    setExecutionStatus(null);
    setIsExecuting(false);
  }, [handleNextMidStage]);

  /// 计算是否还有下一个中阶段
  const hasNextMidStage = useMemo(() => {
    if (!project || !selectedMilestoneId || !selectedMidStageId) return false;
    const currentMilestone = project.milestones?.find(m => m.id === selectedMilestoneId);
    if (!currentMilestone) return false;
    const midStages = currentMilestone.mid_stages || [];
    const sortedMidStages = [...midStages].sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
    const currentIndex = sortedMidStages.findIndex(ms => ms.id === selectedMidStageId);
    return currentIndex >= 0 && currentIndex < sortedMidStages.length - 1;
  }, [project, selectedMilestoneId, selectedMidStageId]);

  //根据项目状态返回默认对话角色
  const getDefaultRole = (status: string): string => {
    switch (status) {
      case "Idle":
      case "Discussing":
        return "策略产品经理";
      case "Planning":
        return "产品经理";
      case "MilestoneReady":
        return "域负责人";
      case "Executing":
        return "全栈技术顾问";
      case "Paused":
        return "策略产品经理"
      default:
        return "策略产品经理";
    }
  };

  if (!project) {
    return <div className="loading">加载中...</div>;
  }

  const currentThread = project.discussion_threads[0];
  if (!currentThread) {
    return <div className="loading">没有可用的讨论线程，请检查项目数据...</div>;
  }

  return (
    <div className="app-layout">
      <aside className="sidebar" style={{ width: sidebarWidth + 'px' }}>
        <ExecutionTree
          milestones={project.milestones}
          onSelectMilestone={handleSelectMilestone}
          onVersionEdit={handleVersionEdit}
          onGenerateMidStages={handleGenerateMidStages}
          onRegenerateMilestones={handleRegenerateMilestones}
          projectPath={projectPath}
          projectId={project.name}
          // Phase B: 从 App.tsx 传入的提升状态
          selectedMilestoneId={selectedMilestoneId}
          selectedMidStageId={selectedMidStageId}
          onSelectMidStage={setSelectedMidStageId}
          quickGeneratedPlan={quickGeneratedPlan}
          generatedPlan={generatedPlan}
          onGenerateQuickPlan={handleGenerateQuickPlan}
          onStartQuickExecution={handleStartQuickExecution}
          isExecuting={isExecuting}
          isGeneratingPlan={isGeneratingPlan}
          executionStatus={executionStatus}
        />
        <div
          className={`resize-handle${isDragging ? ' dragging' : ''}`}
          onMouseDown={handleResizeMouseDown}
          onDoubleClick={() => setSidebarWidth(DEFAULT_SIDEBAR_WIDTH)}
        />
      </aside>

      <main className="main-content">
        {/* === 开发者工具：让你可以手动测试 Tauri 后端的“执行子任务”、“
        检查子任务”、“生成下一步提示词”三个核心命令，并查看结果 === */}
        <div className="project-path-section">
          <h3>📁 项目目录</h3>
          <div className="project-path-row">
            <input
              className="project-path-input"
              type="text"
              value={projectPath}
              onChange={(e) => setProjectPath(e.target.value)}
              placeholder="例如：/home/user/my-project"
            />
            <button
              className="btn-save-path"
              onClick={async () => {
                const updated = { ...project, project_path: projectPath };
                // 验证路径（仅提示，不阻止保存）
                if (projectPath) {
                  try {
                    const validation = await invokeWithTimeout<PathValidationResult>("validate_project_path", { projectPath });
                    if (!validation.is_valid) {
                      setPathSaveStatus(`⚠️ ${validation.error_message}（已保存）`);
                    }
                  } catch (_) { /* 验证失败不影响保存 */ }
                }
                try {
                  await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updated) });
                  setProject(updated);
                  if (!pathSaveStatus.startsWith("⚠️")) {
                    setPathSaveStatus("✅ 已保存");
                  }
                  setTimeout(() => setPathSaveStatus(""), 5000);
                } catch (e: any) {
                  setPathSaveStatus(`❌ 保存失败：${e}`);
                }
              }}
            >
              保存
            </button>
          </div>
          {pathSaveStatus && (
            <div className={`path-status ${pathSaveStatus.startsWith("✅") ? "success" : "error"}`}>
              {pathSaveStatus}
            </div>
          )}
        </div>
        <ExecutionEngineTestPanel projectPath={projectPath} />

        <header className="chat-header">
          <h2>弥 · 工作流指挥中心</h2>
          <h4>Metheus 带来你的灵感，输出你的创意！</h4>
        </header>

        {/* ===== Phase D: 讨论模式（带动画过渡） ===== */}
        {(viewMode.phase === 'discussion' || animatingComponent === 'chatroom') && (
          <div className={`transition-wrapper${animatingComponent === 'chatroom' ? ' animate-fade-out-right' : ''
            }${animatingComponent === 'taskconsole' ? ' animate-fade-in-right' : ''
            }`}>
            {/* 生成版本方案按钮 */}
            {(project.status === "Idle" || project.status === "Discussing") && !project.version_plan && (
              <div className="generate-plan-area">
                <button className="btn-generate-plan" onClick={handleGeneratePlan} disabled={isGeneratingVersionPlan}>
                  📝 {isGeneratingVersionPlan ? "生成中..." : "生成版本方案"}
                </button>
              </div>
            )}
            <ChatRoom
              messages={currentThread.messages || []}
              onAddMessage={handleAddMessage}
              currentRole={getDefaultRole(project.status)}
              mode={project.mode}
              onModeChange={handleModeChange}
              modeLocked={project.status !== "Idle"}
            />
            {project.version_plan && (!project.milestones || project.milestones.length === 0) && (
              <div className="version-plan-panel">
                <h3>📋 版本方案摘要</h3>
                <pre className="version-plan-content">{project.version_plan}</pre>
                <div className="version-plan-actions">
                  <button className="btn-approve" onClick={handleApprove}>✅ 批准</button>
                  <button className="btn-reject" onClick={handleReject}>❌ 驳回</button>
                </div>
              </div>
            )}
            {project.status === "Planning" && project.version_plan && (
              <div className="generate-plan-area">
                <button className="btn-generate-plan" onClick={handleGenerateMilestones} disabled={isGeneratingMilestones}>
                  📊 {isGeneratingMilestones ? "拆解中..." : "根据版本方案拆解大阶段"}
                </button>
              </div>
            )}
          </div>
        )}

        {/* ===== Phase D: 执行模式（带动画过渡） ===== */}
        {(viewMode.phase === 'execution' || animatingComponent === 'taskconsole') && (
          <div className={`execution-layout${animatingComponent === 'taskconsole' ? ' animate-fade-out-left' : ''
            }${animatingComponent === 'chatroom' ? ' animate-fade-in-left' : ''
            }`}>
            <FileTree
              projectPath={projectPath}
            />
            <div className="execution-main">
              <TaskConsole
                projectPath={projectPath}
                projectId={project.name}
                isExecuting={isExecuting}
                isGeneratingPlan={isGeneratingPlan}
                executionStatus={executionStatus}
                generatedPlan={generatedPlan}
                selectedMidStageId={selectedMidStageId}
                qaModalData={qaModalData}
                isSubmitting={isSubmitting}
                testLogs={testLogs}
                onGeneratePlan={handleGeneratePlanForMidStage}
                onStartExecution={handleStartExecution}
                onPause={handlePause}
                onResume={handleResume}
                onStop={handleStop}
                onRegenerateMilestones={handleRegenerateMilestones}
                onEnterReviewMode={() => enterDiscussionMode('review')}
                onDismissQA={() => setQaModalData(null)}
                onQAIgnore={() => setQaModalData(null)}
                projectStatus={project.status}
                onGenerateMilestones={handleGenerateMilestones}
                onNextMidStage={handleNextMidStageAndReset}
                hasNextMidStage={hasNextMidStage}
              />
            </div>
          </div>
        )}

        {/* ===== 阶段二悬浮球 ===== */}
        {(viewMode.phase === 'execution' || animatingComponent === 'taskconsole') && (
          <FloatingChatBalloon
            messages={currentThread.messages || []}
          />
        )}
      </main>
    </div>
  );
}

export default App;
