// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, RotateCcw, Send, Square } from "lucide-react";
import { ChatStreamController } from "./chatStreamController";
import {
  isChatStreamActive,
  mergeChatMessages,
  type ChatStreamSession,
} from "./chatStreamPolicy";
import { isNearChatBottom, nextUnreadState } from "./chatScrollPolicy";
import {
  canSubmitChatMessage,
  CHAT_COMPOSER_MAX_HEIGHT_PX,
  CHAT_COMPOSER_MIN_HEIGHT_PX,
  CHAT_MESSAGE_MAX_CHARS,
  clampComposerHeight,
  shouldSendFromComposer,
} from "./chatComposerPolicy";
import type { ChatMessage, Project } from "./types";
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
function ChatRoomSession({ messages, projectName, currentRole, threadId, onViewDetailedReport, onProjectUpdated, hideInput, hideInputReason }: Props) {
  const [inputValue, setInputValue] = useState("");
  const [streamSession, setStreamSession] = useState<ChatStreamSession | null>(null);
  const controllerRef = useRef<ChatStreamController | null>(null);
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const followLatestRef = useRef(true);
  const lastContentSizeRef = useRef(0);
  const [hasUnread, setHasUnread] = useState(false);
  const projectUpdatedRef = useRef(onProjectUpdated);
  projectUpdatedRef.current = onProjectUpdated;

  useEffect(() => {
    const controller = new ChatStreamController({
      onState: setStreamSession,
      onProject: (project) => projectUpdatedRef.current?.(project),
    });
    controllerRef.current = controller;
    return () => {
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
  }, [projectName, threadId]);

  const isLoading = isChatStreamActive(streamSession);
  const displayedMessages = useMemo(
    () => mergeChatMessages(messages, streamSession),
    [messages, streamSession],
  );
  const displayedContentSize = useMemo(
    () => displayedMessages.reduce((total, message) => total + message.content.length + 1, 0),
    [displayedMessages],
  );
  const canSend = canSubmitChatMessage(inputValue, isLoading);

  const resizeComposer = useCallback(() => {
    const composer = composerRef.current;
    if (!composer) return;
    composer.style.height = `${CHAT_COMPOSER_MIN_HEIGHT_PX}px`;
    const height = clampComposerHeight(composer.scrollHeight);
    composer.style.height = `${height}px`;
    composer.style.overflowY = composer.scrollHeight > CHAT_COMPOSER_MAX_HEIGHT_PX ? "auto" : "hidden";
  }, []);

  useLayoutEffect(() => {
    resizeComposer();
  }, [inputValue, resizeComposer]);

  const scrollToLatest = useCallback((behavior: ScrollBehavior = "smooth") => {
    const container = messagesRef.current;
    if (!container) return;
    followLatestRef.current = true;
    setHasUnread(false);
    container.scrollTo({ top: container.scrollHeight, behavior });
  }, []);

  useLayoutEffect(() => {
    const addedContent = displayedContentSize > lastContentSizeRef.current;
    lastContentSizeRef.current = displayedContentSize;
    if (!addedContent) return;
    if (followLatestRef.current) {
      scrollToLatest("auto");
    } else {
      setHasUnread((current) => nextUnreadState(current, false));
    }
  }, [displayedContentSize, scrollToLatest]);

  useEffect(() => {
    const container = messagesRef.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (followLatestRef.current) scrollToLatest("auto");
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [scrollToLatest]);

  const handleSend = useCallback(() => {
    if (!canSend || !controllerRef.current) return;
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
    if (!actualMessage.trim()) return;
    scrollToLatest("auto");
    setInputValue("");
    void controllerRef.current.start({
      projectName: projectName || "default",
      threadId,
      role: targetRole,
      content: actualMessage,
    });
  }, [canSend, currentRole, inputValue, projectName, scrollToLatest, threadId]);

  const handleRetry = useCallback((message: ChatMessage) => {
    if (isLoading || !controllerRef.current || !message.reply_to_message_id) return;
    scrollToLatest("auto");
    void controllerRef.current.start({
      projectName: projectName || "default",
      threadId,
      role: message.role,
      originalUserMessageId: message.reply_to_message_id,
    });
  }, [isLoading, projectName, scrollToLatest, threadId]);

  const handleStop = useCallback(() => {
    void controllerRef.current?.stop();
  }, []);
  // 计算最新的版本方案消息时间戳，用于判定旧方案是否过期
  const latestVpTimestamp = useMemo(() => {
    const vpMessages = messages.filter(m => m.msg_type === "version_plan");
    if (vpMessages.length === 0) return 0;
    return vpMessages.reduce((max, m) => Math.max(max, m.timestamp), 0);
  }, [messages]);

  return (
    <div className="chat-room">
      <div
        className="chat-messages"
        ref={messagesRef}
        onScroll={(event) => {
          const isFollowing = isNearChatBottom(event.currentTarget);
          followLatestRef.current = isFollowing;
          if (isFollowing) setHasUnread(false);
        }}
      >
        {/*
                如果消息数组为空，显示空提示语；
                否则，用 .map() 遍历每一条消息，生成对应的 DOM 元素。
                */}
        {displayedMessages.length === 0 ? (
          <p className="empty-tip">开始讨论你的想法吧</p>
        ) : (
          displayedMessages.map((msg) => {
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
            const isCancelled = msg.msg_type === "ai_cancelled";
            const isInterrupted = msg.msg_type === "ai_interrupted" || msg.msg_type === "ai_failure";
            return (
              <div
                key={msg.id}
                className={`message message-${msg.role === "user" ? "user" : "ai"}${isCancelled || isInterrupted ? " message-terminal" : ""}`}
              >
                <div className="message-role">
                  {msg.role === "user" ? "你" : msg.role}
                </div>
                <div className="message-content">{msg.content}</div>
                {(isCancelled || isInterrupted) && (
                  <div className="message-status-row" role="status">
                    <span className={`message-status ${isCancelled ? "is-cancelled" : "is-interrupted"}`}>
                      {isCancelled ? "已停止" : "回复中断"}
                    </span>
                    {msg.reply_to_message_id && (
                      <button
                        type="button"
                        className="message-retry-button"
                        onClick={() => handleRetry(msg)}
                        disabled={isLoading}
                      >
                        <RotateCcw size={14} aria-hidden="true" />
                        重新生成
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })
        )}
        {streamSession?.optimisticReply?.content === "" && isLoading && (
          <p className="loading-tip" aria-live="polite">AI 正在输入...</p>
        )}
        <div className="chat-live-region" aria-live="polite" aria-atomic="true">
          {streamSession?.status === "starting" && "正在连接模型"}
          {streamSession?.status === "streaming" && "正在生成回复"}
          {streamSession?.status === "stopping" && "正在停止生成"}
          {streamSession?.status === "cancelled" && "回复已停止"}
        </div>
        {streamSession?.status === "failed" && (
          <div className="chat-stream-error" role="status">
            <span>回复失败：{streamSession.error ?? "未知错误"}</span>
            {streamSession.retryable && streamSession.originalUserMessageId && (
              <button
                type="button"
                className="message-retry-button"
                onClick={() => handleRetry({
                  id: streamSession.optimisticReply?.id ?? streamSession.requestId,
                  role: streamSession.role,
                  content: streamSession.optimisticReply?.content ?? "",
                  timestamp: Date.now(),
                  reply_to_message_id: streamSession.originalUserMessageId,
                })}
              >
                <RotateCcw size={14} aria-hidden="true" />
                重新生成
              </button>
            )}
          </div>
        )}
        {hasUnread && (
          <button
            type="button"
            className="chat-latest-button"
            onClick={() => scrollToLatest()}
            aria-label="回到最新消息，有新内容"
            title="回到最新消息"
          >
            <ArrowDown size={17} aria-hidden="true" />
          </button>
        )}
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
          <textarea
            ref={composerRef}
            className="chat-input"
            placeholder="输入你的想法..."
            value={inputValue}
            rows={1}
            maxLength={CHAT_MESSAGE_MAX_CHARS}
            onChange={(e) => setInputValue(e.target.value)}
            onKeyDown={(e) => {
              if (shouldSendFromComposer({
                key: e.key,
                shiftKey: e.shiftKey,
                isComposing: e.nativeEvent.isComposing,
              })) {
                e.preventDefault();
                handleSend();
              }
            }}
            aria-label="聊天消息"
          />
          {isLoading ? (
            <button
              type="button"
              className="send-button stop-button"
              onClick={handleStop}
              disabled={streamSession?.status === "stopping"}
              aria-label="停止生成"
              title="停止生成"
            >
              <Square size={16} fill="currentColor" aria-hidden="true" />
            </button>
          ) : (
            <button
              type="button"
              className="send-button"
              onClick={handleSend}
              disabled={!canSend}
              aria-label="发送消息"
              title="发送"
            >
              <Send size={17} aria-hidden="true" />
            </button>
          )}
        </footer>
      )}
    </div>
  );
}

function ChatRoom(props: Props) {
  const scopeKey = `${props.projectName || "default"}\0${props.threadId}`;
  return <ChatRoomSession key={scopeKey} {...props} />;
}

export default ChatRoom;
