// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect, useCallback, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import {
  Subtask,
  PipelineState,
  QAResult,
  ConstitutionSummary,
  GitTagInfo,
  TestLog,
} from "./types";

// ========== Props 接口 ==========

interface TaskConsoleProps {
  // 项目信息
  projectPath: string;
  projectId: string;

  // 执行状态
  isExecuting: boolean;
  isGeneratingPlan: boolean;
  executionStatus: PipelineState | null;
  generatedPlan: Map<string, Subtask[]>;
  selectedMidStageId: string | null;
  qaModalData: { milestoneId: string; qaResult: QAResult } | null;
  isSubmitting: boolean;

  // 测试日志
  testLogs: TestLog[];

  // 执行控制回调
  onGeneratePlan: (midStageId: string) => Promise<void>;
  onStartExecution: (midStageId: string) => Promise<void>;
  onPause: () => Promise<void>;
  onResume: () => Promise<void>;
  onStop: () => Promise<void>;
  onRegenerateMilestones: (feedback: string) => Promise<void>;

  // 阶段切换
  onEnterReviewMode: () => void;

  // QA 相关
  onDismissQA: () => void;
  onQAIgnore: () => void;

  // 大阶段生成
  projectStatus: string;
  onGenerateMilestones: () => Promise<void>;

  // 中阶段切换
  onNextMidStage?: () => void;
  hasNextMidStage?: boolean;
}

// ========== 标签页定义 ==========

interface TabDef {
  index: number;
  label: string;
  icon: string;
}

const TABS: TabDef[] = [
  { index: 0, label: "代码变更", icon: "📝" },
  { index: 1, label: "执行日志", icon: "📋" },
  { index: 2, label: "宪法更新", icon: "📜" },
  { index: 3, label: "Git 标签", icon: "🏷️" },
];

// ========== 主组件 ==========

