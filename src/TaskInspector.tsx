import * as Tabs from "@radix-ui/react-tabs";
import {
  Activity,
  CheckCircle2,
  ClipboardList,
  Coins,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import AcceptanceLedgerPanel from "./AcceptanceLedgerPanel";
import ControlDecisionPanel from "./ControlDecisionPanel";
import TaskContractPanel from "./TaskContractPanel";
import TaskCostPanel from "./TaskCostPanel";
import TaskInspectorHeader from "./TaskInspectorHeader";
import { findProjectSubtaskById } from "./taskTreePolicy";
import type {
  Project,
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "./types";

interface Props {
  project: Project;
  snapshot: TaskControlSnapshot | null;
  selectedNode: TaskTreeNodeView | null;
  selectedTaskId: string;
  busy: boolean;
  error: string;
  onClose: () => void;
  onRefresh: () => void;
  onAction: (name: string, options?: { criterionIndexes?: number[]; reason?: string }) => void;
  onChangeMode: (mode: TaskControlMode) => void;
}

const EMPTY_COST = {
  calls: 0,
  known_input_tokens: 0,
  known_output_tokens: 0,
  known_total_tokens: 0,
  usage_known_calls: 0,
  usage_unknown_calls: 0,
  effective_calls: 0,
  no_progress_calls: 0,
};

export default function TaskInspector({
  project,
  snapshot,
  selectedNode,
  selectedTaskId,
  busy,
  error,
  onClose,
  onRefresh,
  onAction,
  onChangeMode,
}: Props) {
  const [activeTab, setActiveTab] = useState("overview");
  const [deviationCriterion, setDeviationCriterion] = useState("");
  const [deviationReason, setDeviationReason] = useState("");
  const selectedSubtask = useMemo(
    () => findProjectSubtaskById(project, selectedTaskId),
    [project, selectedTaskId],
  );
  const acceptance = selectedNode?.acceptance ?? [];
  const deviationOptions = acceptance.filter(item => (
    item.status !== "Satisfied" && item.status !== "AcceptedDeviation"
  ));
  const isCurrentTask = !!selectedNode && selectedNode.id === snapshot?.current_task_id;
  const canAcceptDeviation = isCurrentTask
    && (snapshot?.control_capabilities ?? []).includes("accept_deviation");
  const recovery = project.workflow_state.recovery_state;
  const relatedEvents = selectedNode?.node_type === "Subtask"
    ? (snapshot?.events ?? []).filter(event => event.task_id === selectedNode.id)
    : snapshot?.events ?? [];

  useEffect(() => {
    if (!deviationOptions.some(item => String(item.criterion_index) === deviationCriterion)) {
      setDeviationCriterion(deviationOptions[0] ? String(deviationOptions[0].criterion_index) : "");
    }
  }, [deviationCriterion, deviationOptions]);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <aside id="task-inspector" className="task-inspector" aria-label="任务检查器">
      <TaskInspectorHeader
        snapshot={snapshot}
        selectedNode={selectedNode}
        busy={busy}
        onClose={onClose}
        onRefresh={onRefresh}
        onAction={name => onAction(name)}
        onChangeMode={onChangeMode}
      />
      {error && <div className="task-control-error" role="alert">{error}</div>}
      <Tabs.Root className="task-inspector-tabs" value={activeTab} onValueChange={setActiveTab}>
        <Tabs.List className="task-inspector-tab-list" aria-label="任务详情页面">
          <Tabs.Trigger value="overview" title="概览与合同"><ClipboardList size={15} /><span>概览</span></Tabs.Trigger>
          <Tabs.Trigger value="acceptance" title="验收与证据"><CheckCircle2 size={15} /><span>验收</span></Tabs.Trigger>
          <Tabs.Trigger value="recovery" title="决策与恢复"><Activity size={15} /><span>恢复</span></Tabs.Trigger>
          <Tabs.Trigger value="cost" title="成本与事件"><Coins size={15} /><span>成本</span></Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content className="task-inspector-tab-content" value="overview">
          <TaskContractPanel contract={selectedNode?.contract} />
        </Tabs.Content>

        <Tabs.Content className="task-inspector-tab-content" value="acceptance">
          {selectedSubtask?.test_result && (
            <section className="task-control-panel task-test-summary">
              <div className="task-control-panel-title"><CheckCircle2 size={16} /><h3>自动化验证</h3></div>
              <dl>
                <div><dt>状态</dt><dd>{selectedSubtask.test_result.automated_test_status ?? "Unknown"}</dd></div>
                <div><dt>通道</dt><dd>{selectedSubtask.test_result.verification_kind ?? "Legacy"}</dd></div>
                {selectedSubtask.test_result.test_command && <div><dt>命令</dt><dd><code>{selectedSubtask.test_result.test_command}</code></dd></div>}
                {selectedSubtask.test_result.test_exit_code !== undefined && <div><dt>退出码</dt><dd>{selectedSubtask.test_result.test_exit_code}</dd></div>}
                {selectedSubtask.test_result.test_output_summary && <div><dt>摘要</dt><dd>{selectedSubtask.test_result.test_output_summary}</dd></div>}
              </dl>
            </section>
          )}
          <AcceptanceLedgerPanel items={acceptance} />
          {canAcceptDeviation && deviationOptions.length > 0 && (
            <section className="task-control-panel task-control-deviation-form">
              <h3>接受验收偏差</h3>
              <select value={deviationCriterion} onChange={event => setDeviationCriterion(event.target.value)} aria-label="接受偏差的验收项">
                {deviationOptions.map(item => <option key={item.criterion_index} value={item.criterion_index}>#{item.criterion_index} {item.criterion}</option>)}
              </select>
              <textarea value={deviationReason} onChange={event => setDeviationReason(event.target.value)} placeholder="记录偏差原因与影响" aria-label="偏差原因" />
              <button
                type="button"
                onClick={() => onAction("accept_deviation", {
                  criterionIndexes: [Number(deviationCriterion)],
                  reason: deviationReason.trim(),
                })}
                disabled={busy || !deviationCriterion || !deviationReason.trim()}
              >
                <CheckCircle2 size={14} />接受偏差
              </button>
            </section>
          )}
        </Tabs.Content>

        <Tabs.Content className="task-inspector-tab-content" value="recovery">
          <ControlDecisionPanel
            decision={isCurrentTask ? snapshot?.decision : undefined}
            shadowComparison={snapshot?.shadow_comparison}
          />
          <section className="task-control-panel">
            <div className="task-control-panel-title"><Activity size={16} /><h3>恢复状态</h3></div>
            {!recovery || recovery.subtask_id !== selectedTaskId ? (
              <p className="task-control-muted">当前任务没有活动恢复流程。</p>
            ) : (
              <dl className="task-recovery-details">
                <div><dt>分类</dt><dd>{recovery.error_kind}</dd></div>
                <div><dt>阶段</dt><dd>{recovery.phase}</dd></div>
                <div><dt>恢复次数</dt><dd>{recovery.attempt}/{recovery.max_attempts}</dd></div>
                <div><dt>补证次数</dt><dd>{recovery.evidence_rebuild_attempts}</dd></div>
                <div><dt>待补验收</dt><dd>{recovery.pending_evidence_criteria.join("、") || "无"}</dd></div>
                <div><dt>诊断</dt><dd>{recovery.last_diagnosis || "尚无诊断"}</dd></div>
              </dl>
            )}
          </section>
        </Tabs.Content>

        <Tabs.Content className="task-inspector-tab-content" value="cost">
          <TaskCostPanel
            cost={snapshot?.cost ?? EMPTY_COST}
            stageCost={snapshot?.stage_cost}
            taskCost={isCurrentTask ? snapshot?.task_cost : undefined}
            calls={isCurrentTask ? snapshot?.cost_calls : undefined}
            providerCosts={snapshot?.provider_costs}
            purposeCosts={snapshot?.purpose_costs}
          />
          <section className="task-control-panel task-inspector-events">
            <div className="task-control-panel-title"><Activity size={16} /><h3>控制事件</h3></div>
            {relatedEvents.length === 0 ? <p className="task-control-muted">当前任务没有关联事件。</p> : (
              <ol>
                {relatedEvents.map((event, index) => (
                  <li key={`${event.timestamp}-${index}`}>
                    <time>{event.timestamp}</time>
                    <strong>{event.source}</strong>
                    <p>{event.text}</p>
                    {(event.criterion_index || event.action_id || event.validator_id || event.model_call_id) && (
                      <small>
                        {event.criterion_index ? `验收 #${event.criterion_index}` : ""}
                        {event.action_id ? ` · 动作 ${event.action_id}` : ""}
                        {event.validator_id ? ` · 验证器 ${event.validator_id}` : ""}
                        {event.model_call_id ? ` · 调用 ${event.model_call_id}` : ""}
                      </small>
                    )}
                  </li>
                ))}
              </ol>
            )}
          </section>
        </Tabs.Content>
      </Tabs.Root>
    </aside>
  );
}
