import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { findProjectSubtaskById, findTaskControlNode } from "../taskTreePolicy";
import type {
  Project,
  Subtask,
  TaskControlActionResult,
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "../types";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";

const DEFAULT_POLL_INTERVAL_MS = 2_500;

interface TaskControlActionOptions {
  criterionIndexes?: number[];
  reason?: string;
}

interface UseTaskControlWorkspaceOptions {
  project: Project | null;
  enabled?: boolean;
  pollIntervalMs?: number;
  onProjectUpdated?: (project: Project) => unknown;
}

export interface TaskControlWorkspace {
  snapshot: TaskControlSnapshot | null;
  selectedTaskId: string;
  selectedNode: TaskTreeNodeView | null;
  busy: boolean;
  error: string;
  refresh: () => Promise<void>;
  selectTask: (taskId: string) => void;
  executeAction: (name: string, options?: TaskControlActionOptions) => Promise<void>;
  changeMode: (mode: TaskControlMode) => Promise<void>;
}

function subtaskView(task: Subtask): TaskTreeNodeView {
  const contract = task.contract_snapshot;
  return {
    id: task.id,
    title: task.title,
    node_type: "Subtask",
    status: task.status,
    depth: contract?.depth ?? 0,
    complexity: contract?.complexity ?? "Small",
    risk: contract?.risk ?? "Low",
    contract_fingerprint: contract?.fingerprint ?? "",
    contract,
    dependencies: task.depends_on ?? [],
    acceptance: task.acceptance_ledger ?? [],
    children: (task.child_tasks ?? []).map(subtaskView),
  };
}

export function useTaskControlWorkspace({
  project,
  enabled = true,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  onProjectUpdated,
}: UseTaskControlWorkspaceOptions): TaskControlWorkspace {
  const [snapshot, setSnapshot] = useState<TaskControlSnapshot | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const requestSequence = useRef(0);
  const projectRef = useRef(project);
  projectRef.current = project;
  const projectName = project?.name ?? "";
  const projectRevision = project?.workflow_state.data_revision ?? 0;
  const scope = useRef({
    projectName,
    projectRevision,
  });
  scope.current = {
    projectName,
    projectRevision,
  };

  const refresh = useCallback(async () => {
    const currentProject = projectRef.current;
    if (!enabled || !currentProject?.name) return;
    const requestedProjectName = currentProject.name;
    const sequence = ++requestSequence.current;
    const requestedRevision = projectRevision;
    try {
      const next = await invokeWithTimeout<TaskControlSnapshot>("get_task_control_snapshot", {
        projectName: requestedProjectName,
      }, 10_000);
      const currentScope = scope.current;
      if (
        sequence !== requestSequence.current
        || currentScope.projectName !== requestedProjectName
        || next.project_name !== requestedProjectName
        || next.project_revision < requestedRevision
        || next.project_revision < currentScope.projectRevision
      ) {
        return;
      }
      setSnapshot(current => (
        current && current.project_name === next.project_name
          && current.project_revision > next.project_revision
          ? current
          : next
      ));
      setSelectedTaskId(current => {
        if (findTaskControlNode(next.nodes, current) || findProjectSubtaskById(currentProject, current)) {
          return current;
        }
        return findTaskControlNode(next.nodes, next.current_task_id)?.id
          ?? findProjectSubtaskById(currentProject, next.current_task_id)?.id
          ?? "";
      });
      setError("");
    } catch (reason) {
      if (sequence === requestSequence.current && scope.current.projectName === requestedProjectName) {
        setError(String(reason));
      }
    }
  }, [enabled, projectRevision]);

  useEffect(() => {
    requestSequence.current += 1;
    setSnapshot(null);
    setSelectedTaskId("");
    setError("");
    if (!enabled || !project?.name) return;
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, pollIntervalMs);
    return () => window.clearInterval(timer);
  }, [enabled, pollIntervalMs, projectName, refresh]);

  const selectedNode = useMemo(() => {
    const fromSnapshot = snapshot
      ? findTaskControlNode(snapshot.nodes, selectedTaskId)
      : null;
    if (fromSnapshot) return fromSnapshot;
    const fromProject = project
      ? findProjectSubtaskById(project, selectedTaskId)
      : null;
    return fromProject ? subtaskView(fromProject) : null;
  }, [project, selectedTaskId, snapshot]);

  const selectTask = useCallback((taskId: string) => {
    setSelectedTaskId(taskId);
  }, []);

  const syncProject = useCallback(async (projectName: string) => {
    const latest = await invokeWithTimeout<Project>("get_project", { projectName });
    if (scope.current.projectName === projectName) onProjectUpdated?.(latest);
  }, [onProjectUpdated]);

  const executeAction = useCallback(async (
    name: string,
    options: TaskControlActionOptions = {},
  ) => {
    if (!project?.name || !snapshot || busy) return;
    setBusy(true);
    setError("");
    try {
      const result = await invokeWithTimeout<TaskControlActionResult>("apply_task_control_action", {
        projectName: project.name,
        request: {
          action: name,
          expected_revision: snapshot.project_revision,
          expected_tree_revision: snapshot.task_tree_revision,
          task_id: snapshot.current_task_id || undefined,
          decision_id: snapshot.decision?.decision_id ?? "",
          criterion_indexes: options.criterionIndexes ?? [],
          reason: options.reason ?? "",
        },
      }, 900_000);
      if (scope.current.projectName !== project.name) return;
      requestSequence.current += 1;
      setSnapshot(result.snapshot);
      await syncProject(project.name);
      await refresh();
    } catch (reason) {
      if (scope.current.projectName === project.name) {
        setError(String(reason));
        if (String(reason).includes("修订冲突")) await refresh();
      }
    } finally {
      if (scope.current.projectName === project.name) setBusy(false);
    }
  }, [busy, project, refresh, snapshot, syncProject]);

  const changeMode = useCallback(async (mode: TaskControlMode) => {
    if (!project?.name || !snapshot || busy || mode === snapshot.control_mode) return;
    setBusy(true);
    setError("");
    try {
      const updated = await invokeWithTimeout<Project>("set_task_control_mode", {
        projectName: project.name,
        mode,
        expectedRevision: snapshot.project_revision,
      }, 15_000);
      if (scope.current.projectName !== project.name) return;
      onProjectUpdated?.(updated);
      await refresh();
    } catch (reason) {
      if (scope.current.projectName === project.name) setError(String(reason));
    } finally {
      if (scope.current.projectName === project.name) setBusy(false);
    }
  }, [busy, onProjectUpdated, project, refresh, snapshot]);

  return {
    snapshot,
    selectedTaskId,
    selectedNode,
    busy,
    error,
    refresh,
    selectTask,
    executeAction,
    changeMode,
  };
}