function TaskConsole(props: TaskConsoleProps) {
  const [activeTab, setActiveTab] = useState<number>(0);

  // ---- 标签页 1: 代码变更 ----
  const [diffText, setDiffText] = useState<string>("");
  const [diffLoading, setDiffLoading] = useState(false);

  const fetchDiff = useCallback(async () => {
    if (!props.projectPath) return;
    setDiffLoading(true);
    try {
      const result = await invokeWithTimeout<string>("get_current_diff", {
        projectPath: props.projectPath,
      });
      setDiffText(result);
    } catch (e) {
      console.error("获取 diff 失败:", e);
      setDiffText("");
    } finally {
      setDiffLoading(false);
    }
  }, [props.projectPath]);

  useEffect(() => {
    if (activeTab === 0) fetchDiff();
  }, [activeTab, fetchDiff]);

  // ---- 标签页 2: 执行日志 ----
  const logRef = useRef<HTMLPreElement>(null);
  const currentLog = props.executionStatus?.current_log || "";

  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [currentLog]);

  // ---- 标签页 3: 宪法更新 ----
  const [constitutionSummary, setConstitutionSummary] =
    useState<ConstitutionSummary | null>(null);

  const fetchConstitution = useCallback(async () => {
    if (!props.projectPath) return;
    try {
      const result = await invokeWithTimeout<ConstitutionSummary>(
        "get_constitution_summary",
        { projectPath: props.projectPath }
      );
      setConstitutionSummary(result);
    } catch (e) {
      console.error("获取宪法摘要失败:", e);
      setConstitutionSummary(null);
    }
  }, [props.projectPath]);

  useEffect(() => {
    if (activeTab === 2) fetchConstitution();
  }, [activeTab, fetchConstitution]);

  // ---- 标签页 4: Git 标签 ----
  const [gitTags, setGitTags] = useState<GitTagInfo[]>([]);

  const fetchTags = useCallback(async () => {
    if (!props.projectPath) return;
    try {
      const result = await invokeWithTimeout<GitTagInfo[]>("get_git_tags_summary", {
        projectPath: props.projectPath,
      });
      setGitTags(result);
    } catch (e) {
      console.error("获取 Git 标签失败:", e);
      setGitTags([]);
    }
  }, [props.projectPath]);

  useEffect(() => {
    if (activeTab === 3) fetchTags();
  }, [activeTab, fetchTags]);

  // ---- QA 弹窗状态 ----
  const [isRebuilding, setIsRebuilding] = useState(false);

  // ---- 大阶段生成状态 ----
  const [isGeneratingMilestones, setIsGeneratingMilestones] = useState(false);

  // ---- 渲染 ----

  return (
    <div className="task-console">
      {/* ===== 执行面板 ===== */}
      <div className="task-execution-panel">
        <div className="execution-controls">
          {/* 拆解大阶段按钮：批准方案后、尚未拆解时显示 */}
          {props.projectStatus === "Planning" && (
            <button
              className="btn-exec-action primary"
              onClick={async () => {
                setIsGeneratingMilestones(true);
                try {
                  await props.onGenerateMilestones();
                } catch (_) {
                  /* 错误已在 App.tsx 的 handleGenerateMilestones 中处理 */
                } finally {
                  setIsGeneratingMilestones(false);
                }
              }}
              disabled={isGeneratingMilestones}
            >
              {isGeneratingMilestones ? "⏳ 正在拆解..." : "📊 根据版本方案拆解大阶段"}
            </button>
          )}

          {/* 生成执行计划按钮 */}
          {props.projectStatus !== "Planning" && !props.isExecuting && !props.isGeneratingPlan && props.selectedMidStageId && (
            <button
              className="btn-exec-action"
              onClick={() => props.onGeneratePlan(props.selectedMidStageId!)}
            >
              📋 生成执行计划
            </button>
          )}

          {/* 生成中 */}
          {props.isGeneratingPlan && (
            <span className="exec-loading-text">⏳ 正在生成执行计划...</span>
          )}

          {/* 开始执行按钮 */}
          {!props.isExecuting &&
            props.selectedMidStageId &&
            props.generatedPlan.has(props.selectedMidStageId) && (
              <button
                className="btn-exec-action primary"
                onClick={() => props.onStartExecution(props.selectedMidStageId!)}
              >
                ▶ 开始执行
              </button>
            )}

          {/* 暂停 / 恢复 / 停止 */}
          {props.isExecuting && (
            <>
              {props.executionStatus?.status === "Paused" ? (
                <button className="btn-exec-action primary" onClick={props.onResume}>
                  ▶ 恢复
                </button>
              ) : (
                <button className="btn-exec-action" onClick={props.onPause}>
                  ⏸ 暂停
                </button>
              )}
              <button className="btn-exec-action danger" onClick={props.onStop}>
                ⏹ 停止
              </button>
            </>
          )}

          {/* 未选中中阶段提示（仅在有里程碑后显示） */}
          {props.projectStatus !== "Planning" && !props.selectedMidStageId && !props.isExecuting && (
            <span className="exec-hint">请在左侧执行树中选择一个中阶段</span>
          )}
        </div>

        {/* 进度条 */}
        {props.executionStatus && props.executionStatus.status === "Running" && (
          <div className="progress-bar-container">
            <div className="progress-bar">
              <div
                className="progress-fill"
                style={{
                  width: `${Math.round(
                    (props.executionStatus.current_subtask_index /
                      props.executionStatus.total_subtasks) *
                      100
                  )}%`,
                }}
              />
            </div>
            <span className="progress-text">
              小阶段 {props.executionStatus.current_subtask_index}/
              {props.executionStatus.total_subtasks}
            </span>
          </div>
        )}

        {/* 当前执行的小阶段 */}
        {props.executionStatus && (
          <div className="current-subtask-info">
            {props.executionStatus.subtask_statuses
              .filter((s) => s.status === "executing")
              .map((s) => (
                <span key={s.subtask_id}>⚙️ 当前：{s.title}</span>
              ))}
          </div>
        )}

        {/* 中阶段完成提示 */}
        {props.executionStatus?.status === "Completed" && (
          <div className="completion-prompt">
            <div className="completion-text">✅ 中阶段执行完成</div>
            <div className="completion-actions">
              <button
                className="btn-exec-action"
                onClick={() => props.onNextMidStage?.()}
                disabled={!props.hasNextMidStage}
              >
                ▶ 下一个中阶段
              </button>
              <button
                className="btn-exec-action"
                onClick={props.onEnterReviewMode}
              >
                💬 切换到讨论模式审阅
              </button>
            </div>
            {!props.hasNextMidStage && (
              <div className="completion-hint">
                ℹ️ 这是当前大阶段的最后一个中阶段，所有中阶段执行完成。
              </div>
            )}
          </div>
        )}

        {/* 执行失败提示 */}
        {props.executionStatus?.status === "Failed" && (
          <div
            className="failure-prompt"
            onClick={props.onEnterReviewMode}
          >
            <div className="failure-header">❌ 执行失败</div>
            {props.executionStatus.last_error && (
              <div className="failure-detail">{props.executionStatus.last_error}</div>
            )}
            <div style={{ marginTop: 6, fontSize: 12, opacity: 0.7 }}>
              点击切换到讨论模式查看详情
            </div>
          </div>
        )}
      </div>

      {/* ===== 测试日志 ===== */}
      <div className="task-test-logs">
        <h4 className="test-logs-title">📋 测试日志</h4>
        {props.testLogs.length === 0 ? (
          <div className="tab-empty-state">暂无测试记录</div>
        ) : (
          <div className="test-log-list">
            {props.testLogs.map((log, i) => (
              <TestLogItem key={i} log={log} index={i} />
            ))}
          </div>
        )}
      </div>

      {/* ===== 标签页栏 ===== */}
      <div className="tab-bar">
        {TABS.map((tab) => (
          <div
            key={tab.index}
            className={`tab-item${activeTab === tab.index ? " active" : ""}`}
            onClick={() => setActiveTab(tab.index)}
          >
            <span className="tab-icon">{tab.icon}</span>
            <span>{tab.label}</span>
          </div>
        ))}
      </div>

      {/* ===== 标签页内容 ===== */}
      <div className="tab-content">
        {/* 标签页 1：代码变更 */}
        {activeTab === 0 && (
          <div className="tab-panel">
            <div className="tab-panel-header">
              <span>代码变更 (git diff)</span>
              <button
                className="btn-tab-refresh"
                onClick={fetchDiff}
                disabled={diffLoading}
              >
                🔄 刷新
              </button>
            </div>
            {!diffText ? (
              <div className="tab-empty-state">当前无代码变更</div>
            ) : (
              <pre className="diff-container">
                {diffText.split("\n").map((line, i) => {
                  let cls = "";
                  if (line.startsWith("+") && !line.startsWith("+++ "))
                    cls = "diff-add";
                  else if (line.startsWith("-") && !line.startsWith("--- "))
                    cls = "diff-remove";
                  else if (line.startsWith("@@")) cls = "diff-hunk-header";
                  return (
                    <div key={i} className={cls}>
                      {line}
                    </div>
                  );
                })}
              </pre>
            )}
          </div>
        )}

        {/* 标签页 2：执行日志 */}
        {activeTab === 1 && (
          <div className="tab-panel">
            <div className="tab-panel-header">
              <span>执行日志</span>
            </div>
            <pre className="execution-log-viewer" ref={logRef}>
              {currentLog || "等待执行开始..."}
            </pre>
          </div>
        )}

        {/* 标签页 3：宪法更新 */}
        {activeTab === 2 && (
          <div className="tab-panel">
            <div className="tab-panel-header">
              <span>宪法当前状态</span>
              <button className="btn-tab-refresh" onClick={fetchConstitution}>
                🔄 刷新
              </button>
            </div>
            {constitutionSummary ? (
              <div className="constitution-summary">
                <div className="constitution-field">
                  <label>项目结构描述：</label>
                  <pre>{constitutionSummary.structure_description || "（无）"}</pre>
                </div>
                <div className="constitution-field">
                  <label>公开函数数量：</label>
                  <span className="constitution-value">
                    {constitutionSummary.function_count}
                  </span>
                </div>
                <div className="constitution-field">
                  <label>最近变更：</label>
                  {constitutionSummary.recent_changes.length > 0 ? (
                    <ul className="constitution-list">
                      {constitutionSummary.recent_changes.map((c, i) => (
                        <li key={i}>{c}</li>
                      ))}
                    </ul>
                  ) : (
                    <span className="tab-empty-state">暂无变更记录</span>
                  )}
                </div>
                <div className="constitution-field">
                  <label>Token 估算：</label>
                  <span className="constitution-value">
                    {constitutionSummary.total_tokens.toFixed(0)} tokens
                  </span>
                </div>
              </div>
            ) : (
              <div className="tab-empty-state">
                宪法尚未初始化，执行一个小阶段后将自动建立
              </div>
            )}
          </div>
        )}

        {/* 标签页 4：Git 标签 */}
        {activeTab === 3 && (
          <div className="tab-panel">
            <div className="tab-panel-header">
              <span>Git 存档标签</span>
              <button className="btn-tab-refresh" onClick={fetchTags}>
                🔄 刷新
              </button>
            </div>
            {gitTags.length === 0 ? (
              <div className="tab-empty-state">尚无存档标签</div>
            ) : (
              <GitTagsView tags={gitTags} />
            )}
          </div>
        )}
      </div>

      {/* ===== QA 驳回弹窗 ===== */}
      {props.qaModalData && (
        <div className="qa-modal-overlay">
          <div className="qa-modal">
            <h3>需求质检结果</h3>
            <p style={{ color: "#e74c3c", marginBottom: 12 }}>
              {props.qaModalData.qaResult.reason}
            </p>
            {props.qaModalData.qaResult.details.length > 0 && (
              <div className="qa-details">
                {props.qaModalData.qaResult.details.map((d, i) => (
                  <div key={i} className="qa-detail-item">
                    [{d.issue_type}] {d.description}
                  </div>
                ))}
              </div>
            )}
            <div className="qa-modal-actions">
              <button
                className="btn-qa-ignore"
                onClick={() => {
                  props.onDismissQA();
                  props.onQAIgnore();
                }}
              >
                ✅ 无视，继续推进
              </button>
              <button
                className="btn-qa-adopt"
                onClick={async () => {
                  setIsRebuilding(true);
                  try {
                    const feedback = props.qaModalData!.qaResult.reason;
                    await props.onRegenerateMilestones(feedback);
                    props.onDismissQA();
                  } catch (_) {
                    /* ignore */
                  } finally {
                    setIsRebuilding(false);
                  }
                }}
                disabled={isRebuilding}
              >
                {isRebuilding ? "⏳ 正在重新拆解..." : "🔄 采纳意见，重新拆解"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ========== 测试日志子组件 ==========

function TestLogItem({ log, index }: { log: TestLog; index: number }) {
  const [expanded, setExpanded] = useState(false);

  const statusIcon =
    log.status === "passed"
      ? "✅"
      : log.status === "rejected"
        ? "❌"
        : "🔄";
  const statusText =
    log.status === "passed"
      ? "通过"
      : log.status === "rejected"
        ? "打回"
        : "重试后通过";

  return (
    <div className="test-log-item">
      <div className="test-log-header-row">
        <span className="test-log-index">#{index + 1}</span>
        <span className={`test-log-status-badge ${log.status}`}>
          {statusIcon} {statusText}
        </span>
        <span className="test-log-title">{log.subtask_title}</span>
        {(log.reason || log.files) && (
          <span
            className="test-log-toggle"
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "收起 ↑" : "展开 ↓"}
          </span>
        )}
      </div>
      {expanded && (
        <div className="test-log-detail">
          {log.reason && (
            <div className="test-log-reason">
              <strong>原因：</strong>
              {log.reason}
            </div>
          )}
          {log.files && log.files.length > 0 && (
            <div className="test-log-files">
              <strong>文件：</strong>
              {log.files.join(", ")}
            </div>
          )}
          {log.full_report && (
            <pre className="test-log-full-report">{log.full_report}</pre>
          )}
        </div>
      )}
    </div>
  );
}

// ========== Git 标签子组件 ==========

function GitTagsView({ tags }: { tags: GitTagInfo[] }) {
  // 分组：中阶段标签 (metheus/v*) vs 小阶段标签 (metheus/auto/*)
  const midStageTags = tags.filter(
    (t) => t.name.startsWith("metheus/v") && !t.name.includes("/auto/")
  );
  const autoTags = tags.filter((t) => t.name.includes("/auto/"));

  // 小阶段标签按中阶段版本号分组
  const groupByVersion = (ts: GitTagInfo[]): Map<string, GitTagInfo[]> => {
    const groups = new Map<string, GitTagInfo[]>();
    for (const tag of ts) {
      const parts = tag.name.split("/");
      const version = parts.length >= 3 ? parts[2] : "unknown";
      if (!groups.has(version)) groups.set(version, []);
      groups.get(version)!.push(tag);
    }
    return groups;
  };

  const groupedAuto = groupByVersion(autoTags);
  const [collapsedMid, setCollapsedMid] = useState(false);
  const [collapsedAuto, setCollapsedAuto] = useState(false);
  const [collapsedVersions, setCollapsedVersions] = useState<Set<string>>(
    new Set()
  );

  const toggleVersion = (v: string) => {
    setCollapsedVersions((prev) => {
      const next = new Set(prev);
      if (next.has(v)) next.delete(v);
      else next.add(v);
      return next;
    });
  };

  return (
    <div className="git-tags-view">
      {/* 中阶段标签 */}
      {midStageTags.length > 0 && (
        <div className="tag-group">
          <h4
            className="tag-group-header"
            onClick={() => setCollapsedMid(!collapsedMid)}
          >
            📌 中阶段标签 ({midStageTags.length})
            <span className="collapse-arrow">
              {collapsedMid ? "▶" : "▼"}
            </span>
          </h4>
          {!collapsedMid && (
            <div className="tag-list">
              {midStageTags.map((tag) => (
                <div key={tag.name} className="tag-item">
                  <code className="tag-name">{tag.name}</code>
                  <span className="tag-date">{tag.date}</span>
                  <span className="tag-subject">{tag.subject}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 小阶段标签 */}
      {groupedAuto.size > 0 && (
        <div className="tag-group">
          <h4
            className="tag-group-header"
            onClick={() => setCollapsedAuto(!collapsedAuto)}
          >
            🔖 小阶段标签 ({autoTags.length})
            <span className="collapse-arrow">
              {collapsedAuto ? "▶" : "▼"}
            </span>
          </h4>
          {!collapsedAuto && (
            <div className="tag-subgroups">
              {Array.from(groupedAuto.entries()).map(([version, ts]) => {
                const isCollapsed = collapsedVersions.has(version);
                return (
                  <div key={version} className="tag-subgroup">
                    <h5
                      className="tag-subgroup-header"
                      onClick={() => toggleVersion(version)}
                    >
                      {version} ({ts.length} 个标签)
                      <span className="collapse-arrow">
                        {isCollapsed ? "▶" : "▼"}
                      </span>
                    </h5>
                    {!isCollapsed && (
                      <div className="tag-list">
                        {ts.map((tag) => (
                          <div key={tag.name} className="tag-item">
                            <code className="tag-name">
                              {tag.name.split("/").pop()}
                            </code>
                            <span className="tag-date">{tag.date}</span>
                            <span className="tag-subject">{tag.subject}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default TaskConsole;
