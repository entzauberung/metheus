import { AlertTriangle, BadgeCheck, MessageCircle, RefreshCw, WandSparkles } from "lucide-react";
import ChatRoom from "./ChatRoom";
import { ActionButton } from "./components/ActionButton";
import { FeedbackBanner } from "./components/FeedbackBanner";
import { StageCandidateCard } from "./components/StageCandidateCard";
import type { ChatMessage, Project } from "./types";

interface Props {
  project: Project;
  busy: boolean;
  onGenerate: () => Promise<void>;
  onApprove: () => Promise<void>;
  onProjectUpdated: (project: Project) => void;
  onAddMessage: (message: ChatMessage) => void;
}

export function FuturePlanningWorkspace({
  project,
  busy,
  onGenerate,
  onApprove,
  onProjectUpdated,
  onAddMessage,
}: Props) {
  const thread = project.discussion_threads.find(
    item => item.id === project.workflow_state.active_discussion_thread_id
  );
  const draft = project.milestone_draft?.draft_kind === "FutureOnly"
    ? project.milestone_draft
    : undefined;
  const retainedIds = new Set(draft?.retained_milestone_ids ?? []);
  const retained = project.milestones.filter(milestone => retainedIds.has(milestone.id));
  const candidates = draft?.candidate_milestones ?? [];
  const sourceMatches = !!thread && !!draft
    && draft.source_thread_id === thread.id
    && draft.source_thread_revision === thread.revision
    && draft.source_data_revision + 1 === project.workflow_state.data_revision;
  const canApprove = project.workflow_state.current_step === "FuturePlanApproval"
    && !!draft
    && !draft.expired
    && sourceMatches
    && draft.versions_normalized
    && draft.granularity_check_passed
    && candidates.length > 0;

  if (!thread) {
    return <div className="loading-hint">讨论线程状态需要同步</div>;
  }

  return (
    <section className="future-planning-workspace">
      <header className="future-planning-header">
        <div>
          <h2>调整未来</h2>
          <span>{draft?.expired ? "草稿已过期" : draft ? "草稿待审" : "讨论中"}</span>
        </div>
        <div className="workflow-action-bar">
          <ActionButton
            icon={draft ? <RefreshCw size={16} /> : <WandSparkles size={16} />}
            loading={busy}
            loadingLabel="生成中"
            onClick={() => void onGenerate()}
          >
            {draft ? "重新生成" : "生成草稿"}
          </ActionButton>
          {draft && (
            <ActionButton
              icon={<BadgeCheck size={16} />}
              variant="secondary"
              loading={busy}
              disabled={!canApprove}
              disabledReason={draft.expired ? "草稿已过期，请重新生成" : "草稿来源或校验状态已变化"}
              onClick={() => void onApprove()}
            >
              批准未来规划
            </ActionButton>
          )}
        </div>
      </header>

      <div className="future-planning-grid">
        <section className="future-planning-chat" aria-label="未来规划讨论">
          <div className="future-pane-heading">
            <MessageCircle size={17} />
            <h3>专属讨论</h3>
            <span>修订 {thread.revision}</span>
          </div>
          <ChatRoom
            messages={thread.messages}
            onAddMessage={onAddMessage}
            projectName={project.name}
            currentRole="产品经理"
            threadId={thread.id}
            onProjectUpdated={onProjectUpdated}
          />
        </section>

        <section className="future-planning-draft" aria-label="未来规划草稿">
          <div className="future-pane-heading">
            {draft?.expired ? <AlertTriangle size={17} /> : <BadgeCheck size={17} />}
            <h3>未来大阶段草稿</h3>
            {draft && <span>{candidates.length} 个候选</span>}
          </div>

          {!draft && <div className="future-draft-empty">尚未生成草稿</div>}

          {draft?.expired && (
            <FeedbackBanner
              type="warning"
              message="草稿已过期"
              details={[draft.expiration_reason ?? "来源讨论或项目事实已变化"]}
            />
          )}
          {draft && !draft.expired && !sourceMatches && (
            <FeedbackBanner type="warning" message="草稿来源状态已变化，请重新生成。" />
          )}
          {draft?.granularity_issues && draft.granularity_issues.length > 0 && (
            <FeedbackBanner type="error" message="粒度校验未通过" details={draft.granularity_issues} />
          )}

          {retained.length > 0 && (
            <div className="future-plan-section">
              <div className="future-plan-section-header">
                <span className="future-plan-section-badge retained">已保留</span>
                <span className="future-plan-section-desc">{retained.length} 个已完成大阶段</span>
              </div>
              <div className="candidate-list retained-list">
                {retained.map(milestone => (
                  <StageCandidateCard
                    key={milestone.id}
                    title={milestone.title}
                    version={milestone.version}
                    description={milestone.description}
                    readOnly
                    fields={[{ label: "状态", value: [milestone.status] }]}
                  />
                ))}
              </div>
            </div>
          )}

          {candidates.length > 0 && (
            <div className="future-plan-section">
              <div className="future-plan-section-header">
                <span className="future-plan-section-badge future">新规划</span>
                <span className="future-plan-section-desc">
                  {draft?.normalized_versions.join(" -> ")}
                </span>
              </div>
              <div className="candidate-list">
                {candidates.map(milestone => (
                  <StageCandidateCard
                    key={milestone.id}
                    title={milestone.title}
                    version={milestone.version}
                    description={milestone.description}
                    readOnly
                    fields={[
                      { label: "目标", value: milestone.goal },
                      { label: "范围", value: milestone.scope },
                      { label: "验收标准", value: milestone.acceptance_criteria },
                    ]}
                  />
                ))}
              </div>
            </div>
          )}
        </section>
      </div>
    </section>
  );
}
