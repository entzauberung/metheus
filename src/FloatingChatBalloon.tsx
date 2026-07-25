// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, MessageCircle, X } from "lucide-react";
import { isNearChatBottom, nextUnreadState } from "./chatScrollPolicy";
import { ChatMessage } from "./types";

interface Props {
  messages: ChatMessage[];
}

function FloatingChatBalloon({ messages }: Props) {
  const [isOpen, setIsOpen] = useState(false);
  const [hasUnread, setHasUnread] = useState(false);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const followLatestRef = useRef(true);
  const contentSize = useMemo(
    () => messages.reduce((total, message) => total + message.content.length + 1, 0),
    [messages],
  );
  const previousContentSizeRef = useRef(contentSize);

  const scrollToLatest = useCallback((behavior: ScrollBehavior = "smooth") => {
    const container = contentRef.current;
    if (!container) return;
    followLatestRef.current = true;
    setHasUnread(false);
    container.scrollTo({ top: container.scrollHeight, behavior });
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    requestAnimationFrame(() => scrollToLatest("auto"));
  }, [isOpen, scrollToLatest]);

  useEffect(() => {
    const addedContent = contentSize > previousContentSizeRef.current;
    previousContentSizeRef.current = contentSize;
    if (!isOpen || !addedContent) return;
    if (followLatestRef.current) scrollToLatest("auto");
    else setHasUnread((current) => nextUnreadState(current, false));
  }, [contentSize, isOpen, scrollToLatest]);

  return (
    <div className="floating-balloon-wrapper">
      {/* 悬浮球 */}
      <button
        type="button"
        className="floating-balloon"
        onClick={() => setIsOpen(!isOpen)}
        title={isOpen ? "关闭聊天记录" : "查看阶段一讨论记录"}
        aria-label={isOpen ? "关闭聊天记录" : "查看阶段一讨论记录"}
      >
        <MessageCircle size={22} aria-hidden="true" />
      </button>

      {/* 聊天记录浮窗 */}
      {isOpen && (
        <>
          {/* 背景遮罩 */}
          <div className="floating-overlay" onClick={() => setIsOpen(false)} />

          {/* 浮窗 */}
          <div className="floating-balloon-window">
            <div className="floating-window-header">
              <span>阶段一讨论记录</span>
              <button
                className="floating-window-close"
                onClick={() => setIsOpen(false)}
                aria-label="关闭聊天记录"
                title="关闭"
              >
                <X size={17} aria-hidden="true" />
              </button>
            </div>

            <div
              className="floating-window-content"
              ref={contentRef}
              onScroll={(event) => {
                const isFollowing = isNearChatBottom(event.currentTarget);
                followLatestRef.current = isFollowing;
                if (isFollowing) setHasUnread(false);
              }}
            >
              {messages.length === 0 ? (
                <div className="floating-empty">暂无讨论记录</div>
              ) : (
                messages.map((msg) => (
                  <div
                    key={msg.id}
                    className={`floating-message ${
                      msg.role === "user" ? "msg-user" : "msg-ai"
                    }`}
                  >
                    <div className="floating-message-role">{msg.role}</div>
                    <div className="floating-message-content">{msg.content}</div>
                  </div>
                ))
              )}
              {hasUnread && (
                <button
                  type="button"
                  className="floating-latest-button"
                  onClick={() => scrollToLatest()}
                  aria-label="回到最新消息，有新内容"
                  title="回到最新消息"
                >
                  <ArrowDown size={16} aria-hidden="true" />
                </button>
              )}
            </div>

            <div className="floating-window-footer">
              执行期间聊天暂停 · 只读模式
            </div>
          </div>
        </>
      )}
    </div>
  );
}

export default FloatingChatBalloon;
