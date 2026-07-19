// src/PreflightPanel.tsx — 三项显式检查面板（后端事实驱动，无本地业务状态）
import { useMemo } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { PreflightCheckResult, Project } from "./types";
import { CheckCircle, XCircle, Clock, ArrowRight, ArrowLeft } from "lucide-react";
import { useState } from "react";

interface PreflightPanelProps {
  projectName: string;
  /** 从 Project.preflight_results 读取的业务事实，面板不维护本地副本 */
  preflightResults: PreflightCheckResult[];
  /** 当前讨论修订号，用于判定检查结果是否过期 */
  discussionRevision: number;
  /** 当前项目数据修订号，用于后端乐观并发校验 */
  dataRevision: number;
  /** 检查完成后回传完整 Project */
  onProjectUpdated: (project: Project) => void;
  /** 返回继续讨论 */
  onReturnToDiscussion: () => void;
  /** 全部通过后生成方案 */
  onAllPassed: () => void;
  /** 重新开始全部检查 */
  onRestartChecks: () => void;
  /** 当前是否有操作正在提交 */
  isSubmitting?: boolean;
  /** 启动托管层（ThreeChecks 后自动推进到大阶段批准） */
  onStartManagedFlow?: () => void;
  /** 托管层是否激活 */
  managedFlowActive?: boolean;
}

const CHECK_ORDER = ["goal_completeness", "reality_consistency", "task_executability"] as const;

const CHECK_LABELS: Record<string, { title: string; desc: string }> = {
  goal_completeness: {
    title: "目标完整性检查",
    desc: "检查项目目标、用户、范围、约束和成功标准是否明确。",
  },
  reality_consistency: {
    title: "现实一致性检查",
    desc: "对比项目路径、已有基线、技术栈与用户目标是否匹配。",
  },
  task_executability: {
    title: "任务可执行性检查",
    desc: "检查目标能否拆分为可验证的小任务，列出阻塞问题。",
  },
};

