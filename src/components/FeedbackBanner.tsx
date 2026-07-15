// src/components/FeedbackBanner.tsx — 统一反馈横幅
import { CheckCircle, AlertTriangle, XCircle, Info, RefreshCw } from "lucide-react";

export type FeedbackType = "success" | "warning" | "error" | "info";

interface FeedbackBannerProps {
  type: FeedbackType;
  message: string;
  onRetry?: () => void;
  style?: React.CSSProperties;
  details?: string[];
}

const config: Record<FeedbackType, { bg: string; border: string; color: string; icon: React.ReactNode }> = {
  success: { bg: "#dafbe1", border: "#1a7f37", color: "#1a7f37", icon: <CheckCircle size={16} /> },
  warning: { bg: "#fff8c5", border: "#d4a72c", color: "#664d03", icon: <AlertTriangle size={16} /> },
  error: { bg: "#fff1f0", border: "#cf222e", color: "#cf222e", icon: <XCircle size={16} /> },
  info: { bg: "#ddf4ff", border: "#0969da", color: "#0969da", icon: <Info size={16} /> },
};

export function FeedbackBanner({ type, message, onRetry, style, details }: FeedbackBannerProps) {
  const c = config[type];
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: "8px",
      padding: "10px 14px", background: c.bg, border: `1px solid ${c.border}`,
      borderRadius: "6px", fontSize: "13px", color: c.color, ...style,
    }}>
      {c.icon}
      <div style={{ flex: 1 }}>
        <span>{message}</span>
        {details && details.length > 0 && (
          <ul className="feedback-details">
            {details.map((detail) => <li key={detail}>{detail}</li>)}
          </ul>
        )}
      </div>
      {onRetry && (
        <button onClick={onRetry} style={{
          display: "inline-flex", alignItems: "center", gap: "4px",
          padding: "4px 10px", fontSize: "12px", background: "transparent",
          border: `1px solid ${c.border}`, borderRadius: "4px", color: c.color,
          cursor: "pointer",
        }}>
          <RefreshCw size={12} /> 重试
        </button>
      )}
    </div>
  );
}
