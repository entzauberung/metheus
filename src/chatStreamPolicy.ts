import type { ChatMessage, ChatStreamEvent } from "./types";

export const CHAT_STREAM_FLUSH_INTERVAL_MS = 40;

export type ChatStreamStatus = "starting" | "streaming" | "stopping" | "completed" | "cancelled" | "failed";

export interface ChatStreamSession {
  requestId: string;
  threadId: string;
  role: string;
  status: ChatStreamStatus;
  optimisticUser?: ChatMessage;
  optimisticReply?: ChatMessage;
  originalUserMessageId?: string;
  error?: string;
  retryable: boolean;
  requestPending: boolean;
}

export function createChatStreamSession(input: {
  requestId: string;
  threadId: string;
  role: string;
  content?: string;
  originalUserMessageId?: string;
  timestamp: number;
}): ChatStreamSession {
  return {
    requestId: input.requestId,
    threadId: input.threadId,
    role: input.role,
    status: "starting",
    optimisticUser: input.content === undefined ? undefined : {
      id: `pending-user-${input.requestId}`,
      role: "user",
      content: input.content,
      timestamp: input.timestamp,
    },
    optimisticReply: {
      id: `pending-reply-${input.requestId}`,
      role: input.role,
      content: "",
      timestamp: input.timestamp,
      reply_to_message_id: input.originalUserMessageId,
    },
    originalUserMessageId: input.originalUserMessageId,
    retryable: false,
    requestPending: true,
  };
}

export function eventBelongsToSession(session: ChatStreamSession, event: ChatStreamEvent): boolean {
  return event.request_id === session.requestId && event.thread_id === session.threadId;
}

export function applyChatStreamEvent(
  session: ChatStreamSession,
  event: ChatStreamEvent,
): ChatStreamSession {
  if (!eventBelongsToSession(session, event)) return session;

  switch (event.event) {
    case "started":
      return { ...session, status: "streaming" };
    case "user_saved":
      return {
        ...session,
        optimisticUser: session.optimisticUser ? event.message : session.optimisticUser,
        originalUserMessageId: event.message.id,
      };
    case "reply_started":
      return {
        ...session,
        status: "streaming",
        optimisticReply: {
          id: event.message_id,
          role: event.role,
          content: session.optimisticReply?.content ?? "",
          timestamp: event.timestamp,
          reply_to_message_id: session.optimisticUser?.id ?? session.originalUserMessageId,
        },
      };
    case "delta":
      if (!event.text || !session.optimisticReply) return session;
      return {
        ...session,
        optimisticReply: {
          ...session.optimisticReply,
          content: session.optimisticReply.content + event.text,
        },
      };
    case "completed":
      return { ...session, status: "completed", retryable: false };
    case "cancelled":
      return { ...session, status: "cancelled", retryable: true };
    case "failed":
      return {
        ...session,
        status: "failed",
        error: event.error,
        retryable: event.retryable,
      };
  }
}

export function markChatStreamStopping(session: ChatStreamSession): ChatStreamSession {
  if (session.status !== "starting" && session.status !== "streaming") return session;
  return { ...session, status: "stopping" };
}

export function isChatStreamActive(session: ChatStreamSession | null): boolean {
  return session?.requestPending === true;
}

export function mergeChatMessages(
  persisted: ChatMessage[],
  session: ChatStreamSession | null,
): ChatMessage[] {
  if (!session) return persisted;
  const persistedIds = new Set(persisted.map((message) => message.id));
  const transient = [session.optimisticUser, session.optimisticReply]
    .filter((message): message is ChatMessage => Boolean(message))
    .filter((message) => !persistedIds.has(message.id));
  return [...persisted, ...transient];
}
