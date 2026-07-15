// src/components/StatusBadge.tsx — 统一状态徽章
import { CheckCircle, XCircle, Clock, AlertTriangle, Pause, RotateCw } from "lucide-react";

interface StatusBadgeProps {
  status: "success" | "failure" | "pending" | "warning" | "paused" | "progress";
  label?: string;
}

export type WorkflowVisualStatus = StatusBadgeProps["status"];

const BADGE_CONFIG = {
  success: { icon: CheckCircle, color: "#1a7f37", bg: "#dafbe1" },
  failure: { icon: XCircle, color: "#cf222e", bg: "#ffeef0" },
  pending: { icon: Clock, color: "#656d76", bg: "#f3f4f6" },
  warning: { icon: AlertTriangle, color: "#bf8700", bg: "#fff8c5" },
  paused: { icon: Pause, color: "#bf8700", bg: "#fff8c5" },
  progress: { icon: RotateCw, color: "#0969da", bg: "#ddf4ff" },
};

export function StatusBadge({ status, label }: StatusBadgeProps) {
  const config = BADGE_CONFIG[status];
  const Icon = config.icon;

  return (
    <span
      className="status-badge"
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "4px",
        padding: "2px 10px",
        borderRadius: "12px",
        fontSize: "12px",
        fontWeight: 500,
        color: config.color,
        background: config.bg,
      }}
    >
      <Icon size={12} />
      {label || status}
    </span>
  );
}
