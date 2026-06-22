// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Milestone, Subtask, PipelineState, GeneratedSubtask, QAResult, RollbackToSubtaskPayload } from "./types";
import { Modal } from './components/Modal';

const COPIED_TIMEOUT_MS = 2000;

interface Props {
  milestones: Milestone[];
  onSelectMilestone: (id: string) => void;
  onVersionEdit?: (id: string, newVersion: string) => void;
  onGenerateMidStages?: (id: string) => void;
  onRegenerateMilestones?: (feedback: string) => Promise<void>;
  projectPath?: string;
  projectId?: string;
}

function ExecutionTree({ milestones, onSelectMilestone, onVersionEdit, onGenerateMidStages, onRegenerateMilestones, projectPath, projectId }: Props) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  // === 3Phase 3 新增状态 ===
  const [selectedMidStageId, setSelectedMidStageId] = useState<string | null>(null);
  const [generatedPlan, setGeneratedPlan] = useState<Map<string, Subtask[]>>(new Map());
  const [quickGeneratedPlan, setQuickGeneratedPlan] = useState<Map<string, Subtask[]>>(new Map());
  const [isExecuting, setIsExecuting] = useState(false);
  const [isGeneratingPlan, setIsGeneratingPlan] = useState(false);
  const [executionStatus, setExecutionStatus] = useState<PipelineState | null>(null);
  const [autoAdvance, setAutoAdvance] = useState(false);

  // 新增：回退相关状态
  const [rollbackTarget, setRollbackTarget] = useState<{
    tagName: string;
    version: string;
  } | null>(null);

  // 小阶段回退状态
  const [subtaskRollbackTarget, setSubtaskRollbackTarget] = useState<Omit<RollbackToSubtaskPayload, 'projectPath' | 'projectId'> | null>(null);
  const [copiedTag, setCopiedTag] = useState<string | null>(null);
  const copyTimeoutRef = useRef<number | null>(null);

  const handleCopyTag = (tag: string) => {
    if (copyTimeoutRef.current) {
      clearTimeout(copyTimeoutRef.current);
      copyTimeoutRef.current = null;
    }
    const onCopied = () => {
      setCopiedTag(tag);
      copyTimeoutRef.current = window.setTimeout(() => {
        setCopiedTag(null);
        copyTimeoutRef.current = null;
      }, COPIED_TIMEOUT_MS);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(tag).then(onCopied).catch(() => {
        // Fallback for environments without clipboard API (e.g., HTTP)
        const textarea = document.createElement('textarea');
        textarea.value = tag;
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.appendChild(textarea);
        textarea.select();
        try { document.execCommand('copy'); } catch (_) { /* ignore */ }
        document.body.removeChild(textarea);
        onCopied();
      });
    } else {
      // Fallback for environments without navigator.clipboard
      const textarea = document.createElement('textarea');
      textarea.value = tag;
      textarea.style.position = 'fixed';
      textarea.style.opacity = '0';
      document.body.appendChild(textarea);
      textarea.select();
      try { document.execCommand('copy'); } catch (_) { /* ignore */ }
      document.body.removeChild(textarea);
      onCopied();
    }
  };

  // 宪法查看弹窗状态
  const [constitutionModalOpen, setConstitutionModalOpen] = useState(false);
  const [constitutionContent, setConstitutionContent] = useState<string | null>(null);
  const [constitutionLoading, setConstitutionLoading] = useState(false);

  // 新增：质检弹窗状态
  const [qaModalData, setQaModalData] = useState<{
    milestoneId: string;
    qaResult: QAResult;
  } | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // === Phase 3 轮询执行状态 ===
  useEffect(() => {
    if (!isExecuting) return;

    const interval = setInterval(async () => {
      try {
        const status = await invoke<PipelineState | null>("get_execution_status");
        setExecutionStatus(status);

        if (status && (status.status === "Completed" || status.status === "Failed")) {
          setIsExecuting(false);
          clearInterval(interval);
        }
      } catch (e) {
        console.error("轮询状态失败:", e);
      }
    }, 2000);

    return () => clearInterval(interval);
  }, [isExecuting]);

  const getStatusIcon = (status: string) => {
    switch (status) {
      case "Pending": return "○";
      case "InProgress": return "●";
      case "Completed": return "✅";
      case "Paused": return "⏸";
      default: return "○";
    }
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case "Pending": return "color-gray";
      case "InProgress": return "color-blue";
      case "Completed": return "color-green";
      case "Paused": return "color-yellow";
      default: return "color-gray";
    }
  };

  const startEdit = (id: string, currentVersion: string) => {
    setEditingId(id);
    setEditValue(currentVersion);
  };

  const saveEdit = (milestoneId: string) => {
    if (!editingId) return;
    if (onVersionEdit) {
      onVersionEdit(milestoneId, editValue);
    }
    setEditingId(null);
    setEditValue("");
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditValue("");
  };

  // === 3Phase 3 新增函数 ===
  const handleGeneratePlan = async (midStage: any) => {
    setIsGeneratingPlan(true);
    const generated: Subtask[] = [];
    let prevTitle = "";
    let prevResult = "";

    for (let i = 0; i < 3; i++) {
      try {
        const next = await invoke<GeneratedSubtask>("generate_next_prompt", {
          midStageTitle: midStage.title,
          midStageDescription: midStage.description || "",
          previousSubtaskTitle: prevTitle,
          previousSubtaskResult: prevResult,
          fileChanges: [],
          testResult: "",
          isRetry: false,
          retryReason: "",
        });
        generated.push({
          id: `${midStage.id}-st-${i + 1}`,
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
      next.set(midStage.id, generated);
      return next;
    });
    setIsGeneratingPlan(false);
  };
  const handleStartExecution = async (midStage: any) => {
    if (!projectPath) {
      alert("请先在主界面设置项目目录");
      return;
    }
    const plan = generatedPlan.get(midStage.id);
    if (!plan || plan.length === 0) {
      alert("请先生成执行计划");
      return;
    }
    try {
      await invoke("start_execution", {
        projectId: projectId,
        projectPath: projectPath,
        midStageId: midStage.id,
        midStageTitle: midStage.title,
        midStageDescription: midStage.description,
        subtasksJson: JSON.stringify(plan),
      });
      setIsExecuting(true);
    } catch (e) {
      console.error("启动执行失败:", e);
    }
  };
  // 快速模式：为大阶段生成执行计划（跳过中阶段）
  const handleGenerateQuickPlan = async (milestone: any) => {
    setIsGeneratingPlan(true);
    const generated: Subtask[] = [];
    let prevTitle = "";
    let prevResult = "";

    for (let i = 0; i < 3; i++) {
      try {
        const next = await invoke<GeneratedSubtask>("generate_next_prompt", {
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
  // 快速模式：开始执行大阶段的子任务
  const handleStartQuickExecution = async (milestone: any) => {
    if (!projectPath) {
      alert("请先在主界面设置项目目录");
      return;
    }
    const plan = quickGeneratedPlan.get(milestone.id);
    if (!plan || plan.length === 0) {
      alert("请先生成执行计划");
      return;
    }
    try {
      await invoke("start_execution", {
        projectId: projectId,
        projectPath: projectPath,
        midStageId: milestone.id,
        midStageTitle: milestone.title,
        midStageDescription: milestone.description,
        subtasksJson: JSON.stringify(plan),
      });
      setIsExecuting(true);
    } catch (e) {
      console.error("快速模式启动执行失败:", e);
    }
  };
  // 审批处理函数
  const handleApprove = async (midStageId: string) => {
    if (!projectId) return;
    try {
      const resultJson = await invoke<string>("approve_mid_stage", {
        projectId: projectId,
        midStageId: midStageId,
      });
      const result = JSON.parse(resultJson) as {
        next_mid_stage_id: string | null;
        next_milestone_id: string | null;
        project_completed: boolean;
      };

      if (result.project_completed) {
        alert("🎉 项目所有大阶段已完成！");
        return;
      }

      if (result.next_mid_stage_id) {
        setSelectedMidStageId(result.next_mid_stage_id);
        if (autoAdvance) {
          // 需要等待状态更新后再自动执行，通过查找对应的 midStage 数据
          const nextMid = milestones
            .flatMap((m) => m.mid_stages)
            .find((ms) => ms.id === result.next_mid_stage_id);
          if (nextMid) {
            await handleStartExecution(nextMid);
          }
        } else {
          alert("✅ 已批准，下一阶段已就绪，点击执行");
        }
      } else {
        alert("✅ 已批准");
      }
    } catch (e: any) {
      alert(`❌ 批准失败：${e}`);
    }
  };
  const handleReject = async (midStageId: string) => {
    if (!projectId) return;
    try {
      await invoke("reject_mid_stage", {
        projectId: projectId,
        midStageId: midStageId,
      });
      alert("已驳回");
    } catch (e: any) {
      alert(`❌ 驳回失败：${e}`);
    }
  };
  // 回退处理函数
  // 把项目回退到之前保存的某个中阶段（mid_stage）状态。
  const handleRollback = async () => {
    if (!rollbackTarget || !projectId) return;
    try {
      const result = await invoke('git_rollback_to_mid_stage', {
        projectPath: projectPath || '',
        tagName: rollbackTarget.tagName,
        projectId: projectId,
      });
      console.log('回退成功:', result);
      setRollbackTarget(null);
      // TODO: 触发父组件刷新（需新增 onRollback prop 或引入 useProject）
    } catch (err) {
      console.error('回退失败:', err);
      alert('回退失败: ' + err);
      setRollbackTarget(null);
    }
  };

  // 小阶段回退处理函数
  const handleSubtaskRollback = async () => {
    if (!subtaskRollbackTarget || !projectId) return;
    try {
      const result = await invoke('git_rollback_to_subtask', {
        projectPath: projectPath || '',
        projectId: projectId,
        tagName: subtaskRollbackTarget.tagName,
      });
      console.log('小阶段回退成功:', result);
      setSubtaskRollbackTarget(null);
      // TODO: 触发父组件刷新
    } catch (err) {
      console.error('小阶段回退失败:', err);
      alert('小阶段回退失败: ' + err);
      setSubtaskRollbackTarget(null);
    }
  };

  const handleViewConstitution = async () => {
    if (!projectPath) {
      alert("请先设置项目目录");
      return;
    }
    setConstitutionLoading(true);
    setConstitutionModalOpen(true);
    setConstitutionContent(null);
    try {
      const content = await invoke<string>("read_constitution", { projectPath });
      setConstitutionContent(content);
    } catch (err) {
      setConstitutionContent("读取失败：" + err);
    } finally {
      setConstitutionLoading(false);
    }
  };

  const getSubtaskStatusIcon = (status: string) => {
    switch (status) {
      case "waiting":
        return "⏳";
      case "executing":
        return "⚙️";
      case "testing":
        return "🔍";
      case "passed":
        return "✅";
      case "retrying":
        return "🔄";
      default:
        return "○";
    }
  };

  /**
  * ExecutionTree 组件结构（三层嵌套）
  * 
  * 1. 容器 .execution-tree
  *    ├── 标题“执行树”
  *    ├── 空状态：暂无大阶段
  *    └── 大阶段列表 .tree-list
  *        └── 每个大阶段 .tree-item
  *            ├── 状态图标 + 可编辑版本号 + 标题
  *            ├── 拆解中阶段按钮（专业模式且无中阶段时）
  *            ├── 大阶段状态文字
  *            └── 中阶段列表 .mid-stage-list
  *                └── 每个中阶段 .mid-stage-item
  *                    ├── 前缀图标 + 版本号 + 标题 + 技术焦点 + 状态
  *                    └── 选中后显示操作区 .mid-stage-actions
  *                        ├── 生成执行计划按钮
  *                        ├── 已生成计划（子任务列表 + 开始执行）
  *                        ├── 执行状态面板（进度点 + 当前日志）
  *                        └── 完成后显示测试日志 + 审批按钮
  *  回退按钮 midStage.status === 'Completed' || midStage.status 
  *  回退弹窗 Model 末尾添加
  */
  return (
    <div className="execution-tree">
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <h3 className="tree-title" style={{ margin: 0 }}>执行树</h3>
        <button
          onClick={handleViewConstitution}
          title="查看项目宪法"
          style={{
            background: 'transparent',
            border: '1px solid #ddd',
            borderRadius: '4px',
            padding: '2px 8px',
            cursor: 'pointer',
            fontSize: '16px',
          }}
        >
          📋
        </button>
      </div>
      {milestones.length === 0 ? (
        <p className="tree-empty">暂无大阶段，输入你的灵感吧</p>
      ) : (
        <ul className="tree-list">
          {milestones.map((ms) => (
            <li
              key={ms.id}
              className="tree-item"
              onClick={() => onSelectMilestone(ms.id)}
            >
              {/* 大阶段头部 */}
              <span className={`tree-icon ${getStatusColor(ms.status)}`}>
                {getStatusIcon(ms.status)}
              </span>

              {/* 质检标记 */}
              {ms.qa_result != null && ms.qa_result.passed === true && (
                <span className="qa-passed">✅ 质检通过</span>
              )}
              {ms.qa_result != null && ms.qa_result.passed === false && (
                <span
                  className="qa-rejected"
                  onClick={(e) => {
                    e.stopPropagation();
                    setQaModalData({ milestoneId: ms.id, qaResult: ms.qa_result! });
                  }}
                >
                  ❌ 质检驳回
                </span>
              )}

              {/* 可编辑的版本号 */}
              <span
                className="tree-version"
                onClick={(e) => { e.stopPropagation(); startEdit(ms.id, ms.version); }}
              >
                {editingId === ms.id ? (
                  <input
                    className="version-edit-input"
                    value={editValue}
                    onChange={(e) => setEditValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") saveEdit(ms.id);
                      if (e.key === "Escape") cancelEdit();
                    }}
                    onClick={(e) => e.stopPropagation()}
                    autoFocus
                  />
                ) : (
                  <>{ms.version} ·</>
                )}
              </span>

              <span className="tree-label">{ms.title}</span>

              {/* 拆解中阶段按钮（专业模式且无中阶段） */}
              {ms.mode === "Professional" && ms.mid_stages.length === 0 && onGenerateMidStages && (
                <button
                  className="btn-mid-stage"
                  onClick={(e) => { e.stopPropagation(); onGenerateMidStages(ms.id); }}
                >
                  拆解中阶段
                </button>
              )}

              <span className={`tree-status ${getStatusColor(ms.status)}`}>
                {ms.status === "Pending" && "待开始"}
                {ms.status === "InProgress" && "进行中"}
                {ms.status === "Completed" && "已完成"}
                {ms.status === "Paused" && "暂停"}
              </span>

              {/* 快速模式操作区（跳过中阶段，直接管理 subtasks） */}
              {ms.mode === "Quick" && (
                <div className="quick-mode-actions" onClick={(e) => e.stopPropagation()}>
                  {(!ms.subtasks || ms.subtasks.length === 0) && !quickGeneratedPlan.has(ms.id) && (
                    <button
                      className="btn-generate-plan"
                      onClick={() => handleGenerateQuickPlan(ms)}
                      disabled={isExecuting || isGeneratingPlan}
                    >
                      {isGeneratingPlan ? "⏳ 生成中..." : "📋 生成执行计划"}
                    </button>
                  )}
                  {quickGeneratedPlan.has(ms.id) && (
                    <div className="subtask-plan">
                      {quickGeneratedPlan.get(ms.id)!.map((st, idx) => (
                        <div key={st.id} className="subtask-plan-item">
                          <span>{idx + 1}.</span>
                          <span>{st.title}</span>
                        </div>
                      ))}
                      <button
                        className="btn-start-execution"
                        onClick={() => handleStartQuickExecution(ms)}
                        disabled={isExecuting}
                      >
                        ▶ 开始执行
                      </button>
                    </div>
                  )}
                  {executionStatus && executionStatus.mid_stage_id === ms.id && (
                    <div className="execution-panel">
                      <div className="execution-progress">
                        {executionStatus.subtask_statuses.map((st) => (
                          <span
                            key={st.subtask_id}
                            className="exec-step"
                            title={st.title}
                          >
                            {getSubtaskStatusIcon(st.status)}
                          </span>
                        ))}
                      </div>
                      <div className="execution-log">{executionStatus.current_log}</div>
                    </div>
                  )}
                </div>
              )}
              {/* 中阶段列表（第三级） */}
              {ms.mid_stages.length > 0 && (
                <ul className="mid-stage-list">
                  {ms.mid_stages.map((mid) => (
                    <li
                      key={mid.id}
                      className={`mid-stage-item ${selectedMidStageId === mid.id ? "selected" : ""}`}
                      onClick={() =>
                        setSelectedMidStageId(
                          mid.id === selectedMidStageId ? null : mid.id
                        )
                      }
                    >
                      <span className="mid-stage-icon">├─</span>
                      <span className="mid-stage-version">{mid.version}</span>
                      <span className="mid-stage-title">{mid.title}</span>
                      {mid.git_tag && (
                        <span className="mid-stage-tag" style={{ color: "#888", fontSize: "0.75rem", marginLeft: 8 }}>
                          {mid.git_tag}
                        </span>
                      )}
                      <span className="mid-stage-focus">{mid.tech_focus}</span>
                      <span className="mid-stage-status">
                        {mid.status === "Pending" && "○ 待开始"}
                        {mid.status === "Ready" && "◉ 已就绪"}
                        {mid.status === "InProgress" && "● 进行中"}
                        {mid.status === "Completed" && "✅ 已完成"}
                        {mid.status === "Rejected" && "❌ 驳回"}
                        {mid.status === "RolledBack" && (
                          <span style={{ color: "#999", textDecoration: "line-through" }}>↩ 已回退</span>
                        )}
                        {mid.status === "Approved" && "✅ 已批准"}
                      </span>
                      {/* 回退按钮：已完成或已批准的节点可以回退 */}
                      {(mid.status === "Completed" || mid.status === "Approved") && (
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            const tagName = mid.git_tag || `metheus/${mid.version}`;
                            setRollbackTarget({
                              tagName,
                              version: mid.version,
                            });
                          }}
                          title="回退到此节点"
                          style={{
                            marginLeft: "8px",
                            padding: "2px 8px",
                            fontSize: "12px",
                            border: "1px solid #ccc",
                            borderRadius: "4px",
                            background: "#f8f8f8",
                            cursor: "pointer",
                            color: "#888",
                          }}
                        >
                          ↩ 回退到此
                        </button>
                      )}
                      {/* 中阶段操作区：选中后显示操作区域 */}
                      {selectedMidStageId === mid.id && (
                        <div className="mid-stage-actions">
                          <button
                            className="btn-generate-plan"
                            onClick={(e) => {
                              e.stopPropagation();
                              handleGeneratePlan(mid);
                            }}
                            disabled={isExecuting || isGeneratingPlan}
                          >
                            {isGeneratingPlan ? "⏳ 生成中..." : "📋 生成执行计划"}
                          </button>
                          {/* 已生成的计划 */}
                          {generatedPlan.get(mid.id) && (
                            <div className="subtask-plan">
                              {generatedPlan.get(mid.id)!.map((st, idx) => (
                                <div key={st.id} className="subtask-plan-item">
                                  <span>{idx + 1}.</span>
                                  <span>{st.title}</span>
                                </div>
                              ))}
                              <button
                                className="btn-start-execution"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  handleStartExecution(mid);
                                }}
                                disabled={isExecuting}
                              >
                                ▶ 开始执行
                              </button>
                            </div>
                          )}
                          {/* 执行状态面板 */}
                          {executionStatus && executionStatus.mid_stage_id === mid.id && (
                            <div className="execution-panel">
                              <div className="execution-progress">
                                {executionStatus.subtask_statuses.map(
                                  (st) => (
                                    <span
                                      key={st.subtask_id}
                                      className="exec-step"
                                      title={st.title}
                                    >
                                      {getSubtaskStatusIcon(st.status)}
                                    </span>
                                  )
                                )}
                              </div>
                              <div className="execution-log">
                                {executionStatus.current_log}
                              </div>
                              {executionStatus.status === "Running" && (
                                <button
                                  className="btn-pause"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    invoke("pause_execution").catch((err) =>
                                      console.error("暂停失败:", err)
                                    );
                                  }}
                                >
                                  ⏸ 暂停
                                </button>
                              )}
                              {executionStatus.status === "Paused" && (
                                <button
                                  className="btn-resume"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    invoke("resume_execution").catch((err) =>
                                      console.error("恢复失败:", err)
                                    );
                                  }}
                                >
                                  ▶ 恢复
                                </button>
                              )}
                            </div>
                          )}
                          {/* 完成后显示测试日志 + 审批 */}
                          {executionStatus && executionStatus.mid_stage_id === mid.id && executionStatus.status === "Completed" && (
                            <div className="test-log-section">
                              <h4>📋 测试日志</h4>
                              {executionStatus.subtask_statuses.map((st) => (
                                <div key={st.subtask_id} className="test-log-item">
                                  <div className="test-log-header">
                                    <span>{st.test_result?.passed ? "✅" : st.status === "passed" ? "✅" : "❌"}</span>
                                    <span className="test-log-title">{st.title}</span>
                                    {st.retry_count > 0 && (
                                      <span className="retry-badge">重试 {st.retry_count} 次</span>
                                    )}
                                  </div>
                                  {st.test_result && st.test_result.issues.length > 0 && (
                                    <ul className="test-issues">
                                      {st.test_result.issues.map((issue, i) => (
                                        <li key={i}>{issue}</li>
                                      ))}
                                    </ul>
                                  )}
                                  {st.test_result?.suggestion && (
                                    <div className="test-suggestion">💡 {st.test_result.suggestion}</div>
                                  )}
                                </div>
                              ))}
                              {/* 汇总统计 */}
                              <div className="test-summary">
                                <span>✅ {executionStatus.subtask_statuses.filter(s => s.status === "passed").length}/{executionStatus.total_subtasks} 通过</span>
                                <span>🔄 总重试次数：{executionStatus.subtask_statuses.reduce((sum, s) => sum + s.retry_count, 0)}</span>
                              </div>
                              {/* 审批按钮 */}
                              <div className="approval-buttons">
                                <button className="btn-approve" onClick={() => handleApprove(mid.id)}>✅ 批准</button>
                                <button className="btn-reject" onClick={() => handleReject(mid.id)}>❌ 驳回</button>
                                <label className="auto-advance-toggle" title="开启后，批准当前阶段将自动开始执行下一阶段">
                                  <input
                                    type="checkbox"
                                    checked={autoAdvance}
                                    onChange={(e) => setAutoAdvance(e.target.checked)}
                                  />
                                  <span>⚡ 自动推进下一阶段</span>
                                </label>
                              </div>
                            </div>
                          )}
                          {/* 小阶段列表（已完成的中阶段显示，含回退按钮） */}
                          {(mid.status === 'Completed' || mid.status === 'Approved') && mid.subtasks && mid.subtasks.length > 0 && (
                            <div className="subtask-list-section" onClick={(e) => e.stopPropagation()}>
                              <h4>📋 小阶段</h4>
                              {mid.subtasks.map((st, idx) => {
                                const isRolledBack = st.status === 'RolledBack';
                                return (
                                  <div
                                    key={st.id}
                                    className="subtask-list-item"
                                    style={{
                                      display: 'flex',
                                      alignItems: 'center',
                                      gap: '8px',
                                      padding: '4px 0',
                                      fontSize: '13px',
                                      color: isRolledBack ? '#999' : '#333',
                                      textDecoration: isRolledBack ? 'line-through' : 'none',
                                    }}
                                  >
                                    <span>{idx + 1}.</span>
                                    <span>{st.title}</span>
                                    {/* Tag 展示：已完成且有 auto_tag 的小阶段 */}
                                    {st.status === 'Passed' && st.auto_tag && (
                                      <>
                                        <span
                                          title="点击复制 Tag 名称"
                                          onClick={(e) => {
                                            e.stopPropagation();
                                            handleCopyTag(st.auto_tag!);
                                          }}
                                          style={{
                                            fontSize: '11px',
                                            color: '#888',
                                            fontFamily: 'monospace',
                                            marginLeft: '8px',
                                            whiteSpace: 'nowrap',
                                            padding: '1px 4px',
                                            background: '#f5f5f5',
                                            borderRadius: '3px',
                                            border: '0.5px solid #ddd',
                                            cursor: 'pointer',
                                          }}
                                        >
                                          {st.auto_tag}
                                        </span>
                                        {copiedTag === st.auto_tag && (
                                          <span style={{ color: '#4caf50', fontSize: '10px', marginLeft: '4px' }}>
                                            ✓ 已复制
                                          </span>
                                        )}
                                      </>
                                    )}
                                    {st.auto_tag && !isRolledBack && (
                                      <button
                                        onClick={() => {
                                          setSubtaskRollbackTarget({
                                            tagName: st.auto_tag!,
                                            subtaskTitle: st.title,
                                            midStageVersion: mid.version,
                                            subtaskIndex: idx + 1,
                                          });
                                        }}
                                        title={`回退到小阶段 ${idx + 1}`}
                                        style={{
                                          marginLeft: 'auto',
                                          padding: '1px 6px',
                                          fontSize: '11px',
                                          border: '1px solid #ccc',
                                          borderRadius: '3px',
                                          background: '#f8f8f8',
                                          cursor: 'pointer',
                                          color: '#888',
                                        }}
                                      >
                                        ↩
                                      </button>
                                    )}
                                    {isRolledBack && (
                                      <span style={{ marginLeft: 'auto', color: '#ccc', fontSize: '11px' }}>已回退</span>
                                    )}
                                  </div>
                                );
                              })}
                            </div>
                          )}
                        </div>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
      {/* ===== 回退确认弹窗 ===== */}
      <Modal
        isOpen={rollbackTarget !== null}
        onClose={() => setRollbackTarget(null)}
        title="确认回退"
      >
        <div style={{ padding: '16px' }}>
          <p>项目将回退到 <strong>{rollbackTarget?.tagName}</strong></p>
          <p style={{ color: '#e74c3c', fontSize: '14px', marginTop: '8px' }}>
            该节点之后的所有已完成节点将被标记为"已回退"。
          </p>
          <p style={{ color: '#888', fontSize: '13px', marginTop: '4px' }}>
            如果工作区有未提交的变更，它们将被临时存储。
          </p>
          <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end', marginTop: '16px' }}>
            <button
              onClick={() => setRollbackTarget(null)}
              style={{
                padding: '6px 16px',
                border: '1px solid #ccc',
                borderRadius: '4px',
                background: '#fff',
                cursor: 'pointer',
              }}
            >
              取消
            </button>
            <button
              onClick={handleRollback}
              style={{
                padding: '6px 16px',
                border: 'none',
                borderRadius: '4px',
                background: '#e74c3c',
                color: '#fff',
                cursor: 'pointer',
              }}
            >
              确认回退
            </button>
          </div>
        </div>
      </Modal>

      {/* ===== 小阶段回退确认弹窗 ===== */}
      <Modal
        isOpen={subtaskRollbackTarget !== null}
        onClose={() => setSubtaskRollbackTarget(null)}
        title="确认回退"
      >
        <div style={{ padding: '16px' }}>
          <p>
            将回退到 <strong>{subtaskRollbackTarget?.midStageVersion}</strong> 的
            小阶段 {subtaskRollbackTarget?.subtaskIndex}：{subtaskRollbackTarget?.subtaskTitle}
          </p>
          <p style={{ color: '#e74c3c', fontSize: '14px', marginTop: '8px' }}>
            该小阶段之后的所有变更将被清除。
          </p>
          <p style={{ color: '#888', fontSize: '13px', marginTop: '4px' }}>
            如果工作区有未提交的变更，它们将被临时存储。
          </p>
          <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end', marginTop: '16px' }}>
            <button
              onClick={() => setSubtaskRollbackTarget(null)}
              style={{
                padding: '6px 16px',
                border: '1px solid #ccc',
                borderRadius: '4px',
                background: '#fff',
                cursor: 'pointer',
              }}
            >
              ❌ 取消
            </button>
            <button
              onClick={handleSubtaskRollback}
              style={{
                padding: '6px 16px',
                border: 'none',
                borderRadius: '4px',
                background: '#e74c3c',
                color: '#fff',
                cursor: 'pointer',
              }}
            >
              ✅ 确认回退
            </button>
          </div>
        </div>
      </Modal>

      {/* ===== 质检驳回弹窗 ===== */}
      <Modal
        isOpen={qaModalData !== null}
        onClose={() => setQaModalData(null)}
        title="需求质检结果"
      >
        <div style={{ padding: '16px' }}>
          <p style={{ fontWeight: 600, marginBottom: '8px' }}>质检员发现以下问题：</p>
          <p style={{ color: '#e74c3c', marginBottom: '16px' }}>{qaModalData?.qaResult.reason}</p>

          {qaModalData?.qaResult.details && qaModalData.qaResult.details.length > 0 && (
            <>
              <p style={{ fontWeight: 600, marginBottom: '8px' }}>详细偏差：</p>
              {qaModalData.qaResult.details.map((detail, idx) => (
                <div key={idx} style={{ marginBottom: '8px', paddingLeft: '8px', borderLeft: '3px solid #e74c3c' }}>
                  <p style={{ margin: 0 }}>[{detail.issue_type}] {detail.description}</p>
                  <p style={{ margin: 0, color: '#888', fontSize: '13px' }}>关联需求：{detail.related_requirement}</p>
                </div>
              ))}
            </>
          )}

          {qaModalData?.qaResult.attention_points && qaModalData.qaResult.attention_points.length > 0 && (
            <>
              <p style={{ fontWeight: 600, marginBottom: '8px', marginTop: '16px' }}>需特别关注的要点：</p>
              {qaModalData.qaResult.attention_points.map((point, idx) => (
                <p key={idx} style={{ margin: '4px 0', paddingLeft: '8px' }}>· {point}</p>
              ))}
            </>
          )}

          <hr style={{ margin: '16px 0', border: 'none', borderTop: '1px solid #eee' }} />
          <p style={{ fontWeight: 600, marginBottom: '12px' }}>你希望怎么处理？</p>
          <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
            <button
              onClick={() => setQaModalData(null)}
              style={{
                padding: '6px 16px',
                border: '1px solid #ccc',
                borderRadius: '4px',
                background: '#fff',
                cursor: 'pointer',
              }}
            >
              ✅ 无视，继续推进
            </button>
            <button
              onClick={async () => {
                if (isSubmitting) return;
                setIsSubmitting(true);
                const reason = qaModalData?.qaResult.reason ?? '';
                const details = qaModalData?.qaResult.details;
                let feedback = reason;
                if (details && details.length > 0) {
                  const detailsText = details
                    .map((d) => `[${d.issue_type}] ${d.description}（关联需求：${d.related_requirement}）`)
                    .join('\n');
                  feedback = `${reason}\n\n具体偏差：\n${detailsText}`;
                }
                try {
                  if (onRegenerateMilestones) {
                    await onRegenerateMilestones(feedback);
                  }
                  setQaModalData(null);
                } catch (err) {
                  console.error("重新拆解失败：", err);
                  alert("重新拆解失败：" + err);
                } finally {
                  setIsSubmitting(false);
                }
              }}
              disabled={isSubmitting}
              style={{
                padding: '6px 16px',
                border: 'none',
                borderRadius: '4px',
                background: isSubmitting ? '#c0392b' : '#e74c3c',
                color: '#fff',
                cursor: isSubmitting ? 'not-allowed' : 'pointer',
                opacity: isSubmitting ? 0.7 : 1,
              }}
            >
              {isSubmitting ? "⏳ 正在重新拆解..." : "🔄 采纳意见，重新拆解"}
            </button>
          </div>
        </div>
      </Modal>
      {/* ===== 宪法查看弹窗 ===== */}
      {constitutionModalOpen && (
        <div
          onClick={() => setConstitutionModalOpen(false)}
          style={{
            position: 'fixed',
            inset: 0,
            background: 'rgba(0,0,0,0.4)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 1000,
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              background: '#fff',
              borderRadius: '8px',
              width: '60vw',
              maxHeight: '70vh',
              display: 'flex',
              flexDirection: 'column',
              boxShadow: '0 4px 24px rgba(0,0,0,0.2)',
            }}
          >
            {/* 标题栏 */}
            <div
              style={{
                padding: '12px 16px',
                borderBottom: '1px solid #eee',
                fontWeight: 600,
                fontSize: '16px',
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
              }}
            >
              <span>📋 项目宪法 - CONSTITUTION.md</span>
              <button
                onClick={() => setConstitutionModalOpen(false)}
                style={{
                  background: 'none',
                  border: 'none',
                  fontSize: '18px',
                  cursor: 'pointer',
                  color: '#888',
                }}
              >
                ✕
              </button>
            </div>
            {/* 内容区 */}
            <div
              style={{
                padding: '16px',
                overflow: 'auto',
                flex: 1,
              }}
            >
              {constitutionLoading ? (
                <div style={{ textAlign: 'center', color: '#888', padding: '40px 0' }}>
                  ⏳ 加载中...
                </div>
              ) : (
                <pre
                  style={{
                    whiteSpace: 'pre-wrap',
                    fontFamily: 'monospace',
                    fontSize: '13px',
                    lineHeight: '1.6',
                    color: '#333',
                    margin: 0,
                    padding: '8px',
                    background: '#fafafa',
                    borderRadius: '4px',
                    border: '1px solid #eee',
                  }}
                >
                  {constitutionContent}
                </pre>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default ExecutionTree;
