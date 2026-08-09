import type { ProjectSyncState } from "./hooks/useProjectStateSync";

export interface ConsoleWritePolicy {
  writable: boolean;
  reason: string;
}

export function getConsoleWritePolicy(
  consoleActive: boolean,
  state: Pick<ProjectSyncState, "status" | "subscriptionStatus" | "pendingRevision">,
): ConsoleWritePolicy {
  if (!consoleActive) return { writable: true, reason: "" };
  if (state.status === "syncing") {
    return { writable: false, reason: "正在同步运行时快照，写操作暂不可用" };
  }
  if (state.pendingRevision !== null) {
    return { writable: false, reason: "检测到尚未对账的后端修订，请先同步项目状态" };
  }
  if (state.subscriptionStatus !== "connected") {
    return { writable: false, reason: "状态通知通道未连接，请先同步项目状态" };
  }
  if (state.status !== "synced") {
    return { writable: false, reason: "当前项目状态可能过期，请先同步项目状态" };
  }
  return { writable: true, reason: "" };
}
