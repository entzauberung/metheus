export const DEFAULT_SIDEBAR_WIDTH = 280;
export const MIN_SIDEBAR_WIDTH = 220;
export const MAX_SIDEBAR_WIDTH = 480;
export const DEFAULT_INSPECTOR_WIDTH = 420;
export const MIN_INSPECTOR_WIDTH = 320;
export const MAX_INSPECTOR_WIDTH = 680;

export const SIDEBAR_WIDTH_STORAGE_KEY = "metheus_sidebar_width";
export const INSPECTOR_WIDTH_STORAGE_KEY = "metheus_task_inspector_width";

export function clampPanelWidth(value: number, minimum: number, maximum: number): number {
  if (!Number.isFinite(value)) return minimum;
  return Math.max(minimum, Math.min(maximum, Math.round(value)));
}

export function readStoredPanelWidth(
  storage: Pick<Storage, "getItem"> | null,
  key: string,
  fallback: number,
  minimum: number,
  maximum: number,
): number {
  if (!storage) return clampPanelWidth(fallback, minimum, maximum);
  const raw = storage.getItem(key);
  if (raw == null || raw.trim() === "") return clampPanelWidth(fallback, minimum, maximum);
  const parsed = Number(raw);
  return Number.isFinite(parsed)
    ? clampPanelWidth(parsed, minimum, maximum)
    : clampPanelWidth(fallback, minimum, maximum);
}

export function inspectorPresentation(viewportWidth: number): "docked" | "drawer" | "fullscreen" {
  if (viewportWidth >= 1_200) return "docked";
  if (viewportWidth >= 900) return "drawer";
  return "fullscreen";
}
