// src/components/ActionButton.tsx — 统一操作按钮（含加载和禁用态）
import { Loader2 } from "lucide-react";

export type ActionVariant = "primary" | "secondary" | "danger" | "ghost";

interface ActionButtonProps {
  children: React.ReactNode;
  onClick: () => void;
  icon?: React.ReactNode;
  variant?: ActionVariant;
  loading?: boolean;
  loadingLabel?: string;
  disabled?: boolean;
  disabledReason?: string;
  type?: "button" | "submit" | "reset";
  fullWidth?: boolean;
  style?: React.CSSProperties;
}

const variantStyles: Record<ActionVariant, React.CSSProperties> = {
  primary: { background: "#1a7f37", color: "#fff", border: "none" },
  secondary: { background: "#0969da", color: "#fff", border: "none" },
  danger: { background: "#cf222e", color: "#fff", border: "none" },
  ghost: { background: "transparent", color: "#0969da", border: "1px solid #d0d7de" },
};

export function ActionButton({
  children, onClick, icon, variant = "primary",
  loading = false, loadingLabel, disabled = false, disabledReason,
  type = "button", fullWidth = false, style,
}: ActionButtonProps) {
  const isBusy = loading || disabled;
  const base = variantStyles[variant];
  return (
    <button type={type} onClick={onClick} disabled={isBusy} title={disabledReason || undefined}
      className={`action-button action-button-${variant}${fullWidth ? " action-button-full" : ""}`}
      style={{
        ...base, display: "inline-flex", alignItems: "center", gap: "6px",
        padding: "8px 18px", fontSize: "14px", fontWeight: 600,
        borderRadius: "6px", cursor: isBusy ? "not-allowed" : "pointer",
        opacity: disabled ? 0.6 : 1, ...style,
      }}>
      {loading ? <Loader2 size={14} className="spin" /> : icon}
      {loading && loadingLabel ? loadingLabel : children}
    </button>
  );
}
