import { CheckCircle2, X } from "lucide-react";
import { useEffect } from "react";
import type { RecoveryResultSummary } from "../types";

export const RECOVERY_RESULT_DISPLAY_MS = 12_000;

interface RecoveryResultBannerProps {
  result: RecoveryResultSummary | null;
  onDismiss: () => void;
}

export function RecoveryResultBanner({ result, onDismiss }: RecoveryResultBannerProps) {
  useEffect(() => {
    if (!result) return;
    const timer = window.setTimeout(onDismiss, RECOVERY_RESULT_DISPLAY_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss, result]);

  if (!result) return null;
  return (
    <section className="recovery-result-banner" role="status" aria-live="polite">
      <CheckCircle2 size={19} aria-hidden="true" />
      <div className="recovery-result-content">
        <strong>{result.title}</strong>
        <span>{result.message}</span>
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
