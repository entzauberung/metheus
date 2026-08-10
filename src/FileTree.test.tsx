/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("./utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

import FileTree from "./FileTree";
import type { FileEntry } from "./types";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

const files: FileEntry[] = [
  { path: "src/main.ts", is_dir: false, file_type: "ts" },
  { path: "README.md", is_dir: false, file_type: "md" },
];

describe("FileTree visible states", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    invokeMock.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.restoreAllMocks();
  });

  function render(projectPath = "/projects/demo", onFileSelect = vi.fn()) {
    act(() => root.render(
      <FileTree projectPath={projectPath} onFileSelect={onFileSelect} />,
    ));
    return onFileSelect;
  }

  it("shows a full discoverable file panel and loading status", () => {
    const pending = deferred<FileEntry[]>();
    invokeMock.mockReturnValueOnce(pending.promise);
    render();

    const panel = host.querySelector<HTMLElement>(".file-tree-container");
    const status = host.querySelector<HTMLElement>(".file-tree-loading");
    const refresh = host.querySelector<HTMLButtonElement>(".file-tree-refresh");
    expect(panel?.classList.contains("file-tree")).toBe(true);
    expect(panel?.style.width).toBe("");
    expect(host.textContent).toContain("项目文件");
    expect(host.textContent).toContain("/projects/demo");
    expect(status?.getAttribute("role")).toBe("status");
    expect(status?.getAttribute("aria-live")).toBe("polite");
    expect(status?.textContent).toContain("正在读取项目文件");
    expect(refresh?.disabled).toBe(true);
  });

  it("shows empty only after a successful empty response", async () => {
    invokeMock.mockResolvedValueOnce([]);
    render();
    await flushPromises();

    expect(host.querySelector(".file-tree-error")).toBeNull();
    expect(host.querySelector(".file-tree-empty")?.textContent).toContain("当前项目为空目录");
    expect(host.querySelector(".file-tree-empty")?.getAttribute("aria-live")).toBe("polite");
  });

  it("keeps a failed read distinct from empty and retries with the current project path", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    invokeMock
      .mockRejectedValueOnce(new Error("后端拒绝读取"))
      .mockResolvedValueOnce(files);
    render();
    await flushPromises();

    const error = host.querySelector<HTMLElement>(".file-tree-error");
    expect(error?.getAttribute("role")).toBe("alert");
    expect(error?.textContent).toContain("文件列表读取失败");
    expect(error?.textContent).toContain("后端拒绝读取");
    expect(host.querySelector(".file-tree-empty")).toBeNull();

    act(() => error?.querySelector<HTMLButtonElement>("button")?.click());
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenNthCalledWith(2, "get_project_files", {
      projectPath: "/projects/demo",
    });
    expect(host.querySelector(".file-tree-error")).toBeNull();
    expect(host.textContent).toContain("README.md");
  });

  it("renders ready files, refreshes, and preserves file selection behavior", async () => {
    invokeMock.mockResolvedValue(files);
    const onFileSelect = render();
    await flushPromises();

    expect(host.querySelector(".file-tree-scroll")?.getAttribute("aria-label"))
      .toBe("项目文件列表");
    const sourceDirectory = [...host.querySelectorAll<HTMLElement>(".dir-node")]
      .find(node => node.textContent?.includes("src"));
    expect(sourceDirectory?.tagName).toBe("BUTTON");
    expect(sourceDirectory?.getAttribute("type")).toBe("button");
    act(() => sourceDirectory?.focus());
    expect(document.activeElement).toBe(sourceDirectory);
    act(() => sourceDirectory?.click());
    const mainFile = [...host.querySelectorAll<HTMLElement>(".file-node")]
      .find(node => node.textContent?.includes("main.ts"));
    expect(mainFile?.tagName).toBe("BUTTON");
    act(() => mainFile?.click());
    expect(onFileSelect).toHaveBeenCalledWith("src/main.ts");

    act(() => host.querySelector<HTMLButtonElement>(".file-tree-refresh")?.click());
    await flushPromises();
    expect(invokeMock).toHaveBeenLastCalledWith("get_project_files", {
      projectPath: "/projects/demo",
    });
    expect(host.textContent).toContain("README.md");
  });

  it("shows a visible no-project state without invoking the backend", async () => {
    render("");
    await flushPromises();

    expect(invokeMock).not.toHaveBeenCalled();
    expect(host.textContent).toContain("请先选择项目目录");
    expect(host.querySelector<HTMLButtonElement>(".file-tree-refresh")?.disabled).toBe(true);
  });

  const staleResponseCases: Array<{
    name: string;
    settle: (request: Deferred<FileEntry[]>) => void;
  }> = [
    { name: "empty", settle: request => request.resolve([]) },
    {
      name: "ready",
      settle: request => request.resolve([
        { path: "stale.ts", is_dir: false, file_type: "ts" },
      ]),
    },
    { name: "error", settle: request => request.reject(new Error("旧项目读取失败")) },
  ];

  it.each(staleResponseCases)(
    "ignores a stale $name response after the project path changes",
    async ({ settle }) => {
      const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
      const oldRequest = deferred<FileEntry[]>();
      invokeMock
        .mockReturnValueOnce(oldRequest.promise)
        .mockResolvedValueOnce([
          { path: "current.ts", is_dir: false, file_type: "ts" },
        ]);

      render("/projects/old");
      render("/projects/current");
      await flushPromises();
      expect(host.textContent).toContain("current.ts");

      act(() => settle(oldRequest));
      await flushPromises();

      expect(invokeMock).toHaveBeenNthCalledWith(1, "get_project_files", {
        projectPath: "/projects/old",
      });
      expect(invokeMock).toHaveBeenNthCalledWith(2, "get_project_files", {
        projectPath: "/projects/current",
      });
      expect(host.textContent).toContain("current.ts");
      expect(host.textContent).not.toContain("stale.ts");
      expect(host.querySelector(".file-tree-error")).toBeNull();
      expect(host.querySelector(".file-tree-empty")).toBeNull();
      expect(consoleError).not.toHaveBeenCalled();
    },
  );
});
