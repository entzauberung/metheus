import {
  Pause,
  Play,
  Eye,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Split,
  Square,
  X,
} from "lucide-react";
import { useState } from "react";
import { Modal } from "./components/Modal";
import { findTaskControlNode } from "./taskTreePolicy";
import {
  getModeTransitionImpact,
  getTaskControlModeDescription,
  getTaskControlModeLabel,
  hasControlCapability,
  requiresModeFallbackConfirmation,
} from "./taskControlPolicy";
import type {
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "./types";
import type { TaskSelectionMode } from "./taskSelectionPolicy";

interface Props {
  snapshot: TaskControlSnapshot | null;
  selectedNode: TaskTreeNodeView | null;
  busy: boolean;
  writesDisabled: boolean;
  onClose: () => void;
  onRefresh: () => void;
  onAction: (name: string) => void;
  onChangeMode: (mode: TaskControlMode, reason?: string) => void;
  selectionMode: TaskSelectionMode;
  onFollowCurrentTask: () => void;
}

export default function TaskInspectorHeader({
  snapshot,
  selectedNode,
  busy,
  writesDisabled,
  onClose,
  onRefresh,
  onAction,
  onChangeMode,
  selectionMode,
  onFollowCurrentTask,
}: Props) {
  const [pendingMode, setPendingMode] = useState<TaskControlMode | null>(null);
  const [fallbackReason, setFallbackReason] = useState("");
  const capabilities = snapshot?.control_capabilities ?? [];
  const isCurrent = !!selectedNode && selectedNode.id === snapshot?.current_task_id;
  const currentTaskAvailable = !!snapshot?.current_task_id
    && !!findTaskControlNode(snapshot.nodes, snapshot.current_task_id);
  const hasCapability = (name: string) => hasControlCapability(capabilities, name);
  const canUseTaskAction = (name: string) => !writesDisabled
    && (selectedNode?.capabilities ?? []).includes(name);
  const requestModeChange = (mode: TaskControlMode) => {
    const current = snapshot?.control_mode;
    if (!current || mode === current) return;
    if (requiresModeFallbackConfirmation(current, mode)) {
      setPendingMode(mode);
      setFallbackReason("");
      return;
    }
    onChangeMode(mode, "用户在任务检查器显式选择控制模式");
  };

  return (
    <header className="task-inspector-header">
      <div className="task-inspector-heading">
        <div>
          <span className="task-control-eyebrow">TASK INSPECTOR</span>
          <h2>{selectedNode?.title ?? "任务检查器"}</h2>
        </div>
        <div className="task-inspector-heading-actions">
          <button type="button" className="icon-button" onClick={onRefresh} disabled={busy} title="刷新任务快照" aria-label="刷新任务快照">
            <RefreshCw size={16} />
          </button>
          <button type="button" className="icon-button" onClick={onClose} title="关闭任务检查器" aria-label="关闭任务检查器">
            <X size={17} />
          </button>
        </div>
      </div>

      <div className="task-inspector-summary" aria-label="任务摘要">
        <span className={`task-status task-status-${(selectedNode?.status ?? "unknown").toLowerCase()}`}>{selectedNode?.status ?? "同步中"}</span>
        {selectedNode && <span>深度 {selectedNode.depth}</span>}
        {selectedNode && <span>复杂度 {selectedNode.complexity}</span>}
        {selectedNode && <span>风险 {selectedNode.risk}</span>}
        {snapshot?.current_action && <span>动作 {snapshot.current_action.kind}</span>}
      </div>

      {selectionMode === "pinned" && selectedNode && (
        <div className="task-inspector-selection-state" role="status">
          <span>
            <Eye size={13} aria-hidden="true" />
            {isCurrent ? "固定查看当前任务" : "正在查看历史任务"}
          </span>
          {currentTaskAvailable && (
            <button
              type="button"
              className="task-inspector-follow-button"
              onClick={onFollowCurrentTask}
              aria-label="跟随当前任务"
            >
              <Play size={13} aria-hidden="true" />跟随当前任务
            </button>
          )}
        </div>
      )}

      <div className="task-inspector-mode">
        <label htmlFor="task-control-mode">当前项目实际控制模式</label>
        <select
          id="task-control-mode"
          value={snapshot?.control_mode ?? "Legacy"}
          onChange={event => requestModeChange(event.target.value as TaskControlMode)}
          disabled={busy || writesDisabled || !snapshot}
        >
          {(["Legacy", "Shadow", "SerialTakeover"] as TaskControlMode[]).map(mode => (
            <option key={mode} value={mode}>{getTaskControlModeLabel(mode)}</option>
          ))}
        </select>
        {snapshot && (
          <p className="task-control-mode-description">
            {getTaskControlModeDescription(snapshot.control_mode)}
          </p>
        )}
      </div>

      <div className="task-inspector-toolbar" aria-label="任务控制操作">
        <button type="button" onClick={() => onAction("pause")} disabled={busy || writesDisabled || !hasCapability("pause")}><Pause size={14} />暂停</button>
        <button type="button" onClick={() => onAction("resume")} disabled={busy || writesDisabled || !hasCapability("resume")}><Play size={14} />恢复</button>
        <button type="button" onClick={() => onAction("revalidate")} disabled={busy || !canUseTaskAction("revalidate")}><ShieldCheck size={14} />重新验证</button>
        <button type="button" onClick={() => onAction("split")} disabled={busy || !canUseTaskAction("split")}><Split size={14} />拆分</button>
        <button type="button" onClick={() => onAction("recompile")} disabled={busy || !canUseTaskAction("recompile")}><RotateCcw size={14} />重编译</button>
        <button type="button" onClick={() => onAction("stop")} disabled={busy || !hasCapability("stop")} title={writesDisabled ? "状态陈旧时仅允许停止或同步" : undefined}><Square size={14} />停止</button>
      </div>
      {!isCurrent && selectedNode && (
        <p className="task-inspector-history-note">
          {selectedNode.disabled_reasons.revalidate ?? "非当前任务节点只读"}
        </p>
      )}
      <Modal
        isOpen={pendingMode !== null}
        onClose={() => setPendingMode(null)}
        title="确认回退控制模式"
        description={pendingMode ? `将从串行接管切换到${getTaskControlModeLabel(pendingMode)}` : undefined}
        isDanger
        actions={[
          { label: "取消", onClick: () => setPendingMode(null), variant: "secondary" },
          {
            label: "确认回退",
            onClick: () => {
              if (!pendingMode || !fallbackReason.trim()) return;
              onChangeMode(pendingMode, fallbackReason.trim());
              setPendingMode(null);
            },
            variant: "danger",
            disabled: !fallbackReason.trim(),
          },
        ]}
      >
        <ul className="task-control-mode-impact">
          {(pendingMode ? getModeTransitionImpact(pendingMode) : []).map(item => (
            <li key={item}>{item}</li>
          ))}
        </ul>
        <label className="task-control-mode-reason">
          回退原因
          <textarea
            value={fallbackReason}
            onChange={event => setFallbackReason(event.target.value)}
            placeholder="说明为何需要回退，记录将写入项目审计"
            autoFocus
          />
        </label>
      </Modal>
    </header>
  );
}