export function PreflightPanel({
  projectName,
  preflightResults,
  discussionRevision,
  dataRevision,
  onProjectUpdated,
  onReturnToDiscussion,
  onAllPassed,
  onRestartChecks,
  isSubmitting,
  onStartManagedFlow,
  managedFlowActive,
}: PreflightPanelProps) {
  // 仅保留 UI 状态：当前正在加载的检查类型、错误信息
  const [loadingType, setLoadingType] = useState<string | null>(null);
  const [error, setError] = useState<{ type: "network" | "parse" | "business"; message: string } | null>(null);

  // 从 preflightResults prop 构建快速查找表（纯计算，非业务状态）
  const resultMap = useMemo(() => {
    const map: Record<string, PreflightCheckResult> = {};
    for (const r of preflightResults) {
      map[r.check_type] = r;
    }
    return map;
  }, [preflightResults]);

  // 第一个尚未通过或 stale 的检查索引
  const currentIndex = useMemo(() => {
    for (let i = 0; i < CHECK_ORDER.length; i++) {
      const r = resultMap[CHECK_ORDER[i]];
      if (!r || r.stale || !r.passed) return i;
    }
    return CHECK_ORDER.length;
  }, [resultMap]);

  // 是否有任何检查已过期（同时检查 stale 字段和 discussion_revision 不匹配）
  const hasStale = useMemo(() => {
    return Object.values(resultMap).some(
      (r) => r.stale || r.discussion_revision !== discussionRevision
    );
  }, [resultMap, discussionRevision]);

  const allPassed = CHECK_ORDER.every(
    (t) => resultMap[t]?.passed && !resultMap[t]?.stale && resultMap[t]?.discussion_revision === discussionRevision
  );
  const isRunning = loadingType !== null;

  const handleRunCheck = async (checkType: string) => {
    setLoadingType(checkType);
    setError(null);
    try {
      const updatedProject = await invokeWithTimeout<Project>("run_preflight_check", {
        projectName,
        checkType,
        frontendDiscussionRevision: discussionRevision,
        frontendDataRevision: dataRevision,
      });
      if (updatedProject) {
        onProjectUpdated(updatedProject);
      }
    } catch (e: any) {
      const msg = String(e);
      if (msg.includes("超时") || msg.includes("网络")) {
        setError({ type: "network", message: `网络请求失败：${msg}` });
      } else if (msg.includes("解析") || msg.includes("JSON")) {
        setError({ type: "parse", message: `AI 返回格式异常：${msg}` });
      } else {
        setError({ type: "business", message: msg });
      }
    } finally {
      setLoadingType(null);
    }
  };

  return (
    <div className="preflight-panel">
      {/* 顶部返回按钮 */}
      <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "16px" }}>
        <button
          className="preflight-back-btn"
          onClick={onReturnToDiscussion}
          disabled={isRunning || isSubmitting}
          title={isRunning ? "当前检查完成后可返回讨论" : "返回继续讨论"}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "4px",
            padding: "6px 12px",
            fontSize: "13px",
            background: "transparent",
            border: "1px solid #d0d7de",
            borderRadius: "6px",
            cursor: isRunning || isSubmitting ? "not-allowed" : "pointer",
            color: isRunning || isSubmitting ? "#8c959f" : "#0969da",
            opacity: isRunning || isSubmitting ? 0.6 : 1,
          }}
        >
          <ArrowLeft size={16} />
          返回继续讨论
        </button>
        {isRunning && (
          <span style={{ fontSize: "12px", color: "#656d76" }}>
            当前检查完成后可返回讨论
          </span>
        )}
      </div>

      <h2>📋 三项显式检查</h2>
      <p style={{ color: "#656d76", fontSize: "13px", marginBottom: "16px" }}>
        在生成项目方案前，需要依次通过以下三项检查。前一项通过后才能进行下一项。
      </p>

      {hasStale && (
        <div
          style={{
            padding: "10px 14px",
            background: "#fff8c5",
            border: "1px solid #d4a72c",
            borderRadius: "6px",
            fontSize: "13px",
            color: "#664d03",
            marginBottom: "12px",
          }}
        >
          ⚠️ 已有新讨论消息，旧检查结果已过期。请重新运行检查。
        </div>
      )}

      {CHECK_ORDER.map((type, idx) => {
        const result = resultMap[type];
        const isCurrent = idx === currentIndex;
        const isBlocked = idx > currentIndex;
        const isLoading = loadingType === type;

        return (
          <div key={type} className="preflight-check-item" style={{ opacity: isBlocked ? 0.6 : 1 }}>
            <div className="preflight-check-header">
              <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                {result?.passed && !result?.stale ? (
                  <CheckCircle size={18} color="#1a7f37" />
                ) : result && !result.passed ? (
                  <XCircle size={18} color="#cf222e" />
                ) : result?.stale ? (
                  <Clock size={18} color="#d4a72c" />
                ) : (
                  <Clock size={18} color="#656d76" />
                )}
                <h3>{CHECK_LABELS[type].title}</h3>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                {result && !result.stale && (
                  <span className={`preflight-check-status ${result.passed ? "passed" : "failed"}`}>
                    {result.passed ? "通过" : "未通过"}
                  </span>
                )}
                {result?.stale && (
                  <span className="preflight-check-status" style={{ background: "#fff8c5", color: "#664d03" }}>
                    已过期
                  </span>
                )}
                {isCurrent && (!result || result.stale || !result.passed) && (
                  <button
                    className="preflight-run-btn"
                    onClick={() => handleRunCheck(type)}
                    disabled={isLoading || isSubmitting}
                  >
                    {isLoading ? "检查中..." : result?.stale ? "重新检查" : "开始检查"}
                  </button>
                )}
                {result?.passed && !result?.stale && idx < CHECK_ORDER.length - 1 && (
                  <ArrowRight size={16} color="#0969da" />
                )}
              </div>
            </div>
            {result && (
              <div className="preflight-check-body">
                <p>{result.summary}</p>
                {result.issues.length > 0 && (
                  <>
                    <strong style={{ fontSize: "12px", color: "#cf222e" }}>问题：</strong>
                    <ul className="preflight-check-issues">
                      {result.issues.map((issue, i) => (
                        <li key={i}>{issue}</li>
                      ))}
                    </ul>
                  </>
                )}
              </div>
            )}
          </div>
        );
      })}

      {/* 错误反馈 */}
      {error && (
        <div style={{
          padding: "10px 14px",
          background: error.type === "network" ? "#fff1f0" : "#fff8c5",
          border: `1px solid ${error.type === "network" ? "#cf222e" : "#d4a72c"}`,
          borderRadius: "6px",
          fontSize: "13px",
          color: error.type === "network" ? "#cf222e" : "#664d03",
          marginTop: "12px",
        }}>
          {error.type === "network" && "🌐 "}
          {error.type === "parse" && "📝 "}
          {error.type === "business" && "⚠️ "}
          {error.message}
        </div>
      )}

      {/* 全部通过 */}
      {allPassed && !hasStale && (
        <div style={{ textAlign: "center", marginTop: "20px" }}>
          <p style={{ color: "#1a7f37", fontSize: "14px", marginBottom: "12px" }}>
            ✅ 三项检查全部通过！
          </p>
          <div style={{ display: "flex", gap: "10px", justifyContent: "center", flexWrap: "wrap" }}>
            <button
              className="project-entry-submit"
              onClick={onAllPassed}
              disabled={isSubmitting}
              style={{ maxWidth: "300px" }}
            >
              生成项目方案草稿
            </button>
            {onStartManagedFlow && !managedFlowActive && (
              <button
                className="project-entry-submit"
                onClick={onStartManagedFlow}
                disabled={isSubmitting}
                style={{ maxWidth: "300px", background: "#6e40c9" }}
              >
                🚀 启动托管（自动推进到 Console）
              </button>
            )}
          </div>
          {managedFlowActive && (
            <p style={{ color: "#6e40c9", fontSize: "13px", marginTop: "8px" }}>
              🤖 托管层已激活，正在自动推进…
            </p>
          )}
        </div>
      )}

      {/* 全部检查但有过期 — 提供重新开始按钮 */}
      {!allPassed && hasStale && (
        <div style={{ textAlign: "center", marginTop: "20px" }}>
          <p style={{ color: "#664d03", fontSize: "13px", marginBottom: "12px" }}>
            部分或全部检查结果已过期，请重新执行已过期的检查，或重新开始全部检查。
          </p>
          <button
            className="preflight-restart-btn"
            onClick={onRestartChecks}
            disabled={isRunning || isSubmitting}
            style={{
              padding: "8px 20px",
              fontSize: "14px",
              background: "transparent",
              color: isRunning || isSubmitting ? "#8c959f" : "#cf222e",
              border: "1px solid #cf222e",
              borderRadius: "6px",
              cursor: isRunning || isSubmitting ? "not-allowed" : "pointer",
            }}
          >
            🔄 重新开始全部检查
          </button>
        </div>
      )}
    </div>
  );
}
