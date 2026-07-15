import { ReactNode } from "react";
import { FeedbackBanner, FeedbackType } from "./FeedbackBanner";
import { StatusBadge, WorkflowVisualStatus } from "./StatusBadge";

export interface ConsoleFeedback {
  type: FeedbackType;
  message: string;
}

interface ConsoleStepShellProps {
  icon: ReactNode;
  title: string;
  description: string;
  status: WorkflowVisualStatus;
  statusLabel: string;
  feedback?: ConsoleFeedback | null;
  busy?: boolean;
  children: ReactNode;
  actions?: ReactNode;
}

export function ConsoleStepShell({
  icon,
  title,
  description,
  status,
  statusLabel,
  feedback,
  busy = false,
  children,
  actions,
}: ConsoleStepShellProps) {
  return (
    <section className="console-step-shell" aria-busy={busy}>
      <header className="console-step-header">
        <div className="console-step-icon" aria-hidden="true">{icon}</div>
        <div className="console-step-heading">
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
        <StatusBadge status={status} label={statusLabel} />
      </header>
      {feedback && <FeedbackBanner type={feedback.type} message={feedback.message} />}
      <div className="console-step-content">{children}</div>
      {actions && <div className="console-step-footer">{actions}</div>}
      {busy && <div className="console-step-loading" aria-hidden="true" />}
    </section>
  );
}
