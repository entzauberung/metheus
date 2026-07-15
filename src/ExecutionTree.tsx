import { useState } from "react";
import {
  BookOpen,
  CheckCircle2,
  Circle,
  CircleDot,
  Clock3,
  PauseCircle,
} from "lucide-react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { Project } from "./types";
import { IconButton } from "./components/IconButton";
import { Modal } from "./components/Modal";

interface Props {
  project: Project;
  projectPath: string;
  onSelectMilestone: (id: string) => Promise<void>;
  onSelectMidStage: (id: string) => Promise<void>;
}

function statusIcon(status: string) {
  switch (status) {
    case "InProgress":
    case "Executing":
      return <CircleDot size={15} aria-hidden="true" />;
    case "Completed":
    case "Passed":
      return <CheckCircle2 size={15} aria-hidden="true" />;
    case "Paused":
      return <PauseCircle size={15} aria-hidden="true" />;
    case "AwaitingConfirmation":
      return <Clock3 size={15} aria-hidden="true" />;
    default:
      return <Circle size={15} aria-hidden="true" />;
  }
}

export default function ExecutionTree({
  project,
  projectPath,
  onSelectMilestone,
  onSelectMidStage,
}: Props) {
  const [constitutionOpen, setConstitutionOpen] = useState(false);
  const [constitutionContent, setConstitutionContent] = useState("");
  const [constitutionLoading, setConstitutionLoading] = useState(false);
  const [selectionBusy, setSelectionBusy] = useState(false);
  const step = project.workflow_state.current_step;
  const canSelectMilestone = step === "MilestoneSelection";
  const canSelectMidStage = step === "MidStageSelection";

  const viewConstitution = async () => {
    setConstitutionOpen(true);
    setConstitutionLoading(true);
    try {
      const content = await invokeWithTimeout<string>("read_constitution", { projectPath });
      setConstitutionContent(content);
    } catch (error) {
      setConstitutionContent("读取失败：" + String(error));
    } finally {
      setConstitutionLoading(false);
    }
  };

  const selectMilestone = async (id: string) => {
    if (!canSelectMilestone || selectionBusy || id === project.current_milestone_id) return;
    setSelectionBusy(true);
    try {
      await onSelectMilestone(id);
    } finally {
      setSelectionBusy(false);
    }
  };

  const selectMidStage = async (id: string) => {
    if (!canSelectMidStage || selectionBusy || id === project.current_mid_stage_id) return;
    setSelectionBusy(true);
    try {
      await onSelectMidStage(id);
    } finally {
      setSelectionBusy(false);
    }
  };

  return (
    <div className="execution-tree">
      <div className="tree-header">
        <h3 className="tree-title">执行树</h3>
        <IconButton
          icon={<BookOpen size={16} />}
          tooltip="查看项目宪法"
          onClick={viewConstitution}
          disabled={!projectPath}
        />
      </div>

      <p className="tree-selection-hint">
        {canSelectMilestone
          ? "请选择大阶段"
          : canSelectMidStage
            ? "请选择中阶段"
            : "当前步骤不允许切换阶段"}
      </p>

      <ul className="tree-list">
        {project.milestones.map((milestone) => {
          const selected = milestone.id === project.current_milestone_id;
          return (
            <li key={milestone.id} className="tree-group">
              <button
                className={`tree-node tree-milestone${selected ? " selected" : ""}`}
                onClick={() => selectMilestone(milestone.id)}
                disabled={!canSelectMilestone || selectionBusy}
                title={!canSelectMilestone ? "当前步骤不允许切换大阶段" : undefined}
              >
                {statusIcon(milestone.status)}
                <span className="tree-version">{milestone.version}</span>
                <span className="tree-label">{milestone.title}</span>
              </button>

              {milestone.mid_stages.length > 0 && (
                <ul className="tree-children">
                  {milestone.mid_stages.map((midStage) => {
                    const midSelected = midStage.id === project.current_mid_stage_id;
                    return (
                      <li key={midStage.id}>
                        <button
                          className={`tree-node tree-mid-stage${midSelected ? " selected" : ""}`}
                          onClick={() => selectMidStage(midStage.id)}
                          disabled={!canSelectMidStage || !selected || selectionBusy}
                          title={!canSelectMidStage ? "当前步骤不允许切换中阶段" : undefined}
                        >
                          {statusIcon(midStage.status)}
                          <span className="tree-version">{midStage.version}</span>
                          <span className="tree-label">{midStage.title}</span>
                        </button>
                        {midStage.subtasks.length > 0 && (
                          <ul className="tree-children tree-subtasks">
                            {midStage.subtasks.map((subtask) => (
                              <li key={subtask.id} className="tree-node tree-subtask" title={subtask.title}>
                                {statusIcon(subtask.status)}
                                <span className="tree-label">{subtask.title}</span>
                                {subtask.auto_tag && <code className="tree-tag">{subtask.auto_tag}</code>}
                              </li>
                            ))}
                          </ul>
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
            </li>
          );
        })}
      </ul>

      <Modal
        isOpen={constitutionOpen}
        onClose={() => setConstitutionOpen(false)}
        title="项目宪法"
        description="当前项目的持久化规则与事实"
      >
        <pre className="constitution-content">
          {constitutionLoading ? "读取中..." : constitutionContent || "宪法为空"}
        </pre>
      </Modal>
    </div>
  );
}
