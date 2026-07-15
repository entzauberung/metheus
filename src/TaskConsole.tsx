import { useCallback, useEffect, useRef, useState } from "react";
import * as Tabs from "@radix-ui/react-tabs";
import { CheckCircle2, FileDiff, FileText, History, Layers, Milestone, Tags } from "lucide-react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import type { ChangeHistoryEntry, ConstitutionChangeHistory, ExecutionHistoryEntry, GitTagTree, PipelineState, TestLog } from "./types";

const LOG_LEVEL_ICON: Record<string, string> = {
  info: "ℹ",
  success: "✅",
  error: "❌",
  pause: "⏸",
};

function formatLogTime(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return "";
  }
}

/** 将 diff 文本逐行着色：增行绿色、删行红色、文件头/区块头高亮 */
function renderDiffLines(diffText: string): React.ReactNode[] {
  if (!diffText) return [];
  return diffText.split("\n").map((line, i) => {
    let cls = "diff-line";
    if (line.startsWith("+") && !line.startsWith("+++")) {
      cls += " diff-add";
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      cls += " diff-del";
    } else if (line.startsWith("@@")) {
      cls += " diff-hunk";
    } else if (line.startsWith("diff ")) {
      cls += " diff-header";
    } else if (line.startsWith("---") || line.startsWith("+++")) {
      cls += " diff-file";
    }
    return <div key={i} className={cls}>{line}</div>;
  });
}

interface TaskConsoleProps {
  projectPath: string;
  /** 项目名称（用于获取变更历史） */
  projectName?: string;
  executionStatus: PipelineState | null;
  testLogs: TestLog[];
  /** Whether the Git workspace is ready for read-only Git operations */
  workspaceReady?: boolean;
  /** 持久化执行操作历史（刷新不丢） */
  executionHistory?: ExecutionHistoryEntry[];
}

