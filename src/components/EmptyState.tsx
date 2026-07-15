import { Inbox } from "lucide-react";
import { ActionButton } from "./ActionButton";

interface EmptyStateProps {
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}

export function EmptyState({ title, message, actionLabel, onAction }: EmptyStateProps) {
  return (
    <div className="console-empty-state">
      <Inbox size={28} aria-hidden="true" />
      <h3>{title}</h3>
      <p>{message}</p>
      {actionLabel && onAction && (
        <ActionButton variant="ghost" onClick={onAction}>{actionLabel}</ActionButton>
      )}
    </div>
  );
}
