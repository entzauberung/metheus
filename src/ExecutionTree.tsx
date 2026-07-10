// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { Milestone, Subtask, PipelineState, RollbackToSubtaskPayload } from "./types";
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

  // === Phase B: 从 App.tsx 传入的状态和回调 ===
  selectedMilestoneId: string | null;
  selectedMidStageId: string | null;
  onSelectMidStage: (id: string | null) => void;
  quickGeneratedPlan: Map<string, Subtask[]>;
  generatedPlan?: Map<string, Subtask[]>;
  onGenerateQuickPlan: (milestone: any) => Promise<void>;
  onStartQuickExecution: (milestone: any) => Promise<void>;
  isExecuting: boolean;
  isGeneratingPlan: boolean;
  executionStatus: PipelineState | null;

  // === 阶段三：小阶段回退成功后刷新项目 ===
  onSubtaskRollbackSuccess?: (projectJson: string) => void;
}

function ExecutionTree({
  milestones,
  onSelectMilestone,
  onVersionEdit,
  onGenerateMidStages,
  onRegenerateMilestones,
  projectPath,
  projectId,
  selectedMilestoneId,
  selectedMidStageId,
  onSelectMidStage,
  quickGeneratedPlan,
  generatedPlan,
  onGenerateQuickPlan,
  onStartQuickExecution,
  isExecuting,
  isGeneratingPlan,
  executionStatus,
  onSubtaskRollbackSuccess,
}: Props) {
  // Phase B: QA 弹窗已移至 App.tsx；onRegenerateMilestones 后续由 TaskConsole 使用
  void onRegenerateMilestones;

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  // 回退相关状态（保留在 ExecutionTree）
  const [rollbackTarget, setRollbackTarget] = useState<{
    tagName: string;
    version: string;
  } | null>(null);

  // 小阶段回退状态（保留在 ExecutionTree）
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

  // 宪法查看弹窗状态（保留在 ExecutionTree）
  const [constitutionModalOpen, setConstitutionModalOpen] = useState(false);
  const [constitutionContent, setConstitutionContent] = useState<string | null>(null);
  const [constitutionLoading, setConstitutionLoading] = useState(false);

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

  // 回退处理函数（保留在 ExecutionTree）
  const handleRollback = async () => {
    if (!rollbackTarget || !projectId) return;
    try {
      const result = await invokeWithTimeout('git_rollback_to_mid_stage', {
        projectPath: projectPath || '',
        tagName: rollbackTarget.tagName,
        projectId: projectId,
      });
      console.log('回退成功:', result);
      setRollbackTarget(null);
    } catch (err) {
      console.error('回退失败:', err);
      alert('回退失败: ' + err);
      setRollbackTarget(null);
    }
  };

  // 小阶段回退处理函数（保留在 ExecutionTree）
  const handleSubtaskRollback = async () => {
    if (!subtaskRollbackTarget || !projectId) return;
    try {
      const result = await invokeWithTimeout('rollback_to_subtask_with_reset', {
        projectPath: projectPath || '',
        projectId: projectId,
        tagName: subtaskRollbackTarget.tagName,
      });
      console.log('小阶段回退成功:', result);
      // 通知父组件刷新项目状态
      if (onSubtaskRollbackSuccess && typeof result === 'string') {
        onSubtaskRollbackSuccess(result);
      }
      setSubtaskRollbackTarget(null);
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
      const content = await invokeWithTimeout<string>("read_constitution", { projectPath });
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
  * Phase B 后：专业模式执行面板和 QA 弹窗已移至 App.tsx/TaskConsole。
  * ExecutionTree 只负责树结构渲染 + 版本编辑 + 回退 + 快速模式入口。
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
              className={`tree-item${selectedMilestoneId === ms.id ? " selected" : ""}`}
              style={selectedMilestoneId === ms.id ? { borderLeft: '3px solid #4a90d9', backgroundColor: '#f0f7ff' } : undefined}
              onClick={() => onSelectMilestone(ms.id)}
            >
              {/* 大阶段头部 */}
              <span className={`tree-icon ${getStatusColor(ms.status)}`}>
                {getStatusIcon(ms.status)}
              </span>

              {/* 质检标记 */}
              {ms.qa_result != null && ms.qa_result.passed === true && (
                <span
                  className="qa-passed"
                  title={`✅ 质检通过：${ms.qa_result.reason || "全部对齐"}${
                    (ms.qa_result.warnings && ms.qa_result.warnings.length > 0)
                      ? "\n\n诊断信息：\n" + ms.qa_result.warnings.map(w => "• " + w).join("\n")
                      : ""
                  }`}
                >
                  ✅ 质检通过
                  {ms.qa_result.warnings && ms.qa_result.warnings.length > 0 && (
                    <span className="qa-warning-icon" title={ms.qa_result.warnings.join("\n")}> ⚠️</span>
                  )}
                </span>
              )}
              {ms.qa_result != null && ms.qa_result.passed === false && (
                <span
                  className="qa-rejected"
                  title={`❌ 质检驳回\n\n驳回原因：${ms.qa_result.reason || "未提供具体原因"}${
                    (ms.qa_result.warnings && ms.qa_result.warnings.length > 0)
                      ? "\n\n诊断信息：\n" + ms.qa_result.warnings.map(w => "• " + w).join("\n")
                      : ""
                  }`}
                >
                  ❌ 质检驳回
                  {ms.qa_result.warnings && ms.qa_result.warnings.length > 0 && (
                    <span className="qa-warning-icon" title={ms.qa_result.warnings.join("\n")}> ⚠️</span>
                  )}
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

              {/* 快速模式操作区：通过 props 接收回调 */}
              {ms.mode === "Quick" && (
                <div className="quick-mode-actions" onClick={(e) => e.stopPropagation()}>
                  {(!ms.subtasks || ms.subtasks.length === 0) && !quickGeneratedPlan.has(ms.id) && (
                    <button
                      className="btn-generate-plan"
                      onClick={() => onGenerateQuickPlan(ms)}
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
                        onClick={() => onStartQuickExecution(ms)}
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
                        onSelectMidStage(
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

                      {/* 中阶段操作区：保留但不含专业模式执行面板 */}
                      {selectedMidStageId === mid.id && (
                        <div className="mid-stage-actions">
                          {/* Phase B: 执行计划/执行/审批/测试日志已移至 TaskConsole */}
                          {/* 专业模式 generatedPlan（仅当 subtasks 未持久化时显示） */}
                          {generatedPlan && generatedPlan.has(mid.id) && (!mid.subtasks || mid.subtasks.length === 0) && (
                            <div className="subtask-plan" onClick={(e) => e.stopPropagation()}>
                              {generatedPlan.get(mid.id)!.map((st, idx) => (
                                <div key={st.id} className="subtask-plan-item">
                                  <span>{idx + 1}.</span>
                                  <span>{st.title || `小阶段 ${idx + 1}`}</span>
                                </div>
                              ))}
                            </div>
                          )}
                          {/* 小阶段列表（已完成的中阶段显示，含回退按钮） */}
                          {(mid.status === 'Completed' || mid.status === 'Approved') && mid.subtasks && mid.subtasks.length > 0 && (
                            <div className="subtask-list-section" onClick={(e) => e.stopPropagation()}>
                              <h4>📋 小阶段</h4>
                              {mid.subtasks.map((st, idx) => {
                                const isRolledBack = st.status === 'RolledBack';
                                const prevSt = idx > 0 ? mid.subtasks[idx - 1] : null;
                                const showDivider = prevSt?.status === 'Passed' && st.status === 'Pending';
                                return (
                                  <div key={st.id}>
                                    {showDivider && (
                                      <div className="rollback-divider">
                                        <span>↩ 回退分割点 — 从此处开始后续任务</span>
                                      </div>
                                    )}
                                    <div
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
