import { describe, expect, it } from "vitest";
import {
  createFollowingTaskSelection,
  createPinnedTaskSelection,
  reconcileTaskSelection,
} from "./taskSelectionPolicy";

describe("task selection policy", () => {
  it("follows the backend current task initially and when current changes", () => {
    const initial = reconcileTaskSelection(
      createFollowingTaskSelection(),
      "task-a",
      { currentTaskExists: true, selectedTaskExists: false },
    );
    expect(initial).toEqual({ selectedTaskId: "task-a", mode: "follow" });

    expect(reconcileTaskSelection(initial, "task-b", {
      currentTaskExists: true,
      selectedTaskExists: true,
    })).toEqual({ selectedTaskId: "task-b", mode: "follow" });
  });

  it("preserves a valid pinned task when backend current changes", () => {
    const pinned = createPinnedTaskSelection("task-history");
    expect(reconcileTaskSelection(pinned, "task-current", {
      currentTaskExists: true,
      selectedTaskExists: true,
    })).toBe(pinned);
  });

  it("returns to follow when the pinned task disappears", () => {
    expect(reconcileTaskSelection(
      createPinnedTaskSelection("task-removed"),
      "task-current",
      { currentTaskExists: true, selectedTaskExists: false },
    )).toEqual({ selectedTaskId: "task-current", mode: "follow" });
  });

  it("clears selection when neither the selected nor current task exists", () => {
    expect(reconcileTaskSelection(
      createPinnedTaskSelection("task-removed"),
      "task-missing",
      { currentTaskExists: false, selectedTaskExists: false },
    )).toEqual({ selectedTaskId: "", mode: "follow" });
  });

  it("treats an empty manual selection as follow", () => {
    expect(createPinnedTaskSelection("")).toEqual({ selectedTaskId: "", mode: "follow" });
  });
});
