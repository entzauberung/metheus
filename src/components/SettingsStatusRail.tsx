import { Bot, BrainCircuit, Image } from "lucide-react";

export type SettingsStatusTarget = "decision" | "builtin-grok" | "vision";
export type SettingsStatusState =
  | "configured"
  | "verified"
  | "available"
  | "disabled"
  | "error"
  | "unknown";

export interface SettingsStatusItem {
  target: SettingsStatusTarget;
  label: string;
  state: SettingsStatusState;
  detail: string;
}

interface Props {
  items: readonly SettingsStatusItem[];
  activeTarget?: SettingsStatusTarget;
  onSelect: (target: SettingsStatusTarget) => void;
}

const STATUS_LABELS: Record<SettingsStatusState, string> = {
  configured: "已配置",
  verified: "已验证",
  available: "可用",
  disabled: "已关闭",
  error: "异常",
  unknown: "未知",
};

function StatusIcon({ target }: { target: SettingsStatusTarget }) {
  if (target === "decision") return <BrainCircuit size={17} aria-hidden="true" />;
  if (target === "builtin-grok") return <Bot size={17} aria-hidden="true" />;
  return <Image size={17} aria-hidden="true" />;
}

export function SettingsStatusRail({ items, activeTarget, onSelect }: Props) {
  return (
    <nav className="settings-status-rail" aria-label="模型服务状态导航">
      {items.map((item) => {
        const statusLabel = STATUS_LABELS[item.state];
        return (
          <button
            type="button"
            className={`settings-status-card tone-${item.state}`}
            aria-label={`${item.label}：${statusLabel}。${item.detail}`}
            aria-pressed={activeTarget === item.target}
            key={item.target}
            onClick={() => onSelect(item.target)}
          >
            <span className="settings-status-icon"><StatusIcon target={item.target} /></span>
            <span className="settings-status-copy">
              <span className="settings-status-label">{item.label}</span>
              <span className="settings-status-detail" title={item.detail}>{item.detail}</span>
            </span>
            <span className="settings-status-value">{statusLabel}</span>
          </button>
        );
      })}
    </nav>
  );
}
