import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ChatStreamController,
  type ChatStreamTransport,
} from "./chatStreamController";
import type { ChatStreamEvent, Project } from "./types";
import type { ChatStreamSession } from "./chatStreamPolicy";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function project(name: string): Project {
  return { name } as Project;
}

function createHarness(
  stream: Deferred<Project>,
  reconciled = project("project"),
  boundedFailure?: { command: string; error: Error },
) {
  const channels: Array<{ onmessage: (event: ChatStreamEvent) => void }> = [];
  const streamCalls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const boundedCalls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const transport: ChatStreamTransport = {
    createEventChannel(onmessage) {
      const channel = { onmessage };
      channels.push(channel);
      return channel;
    },
    invokeStream(command, args) {
      streamCalls.push({ command, args });
      return stream.promise;
    },
    async invokeBounded<T>(command: string, args: Record<string, unknown>): Promise<T> {
      boundedCalls.push({ command, args });
      if (boundedFailure?.command === command) throw boundedFailure.error;
      return (command === "get_project" ? reconciled : true) as T;
    },
  };
  return { transport, channels, streamCalls, boundedCalls };
}

function createController(transport: ChatStreamTransport) {
  const states: Array<ChatStreamSession | null> = [];
  const projects: Project[] = [];
  const controller = new ChatStreamController({
    onState: (state) => states.push(state),
    onProject: (value) => projects.push(value),
  }, transport);
  return { controller, states, projects };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("ChatStreamController", () => {
  it("does not fail or cancel a stream after the former 150 second deadline", async () => {
    vi.useFakeTimers();
    const stream = deferred<Project>();
    const harness = createHarness(stream);
    const { controller, states, projects } = createController(harness.transport);

    const running = controller.start({
      projectName: "project",
      threadId: "thread-a",
      role: "产品经理",
      content: "long answer",
    });
    await vi.advanceTimersByTimeAsync(151_000);

    expect(states[states.length - 1]?.requestPending).toBe(true);
    expect(harness.boundedCalls).toHaveLength(0);

    const finalProject = project("project");
    stream.resolve(finalProject);
    await running;
    expect(projects).toEqual([finalProject]);
    expect(states[states.length - 1]).toBeNull();
  });

  it("reconciles the persisted project and clears optimistic messages after failure", async () => {
    const stream = deferred<Project>();
    const reconciled = project("project");
    const harness = createHarness(stream, reconciled);
    const { controller, states, projects } = createController(harness.transport);

    const running = controller.start({
      projectName: "project",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
    });
    stream.reject(new Error("stream transport failed"));
    await running;

    expect(harness.boundedCalls).toContainEqual({
      command: "get_project",
      args: { projectName: "project" },
    });
    expect(projects).toEqual([reconciled]);
    expect(states[states.length - 1]).toMatchObject({
      status: "failed",
      requestPending: false,
      retryable: false,
      optimisticUser: undefined,
      optimisticReply: undefined,
    });
  });

  it("cancels and ignores stale view events after disposal but still reconciles final data", async () => {
    const stream = deferred<Project>();
    const harness = createHarness(stream);
    const { controller, states, projects } = createController(harness.transport);
    const running = controller.start({
      projectName: "project",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
    });
    const requestId = harness.streamCalls[0].args.requestId as string;
    const stateCount = states.length;

    controller.dispose();
    harness.channels[0].onmessage({
      event: "delta",
      request_id: requestId,
      thread_id: "thread-a",
      role: "产品经理",
      text: "stale",
    });
    expect(states).toHaveLength(stateCount);
    expect(harness.boundedCalls).toContainEqual({
      command: "cancel_chat_stream",
      args: { requestId, threadId: "thread-a" },
    });

    const finalProject = project("project");
    stream.resolve(finalProject);
    await running;
    expect(projects).toEqual([finalProject]);
    expect(states).toHaveLength(stateCount);
  });

  it("regenerates from the existing user message without creating an optimistic duplicate", async () => {
    const stream = deferred<Project>();
    const harness = createHarness(stream);
    const { controller, states } = createController(harness.transport);
    const running = controller.start({
      projectName: "project",
      threadId: "thread-a",
      role: "产品经理",
      originalUserMessageId: "user-1",
    });

    expect(harness.streamCalls[0].command).toBe("regenerate_chat_reply_stream_runtime");
    expect(harness.streamCalls[0].args).toMatchObject({ userMessageId: "user-1" });
    expect(harness.streamCalls[0].args).not.toHaveProperty("message");
    expect(states[0]?.optimisticUser).toBeUndefined();

    stream.resolve(project("project"));
    await running;
  });

  it("keeps submission locked when cancellation delivery fails", async () => {
    const stream = deferred<Project>();
    const harness = createHarness(stream, project("project"), {
      command: "cancel_chat_stream",
      error: new Error("cancel IPC failed"),
    });
    const { controller, states } = createController(harness.transport);
    const running = controller.start({
      projectName: "project",
      threadId: "thread-a",
      role: "产品经理",
      content: "hello",
    });

    await controller.stop();
    expect(states[states.length - 1]).toMatchObject({
      status: "failed",
      requestPending: true,
    });

    stream.resolve(project("project"));
    await running;
    expect(states[states.length - 1]).toMatchObject({
      status: "failed",
      requestPending: false,
    });
  });
});
