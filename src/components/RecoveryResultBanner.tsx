import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import { useEffect } from "react";
import type { RuntimeOutcomePresentation } from "../runtimeOutcomePresentation";
import type { RecoveryResultSummary } from "../types";

export const RECOVERY_RESULT_DISPLAY_MS = 12_000;

interface RecoveryResultBannerProps {
  result: RecoveryResultSummary | null;
  runtimeOutcome?: RuntimeOutcomePresentation;
  onDismiss: () => void;
}

export function RecoveryResultBanner({ result, runtimeOutcome, onDismiss }: RecoveryResultBannerProps) {
  useEffect(() => {
    if (!result) return;
    const timer = window.setTimeout(onDismiss, RECOVERY_RESULT_DISPLAY_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss, result]);

  if (!result) return null;
  const OutcomeIcon = runtimeOutcome?.tone === "error"
    ? AlertTriangle
    : runtimeOutcome?.state === "completed"
      ? CheckCircle2
      : Info;
  return (
    <section className="recovery-result-banner" role="status" aria-live="polite">
      <OutcomeIcon size={19} aria-hidden="true" />
      <div className="recovery-result-content">
        <strong>恢复动作：{result.title}</strong>
        <span>动作摘要：{result.message}</span>
        {runtimeOutcome && (
          <small>
            当前任务：{runtimeOutcome.statusLabel}。{runtimeOutcome.summary}
            {!runtimeOutcome.writeAllowed ? ` ${runtimeOutcome.writeBlockedReason}` : ""}
          </small>
        )}
        {result.baseline_summary && <small>{result.baseline_summary}</small>}
        {result.discarded_files.length > 0 && (
          <details className="recovery-result-files">
            <summary>{result.discarded_files_summary}</summary>
            <ul>
              {result.discarded_files.map(path => <li key={path}><code>{path}</code></li>)}
            </ul>
          </details>
        )}
        <small>{result.background_job_summary}</small>
        <small>{result.next_step_summary}</small>
      </div>
      <button type="button" onClick={onDismiss} aria-label="关闭恢复结果">
        <X size={15} />
      </button>
    </section>
  );
}
