// src/RollbackImpactDialog.tsx — 回退影响范围弹窗
import { useState } from "react";
import { Project, RollbackImpact } from "./types";
import { Modal } from "./components/Modal";
import { ActionButton } from "./components/ActionButton";
import { FeedbackBanner } from "./components/FeedbackBanner";
import { RotateCcw, Search } from "lucide-react";

interface RollbackImpactData {
  target_checkpoint: string;
  retained_nodes: string[];
  discarded_nodes: string[];
  deleted_tags: string[];
  regeneration_scope: string;
  includes_code_rollback: boolean;
}

// Legacy props (pre-V1)
interface LegacyProps {
  isOpen: boolean;
  impact: RollbackImpactData | null;
  onConfirm: () => void;
  onCancel: () => void;
}

// V1 props
interface V1Props {
  project: Project;
  onPreview: (checkpointSubtaskId: string) => Promise<RollbackImpact | null>;
  onConfirm: (checkpointSubtaskId: string) => Promise<void>;
}

type Props = LegacyProps | V1Props;

function isV1(props: Props): props is V1Props {
  return 'project' in props;
}

export function RollbackImpactDialog(props: Props) {
  // V1 mode: stateful wrapper
  if (isV1(props)) {
    return <RollbackPreviewV1 {...props} />;
  }
  // Legacy mode
  return <RollbackDialogLegacy {...props} />;
}

// === V1 rollback preview panel ===
function RollbackPreviewV1({ project, onPreview, onConfirm }: V1Props) {
  const [selectedCheckpoint, setSelectedCheckpoint] = useState<string>("");
  const [impact, setImpact] = useState<RollbackImpact | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Collect all passed subtasks as potential checkpoints
  const checkpoints: { id: string; title: string; tag?: string }[] = [];
  for (const ms of project.milestones) {
    for (const mid of ms.mid_stages) {
      for (const st of mid.subtasks) {
        if (st.status === "Passed") {
          checkpoints.push({ id: st.id, title: st.title, tag: st.auto_tag });
        }
      }
    }
  }

  const handlePreview = async () => {
    if (!selectedCheckpoint || busy) return;
    setBusy(true); setError(null);
    try {
      const result = await onPreview(selectedCheckpoint);
      setImpact(result);
    } catch (e) {
      setError(String(e));
    } finally { setBusy(false); }
  };

  const handleConfirm = async () => {
    if (!selectedCheckpoint || busy) return;
    setBusy(true); setError(null);
    try {
      await onConfirm(selectedCheckpoint);
    } catch (e) {
      setError(String(e));
    } finally { setBusy(false); }
  };

  return (
    <div className="rollback-preview-panel">
      <h2>回退预览</h2>
      <p style={{ color: "#656d76", fontSize: "13px", marginBottom: "16px" }}>
        选择要回退到的稳定检查点。检查点本身保留，之后的内容将被作废。
      </p>

      {error && <FeedbackBanner type="error" message={error} />}

      <div style={{ marginBottom: "16px" }}>
        <h3 style={{ fontSize: "14px" }}>可用检查点（{checkpoints.length} 个）</h3>
        {checkpoints.map(cp => (
          <button key={cp.id} onClick={() => setSelectedCheckpoint(cp.id)}
            className={`rollback-checkpoint${selectedCheckpoint === cp.id ? " selected" : ""}`}>
            {cp.title}{cp.tag ? ` (tag: ${cp.tag})` : ""}
          </button>
        ))}
      </div>

      <ActionButton icon={<Search size={16} />} loading={busy} loadingLabel="分析中" disabled={!selectedCheckpoint} onClick={handlePreview}>预览影响范围</ActionButton>

      {impact && (
        <div style={{ marginTop: "20px" }}>
          <div style={{ padding: "10px", background: "#dafbe1", borderRadius: "6px", marginBottom: "12px" }}>
            <strong>检查点：</strong>{impact.target_checkpoint}
          </div>
          {impact.retained_nodes.length > 0 && (
            <div style={{ marginBottom: "8px" }}>
              <strong>保留（{impact.retained_nodes.length}）：</strong>
              <ul style={{ fontSize: "13px" }}>{impact.retained_nodes.map((n, i) => <li key={i}>{n}</li>)}</ul>
            </div>
          )}
          {impact.discarded_nodes.length > 0 && (
            <div style={{ marginBottom: "8px" }}>
              <strong>作废（{impact.discarded_nodes.length}）：</strong>
              <ul style={{ fontSize: "13px", color: "#cf222e" }}>{impact.discarded_nodes.map((n, i) => <li key={i}>{n}</li>)}</ul>
            </div>
          )}
          {impact.deleted_tags.length > 0 && (
            <div style={{ marginBottom: "8px" }}>
              <strong>删除标签：</strong> {impact.deleted_tags.join(", ")}
            </div>
          )}
          <div style={{ marginBottom: "16px" }}>
            <strong>重生成范围：</strong> {impact.regeneration_scope}
          </div>
          <ActionButton icon={<RotateCcw size={16} />} variant="danger" loading={busy} loadingLabel="回退中" onClick={handleConfirm}>确认回退</ActionButton>
        </div>
      )}
    </div>
  );
}

// === Legacy modal dialog ===
function RollbackDialogLegacy({ isOpen, impact, onConfirm, onCancel }: LegacyProps) {
  if (!impact) return null;
  return (
    <Modal isOpen={isOpen} onClose={onCancel} title="确认回退影响范围" isDanger
      actions={[
        { label: "取消", onClick: onCancel, variant: "secondary" },
        { label: "确认回退", onClick: onConfirm, variant: "danger" },
      ]}>
      <p style={{ fontSize: "13px", color: "#656d76", marginBottom: "16px" }}>
        以下是将要保留、作废和重新生成的范围。
      </p>
      <div><h4>目标检查点</h4><p>{impact.target_checkpoint}</p></div>
      {impact.retained_nodes.length > 0 && (
        <div><h4>保留（{impact.retained_nodes.length}）</h4><ul>{impact.retained_nodes.map((n, i) => <li key={i}>{n}</li>)}</ul></div>
      )}
      {impact.discarded_nodes.length > 0 && (
        <div><h4>作废（{impact.discarded_nodes.length}）</h4><ul>{impact.discarded_nodes.map((n, i) => <li key={i} style={{ color: "#cf222e" }}>{n}</li>)}</ul></div>
      )}
      <div><h4>重生成范围</h4><p>{impact.regeneration_scope}</p></div>
    </Modal>
  );
}
