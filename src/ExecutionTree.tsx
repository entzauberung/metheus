// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Milestone, Subtask, PipelineState, GeneratedSubtask } from "./types";

interface Props {
  milestones: Milestone[];
  onSelectMilestone: (id: string) => void;
  onVersionEdit?: (id: string, newVersion: string) => void;
  onGenerateMidStages?: (id: string) => void;
  projectPath?: string;
  projectId?: string;
}

function ExecutionTree({ milestones, onSelectMilestone, onVersionEdit, onGenerateMidStages, projectPath, projectId }: Props) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  // === 3Phase 3 新增状态 ===
  const [selectedMidStageId, setSelectedMidStageId] = useState<string | null>(null);
  const [generatedPlan, setGeneratedPlan] = useState<Map<string, Subtask[]>>(new Map());
  const [isExecuting, setIsExecuting] = useState(false);
  const [isGeneratingPlan, setIsGeneratingPlan] = useState(false);
  const [executionStatus, setExecutionStatus] = useState<PipelineState | null>(null);

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
  // 审批处理函数
  const handleApprove = async (midStageId: string) => {
    if (!projectId) return;
    try {
      await invoke("approve_mid_stage", {
        projectId: projectId,
        midStageId: midStageId,
      });
      alert("✅ 已批准");
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
  */
  return (
    <div className="execution-tree">
      <h3 className="tree-title">执行树</h3>
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
                      <span className="mid-stage-focus">{mid.tech_focus}</span>
                      <span className="mid-stage-status">
                        {mid.status === "Pending" && "○ 待开始"}
                        {mid.status === "Ready" && "◉ 已就绪"}
                        {mid.status === "InProgress" && "● 进行中"}
                        {mid.status === "Completed" && "✅ 已完成"}
                        {mid.status === "Rejected" && "❌ 驳回"}
                        {mid.status === "Approved" && "✅ 已批准"}
                      </span>
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
                              </div>
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
    </div>
  );
}

export default ExecutionTree;
