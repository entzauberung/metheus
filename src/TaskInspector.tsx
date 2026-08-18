import * as Tabs from "@radix-ui/react-tabs";
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  ClipboardList,
  Coins,
  Square,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import AcceptanceLedgerPanel from "./AcceptanceLedgerPanel";
import ControlDecisionPanel from "./ControlDecisionPanel";
import TaskContractPanel from "./TaskContractPanel";
import TaskCostPanel from "./TaskCostPanel";
import TaskInspectorHeader from "./TaskInspectorHeader";
import { resolveRecoveryBarProgress } from "./autopilotBarPresentation";
import { findProjectSubtaskById } from "./taskTreePolicy";
import type { TaskSelectionMode } from "./taskSelectionPolicy";
import type {
  Project,
  RecoveryPresentation,
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
  Provability,
} from "./types";
import type { RuntimeOutcomePresentation } from "./runtimeOutcomePresentation";

interface Props {
  project: Project;
  snapshot: TaskControlSnapshot | null;
  selectedNode: TaskTreeNodeView | null;
  selectedTaskId: string;
  busy: boolean;
  error: string;
  recoveryPresentation: RecoveryPresentation | null;
  runtimeOutcome?: RuntimeOutcomePresentation;
  expectedEventSequence: number;
  detailsSyncing: boolean;
  onClose: () => void;
  onRefresh: () => void;
  onAction: (name: string, options?: { criterionIndexes?: number[]; reason?: string }) => void;
  onConfirmHumanReview?: (criterionIndex: number, reason: string) => void;
  onChangeMode: (mode: TaskControlMode, reason?: string) => void;
  selectionMode?: TaskSelectionMode;
  onFollowCurrentTask?: () => void;
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

function formatRecoveryTime(value: string | null | undefined): string {
  if (!value) return "未记录";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

function formatRecoveryElapsed(value: number | null | undefined): string {
  if (value === null || value === undefined) return "未记录";
  const seconds = Math.max(0, Math.floor(value));
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes === 0) return `${remainder} 秒`;
  return remainder > 0 ? `${minutes} 分 ${remainder} 秒` : `${minutes} 分`;
}

export default function TaskInspector({
  project,
  snapshot,
  selectedNode,
  selectedTaskId,
  busy,
  error,
  recoveryPresentation,
  runtimeOutcome,
  expectedEventSequence,
  detailsSyncing,
  onClose,
  onRefresh,
  onAction,
  onConfirmHumanReview,
  onChangeMode,
  selectionMode = "follow",
  onFollowCurrentTask = () => undefined,
}: Props) {
  const [activeTab, setActiveTab] = useState("overview");
  const [deviationCriterion, setDeviationCriterion] = useState("");
  const [deviationReason, setDeviationReason] = useState("");
  const [humanCriterion, setHumanCriterion] = useState("");
  const [humanReason, setHumanReason] = useState("");
  const selectedSubtask = useMemo(
    () => findProjectSubtaskById(project, selectedTaskId),
    [project, selectedTaskId],
  );
  const acceptance = selectedNode?.acceptance ?? [];
  const acceptanceCriteria = selectedSubtask?.acceptance_criteria?.length
    ? selectedSubtask.acceptance_criteria
    : acceptance.map(item => item.criterion);
  const actionableAcceptance = selectedNode?.actionable_acceptance_criteria ?? [];
  const deviationOptions = acceptance.filter(item => (
    actionableAcceptance.includes(item.criterion_index)
  ));
  const provabilityByIndex = useMemo(() => Object.fromEntries(
    acceptance.map(item => {
      const offset = item.criterion_index - 1;
      const tagged = selectedSubtask?.acceptance_criteria_meta?.[offset]?.provability;
      const contracted = selectedNode?.contract?.verification_modes?.[offset] as Provability | undefined;
      return [item.criterion_index, tagged ?? contracted];
    }),
  ) as Partial<Record<number, Provability>>, [acceptance, selectedNode, selectedSubtask]);
  const humanOptions = acceptance.filter(item => (
    actionableAcceptance.includes(item.criterion_index)
    && provabilityByIndex[item.criterion_index] === "HumanReview"
  ));
  const isCurrentTask = !!selectedNode && selectedNode.id === snapshot?.current_task_id;
  const recovery = recoveryPresentation?.kind !== "None" ? recoveryPresentation : null;
  const recoveryProgress = recovery ? resolveRecoveryBarProgress(recovery) : null;
  const snapshotStale = !!snapshot
    && expectedEventSequence > 0
    && snapshot.source_event_sequence < expectedEventSequence;
  const writesDisabled = !snapshot || snapshotStale || detailsSyncing
    || runtimeOutcome?.writeAllowed === false;
  const canAcceptDeviation = !writesDisabled
    && (selectedNode?.capabilities ?? []).includes("accept_deviation");
  const canConfirmHumanReview = !writesDisabled
    && !!onConfirmHumanReview
    && (selectedNode?.capabilities ?? []).includes("confirm_actual_pass")
    && project.workflow_state.recovery_state?.phase === "WaitingHuman"
    && project.workflow_state.recovery_state.subtask_id === selectedTaskId;
  const relatedEvents = selectedNode?.node_type === "Subtask"
    ? (snapshot?.events ?? []).filter(event => event.task_id === selectedNode.id)
    : snapshot?.events ?? [];

  useEffect(() => {
    if (!deviationOptions.some(item => String(item.criterion_index) === deviationCriterion)) {
      setDeviationCriterion(deviationOptions[0] ? String(deviationOptions[0].criterion_index) : "");
    }
  }, [deviationCriterion, deviationOptions]);

  useEffect(() => {
    if (!humanOptions.some(item => String(item.criterion_index) === humanCriterion)) {
      setHumanCriterion(humanOptions[0] ? String(humanOptions[0].criterion_index) : "");
    }
  }, [humanCriterion, humanOptions]);

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
        writesDisabled={writesDisabled}
        onClose={onClose}
        onRefresh={onRefresh}
        onAction={name => onAction(name)}
        onChangeMode={onChangeMode}
        selectionMode={selectionMode}
        onFollowCurrentTask={onFollowCurrentTask}
      />
      {error && <div className="task-control-error" role="alert">{error}</div>}
      <div className={`task-control-freshness${snapshotStale || detailsSyncing || runtimeOutcome?.writeAllowed === false ? " stale" : ""}`} role="status">
        {runtimeOutcome
          ? `${runtimeOutcome.statusLabel}：${runtimeOutcome.summary}${runtimeOutcome.writeAllowed ? "" : `；${runtimeOutcome.writeBlockedReason}`}`
          : detailsSyncing
          ? error
            ? "控制详情暂不可用；主状态已更新，正在后台重试"
            : "控制详情正在同步"
          : snapshot
          ? `控制快照来自状态事件 #${snapshot.source_event_sequence}${snapshotStale ? "，正在等待刷新" : ""}`
          : "控制数据暂时不可用"}
      </div>
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
          <AcceptanceLedgerPanel
            items={acceptance}
            criteria={acceptanceCriteria}
            taskStatus={selectedSubtask?.status}
            runtimeOutcome={runtimeOutcome}
            provabilityByIndex={provabilityByIndex}
          />
          {canConfirmHumanReview && humanOptions.length > 0 && (
            <section className="task-control-panel task-control-human-review-form">
              <h3>人工确认边界</h3>
              <p className="task-control-muted">视觉、体验或真实运行时结果不能由静态代码证据代替，请只确认你实际检查过的验收项。</p>
              <select value={humanCriterion} onChange={event => setHumanCriterion(event.target.value)} aria-label="人工确认的验收项">
                {humanOptions.map(item => <option key={item.criterion_index} value={item.criterion_index}>#{item.criterion_index} {item.criterion}</option>)}
              </select>
              <textarea value={humanReason} onChange={event => setHumanReason(event.target.value)} placeholder="记录实际检查方式与结论" aria-label="人工确认依据" />
              <button
                type="button"
                onClick={() => onConfirmHumanReview?.(Number(humanCriterion), humanReason.trim())}
                disabled={busy || !humanCriterion || !humanReason.trim()}
              >
                <CheckCircle2 size={14} />我已确认该项
              </button>
            </section>
          )}
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
          {!canAcceptDeviation && selectedNode?.node_type === "Subtask"
            && selectedNode.disabled_reasons.accept_deviation && (
              <p className="task-control-muted" data-testid="accept-deviation-disabled-reason">
                {selectedNode.disabled_reasons.accept_deviation}
              </p>
            )}
        </Tabs.Content>

        <Tabs.Content className="task-inspector-tab-content" value="recovery">
          <ControlDecisionPanel
            decision={isCurrentTask ? snapshot?.decision : undefined}
            shadowComparison={snapshot?.shadow_comparison}
          />
          <section className="task-control-panel">
            <div className="task-control-panel-title"><Activity size={16} /><h3>恢复状态</h3></div>
            {!recovery ? (
              <p className="task-control-muted">
                {runtimeOutcome?.state === "waiting_human"
                  ? runtimeOutcome.summary
                  : "当前任务没有活动恢复流程。"}
              </p>
            ) : (
              <dl className="task-recovery-details">
                <div data-recovery-progress={recoveryProgress?.status}>
                  <dt>进度</dt>
                  <dd className="task-recovery-status">
                    {recoveryProgress?.status === "stalled"
                      || recoveryProgress?.status === "warning"
                      ? <AlertTriangle size={14} aria-hidden="true" />
                      : recoveryProgress?.status === "waiting_human"
                        ? <Square size={13} aria-hidden="true" />
                        : <Activity size={14} aria-hidden="true" />}
                    {recoveryProgress?.statusLabel ?? "恢复进度未记录"}
                  </dd>
                </div>
                <div><dt>恢复类型</dt><dd>{recovery.title || "未记录"}</dd></div>
                <div><dt>阶段</dt><dd>{recovery.phase_label || "未记录"}</dd></div>
                {recovery.validation_phase_label && <div><dt>验证</dt><dd>{recovery.validation_phase_label}</dd></div>}
                <div><dt>原因</dt><dd>{recovery.reason || "未记录"}</dd></div>
                {recovery.affected_task_label && <div><dt>任务</dt><dd>{recovery.affected_task_label}</dd></div>}
                <div><dt>动作</dt><dd>{recovery.current_action || "未记录"}</dd></div>
                <div><dt>开始时间</dt><dd>{formatRecoveryTime(recovery.action_started_at)}</dd></div>
                <div><dt>已持续</dt><dd>{formatRecoveryElapsed(recovery.elapsed_seconds)}</dd></div>
                <div><dt>最后业务进展</dt><dd>{formatRecoveryTime(recovery.last_progress_at)}</dd></div>
                <div><dt>进展警告</dt><dd>{formatRecoveryTime(recovery.warning_at)}</dd></div>
                <div><dt>最迟终止</dt><dd>{formatRecoveryTime(recovery.hard_deadline_at)}</dd></div>
                <div>
                  <dt>下一重试</dt>
                  <dd>{formatRecoveryTime(
                    recovery.next_validation_retry_at ?? recovery.next_retry_at,
                  )}</dd>
                </div>
                <div><dt>基线</dt><dd>{recovery.code_impact_summary || "未记录"}</dd></div>
                {recovery.background_retry_active && recovery.background_retry_summary && (
                  <div><dt>后台重试</dt><dd>{recovery.background_retry_summary}</dd></div>
                )}
                {(recovery.retry_limit ?? 0) > 0 && <div><dt>重试计数</dt><dd>{recovery.retry_count}/{recovery.retry_limit}</dd></div>}
                {(recovery.validation_retry_limit ?? 0) > 0 && <div><dt>验证重试</dt><dd>{recovery.validation_retry_count}/{recovery.validation_retry_limit}</dd></div>}
                <div><dt>心跳</dt><dd>{recovery.heartbeat_status || "未记录"}</dd></div>
                {recovery.control_action_description && <div><dt>控制占用</dt><dd>{recovery.control_action_description}</dd></div>}
                {recovery.kind === "ControlActionOccupied" && <div><dt>已持续</dt><dd>{recovery.control_action_elapsed_seconds ?? 0} 秒</dd></div>}
                {recovery.control_lock_failure_reason && <div><dt>失效原因</dt><dd>{recovery.control_lock_failure_reason}</dd></div>}
                {recovery.automated_test_status && <div><dt>自动化测试</dt><dd>{recovery.automated_test_status}</dd></div>}
                {recovery.code_review_status && <div><dt>代码审查</dt><dd>{recovery.code_review_status}</dd></div>}
                {recovery.review_protocol_status && <div><dt>审查协议</dt><dd>{recovery.review_protocol_status}</dd></div>}
                {recovery.acceptance_evidence_status && <div><dt>验收证据</dt><dd>{recovery.acceptance_evidence_status}</dd></div>}
                {recovery.post_action_expectation && <div><dt>动作后</dt><dd>{recovery.post_action_expectation}</dd></div>}
                {recovery.sync_risk_summary && <div><dt>同步风险</dt><dd>{recovery.sync_risk_summary}</dd></div>}
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
