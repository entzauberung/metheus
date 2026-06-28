// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState } from "react";
import { ChatMessage } from "./types";

interface Props {
  messages: ChatMessage[];
}

function FloatingChatBalloon({ messages }: Props) {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <div className="floating-balloon-wrapper">
      {/* 悬浮球 */}
      <div
        className="floating-balloon"
        onClick={() => setIsOpen(!isOpen)}
        title={isOpen ? "关闭聊天记录" : "查看阶段一讨论记录"}
      >
        💬
      </div>

      {/* 聊天记录浮窗 */}
      {isOpen && (
        <>
          {/* 背景遮罩 */}
          <div className="floating-overlay" onClick={() => setIsOpen(false)} />

          {/* 浮窗 */}
          <div className="floating-balloon-window">
            <div className="floating-window-header">
              <span>💬 阶段一讨论记录</span>
              <button
                className="floating-window-close"
                onClick={() => setIsOpen(false)}
              >
                ✕
              </button>
            </div>

            <div className="floating-window-content">
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