export default function TaskConsole({
  projectPath,
  projectName,
  executionStatus,
  testLogs: _testLogs,
  workspaceReady = false,
  executionHistory,
}: TaskConsoleProps) {
  const [activeTab, setActiveTab] = useState("logs");
  const [currentDiff, setCurrentDiff] = useState("");
  const [changeHistory, setChangeHistory] = useState<ChangeHistoryEntry[]>([]);
  const [constitutionHistory, setConstitutionHistory] = useState<ConstitutionChangeHistory | null>(null);
  const [gitTagTree, setGitTagTree] = useState<GitTagTree | null>(null);
  const [loading, setLoading] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);

  const loadTab = useCallback(async () => {
    if (!projectPath) return;
    // Git-dependent tabs: skip when workspace not ready
    if (!workspaceReady && (activeTab === "diff" || activeTab === "tags")) {
      return;
    }
    setLoading(true);
    try {
      if (activeTab === "diff") {
        setCurrentDiff(await invokeWithTimeout<string>("get_current_diff", { projectPath }));
        if (projectName) {
          setChangeHistory(await invokeWithTimeout<ChangeHistoryEntry[]>("get_change_history", { projectName }));
        }
      } else if (activeTab === "constitution") {
        if (projectName) {
          setConstitutionHistory(await invokeWithTimeout<ConstitutionChangeHistory>(
            "get_constitution_change_history", { projectName, projectPath }));
        }
      } else if (activeTab === "tags") {
        if (projectName) {
          setGitTagTree(await invokeWithTimeout<GitTagTree>("get_git_tags_summary", { projectName }));
        }
      }
    } catch (error) {
      console.error("加载项目检查信息失败", error);
    } finally {
      setLoading(false);
    }
  }, [activeTab, projectPath, projectName, workspaceReady]);

  useEffect(() => {
    loadTab();
  }, [loadTab]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [executionStatus?.log_history?.length, executionStatus?.current_log]);

  return (
    <div className="task-console task-console-readonly">
      <Tabs.Root value={activeTab} onValueChange={setActiveTab}>
        <Tabs.List className="task-tabs" aria-label="项目检查信息">
          <Tabs.Trigger className="task-tab" value="logs"><History size={15} />执行日志</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="diff"><FileDiff size={15} />代码变更</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="constitution"><FileText size={15} />宪法摘要</Tabs.Trigger>
          <Tabs.Trigger className="task-tab" value="tags"><Tags size={15} />Git 标签</Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content className="task-tab-content" value="logs">
          <div ref={logRef} className="execution-log-list">
            {/* 持久化执行历史（主视图，刷新不丢） */}
            {executionHistory && executionHistory.length > 0 ? (
              executionHistory.map((entry, i) => (
                <div key={`${entry.timestamp}-${i}`} className={`execution-log-entry log-${entry.level}`}>
                  <span className="execution-log-time">{formatLogTime(entry.timestamp)}</span>
                  <span className="execution-log-level">{LOG_LEVEL_ICON[entry.level] || ""}</span>
                  <span className="execution-log-text">{entry.text}</span>
                </div>
              ))
            ) : executionStatus?.log_history?.length ? (
              // 回退：旧项目没有持久化历史时，使用内存日志
              executionStatus.log_history.map((entry, i) => (
                <div key={`${entry.timestamp}-${i}`} className={`execution-log-entry log-${entry.level}`}>
                  <span className="execution-log-time">{formatLogTime(entry.timestamp)}</span>
                  <span className="execution-log-level">{LOG_LEVEL_ICON[entry.level] || ""}</span>
                  <span className="execution-log-text">{entry.text}</span>
                </div>
              ))
            ) : executionStatus?.current_log ? (
              <pre>{executionStatus.current_log}</pre>
            ) : (
              <p className="execution-log-empty">暂无执行日志。执行操作后将在此显示历史记录。</p>
            )}
            {/* 当前实时状态（如果正在执行中） */}
            {executionStatus?.status === "Running" && executionStatus.current_log && (
              <div className="execution-log-entry log-live">
                <span className="execution-log-time">现在</span>
                <span className="execution-log-level">⚡</span>
                <span className="execution-log-text">{executionStatus.current_log}</span>
              </div>
            )}
          </div>
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="diff">
          {!workspaceReady ? (
            <p>Git 尚未准备好，暂不显示代码变更。</p>
          ) : loading ? (
            <p>加载中...</p>
          ) : (
            <div className="change-history-view">
              {/* 变更历史（按确认时间排列） */}
              {changeHistory.length > 0 ? (
                changeHistory.slice().reverse().map((entry, i) => (
                  <details key={`${entry.subtask_id}-${i}`} className="change-history-entry">
                    <summary className="change-history-summary">
                      <span className="change-history-time">{formatLogTime(entry.recorded_at)}</span>
                      <span className="change-history-title">{entry.subtask_title}</span>
                      <span className="change-history-files">
                        {entry.files_changed.length} 个文件
                        {entry.diff_truncated && "（diff 已截断）"}
                      </span>
                    </summary>
                    <div className="change-history-files-list">
                      {entry.files_changed.map((f) => (
                        <span key={f} className="changed-file-tag">{f}</span>
                      ))}
                    </div>
                    <pre className="diff-view">{renderDiffLines(entry.diff_text)}</pre>
                  </details>
                ))
              ) : currentDiff ? (
                <>
                  <div className="change-history-current-label">当前工作区未提交变更</div>
                  <pre className="diff-view">{renderDiffLines(currentDiff)}</pre>
                </>
              ) : (
                <p>暂无代码变更</p>
              )}
            </div>
          )}
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="constitution">
          {loading ? <p>加载中...</p> : constitutionHistory ? (
            <div className="constitution-history-view">
              {/* Token 预测卡片 */}
              <div className={`constitution-token-card${constitutionHistory.needs_compaction ? " needs-compaction" : ""}`}>
                <div className="constitution-token-label">宪法第二部分 Token 预测</div>
                <div className="constitution-token-value">
                  {constitutionHistory.current_token_estimate.toFixed(0)}
                  <span className="constitution-token-unit"> tokens</span>
                </div>
                <div className="constitution-token-threshold">
                  剪枝阈值：{constitutionHistory.compaction_threshold} tokens
                </div>
                {constitutionHistory.needs_compaction && (
                  <div className="constitution-compaction-warning">⚠ 已超过剪枝阈值，建议执行宪法压缩</div>
                )}
              </div>

              {/* 变更历史列表 */}
              {constitutionHistory.entries.length > 0 ? (
                <div className="constitution-change-list">
                  <div className="constitution-section-title">第二部分变更历史</div>
                  {constitutionHistory.entries.slice().reverse().map((entry, i) => (
                    <div key={`${entry.timestamp}-${i}`} className="constitution-change-entry">
                      <div className="constitution-change-header">
                        <span className="constitution-change-time">{formatLogTime(entry.timestamp)}</span>
                        <span className="constitution-change-subtask">{entry.subtask_title}</span>
                      </div>
                      <div className="constitution-change-summary">{entry.change_summary}</div>
                      <div className="constitution-change-tokens">
                        当时 token 估算：{entry.token_estimate.toFixed(0)}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <p style={{ color: "#656d76", fontSize: "13px", marginTop: "12px" }}>
                  暂无宪法第二部分变更历史。确认小阶段后若宪法更新，变更记录将在此显示。
                </p>
              )}

              {/* 次级入口：查看完整宪法 */}
              <details className="constitution-full-view">
                <summary className="constitution-full-toggle">查看当前完整宪法</summary>
                {projectPath ? (
                  <ConstitutionReader projectPath={projectPath} />
                ) : (
                  <p style={{ color: "#656d76", fontSize: "12px" }}>项目路径未设置。</p>
                )}
              </details>
            </div>
          ) : <p>暂无宪法变更历史</p>}
        </Tabs.Content>
        <Tabs.Content className="task-tab-content" value="tags">
          {loading ? (
            <p>加载中...</p>
          ) : gitTagTree && gitTagTree.milestones.length > 0 ? (
            <div className="git-tag-tree">
              {gitTagTree.milestones.map((ms) => (
                <details key={ms.milestone_id} className={`tag-tree-milestone status-${ms.milestone_status.toLowerCase()}`}>
                  <summary className="tag-tree-milestone-header">
                    <Milestone size={14} className="tag-tree-icon" />
                    <span className="tag-tree-label">{ms.milestone_title}</span>
                    <code className="tag-tree-version">{ms.milestone_version}</code>
                    <StatusBadgeInline status={ms.milestone_status} />
                  </summary>
                  {ms.mid_stages.length > 0 ? (
                    <div className="tag-tree-children">
                      {ms.mid_stages.map((mid) => (
                        <details key={mid.mid_stage_id} className={`tag-tree-midstage status-${mid.mid_stage_status.toLowerCase()}`}>
                          <summary className="tag-tree-midstage-header">
                            <Layers size={13} className="tag-tree-icon" />
                            <span className="tag-tree-label">{mid.mid_stage_title}</span>
                            <code className="tag-tree-version">{mid.mid_stage_version}</code>
                            {mid.mid_stage_tag ? (
                              <code className="tag-tree-tag-badge" title={mid.mid_stage_tag}>{mid.mid_stage_tag}</code>
                            ) : (
                              <span className="tag-tree-no-tag">无标签</span>
                            )}
                            <StatusBadgeInline status={mid.mid_stage_status} />
                          </summary>
                          {mid.subtasks.length > 0 ? (
                            <div className="tag-tree-children">
                              {mid.subtasks.map((st) => (
                                <div key={st.subtask_id} className={`tag-tree-subtask status-${st.subtask_status.toLowerCase()}`}>
                                  <CheckCircle2 size={12} className={`tag-tree-icon tag-tree-icon-${st.subtask_status.toLowerCase()}`} />
                                  <span className="tag-tree-subtask-index">#{st.subtask_index}</span>
                                  <span className="tag-tree-subtask-title">{st.subtask_title}</span>
                                  {st.subtask_tag ? (
                                    <code className="tag-tree-tag-badge tag-tree-tag-sub" title={st.subtask_tag}>
                                      {mid.mid_stage_version} · 任务 {st.subtask_index}
                                    </code>
                                  ) : (
                                    <span className="tag-tree-no-tag">—</span>
                                  )}
                                  <span className="tag-tree-subtask-status-text">{st.subtask_status}</span>
                                </div>
                              ))}
                            </div>
                          ) : (
                            <p className="tag-tree-empty">暂无小阶段</p>
                          )}
                        </details>
                      ))}
                    </div>
                  ) : (
                    <p className="tag-tree-empty">暂无中阶段</p>
                  )}
                </details>
              ))}
            </div>
          ) : (
            <p>暂无 Git 标签</p>
          )}
        </Tabs.Content>
      </Tabs.Root>
    </div>
  );
}

/** 树节点状态内联标记 */
function StatusBadgeInline({ status }: { status: string }) {
  const lower = status.toLowerCase();
  const color = lower.includes("completed") || lower.includes("passed")
    ? "#1a7f37" : lower.includes("progress") || lower.includes("executing")
    ? "#0969da" : lower.includes("rejected") || lower.includes("failed")
    ? "#cf222e" : lower.includes("rolledback")
    ? "#9a6700" : "#656d76";
  return <span className="tag-tree-status" style={{ color, fontSize: "10px", fontWeight: 600 }}>{status}</span>;
}

/** 只读宪法查看器（次级入口，按需加载） */
function ConstitutionReader({ projectPath }: { projectPath: string }) {
  const [text, setText] = useState<string | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    let cancelled = false;
    invokeWithTimeout<string>("read_constitution", { projectPath })
      .then((t) => { if (!cancelled) setText(t); })
      .catch((e) => { if (!cancelled) setErr(String(e)); });
    return () => { cancelled = true; };
  }, [projectPath]);

  if (err) return <p style={{ color: "#cf222e", fontSize: "12px" }}>读取失败：{err}</p>;
  if (text === null) return <p style={{ color: "#656d76", fontSize: "12px" }}>加载中...</p>;
  if (!text) return <p style={{ color: "#656d76", fontSize: "12px" }}>宪法文件不存在或为空。</p>;
  return <pre className="constitution-full-text">{text}</pre>;
}
