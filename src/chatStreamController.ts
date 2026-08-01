import { Channel, invoke } from "@tauri-apps/api/core";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import type { ChatStreamEvent, Project, RuntimeMutationResult } from "./types";
import {
  applyChatStreamEvent,
  CHAT_STREAM_FLUSH_INTERVAL_MS,
  createChatStreamSession,
  isChatStreamActive,
  markChatStreamStopping,
  type ChatStreamSession,
} from "./chatStreamPolicy";

interface ControllerCallbacks {
  onState: (state: ChatStreamSession | null) => void;
  onProject: (project: Project) => void;
  onRuntimeMutation?: (result: RuntimeMutationResult) => void;
}

interface StartRequest {
  projectName: string;
  threadId: string;
  role: string;
  content?: string;
  originalUserMessageId?: string;
}

interface ChatEventChannel {
  onmessage: (event: ChatStreamEvent) => void;
}

export interface ChatStreamTransport {
  createEventChannel: (onmessage: (event: ChatStreamEvent) => void) => ChatEventChannel;
  invokeStream: (command: string, args: Record<string, unknown>) => Promise<RuntimeMutationResult | Project>;
  invokeBounded: <T>(command: string, args: Record<string, unknown>) => Promise<T>;
}

const defaultTransport: ChatStreamTransport = {
  createEventChannel(onmessage) {
    const channel = new Channel<ChatStreamEvent>();
    channel.onmessage = onmessage;
    return channel;
  },
  invokeStream(command, args) {
    return invoke<RuntimeMutationResult>(command, args);
  },
  invokeBounded(command, args) {
    return invokeWithTimeout(command, args);
  },
};

export class ChatStreamController {
  private callbacks: ControllerCallbacks;
  private session: ChatStreamSession | null = null;
  private pendingDelta = "";
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
  private readonly transport: ChatStreamTransport;

  constructor(callbacks: ControllerCallbacks, transport: ChatStreamTransport = defaultTransport) {
    this.callbacks = callbacks;
    this.transport = transport;
  }

  updateCallbacks(callbacks: ControllerCallbacks): void {
    this.callbacks = callbacks;
  }

  async start(request: StartRequest): Promise<void> {
    if (this.disposed || isChatStreamActive(this.session)) return;
    const requestId = crypto.randomUUID();
    this.session = createChatStreamSession({
      requestId,
      threadId: request.threadId,
      role: request.role,
      content: request.content,
      originalUserMessageId: request.originalUserMessageId,
      timestamp: Date.now(),
    });
    this.emitState();

    const channel = this.transport.createEventChannel((event) => this.receive(event));
    const command = request.originalUserMessageId
      ? "regenerate_chat_reply_stream_runtime"
      : "chat_with_role_stream_runtime";
    const args: Record<string, unknown> = {
      projectName: request.projectName,
      role: request.role,
      threadId: request.threadId,
      requestId,
      onEvent: channel,
    };
    if (request.originalUserMessageId) {
      args.userMessageId = request.originalUserMessageId;
    } else {
      args.message = request.content ?? "";
    }

    try {
      const result = await this.transport.invokeStream(command, args);
      if (this.session?.requestId !== requestId) return;
      this.flushDeltas();
      if ("runtime_snapshot" in result) {
        if (this.callbacks.onRuntimeMutation) this.callbacks.onRuntimeMutation(result);
        else this.callbacks.onProject(result.runtime_snapshot.project);
      } else {
        this.callbacks.onProject(result);
      }
      if (this.disposed) return;
      this.session = this.session.status === "failed"
        ? {
          ...this.session,
          optimisticUser: undefined,
          optimisticReply: undefined,
          requestPending: false,
        }
        : null;
      this.emitState();
    } catch (error) {
      if (this.session?.requestId !== requestId) return;
      this.flushDeltas();
      const reconciliationError = await this.reconcileProject(request.projectName);
      if (this.disposed) return;
      if (this.session.status !== "cancelled" && this.session.status !== "failed") {
        this.session = {
          ...this.session,
          status: "failed",
          error: reconciliationError
            ? `${String(error)}；同步项目失败：${reconciliationError}`
            : String(error),
          retryable: false,
          requestPending: false,
        };
      } else {
        this.session = { ...this.session, requestPending: false };
      }
      if (!reconciliationError) {
        this.session = {
          ...this.session,
          optimisticUser: undefined,
          optimisticReply: undefined,
        };
      }
      this.emitState();
    }
  }

  async stop(): Promise<void> {
    const current = this.session;
    if (!current || !isChatStreamActive(current)) return;
    this.session = markChatStreamStopping(current);
    this.emitState();
    try {
      await this.transport.invokeBounded<boolean>("cancel_chat_stream", {
        requestId: current.requestId,
        threadId: current.threadId,
      });
    } catch (error) {
      if (this.session?.requestId !== current.requestId) return;
      this.session = {
        ...this.session,
        status: "failed",
        error: `停止生成失败：${String(error)}`,
        retryable: false,
        requestPending: true,
      };
      this.emitState();
    }
  }

  dispose(): void {
    if (this.disposed) return;
    const current = this.session;
    this.disposed = true;
    this.clearFlushTimer();
    if (current && isChatStreamActive(current)) {
      void this.transport.invokeBounded<boolean>("cancel_chat_stream", {
        requestId: current.requestId,
        threadId: current.threadId,
      }).catch(() => undefined);
    }
  }

  private receive(event: ChatStreamEvent): void {
    if (this.disposed || !this.session) return;
    if (event.event === "delta") {
      if (event.request_id !== this.session.requestId || event.thread_id !== this.session.threadId) return;
      this.pendingDelta += event.text;
      if (!this.flushTimer) {
        this.flushTimer = setTimeout(() => this.flushDeltas(), CHAT_STREAM_FLUSH_INTERVAL_MS);
      }
      return;
    }

    this.flushDeltas();
    this.session = applyChatStreamEvent(this.session, event);
    this.emitState();
  }

  private async reconcileProject(projectName: string): Promise<string | null> {
    try {
      const project = await this.transport.invokeBounded<Project>("get_project", { projectName });
      this.callbacks.onProject(project);
      return null;
    } catch (error) {
      return String(error);
    }
  }

  private flushDeltas(): void {
    this.clearFlushTimer();
    if (!this.pendingDelta || !this.session) return;
    const text = this.pendingDelta;
    this.pendingDelta = "";
    this.session = applyChatStreamEvent(this.session, {
      event: "delta",
      request_id: this.session.requestId,
      thread_id: this.session.threadId,
      role: this.session.role,
      text,
    });
    this.emitState();
  }

  private clearFlushTimer(): void {
    if (this.flushTimer) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
  }

  private emitState(): void {
    if (!this.disposed) this.callbacks.onState(this.session);
  }
}
