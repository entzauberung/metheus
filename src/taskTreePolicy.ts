import type { Project, Subtask, TaskTreeNodeView } from "./types";

function childTasks(task: Subtask): Subtask[] {
  return task.child_tasks ?? [];
}

export function isSubtaskLeaf(task: Subtask): boolean {
  return childTasks(task).length === 0;
}

export function flattenSubtasks(tasks: Subtask[]): Subtask[] {
  const flattened: Subtask[] = [];
  for (const task of tasks) {
    flattened.push(task, ...flattenSubtasks(childTasks(task)));
  }
  return flattened;
}

export function collectLeafSubtasks(tasks: Subtask[]): Subtask[] {
  const leaves: Subtask[] = [];
  for (const task of tasks) {
    if (isSubtaskLeaf(task)) {
      leaves.push(task);
    } else {
      leaves.push(...collectLeafSubtasks(childTasks(task)));
    }
  }
  return leaves;
}

export function findSubtaskById(tasks: Subtask[], id: string): Subtask | null {
  if (!id) return null;
  for (const task of tasks) {
    if (task.id === id) return task;
    const found = findSubtaskById(childTasks(task), id);
    if (found) return found;
  }
  return null;
}

export function findProjectSubtaskById(project: Project, id: string): Subtask | null {
  if (!id) return null;
  for (const milestone of project.milestones) {
    const milestoneTask = findSubtaskById(milestone.subtasks ?? [], id);
    if (milestoneTask) return milestoneTask;
    for (const midStage of milestone.mid_stages ?? []) {
      const midStageTask = findSubtaskById(midStage.subtasks ?? [], id);
      if (midStageTask) return midStageTask;
    }
  }
  return null;
}

export function findFirstLeafByStatus(tasks: Subtask[], status: string): Subtask | null {
  return collectLeafSubtasks(tasks).find(task => task.status === status) ?? null;
}

export function findSubtaskPath(tasks: Subtask[], id: string): string[] {
  if (!id) return [];
  for (const task of tasks) {
    if (task.id === id) return [task.id];
    const descendantPath = findSubtaskPath(childTasks(task), id);
    if (descendantPath.length > 0) return [task.id, ...descendantPath];
  }
  return [];
}

export function findProjectSubtaskPath(project: Project, id: string): string[] {
  if (!id) return [];
  for (const milestone of project.milestones) {
    const milestonePath = findSubtaskPath(milestone.subtasks ?? [], id);
    if (milestonePath.length > 0) return milestonePath;
    for (const midStage of milestone.mid_stages ?? []) {
      const midStagePath = findSubtaskPath(midStage.subtasks ?? [], id);
      if (midStagePath.length > 0) return midStagePath;
    }
  }
  return [];
}

export function findTaskControlNode(
  nodes: TaskTreeNodeView[],
  id: string,
): TaskTreeNodeView | null {
  if (!id) return null;
  for (const node of nodes) {
    if (node.id === id) return node;
    const found = findTaskControlNode(node.children ?? [], id);
    if (found) return found;
  }
  return null;
}

export function areSubtaskDependenciesSatisfied(project: Project, task: Subtask): boolean {
  return (task.depends_on ?? []).every(dependencyId => {
    const dependency = findProjectSubtaskById(project, dependencyId);
    return dependency != null
      && ["Passed", "AcceptedDeviation", "Skipped"].includes(dependency.status);
  });
}

function isTerminalStatus(status: string): boolean {
  return ["Passed", "AcceptedDeviation", "Skipped"].includes(status);
}

export function isSubtaskRunnableLeaf(
  project: Project,
  task: Subtask,
  status: string = "Pending",
): boolean {
  if (!isSubtaskLeaf(task) || task.status !== status) return false;
  const path = findProjectSubtaskPath(project, task.id);
  if (path.length === 0) return false;
  return path.every(taskId => {
    const pathTask = findProjectSubtaskById(project, taskId);
    return pathTask != null
      && !isTerminalStatus(pathTask.status)
      && areSubtaskDependenciesSatisfied(project, pathTask);
  });
}

export function findFirstRunnableLeaf(
  project: Project,
  tasks: Subtask[],
  status: string = "Pending",
): Subtask | null {
  return collectLeafSubtasks(tasks).find(task =>
    isSubtaskRunnableLeaf(project, task, status),
  ) ?? null;
}
