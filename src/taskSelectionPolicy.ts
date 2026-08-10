export type TaskSelectionMode = "follow" | "pinned";

export interface TaskSelectionState {
  selectedTaskId: string;
  mode: TaskSelectionMode;
}

export interface TaskSelectionAvailability {
  currentTaskExists: boolean;
  selectedTaskExists: boolean;
}

export function createFollowingTaskSelection(selectedTaskId = ""): TaskSelectionState {
  return { selectedTaskId, mode: "follow" };
}

export function createPinnedTaskSelection(taskId: string): TaskSelectionState {
  return taskId
    ? { selectedTaskId: taskId, mode: "pinned" }
    : createFollowingTaskSelection();
}

export function reconcileTaskSelection(
  selection: TaskSelectionState,
  currentTaskId: string,
  availability: TaskSelectionAvailability,
): TaskSelectionState {
  if (selection.mode === "pinned" && availability.selectedTaskExists) {
    return selection;
  }
  return createFollowingTaskSelection(
    availability.currentTaskExists ? currentTaskId : "",
  );
}
