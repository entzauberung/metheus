// src/PauseDecisionPanel.tsx — 暂停决策面板
import { useState } from "react";
import { Hammer, Pause, Play, RotateCcw, Square } from "lucide-react";
interface PauseDecisionPanelProps {
  pauseType: 'in_stop' | 'ed_stop';
  onContinue: () => Promise<void>;
  onAdjustOnly: () => Promise<void>;
  onRollback: () => Promise<void>;
  busy?: boolean;
  error?: string | null;
}

export function PauseDecisionPanel({
  pauseType,
  onContinue,
  onAdjustOnly,
  onRollback,
  busy = false,
  error = null,
}: PauseDecisionPanelProps) {
  const [localBusy, setLocalBusy] = useState(false);
  const isBusy = busy || localBusy;

  const wrap = (fn: () => Promise<void>) => async () => {
    setLocalBusy(true);
    try {
      await fn();
    } finally {
      setLocalBusy(false);
    }
  };

  return (
    <div className="pause-decision-panel">
      <div className={`pause-type-badge ${pauseType === 'in_stop' ? 'in-stop' : 'ed-stop'}`}>
        {pauseType === 'in_stop' ? <><Square size={14} />立即暂停 (In Stop)</> : <><Pause size={14} />小阶段完成后暂停 (ED Stop)</>}
      </div>

      <h2>执行已暂停，请选择下一步</h2>
      <p style={{ color: '#656d76', fontSize: '13px', marginBottom: '20px' }}>
        {pauseType === 'in_stop'
          ? '当前执行中的任务未完成，不会保留部分结果。'
          : '刚完成的任务已通过测试并写入标签，得到保留。'}
      </p>

      {error && (
        <div style={{ padding: "10px 14px", background: "#fff1f0", borderRadius: "6px", border: "1px solid #cf222e", marginBottom: "16px", fontSize: "13px", color: "#cf222e" }}>
          {error}
        </div>
      )}

      <div className="pause-actions">
        <button className="pause-action-btn" onClick={wrap(onContinue)} disabled={isBusy}>
          <span className="pause-action-btn-icon"><Play size={20} /></span>
          <div>
            <div className="pause-action-btn-title">{isBusy ? '处理中...' : '继续原计划'}</div>
            <div className="pause-action-btn-desc">
              {pauseType === 'in_stop'
                ? '回到上一个稳定检查点，重新执行当前任务'
                : '从下一个待执行任务继续'}
            </div>
          </div>
        </button>

        <button className="pause-action-btn" onClick={wrap(onAdjustOnly)} disabled={isBusy}>
          <span className="pause-action-btn-icon"><Hammer size={20} /></span>
          <div>
            <div className="pause-action-btn-title">保留已完成，只调整后续</div>
            <div className="pause-action-btn-desc">保留所有已通过任务的结果，只调整后续未执行的任务</div>
          </div>
        </button>

        <button className="pause-action-btn" onClick={wrap(onRollback)} disabled={isBusy}>
          <span className="pause-action-btn-icon"><RotateCcw size={20} /></span>
          <div>
            <div className="pause-action-btn-title">回退到更早稳定点</div>
            <div className="pause-action-btn-desc">回退到更早的已完成任务，重新规划后续</div>
          </div>
        </button>
      </div>
    </div>
  );
}
