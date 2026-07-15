// src/ExistingBaselinePanel.tsx — Half Project: Already 基线展示和批准
import { useState, useEffect } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { Project } from "./types";
import { ArrowLeft, RefreshCw, CheckCircle, FileText } from "lucide-react";

interface ExistingBaselinePanelProps {
  projectName: string;
  projectPath: string;
  onBaselineApproved: (project: Project) => void;
  onReject: () => void;
}

export function ExistingBaselinePanel({
  projectName,
  projectPath,
  onBaselineApproved,
  onReject,
}: ExistingBaselinePanelProps) {
  const [baseline, setBaseline] = useState<Project | null>(null);
  const [loading, setLoading] = useState(true);
  const [approving, setApproving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    handleAnalyze();
  }, []);

  const handleAnalyze = async () => {
    setLoading(true);
    setError("");
    try {
      const project = await invokeWithTimeout<Project>("analyze_existing_project", {
        projectName,
      });
      setBaseline(project);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleApprove = async () => {
    setApproving(true);
    setError("");
    try {
      const project = await invokeWithTimeout<Project>("approve_existing_baseline", {
        projectName,
      });
      onBaselineApproved(project);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setApproving(false);
    }
  };

  if (loading) {
    return (
      <div className="baseline-panel" style={{ padding: "24px" }}>
        <h2>🔍 正在分析项目...</h2>
        <p style={{ color: "#656d76", fontSize: "14px" }}>
          正在扫描 {projectPath} 的项目结构并分析技术栈与已有能力...
        </p>
      </div>
    );
  }

  if (error && !baseline) {
    return (
      <div className="baseline-panel" style={{ padding: "24px" }}>
        <h2>❌ 分析失败</h2>
        <div className="project-entry-error">{error}</div>
        <div style={{ display: "flex", gap: "12px", marginTop: "16px" }}>
          <button onClick={onReject} style={{ padding: "8px 16px" }}>
            <ArrowLeft size={16} style={{ verticalAlign: "middle" }} /> 返回入口
          </button>
          <button onClick={handleAnalyze} style={{ padding: "8px 16px", background: "#0969da", color: "#fff", border: "none", borderRadius: "6px" }}>
            <RefreshCw size={16} style={{ verticalAlign: "middle" }} /> 重试分析
          </button>
        </div>
      </div>
    );
  }

  const existing = baseline?.existing_baseline;
  if (!existing) return null;

  const alreadyApproved = existing.approved;

  return (
    <div className="baseline-panel" style={{ padding: "24px", maxHeight: "calc(100vh - 40px)", overflowY: "auto" }}>
      <h2>
        {alreadyApproved ? (
          <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>
            <CheckCircle size={22} color="#1a7f37" />
            已有项目基线（已批准）
          </span>
        ) : (
          "📋 已有项目基线"
        )}
      </h2>
      <p style={{ color: "#656d76", fontSize: "13px", marginBottom: "20px" }}>
        Metheus 已扫描您的项目，以下是对现有代码的分析结果。
        {alreadyApproved && " 基线已批准，可以进入讨论。"}
      </p>

      {/* 证据摘要 */}
      <div className="baseline-section" style={{
        background: "#f6f8fa", borderRadius: "6px", padding: "12px", marginBottom: "12px",
        fontSize: "12px", color: "#656d76",
      }}>
        <FileText size={14} style={{ verticalAlign: "middle", marginRight: "4px" }} />
        {existing.evidence_summary}
      </div>

      {/* 项目摘要 */}
      <div className="baseline-section">
        <h3>📊 项目摘要</h3>
        <p>{existing.project_summary}</p>
      </div>

      {/* 技术栈 */}
      <div className="baseline-section">
        <h3>🔧 技术栈</h3>
        <p>{existing.tech_stack}</p>
      </div>

      {/* 架构证据 */}
      {existing.architecture_evidence && (
        <div className="baseline-section">
          <h3>🏗️ 架构证据</h3>
          <p style={{ fontSize: "13px", color: "#656d76" }}>{existing.architecture_evidence}</p>
        </div>
      )}

      {/* 已完成能力 */}
      {existing.completed_capabilities.length > 0 && (
        <div className="baseline-section">
          <h3>✅ 已完成能力</h3>
          <ul>
            {existing.completed_capabilities.map((cap, i) => (
              <li key={i}>{cap}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 待处理能力 */}
      {existing.pending_capabilities.length > 0 && (
        <div className="baseline-section">
          <h3>📝 待处理能力</h3>
          <ul>
            {existing.pending_capabilities.map((cap, i) => (
              <li key={i}>{cap}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 风险 */}
      {existing.risks.length > 0 && (
        <div className="baseline-section">
          <h3>⚠️ 风险</h3>
          <ul>
            {existing.risks.map((risk, i) => (
              <li key={i}>{risk}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 不确定项 */}
      {existing.uncertainties.length > 0 && (
        <div className="baseline-section">
          <h3>❓ 不确定项</h3>
          <ul>
            {existing.uncertainties.map((item, i) => (
              <li key={i}>{item}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 已扫描文件 */}
      <details style={{ marginBottom: "16px" }}>
        <summary style={{ cursor: "pointer", fontSize: "13px", color: "#656d76" }}>
          📂 已扫描文件（{existing.scanned_files.length} 个）
        </summary>
        <div style={{ maxHeight: "200px", overflowY: "auto", fontSize: "12px", color: "#999", marginTop: "8px" }}>
          {existing.scanned_files.map((f, i) => (
            <div key={i}>{f}</div>
          ))}
        </div>
      </details>

      {/* 错误 */}
      {error && <div className="project-entry-error" style={{ marginBottom: "12px" }}>{error}</div>}

      {/* 操作按钮 */}
      {!alreadyApproved && (
        <div className="baseline-actions" style={{ display: "flex", gap: "12px" }}>
          <button onClick={onReject} style={{ padding: "10px 20px" }}>
            <ArrowLeft size={16} style={{ verticalAlign: "middle" }} /> 返回入口
          </button>
          <button
            onClick={handleAnalyze}
            disabled={loading || approving}
            style={{ padding: "10px 20px", background: "#f6f8fa", border: "1px solid #d0d7de", borderRadius: "6px" }}
          >
            <RefreshCw size={16} style={{ verticalAlign: "middle" }} /> 重新分析
          </button>
          <button
            className="baseline-approve"
            onClick={handleApprove}
            disabled={approving}
            style={{
              padding: "10px 20px", fontWeight: 600,
              background: approving ? "#8c959f" : "#1a7f37", color: "#fff",
              border: "none", borderRadius: "6px",
              cursor: approving ? "not-allowed" : "pointer",
            }}
          >
            {approving ? "批准中..." : "✅ 批准基线并开始讨论"}
          </button>
        </div>
      )}

      {alreadyApproved && (
        <div style={{ textAlign: "center" }}>
          <button
            onClick={() => onBaselineApproved(baseline!)}
            style={{
              padding: "10px 28px", fontSize: "15px", fontWeight: 600,
              background: "#0969da", color: "#fff", border: "none", borderRadius: "8px",
              cursor: "pointer",
            }}
          >
            进入讨论
          </button>
        </div>
      )}
    </div>
  );
}
