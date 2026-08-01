import type { ExecutionRecoveryImpact } from "./types";
import { Modal } from "./components/Modal";

function FileList({ title, files }: { title: string; files: string[] }) {
  if (files.length === 0) return null;
  return (
    <section style={{ marginTop: "12px" }}>
      <strong>{title}（{files.length}）</strong>
      <ul style={{ margin: "6px 0 0", paddingLeft: "20px", maxHeight: "140px", overflow: "auto" }}>
        {files.map(file => <li key={file}><code>{file}</code></li>)}
      </ul>
    </section>
  );
}

export function RecoveryImpactDialog({
  impact,
  busy,
  onCancel,
  onConfirm,
}: {
  impact: ExecutionRecoveryImpact | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      isOpen={impact !== null}
      onClose={onCancel}
      title={impact?.confirmation_title ?? ""}
      description={impact?.presentation_description}
      isDanger
      lockClose={busy}
      isSubmitting={busy}
      actions={[
        { label: "取消", onClick: onCancel, variant: "secondary", disabled: busy },
        { label: busy ? "处理中…" : impact?.action_label ?? "", onClick: onConfirm, variant: "danger", disabled: busy },
      ]}
    >
      {impact && (
        <div style={{ fontSize: "13px", overflowWrap: "anywhere" }}>
          <div>恢复基线：<code>{impact.baseline_commit}</code></div>
          <div style={{ marginTop: "6px" }}>当前 HEAD：<code>{impact.current_head}</code></div>
          <p style={{ marginTop: "10px", color: impact.external_changes.length > 0 ? "#cf222e" : "#656d76" }}>
            {impact.safety_stash_summary}
          </p>
          <FileList title="系统受管修改" files={impact.managed_changes} />
          <FileList title="外部未知修改" files={impact.external_changes} />
          <FileList title="未跟踪文件" files={impact.untracked_files} />
          <FileList title="将从当前工作区移除" files={impact.discarded_files} />
        </div>
      )}
    </Modal>
  );
}
