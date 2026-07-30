import { describe, expect, it } from "vitest";
import {
  areSubtaskDependenciesSatisfied,
  collectLeafSubtasks,
  findFirstLeafByStatus,
  findFirstRunnableLeaf,
  findProjectSubtaskById,
  findProjectSubtaskPath,
  findSubtaskById,
  findSubtaskPath,
  findTaskControlNode,
  flattenSubtasks,
  isSubtaskRunnableLeaf,
} from "./taskTreePolicy";
import type { Project, Subtask, TaskTreeNodeView } from "./types";

function task(
  id: string,
  status: Subtask["status"] = "Pending",
  children: Subtask[] = [],
  dependsOn: string[] = [],
): Subtask {
  return {
    id,
    title: id,
    status,
    child_tasks: children,
    depends_on: dependsOn,
  } as Subtask;
}

function project(milestoneTasks: Subtask[], midStageTasks: Subtask[]): Project {
  return {
    milestones: [{
      id: "milestone",
      subtasks: milestoneTasks,
      mid_stages: [{ id: "mid-stage", subtasks: midStageTasks }],
    }],
  } as Project;
}

describe("recursive task tree policy", () => {
  it("flattens a four-level tree in source order and collects leaves only", () => {
    const sibling = task("sibling", "Passed");
    const level4 = task("level-4", "AwaitingConfirmation");
    const level3 = task("level-3", "Pending", [level4]);
    const level2 = task("level-2", "Pending", [level3]);
    const level1 = task("level-1", "Pending", [level2, sibling]);

    expect(flattenSubtasks([level1]).map(item => item.id)).toEqual([
      "level-1", "level-2", "level-3", "level-4", "sibling",
    ]);
    expect(collectLeafSubtasks([level1]).map(item => item.id)).toEqual(["level-4", "sibling"]);
    expect(findFirstLeafByStatus([level1], "Pending")).toBeNull();
    expect(findFirstLeafByStatus([level1], "AwaitingConfirmation")?.id).toBe("level-4");
  });

  it("finds deep nodes and their ancestor path without treating a parent as a leaf", () => {
    const leaf = task("leaf");
    const roots = [task("parent", "Pending", [task("child-parent", "Pending", [leaf])])];

    expect(findSubtaskById(roots, "leaf")).toBe(leaf);
    expect(findSubtaskPath(roots, "leaf")).toEqual(["parent", "child-parent", "leaf"]);
    expect(findSubtaskPath(roots, "missing")).toEqual([]);
    expect(findSubtaskById(roots, "missing")).toBeNull();
  });

  it("searches milestone roots and mid-stage descendants", () => {
    const milestoneLeaf = task("milestone-leaf");
    const deepMidLeaf = task("deep-mid-leaf");
    const source = project(
      [task("milestone-parent", "Pending", [milestoneLeaf])],
      [task("mid-parent", "Pending", [task("mid-child", "Pending", [deepMidLeaf])])],
    );

    expect(findProjectSubtaskById(source, "milestone-leaf")).toBe(milestoneLeaf);
    expect(findProjectSubtaskById(source, "deep-mid-leaf")).toBe(deepMidLeaf);
    expect(findProjectSubtaskPath(source, "deep-mid-leaf")).toEqual([
      "mid-parent", "mid-child", "deep-mid-leaf",
    ]);
  });

  it("selects the first pending leaf whose dependencies are terminal", () => {
    const complete = task("complete", "Passed");
    const blocked = task("blocked", "Pending", [], ["unfinished"]);
    const accepted = task("accepted", "AcceptedDeviation");
    const runnable = task("runnable", "Pending", [], ["complete", "accepted"]);
    const source = project([complete, accepted], [
      task("parent", "Pending", [blocked, runnable]),
      task("unfinished", "Executing"),
    ]);

    expect(areSubtaskDependenciesSatisfied(source, blocked)).toBe(false);
    expect(areSubtaskDependenciesSatisfied(source, runnable)).toBe(true);
    expect(isSubtaskRunnableLeaf(source, runnable)).toBe(true);
    expect(findFirstRunnableLeaf(source, source.milestones[0].mid_stages[0].subtasks)?.id)
      .toBe("runnable");
  });

  it("does not descend through a parent whose dependency is unfinished", () => {
    const unfinished = task("unfinished", "Executing");
    const blockedChild = task("blocked-child");
    const runnable = task("runnable");
    const source = project([unfinished], [
      task("blocked-parent", "Pending", [blockedChild], ["unfinished"]),
      runnable,
    ]);

    expect(isSubtaskRunnableLeaf(source, blockedChild)).toBe(false);
    expect(findFirstRunnableLeaf(source, source.milestones[0].mid_stages[0].subtasks))
      .toBe(runnable);
  });

  it("keeps missing dependencies blocked and accepts skipped dependencies", () => {
    const skipped = task("skipped", "Skipped");
    const afterSkipped = task("after-skipped", "Pending", [], ["skipped"]);
    const missing = task("missing-dependency", "Pending", [], ["does-not-exist"]);
    const source = project([skipped], [missing, afterSkipped]);

    expect(areSubtaskDependenciesSatisfied(source, missing)).toBe(false);
    expect(findFirstRunnableLeaf(source, source.milestones[0].mid_stages[0].subtasks))
      .toBe(afterSkipped);
  });

  it("finds arbitrary-depth control snapshot nodes", () => {
    const leaf = { id: "control-leaf", children: [] } as unknown as TaskTreeNodeView;
    const nodes = [{
      id: "control-root",
      children: [{ id: "control-child", children: [leaf] }],
    }] as unknown as TaskTreeNodeView[];

    expect(findTaskControlNode(nodes, "control-leaf")).toBe(leaf);
    expect(findTaskControlNode(nodes, "missing")).toBeNull();
  });

  it("handles empty trees and does not mutate source arrays", () => {
    const leaf = task("leaf");
    const roots = [task("root", "Pending", [leaf])];
    const before = roots.map(item => item.id);

    expect(flattenSubtasks([])).toEqual([]);
    expect(collectLeafSubtasks([])).toEqual([]);
    expect(findFirstLeafByStatus([], "Pending")).toBeNull();
    expect(findProjectSubtaskById(project([], []), "missing")).toBeNull();
    flattenSubtasks(roots);
    collectLeafSubtasks(roots);
    expect(roots.map(item => item.id)).toEqual(before);
    expect(roots[0].child_tasks[0]).toBe(leaf);
  });
});
