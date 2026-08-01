import { useEffect, useMemo, useState } from "react";
import { findProjectSubtaskById, isSubtaskLeaf } from "../taskTreePolicy";
import type {
  Project,
  RecoveryDecisionOption,
  RecoveryDecisionResolution,
  RecoveryPresentation,
} from "../types";
import { Modal } from "./Modal";

export interface RecoveryDecisionSubmission {
  resolution: RecoveryDecisionResolution;
  reason: string;
  acceptedCriteria: number[];
}

interface RecoveryDecisionDialogProps {
  isOpen: boolean;
  project: Project;
  presentation: RecoveryPresentation;
  busy: boolean;
  onClose: () => void;
  onSubmit: (submission: RecoveryDecisionSubmission) => Promise<void>;
}

function firstEnabled(options: RecoveryDecisionOption[]): RecoveryDecisionResolution | null {
  return options.find(option => option.enabled)?.resolution ?? null;
}

export function RecoveryDecisionDialog({
  isOpen,
  project,
  presentation,
  busy,
  onClose,
  onSubmit,
}: RecoveryDecisionDialogProps) {
  const [resolution, setResolution] = useState<RecoveryDecisionResolution | null>(
    firstEnabled(presentation.decision_options),
  );
  const [reason, setReason] = useState("");
  const [acceptedCriteria, setAcceptedCriteria] = useState<number[]>([]);

  useEffect(() => {
    if (!isOpen) return;
    setResolution(firstEnabled(presentation.decision_options));
    setReason("");
    setAcceptedCriteria([]);
  }, [isOpen, presentation.state_fingerprint, presentation.decision_options]);

  const selectedOption = presentation.decision_options.find(
    option => option.resolution === resolution,
  ) ?? null;
  const recoveryTask = useMemo(() => {
    const taskId = project.workflow_state.recovery_state?.subtask_id
      ?? project.execution_session?.subtask_id
      ?? "";
    const task = findProjectSubtaskById(project, taskId);
    return task && isSubtaskLeaf(task) ? task : null;
  }, [project]);
  const acceptanceItems = recoveryTask?.acceptance_ledger?.length
    ? recoveryTask.acceptance_ledger
      .filter(item => item.status !== "Satisfied")
      .map(item => ({
        index: item.criterion_index,
        label: recoveryTask.acceptance_criteria?.[item.criterion_index - 1]
          ?? `验收项 ${item.criterion_index}`,
        status: item.status,
      }))
    : (recoveryTask?.acceptance_criteria ?? []).map((label, index) => ({
      index: index + 1,
      label,
      status: "Unknown",
    }));
  const reasonMissing = selectedOption?.requires_reason === true && !reason.trim();
  const criteriaMissing = selectedOption?.requires_acceptance_selection === true
    && acceptedCriteria.length === 0;
  const canSubmit = selectedOption?.enabled === true && !reasonMissing && !criteriaMissing && !busy;

  const submit = async () => {
    if (!selectedOption || !canSubmit) return;
    await onSubmit({
      resolution: selectedOption.resolution,
      reason: reason.trim(),
      acceptedCriteria,
    });
  };

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title="选择恢复处理方式"
      description={presentation.reason}
      lockClose={busy}
      isSubmitting={busy}
      actions={[
        { label: "取消", onClick: onClose, variant: "secondary", disabled: busy },
        { label: busy ? "处理中..." : "确认处理", onClick: () => { void submit(); }, variant: "primary", disabled: !canSubmit },
      ]}
    >
      <div className="recovery-decision-options" role="radiogroup" aria-label="恢复处理方式">
        {presentation.decision_options.map(option => (
          <label
            className={`recovery-decision-option ${option.enabled ? "" : "is-disabled"}`}
            key={option.resolution}
            title={option.disabled_reason ?? undefined}
          >
            <input
              type="radio"
              name="recovery-resolution"
              value={option.resolution}
              checked={resolution === option.resolution}
              disabled={busy || !option.enabled}
              onChange={() => {
                setResolution(option.resolution);
                setReason("");
                setAcceptedCriteria([]);
              }}
            />
            <span>
              <strong>{option.label}</strong>
              {option.disabled_reason && <small>{option.disabled_reason}</small>}
              {option.preview_message && <small>{option.preview_message}</small>}
            </span>
          </label>
        ))}
      </div>

      {selectedOption?.requires_reason && (
        <label className="recovery-decision-field">
          <span>处理依据（必填）</span>
          <textarea
            value={reason}
            disabled={busy}
            onChange={event => setReason(event.target.value)}
            placeholder="请记录证据、影响或人工决策原因"
          />
        </label>
      )}

      {selectedOption?.requires_acceptance_selection && (
        <fieldset className="recovery-acceptance-options">
          <legend>接受偏差的验收项（至少选择一项）</legend>
          {acceptanceItems.length === 0 ? (
            <p>当前任务没有可选择的验收项，不能提交此决策。</p>
          ) : acceptanceItems.map(item => (
            <label key={item.index}>
              <input
                type="checkbox"
                checked={acceptedCriteria.includes(item.index)}
                disabled={busy}
                onChange={event => setAcceptedCriteria(current => event.target.checked
                  ? [...current, item.index].sort((a, b) => a - b)
                  : current.filter(index => index !== item.index))}
              />
              <span>{item.index}. {item.label}（{item.status}）</span>
            </label>
          ))}
        </fieldset>
      )}
    </Modal>
  );
}
