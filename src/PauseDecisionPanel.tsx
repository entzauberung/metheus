// src/PauseDecisionPanel.tsx — 暂停决策面板
import { Bot, Hammer, MessageSquare, Pause, Play, RotateCcw, Square } from "lucide-react";
interface PauseDecisionPanelProps {
  pauseType: 'in_stop' | 'ed_stop';
  onContinue: () => void;
  onAdjustOnly: () => void;
  onRollback: () => void;
  /** 是否由 autopilot 暂停触发 */
  isAutopilot?: boolean;
  /** autopilot: 继续自动驾驶 */
  onResumeAutopilot?: () => void;
  /** autopilot: 进入暂停讨论 */
  onEnterAutopilotDiscussion?: () => void;
  /** autopilot: 退出自动驾驶改手动 */
  onExitAutopilot?: () => void;
}

export function PauseDecisionPanel({
  pauseType,
  onContinue,
  onAdjustOnly,
  onRollback,
  isAutopilot,
  onResumeAutopilot,
  onEnterAutopilotDiscussion,
  onExitAutopilot,
}: PauseDecisionPanelProps) {
  return (
    <div className="pause-decision-panel">
      <div className={`pause-type-badge ${pauseType === 'in_stop' ? 'in-stop' : 'ed-stop'}`}>
        {pauseType === 'in_stop' ? <><Square size={14} />立即暂停 (In Stop)</> : <><Pause size={14} />小阶段完成后暂停 (ED Stop)</>}
        {isAutopilot && <span style={{ marginLeft: 8, opacity: 0.7 }}>| 自动驾驶</span>}
      </div>

      <h2>执行已暂停，请选择下一步</h2>
      <p style={{ color: '#656d76', fontSize: '13px', marginBottom: '20px' }}>
        {pauseType === 'in_stop'
          ? '当前执行中的任务未完成，不会保留部分结果。'
          : '刚完成的任务已通过测试并写入标签，得到保留。'}
        {isAutopilot && ' 自动驾驶已暂停，你可以继续自动驾驶、进入讨论或改用手动操作。'}
      </p>

      {/* autopilot 专属操作区 */}
      {isAutopilot && (
        <div className="pause-actions" style={{ marginBottom: 16 }}>
          <button className="pause-action-btn primary" onClick={onResumeAutopilot}>
            <span className="pause-action-btn-icon"><Bot size={20} /></span>
            <div>
              <div className="pause-action-btn-title">继续自动驾驶</div>
              <div className="pause-action-btn-desc">从当前位置恢复自动驾驶，自动推进后续步骤</div>
            </div>
          </button>

          <button className="pause-action-btn" onClick={onEnterAutopilotDiscussion}>
            <span className="pause-action-btn-icon"><MessageSquare size={20} /></span>
            <div>
              <div className="pause-action-btn-title">暂停讨论</div>
              <div className="pause-action-btn-desc">在阶段中讨论当前问题，讨论后可恢复自动驾驶</div>
            </div>
          </button>
        </div>
      )}

      {/* 分隔线（autopilot 模式下区分自动驾驶操作和手动操作） */}
      {isAutopilot && (
        <div style={{
          borderTop: '1px solid #30363d', margin: '16px 0',
          fontSize: '11px', color: '#656d76', textAlign: 'center',
          paddingTop: 8,
        }}>
          手动操作选项
        </div>
      )}

      <div className="pause-actions">
        <button className="pause-action-btn" onClick={onContinue}>
          <span className="pause-action-btn-icon"><Play size={20} /></span>
          <div>
            <div className="pause-action-btn-title">继续原计划</div>
            <div className="pause-action-btn-desc">
              {pauseType === 'in_stop'
                ? '回到上一个稳定检查点，重新执行当前任务'
                : '从下一个待执行任务继续'}
            </div>
          </div>
        </button>

        <button className="pause-action-btn" onClick={onAdjustOnly}>
          <span className="pause-action-btn-icon"><Hammer size={20} /></span>
          <div>
            <div className="pause-action-btn-title">保留已完成，只调整后续</div>
            <div className="pause-action-btn-desc">保留所有已通过任务的结果，只调整后续未执行的任务</div>
          </div>
        </button>

        <button className="pause-action-btn" onClick={onRollback}>
          <span className="pause-action-btn-icon"><RotateCcw size={20} /></span>
          <div>
            <div className="pause-action-btn-title">回退到更早稳定点</div>
            <div className="pause-action-btn-desc">回退到更早的已完成任务，重新规划后续</div>
          </div>
        </button>

        {isAutopilot && onExitAutopilot && (
          <button className="pause-action-btn secondary" onClick={onExitAutopilot}>
            <span className="pause-action-btn-icon"><Square size={20} /></span>
            <div>
              <div className="pause-action-btn-title">退出自动驾驶改手动</div>
              <div className="pause-action-btn-desc">关闭自动驾驶，保留当前阶段进度，之后手动操作</div>
            </div>
          </button>
        )}
      </div>
    </div>
  );
}
