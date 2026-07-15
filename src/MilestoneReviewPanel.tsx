// src/MilestoneReviewPanel.tsx — 大阶段审阅 A/B/C 分支
import { useState } from "react";
import { CheckCircle2, GitBranch, RotateCcw } from "lucide-react";
import { ActionButton } from "./components/ActionButton";

interface MilestoneReviewPanelProps {
  milestoneTitle: string;
  onContinue: () => void;
  onFixPast: () => void;
  onAdjustFuture: () => void;
}

export function MilestoneReviewPanel({
  milestoneTitle,
  onContinue,
  onFixPast,
  onAdjustFuture,
}: MilestoneReviewPanelProps) {
  const [selected, setSelected] = useState<string | null>(null);

  const handleConfirm = () => {
    if (selected === 'continue') onContinue();
    else if (selected === 'fix') onFixPast();
    else if (selected === 'adjust') onAdjustFuture();
  };

  return (
    <div className="milestone-review-panel">
      <h2>大阶段「{milestoneTitle}」已完成</h2>
      <p>请选择下一步方向：</p>

      <div className="branch-cards">
        <div
          className={`branch-card ${selected === 'continue' ? 'selected' : ''}`}
          onClick={() => setSelected('continue')}
        >
          <div className="branch-card-icon"><CheckCircle2 size={24} /></div>
          <div>
            <div className="branch-card-title">A：正常继续</div>
            <div className="branch-card-desc">
              批准当前大阶段成果，手动选择下一个大阶段继续执行
            </div>
          </div>
        </div>

        <div
          className={`branch-card ${selected === 'fix' ? 'selected' : ''}`}
          onClick={() => setSelected('fix')}
        >
          <div className="branch-card-icon"><RotateCcw size={24} /></div>
          <div>
            <div className="branch-card-title">B：修正过去</div>
            <div className="branch-card-desc">
              与产品经理讨论问题，生成回退建议，预览影响后再执行回退
            </div>
          </div>
        </div>

        <div
          className={`branch-card ${selected === 'adjust' ? 'selected' : ''}`}
          onClick={() => setSelected('adjust')}
        >
          <div className="branch-card-icon"><GitBranch size={24} /></div>
          <div>
            <div className="branch-card-title">C：调整未来</div>
            <div className="branch-card-desc">
              保留已完成大阶段，只重新生成后续大阶段（新阶段需经质检）
            </div>
          </div>
        </div>
      </div>

      {selected && (
        <ActionButton onClick={handleConfirm} style={{ marginTop: '20px', maxWidth: '300px' }}>
          {selected === 'continue' ? '确认继续' :
           selected === 'fix' ? '开始讨论修正' : '重新生成后续'}
        </ActionButton>
      )}
    </div>
  );
}
