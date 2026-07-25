/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatStreamSession } from "./chatStreamPolicy";
import type { Project } from "./types";

const controllerHarness = vi.hoisted(() => ({
  instances: [] as Array<{
    callbacks: {
      onState: (state: ChatStreamSession | null) => void;
      onProject: (project: Project) => void;
    };
    disposed: boolean;
  }>,
}));

vi.mock("./chatStreamController", () => ({
  ChatStreamController: class {
    callbacks: {
      onState: (state: ChatStreamSession | null) => void;
      onProject: (project: Project) => void;
    };
    disposed = false;

    constructor(callbacks: {
      onState: (state: ChatStreamSession | null) => void;
      onProject: (project: Project) => void;
    }) {
      this.callbacks = callbacks;
      controllerHarness.instances.push(this);
    }

    dispose() {
      this.disposed = true;
    }

    start() {
      return Promise.resolve();
    }

    stop() {
      return Promise.resolve();
    }
  },
}));

import ChatRoom from "./ChatRoom";

function streamSession(content: string): ChatStreamSession {
  return {
    requestId: "request-a",
    threadId: "thread-a",
    role: "产品经理",
    status: "streaming",
    optimisticReply: {
      id: "reply-a",
      role: "产品经理",
      content,
      timestamp: 2,
    },
    retryable: false,
    requestPending: true,
  };
}

function failedStreamSession(error: string): ChatStreamSession {
  return {
    requestId: "request-failed",
    threadId: "thread-a",
    role: "产品经理",
    status: "failed",
    error,
    retryable: false,
    requestPending: false,
  };
}

describe("ChatRoom scope lifecycle", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    controllerHarness.instances.length = 0;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      value: vi.fn(),
    });
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.restoreAllMocks();
  });

  it("resets draft, stream, follow state, and unread state when the scope changes", () => {
    act(() => {
      root.render(
        <ChatRoom
          messages={[{ id: "user-a", role: "user", content: "old scope", timestamp: 1 }]}
          onAddMessage={() => undefined}
          projectName="project-a"
          currentRole="产品经理"
          threadId="thread-a"
        />,
      );
    });
    const oldController = controllerHarness.instances[0];
    const textarea = host.querySelector("textarea") as HTMLTextAreaElement;
    act(() => {
      textarea.value = "draft for old scope";
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("draft for old scope");

    const messages = host.querySelector(".chat-messages") as HTMLDivElement;
    Object.defineProperties(messages, {
      scrollTop: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, value: 1_000 },
      clientHeight: { configurable: true, value: 400 },
    });
    act(() => messages.dispatchEvent(new Event("scroll", { bubbles: true })));
    act(() => oldController.callbacks.onState(streamSession("first delta")));
    act(() => oldController.callbacks.onState(streamSession("first delta and more")));

    expect(host.querySelectorAll('[aria-label="回到最新消息，有新内容"]')).toHaveLength(1);
    expect(host.textContent).toContain("first delta and more");

    act(() => {
      root.render(
        <ChatRoom
          messages={[{ id: "user-b", role: "user", content: "new scope", timestamp: 3 }]}
          onAddMessage={() => undefined}
          projectName="project-b"
          currentRole="产品经理"
          threadId="thread-b"
        />,
      );
    });

    expect(oldController.disposed).toBe(true);
    expect(controllerHarness.instances).toHaveLength(2);
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("");
    expect(host.textContent).toContain("new scope");
    expect(host.textContent).not.toContain("first delta");
    expect(host.querySelector('[aria-label="回到最新消息，有新内容"]')).toBeNull();

    act(() => oldController.callbacks.onState(streamSession("stale old event")));
    expect(host.textContent).not.toContain("stale old event");
  });

  it("renders and announces a stream failure only once", () => {
    act(() => {
      root.render(
        <ChatRoom
          messages={[]}
          onAddMessage={() => undefined}
          projectName="project-a"
          currentRole="产品经理"
          threadId="thread-a"
        />,
      );
    });
    act(() => {
      controllerHarness.instances[0].callbacks.onState(
        failedStreamSession("模型完成了推理，但未返回最终答案"),
      );
    });

    const failureText = "回复失败：模型完成了推理，但未返回最终答案";
    expect(host.querySelector(".chat-stream-error")?.textContent).toBe(failureText);
    expect(host.querySelector(".chat-live-region")?.textContent).not.toContain("回复失败");
    expect(host.textContent?.split(failureText)).toHaveLength(2);
  });
});
