import { describe, expect, it } from "vitest";
import {
  applyChatStreamEvent,
  createChatStreamSession,
  isChatStreamActive,
  mergeChatMessages,
} from "./chatStreamPolicy";

describe("chat stream merge policy", () => {
  it("ignores events from stale requests and other threads", () => {
    const session = createChatStreamSession({
      requestId: "request-current",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
      timestamp: 1,
    });
    const stale = applyChatStreamEvent(session, {
      event: "delta",
      request_id: "request-old",
      thread_id: "thread-a",
      role: "产品经理",
      text: "wrong",
    });
    const otherThread = applyChatStreamEvent(session, {
      event: "delta",
      request_id: "request-current",
      thread_id: "thread-b",
      role: "产品经理",
      text: "wrong",
    });

    expect(stale).toBe(session);
    expect(otherThread).toBe(session);
  });

  it("appends deltas in order and replaces temporary ids with persisted ids", () => {
    let session = createChatStreamSession({
      requestId: "request-1",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
      timestamp: 1,
    });
    session = applyChatStreamEvent(session, {
      event: "user_saved",
      request_id: "request-1",
      thread_id: "thread-a",
      role: "产品经理",
      message: { id: "user-1", role: "user", content: "hello", timestamp: 2 },
    });
    session = applyChatStreamEvent(session, {
      event: "reply_started",
      request_id: "request-1",
      thread_id: "thread-a",
      role: "产品经理",
      message_id: "reply-1",
      timestamp: 3,
    });
    for (const text of ["你", "好"] as const) {
      session = applyChatStreamEvent(session, {
        event: "delta",
        request_id: "request-1",
        thread_id: "thread-a",
        role: "产品经理",
        text,
      });
    }

    expect(session.optimisticUser?.id).toBe("user-1");
    expect(session.originalUserMessageId).toBe("user-1");
    expect(session.optimisticReply).toMatchObject({ id: "reply-1", content: "你好" });
  });

  it("does not duplicate messages already present in the final project", () => {
    const persisted = [{ id: "user-1", role: "user", content: "hello", timestamp: 2 }];
    let session = createChatStreamSession({
      requestId: "request-1",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
      timestamp: 1,
    });
    session = applyChatStreamEvent(session, {
      event: "user_saved",
      request_id: "request-1",
      thread_id: "thread-a",
      role: "产品经理",
      message: persisted[0],
    });

    expect(mergeChatMessages(persisted, session).filter((message) => message.id === "user-1"))
      .toHaveLength(1);
  });

  it("keeps submission locked until the final project promise settles", () => {
    const session = createChatStreamSession({
      requestId: "request-1",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
      timestamp: 1,
    });
    const completed = applyChatStreamEvent(session, {
      event: "completed",
      request_id: "request-1",
      thread_id: "thread-a",
      role: "产品经理",
      message_id: "reply-1",
    });

    expect(completed.status).toBe("completed");
    expect(isChatStreamActive(completed)).toBe(true);
  });
});
