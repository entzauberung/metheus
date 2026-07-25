export const CHAT_BOTTOM_THRESHOLD_PX = 72;

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

export function distanceFromBottom(metrics: ScrollMetrics): number {
  return Math.max(0, metrics.scrollHeight - metrics.clientHeight - metrics.scrollTop);
}

export function isNearChatBottom(
  metrics: ScrollMetrics,
  threshold = CHAT_BOTTOM_THRESHOLD_PX,
): boolean {
  return distanceFromBottom(metrics) <= threshold;
}

export function nextUnreadState(
  current: boolean,
  isFollowing: boolean,
  addedContent = true,
): boolean {
  return isFollowing ? false : current || addedContent;
}
