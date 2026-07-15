// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useMemo } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout"
import { ChatMessage, Project } from "./types"
interface Props {
  messages: ChatMessage[];
  onAddMessage: (msg: ChatMessage) => void;
  projectName?: string;
  currentRole: string;
  threadId: string;
  onViewDetailedReport?: () => void;
  // === V1：项目状态更新回调（替代乐观插入） ===
  onProjectUpdated?: (project: Project) => void;
  // === V1：方案已批准时隐藏聊天输入 ===
  hideInput?: boolean;
  hideInputReason?: string;
}
function ChatRoom({ messages, onAddMessage, projectName, currentRole, threadId, onViewDetailedReport, onProjectUpdated, hideInput, hideInputReason }: Props) {
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
    // 不再乐观插入用户消息 — 后端 chat_with_role 负责持久化所有消息
    // 前端通过 onProjectUpdated 接收后端返回的完整 Project（含已持久化的消息列表）
    // 即使 AI 失败，后端也返回含用户消息+失败提示的完整 Project
    setInputValue("");
    setIsLoading(true);
    try {
      const updatedProject = await invokeWithTimeout<Project>("chat_with_role", {
        projectName: projectName || "default",
        message: actualMessage,
        role: targetRole,
        threadId: threadId,
      });
      // 将完整 Project 传回 App 以替换本地状态（用户消息已持久化，AI 回复或失败提示也在其中）
      if (onProjectUpdated && updatedProject) {
        onProjectUpdated(updatedProject);
      }
    } catch (error) {
      // 仅在后端完全无法保存用户消息时才走到这里（网络断开等极端情况）
      const errorMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "system",
        content: `❌ 消息发送失败：${error}`,
        timestamp: Date.now(),
      };
      onAddMessage(errorMessage);
    } finally {
      setIsLoading(false);
    }
  };
  // 计算最新的版本方案消息时间戳，用于判定旧方案是否过期
  const latestVpTimestamp = useMemo(() => {
    const vpMessages = messages.filter(m => m.msg_type === "version_plan");
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
          messages.map((msg) => {
            // 版本方案消息：特殊渲染
            if (msg.msg_type === "version_plan") {
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
                      <span className="vp-status" style={{ background: "#fff8c5", color: "#664d03" }}>
                        📝 历史草稿（请在方案审批页面操作）
                      </span>
                    )}
                  </div>
                </div>
              );
            }
            // 质检失败消息：红色边框特殊渲染
            if (msg.msg_type === "qa_failed") {
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
            if (msg.msg_type === "milestone_summary") {
              return (
                <div key={msg.id} className="message message-ai message-milestone-summary">
                  <div className="message-role">
                    {msg.role === "user" ? "你" : msg.role}
                  </div>
                  <div className="message-content">{msg.content}</div>
                  <div className="ms-actions">
                    <button className="ms-btn-report" onClick={onViewDetailedReport}>📊 查看详细报告</button>
                  </div>
                </div>
              );
            }
            // 普通消息：保持现有渲染逻辑
            return (
              <div key={msg.id} className={`message message-${msg.role === "user" ? "user" : "ai"}`}>
                <div className="message-role">
                  {msg.role === "user" ? "你" : msg.role}
                </div>
                <div className="message-content">{msg.content}</div>
              </div>
            );
          })
        )}
        {/* 如果正在等待 AI 回复，显示 "AI 正在输入..." 的提示 */}
        {isLoading && <p className="loading-tip">AI 正在输入...</p>}
      </div>
      {/* 底部输入区域：方案已批准时隐藏 */}
      {hideInput ? (
        <footer className="input-area" style={{
          padding: "12px 16px",
          textAlign: "center",
          color: "#656d76",
          fontSize: "13px",
          background: "#f6f8fa",
          borderTop: "1px solid #d0d7de",
        }}>
          <p style={{ margin: 0 }}>
            {hideInputReason || "方案已批准，聊天输入已锁定。请使用方案审批页面的操作按钮。"}
          </p>
        </footer>
      ) : (
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
      )}
    </div>
  );
}
export default ChatRoom;
