import { RefreshCw, Wifi, WifiOff } from "lucide-react";
import type { ProjectSyncState, ProjectSyncStatus } from "../hooks/useProjectStateSync";
import type { TerminalSyncPhase } from "../executionSyncPolicy";
import type { TaskControlDetailStatus } from "../types";

interface SyncStatusIndicatorProps {
  state: ProjectSyncState;
  onRetry: () => void | Promise<unknown>;
  terminalPhase?: TerminalSyncPhase;
}

const STATUS_LABELS: Record<ProjectSyncStatus, string> = {
  idle: "等待同步",
  syncing: "正在同步",
  synced: "已同步",
  delayed: "同步延迟",
  disconnected: "后端断开",
};

const DETAIL_STATUS_LABELS: Record<TaskControlDetailStatus, string> = {
  idle: "任务详情未请求",
  syncing: "任务详情同步中",
  ready: "任务详情已同步",
  unavailable: "任务详情暂不可用",
};

function formatLastSync(value: string | null): string {
  if (!value) return "尚未成功同步";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "最近同步时间未知";
  return `最近同步 ${date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}`;
}

export function SyncStatusIndicator({ state, onRetry, terminalPhase = "idle" }: SyncStatusIndicatorProps) {
  const subscriptionReconnecting = state.subscriptionStatus === "reconnecting";
  const effectiveStatus: ProjectSyncStatus = state.status !== "disconnected" && subscriptionReconnecting
    ? "delayed"
    : state.status;
  const unhealthy = effectiveStatus === "delayed" || effectiveStatus === "disconnected";
  const terminalActive = terminalPhase !== "idle";
  const detail = terminalPhase === "terminal_reconciling"
    ? "后台动作已结束，正在读取最终状态"
    : terminalPhase === "terminal_delayed"
      ? "最终状态暂未就绪，系统正在低频重试"
      : subscriptionReconnecting
    ? `状态通知正在重连，低频快照兜底；${formatLastSync(state.lastSuccessfulSyncAt)}`
    : effectiveStatus === "synced" || unhealthy
      ? formatLastSync(state.lastSuccessfulSyncAt)
    : effectiveStatus === "syncing"
      ? "正在读取后端事实状态"
      : "等待建立状态订阅";

  return (
    <div
      className={`sync-status-indicator sync-status-${terminalActive ? terminalPhase : effectiveStatus}`}
      data-sync-status={terminalActive ? terminalPhase : effectiveStatus}
      data-subscription-status={state.subscriptionStatus}
      data-task-control-detail-status={state.taskControlDetailStatus}
      role="status"
      aria-live="polite"
      aria-atomic="true"
      title={detail}
    >
      {effectiveStatus === "disconnected" || subscriptionReconnecting || terminalPhase === "terminal_delayed"
        ? <WifiOff size={14} />
        : <Wifi size={14} />}
      <span>{terminalPhase === "terminal_reconciling"
        ? "正在获取最终状态"
        : terminalPhase === "terminal_delayed"
          ? "最终状态延迟"
          : subscriptionReconnecting ? "通知重连中" : STATUS_LABELS[effectiveStatus]}</span>
      <span className="sync-status-detail" aria-hidden="true">{detail}</span>
      <span className="sync-status-detail" data-testid="task-control-detail-status">
        {DETAIL_STATUS_LABELS[state.taskControlDetailStatus]}
      </span>
      {(unhealthy || terminalActive) && (
        <button
          type="button"
          className="sync-status-retry"
          onClick={() => { void onRetry(); }}
          aria-label="立即重试状态同步"
        >
          <RefreshCw size={12} /> 重试
        </button>
      )}
    </div>
  );
}
