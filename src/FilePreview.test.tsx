/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("./utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

import FilePreview from "./FilePreview";
import type { FilePreviewResult } from "./types";

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("FilePreview", () => {
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

  function render(filePath: string | null) {
    act(() => root.render(
      <FilePreview projectPath="/projects/demo" filePath={filePath} />,
    ));
  }

  it("shows an explicit empty state without reading when no file is selected", async () => {
    render(null);
    await flushPromises();

    expect(invokeMock).not.toHaveBeenCalled();
    expect(host.textContent).toContain("请从文件树选择");
    expect(host.textContent).toContain("只读文件预览");
    const panel = host.querySelector<HTMLElement>(".file-preview");
    const path = host.querySelector<HTMLElement>(".file-preview-path");
    expect(panel?.dataset.previewLayout).toBe("bounded");
    expect(panel?.style.gridTemplateRows).toContain("minmax(0, 1fr)");
    expect(panel?.style.overflow).toBe("hidden");
    expect(path?.style.textOverflow).toBe("ellipsis");
  });

  it("shows loading and then bounded text with a truncation notice", async () => {
    let resolve!: (value: FilePreviewResult) => void;
    invokeMock.mockReturnValueOnce(new Promise<FilePreviewResult>(resolvePromise => {
      resolve = resolvePromise;
    }));
    render("src/main.ts");

    expect(host.textContent).toContain("正在读取文件预览");
    expect(host.querySelector(".file-preview")?.getAttribute("aria-busy")).toBe("true");

    await act(async () => {
      resolve({
        path: "src/main.ts",
        content: "export const ready = true;",
        file_type: "ts",
        truncated: true,
        binary: false,
        error: null,
      });
      await Promise.resolve();
    });

    expect(invokeMock).toHaveBeenCalledWith("read_project_file_preview", {
      projectPath: "/projects/demo",
      path: "src/main.ts",
    });
    expect(host.querySelector(".file-preview-content")?.textContent)
      .toContain("export const ready");
    expect(host.textContent).toContain("内容已截断");
    const content = host.querySelector<HTMLElement>(".file-preview-content");
    expect(content?.style.overflow).toBe("auto");
    expect(content?.style.whiteSpace).toBe("pre");
    expect(content?.style.maxWidth).toBe("100%");
  });

  it("shows binary and invalid text as unsupported without rendering content", async () => {
    invokeMock.mockResolvedValueOnce({
      path: "image.bin",
      content: "",
      file_type: "bin",
      truncated: false,
      binary: true,
      error: "该文件包含二进制内容，无法预览",
    } satisfies FilePreviewResult);
    render("image.bin");
    await flushPromises();

    expect(host.querySelector(".file-preview-unsupported")?.textContent).toContain("二进制内容");
    expect(host.querySelector(".file-preview-content")).toBeNull();
  });

  it("shows command errors and retries the same project file", async () => {
    invokeMock
      .mockRejectedValueOnce(new Error("文件不存在"))
      .mockResolvedValueOnce({
        path: "README.md",
        content: "# Demo",
        file_type: "md",
        truncated: false,
        binary: false,
        error: null,
      } satisfies FilePreviewResult);
    render("README.md");
    await flushPromises();

    const error = host.querySelector<HTMLElement>(".file-preview-error");
    expect(error?.getAttribute("role")).toBe("alert");
    expect(error?.textContent).toContain("文件不存在");

    act(() => error?.querySelector<HTMLButtonElement>("button")?.click());
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenLastCalledWith("read_project_file_preview", {
      projectPath: "/projects/demo",
      path: "README.md",
    });
    expect(host.querySelector(".file-preview-content")?.textContent).toContain("# Demo");
  });
});
