// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useMemo } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout"
import { ChatMessage } from "./types"
interface Props {
  messages: ChatMessage[];
  onAddMessage: (msg: ChatMessage) => void;
  currentRole: string;
  mode: "Quick" | "Professional";
  onModeChange: (mode: "Quick" | "Professional") => void;
  modeLocked: boolean;
  onApproveVersionPlan?: () => void;
  onRejectVersionPlan?: () => void;
  onContinueNextMilestone?: () => void;
  onDiscussWithPM?: () => void;
  onViewDetailedReport?: () => void;
  hasNextMilestone?: boolean;
  // === 阶段三：分支讨论 ===
  discussionBranchType?: 'rollback' | 'redirect' | null;
  onConfirmPMSuggestion?: () => void;
  projectStatus?: string;
  hasMilestones?: boolean;
}
function ChatRoom({ messages, onAddMessage, currentRole, mode, onModeChange, modeLocked, onApproveVersionPlan, onRejectVersionPlan, onContinueNextMilestone, onDiscussWithPM, onViewDetailedReport, hasNextMilestone, discussionBranchType, onConfirmPMSuggestion, projectStatus, hasMilestones }: Props) {
  const [inputValue, setInputValue] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  // 这是【发送消息】功能。当用户点击发送按钮时执行。
  // 1. 如果输入框是空的，或者正在等待AI回复，直接退出。
  // 2. 创建一条用户消息对象（带ID、角色"user"、当前时间戳）。
  // 3. 把用户消息加到聊天记录（onAddMessage）。
  // 4. 清空输入框，并将加载状态设为true（禁用发送按钮）。
  // 5. 使用Tauri的invoke调用Rust后端的send_message函数，传入用户消息。
  // 6. 等待结果：
  //    - 成功 → 创建AI消息对象，加到聊天记录。
  //    - 失败 → 创建错误消息对象，加到聊天记录。
  // 7. 最后（无论成功失败）恢复加载状态为false（解锁发送按钮）。
  const handleSend = async () => {
    if (inputValue.trim() === "" || isLoading) return;
    /**
    * 解析输入中的 @角色 标记。
    * 若包含 @策略 / @产品 / @技术 / @测试，则将目标角色映射为全称，
    * 并将实际消息中的 @标记 移除并去除首尾空格。
    *
    * @param inputValue   - 用户输入的原始字符串
    * @param currentRole  - 当前默认角色（fallback）
    * @returns { targetRole, actualMessage }
    *   - targetRole: 角色全称（或 currentRole）
    *   - actualMessage: 去除 @标记 后的消息
    */

    let targetRole = currentRole;
    let actualMessage = inputValue;
    const mentionRegex = /@(策略|产品|技术|测试|域)/;
    const match = inputValue.match(mentionRegex);
    if (match) {
      const roleMap: Record<string, string> = {
        "策略": "策略产品经理",
        "产品": "产品经理",
        "技术": "全栈技术顾问",
        "测试": "测试工程师",
        "域": "域负责人",
      };
      targetRole = roleMap[match[1]];
      actualMessage = inputValue.replace(match[0], "").trim();
    }
    // 分支B：注入上下文信息
    let contextPrefix = '';
    if (discussionBranchType === 'redirect') {
      contextPrefix = `[上下文：当前项目阶段为"${projectStatus || '未开始'}"，已有${hasMilestones ? '若干' : '无'}大阶段。用户选择调整后续方向，请根据用户反馈重新规划后续大阶段。] `;
    }
    const finalMessage = contextPrefix + actualMessage;

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: actualMessage,
      timestamp: Date.now(),
    };
    onAddMessage(userMessage);
    setInputValue("");
    setIsLoading(true);
    try {
      const reply = await invokeWithTimeout("chat_with_role", {
        message: finalMessage,
        role: targetRole,
        threadId: "thread-init",
      });
      const replyData = reply as { id: string; role: string; content: string; timestamp: number };
      const aiMessage: ChatMessage = {
        id: replyData.id,
        role: replyData.role,
        content: replyData.content,
        timestamp: replyData.timestamp,
      };
      onAddMessage(aiMessage);
    } catch (error) {
      const errorMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "ai",
        content: `❌ 出错啦：${error}`,
        timestamp: Date.now(),
      };
      onAddMessage(errorMessage);
    } finally {
      setIsLoading(false);
    }
  };
  // 计算最新的版本方案消息时间戳，用于判定旧方案是否过期
  const latestVpTimestamp = useMemo(() => {
    const vpMessages = messages.filter(m => m.msgType === "version_plan");
    if (vpMessages.length === 0) return 0;
    return vpMessages.reduce((max, m) => Math.max(max, m.timestamp), 0);
  }, [messages]);

  return (
    <div className="chat-room">
      <div className="chat-messages">
        {/*
                如果消息数组为空，显示空提示语；
                否则，用 .map() 遍历每一条消息，生成对应的 DOM 元素。
                */}
        {messages.length === 0 ? (
          <p className="empty-tip">开始讨论你的想法吧</p>
        ) : (
          messages.map((msg, index) => {
            // 版本方案消息：特殊渲染
            if (msg.msgType === "version_plan") {
              const isExpired = msg.timestamp < latestVpTimestamp;
              const hasApproved = msg.approved === true;
              const hasRejected = msg.rejected === true;
              return (
                <div key={msg.id} className="message message-ai message-version-plan">
                  <div className="message-role">
                    {msg.role === "user" ? "你" : msg.role}
                  </div>
                  <div className="message-content">{msg.content}</div>
                  <div className="vp-actions">
                    {hasApproved ? (
                      <span className="vp-status vp-status-approved">✅ 已批准</span>
                    ) : hasRejected ? (
                      <span className="vp-status vp-status-rejected">❌ 已驳回</span>
                    ) : isExpired ? (
                      <span className="vp-status vp-status-expired">⏳ 已过期</span>
                    ) : (
                      <>
                        <button className="vp-btn-approve" onClick={onApproveVersionPlan}>✅ 批准版本方案</button>
                        <button className="vp-btn-reject" onClick={onRejectVersionPlan}>❌ 驳回，继续讨论</button>
                      </>
                    )}
                  </div>
                </div>
              );
            }
            // 质检失败消息：红色边框特殊渲染
            if (msg.msgType === "qa_failed") {
              return (
                <div key={msg.id} className="message message-system message-qa-failed">
                  <div className="message-role">
                    {msg.role === "user" ? "你" : msg.role}
                  </div>
                  <div className="message-content">{msg.content}</div>
                </div>
              );
            }
            // 大阶段总结消息：特殊渲染
            if (msg.msgType === "milestone_summary") {
              return (
                <div key={msg.id} className="message message-ai message-milestone-summary">
                  <div className="message-role">
                    {msg.role === "user" ? "你" : msg.role}
                  </div>
                  <div className="message-content">{msg.content}</div>
                  <div className="ms-actions">
                    {hasNextMilestone ? (
                      <button className="ms-btn-continue" onClick={onContinueNextMilestone}>▶ 继续下一大阶段</button>
                    ) : (
                      <>
                        <button className="ms-btn-continue ms-btn-disabled" disabled>所有大阶段已完成</button>
                        <p className="ms-completed-hint">✅ 所有大阶段已执行完成，项目已全部交付。</p>
                      </>
                    )}
                    <button className="ms-btn-discuss" onClick={onDiscussWithPM}>💬 与产品经理讨论</button>
                    <button className="ms-btn-report" onClick={onViewDetailedReport}>📊 查看详细报告</button>
                  </div>
                </div>
              );
            }
            // 普通消息：保持现有渲染逻辑
            const isLastAiMsg =
              discussionBranchType != null &&
              msg.role !== 'user' &&
              msg.role !== 'system' &&
              msg.msgType !== 'version_plan' &&
              msg.msgType !== 'milestone_summary' &&
              index === messages.length - 1;

            return (
              <div key={msg.id} className={`message message-${msg.role === "user" ? "user" : "ai"}`}>
                <div className="message-role">
                  {msg.role === "user" ? "你" : msg.role}
                </div>
                <div className="message-content">{msg.content}</div>
                {isLastAiMsg && onConfirmPMSuggestion && (
                  <button
                    className="btn-confirm-suggestion"
                    onClick={onConfirmPMSuggestion}
                  >
                    {discussionBranchType === 'rollback'
                      ? '🔄 按照建议回退并继续'
                      : '🔄 按照新方案生成后续大阶段'}
                  </button>
                )}
              </div>
            );
          })
        )}
        {/* 如果正在等待 AI 回复，显示 "AI 正在输入..." 的提示 */}
        {isLoading && <p className="loading-tip">AI 正在输入...</p>}
      </div>
      {/* 模式选择器：仅在未锁定时可切换 */}
      <div className="mode-selector">
        <span className="mode-label">项目模式：</span>
        <button
          className={`mode-btn ${mode === "Quick" ? "mode-active" : ""}`}
          onClick={() => onModeChange("Quick")}
          disabled={modeLocked}
        >
          快速
        </button>
        <button
          className={`mode-btn ${mode === "Professional" ? "mode-active" : ""}`}
          onClick={() => onModeChange("Professional")}
          disabled={modeLocked}
        >
          专业
        </button>
        {modeLocked && <span className="mode-locked-hint">🔒 项目已开始，项目已锁定</span>}
      </div>
      {/* 底部输入区域：文本输入框和发送按钮 */}
      <footer className="input-area">
        <input
          className="chat-input"
          type="text"
          placeholder="输入你的想法..."
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSend();
          }}
        />
        <button className="send-button" onClick={handleSend}>
          发送
        </button>
      </footer>
    </div>
  );
}
export default ChatRoom;
