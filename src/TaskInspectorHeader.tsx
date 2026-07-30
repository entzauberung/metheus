import {
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Split,
  Square,
  X,
} from "lucide-react";
import { getTaskControlModeLabel, hasControlCapability } from "./taskControlPolicy";
import type {
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "./types";

interface Props {
  snapshot: TaskControlSnapshot | null;
  selectedNode: TaskTreeNodeView | null;
  busy: boolean;
  onClose: () => void;
  onRefresh: () => void;
  onAction: (name: string) => void;
  onChangeMode: (mode: TaskControlMode) => void;
}

export default function TaskInspectorHeader({
  snapshot,
  selectedNode,
  busy,
  onClose,
  onRefresh,
  onAction,
  onChangeMode,
}: Props) {
  const capabilities = snapshot?.control_capabilities ?? [];
  const isCurrent = !!selectedNode && selectedNode.id === snapshot?.current_task_id;
  const hasCapability = (name: string) => hasControlCapability(capabilities, name);
  const canUseTaskAction = (name: string) => isCurrent && hasCapability(name);

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

      <div className="task-inspector-mode">
        <label htmlFor="task-control-mode">控制模式</label>
        <select
          id="task-control-mode"
          value={snapshot?.control_mode ?? "Legacy"}
          onChange={event => onChangeMode(event.target.value as TaskControlMode)}
          disabled={busy || !snapshot}
        >
          {(["Legacy", "Shadow", "SerialTakeover"] as TaskControlMode[]).map(mode => (
            <option key={mode} value={mode}>{getTaskControlModeLabel(mode)}</option>
          ))}
        </select>
      </div>

      <div className="task-inspector-toolbar" aria-label="任务控制操作">
        <button type="button" onClick={() => onAction("pause")} disabled={busy || !hasCapability("pause")}><Pause size={14} />暂停</button>
        <button type="button" onClick={() => onAction("resume")} disabled={busy || !hasCapability("resume")}><Play size={14} />恢复</button>
        <button type="button" onClick={() => onAction("revalidate")} disabled={busy || !canUseTaskAction("revalidate")}><ShieldCheck size={14} />重新验证</button>
        <button type="button" onClick={() => onAction("split")} disabled={busy || !canUseTaskAction("split")}><Split size={14} />拆分</button>
        <button type="button" onClick={() => onAction("recompile")} disabled={busy || !canUseTaskAction("recompile")}><RotateCcw size={14} />重编译</button>
        <button type="button" onClick={() => onAction("stop")} disabled={busy || !hasCapability("stop")}><Square size={14} />停止</button>
      </div>
      {!isCurrent && selectedNode && (
        <p className="task-inspector-history-note">当前正在查看历史任务，任务级操作仅对正在执行的任务开放。</p>
      )}
    </header>
  );
}
