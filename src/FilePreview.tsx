import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import type { FilePreviewResult } from "./types";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";

interface FilePreviewProps {
  projectPath: string;
  filePath: string | null;
}

type FilePreviewStatus = "idle" | "loading" | "ready" | "unsupported" | "error";

const PREVIEW_LAYOUT: CSSProperties = {
  display: "grid",
  gridTemplateRows: "auto minmax(0, 1fr)",
  width: "100%",
  height: "100%",
  minWidth: 0,
  minHeight: 0,
  overflow: "hidden",
};

const PREVIEW_HEADER_LAYOUT: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "8px",
  minWidth: 0,
  padding: "8px 12px",
  borderBottom: "1px solid #d0d7de",
};

const PREVIEW_PATH_LAYOUT: CSSProperties = {
  minWidth: 0,
  overflow: "hidden",
  color: "#656d76",
  fontSize: "12px",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const PREVIEW_STATE_LAYOUT: CSSProperties = {
  minWidth: 0,
  margin: 0,
  padding: "16px",
  overflow: "auto",
  overflowWrap: "anywhere",
};

const PREVIEW_READY_LAYOUT: CSSProperties = {
  display: "grid",
  gridTemplateRows: "auto minmax(0, 1fr)",
  minWidth: 0,
  minHeight: 0,
  overflow: "hidden",
};

const PREVIEW_CONTENT_LAYOUT: CSSProperties = {
  minWidth: 0,
  minHeight: 0,
  maxWidth: "100%",
  margin: 0,
  overflow: "auto",
  whiteSpace: "pre",
};

function previewErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "无法读取文件预览，请重试";
}

export default function FilePreview({ projectPath, filePath }: FilePreviewProps) {
  const [status, setStatus] = useState<FilePreviewStatus>("idle");
  const [preview, setPreview] = useState<FilePreviewResult | null>(null);
  const [errorMessage, setErrorMessage] = useState("");
  const requestIdRef = useRef(0);

  const loadPreview = useCallback(async () => {
    const requestId = ++requestIdRef.current;
    setPreview(null);
    setErrorMessage("");
    if (!projectPath || !filePath) {
      setStatus("idle");
      return;
    }

    setStatus("loading");
    try {
      const result = await invokeWithTimeout<FilePreviewResult>(
        "read_project_file_preview",
        { projectPath, path: filePath },
      );
      if (requestId !== requestIdRef.current) return;
      setPreview(result);
      setStatus(result.binary || Boolean(result.error) ? "unsupported" : "ready");
    } catch (error) {
      if (requestId !== requestIdRef.current) return;
      setErrorMessage(previewErrorMessage(error));
      setStatus("error");
    }
  }, [filePath, projectPath]);

  useEffect(() => {
    void loadPreview();
    return () => {
      requestIdRef.current += 1;
    };
  }, [loadPreview]);

  return (
    <section
      className="file-preview"
      aria-busy={status === "loading"}
      aria-label="只读文件预览"
      data-preview-layout="bounded"
      style={PREVIEW_LAYOUT}
    >
      <header className="file-preview-header" style={PREVIEW_HEADER_LAYOUT}>
        <strong>只读文件预览</strong>
        <span className="file-preview-path" style={PREVIEW_PATH_LAYOUT} title={filePath ?? ""}>
          {filePath ?? "未选择文件"}
        </span>
      </header>

      {status === "idle" && (
        <p className="file-preview-empty" style={PREVIEW_STATE_LAYOUT} role="status">请从文件树选择要预览的文件。</p>
      )}
      {status === "loading" && (
        <p className="file-preview-loading" style={PREVIEW_STATE_LAYOUT} role="status" aria-live="polite">正在读取文件预览…</p>
      )}
      {status === "error" && (
        <div className="file-preview-error" style={PREVIEW_STATE_LAYOUT} role="alert">
          <strong>文件预览失败</strong>
          <p>{errorMessage}</p>
          <button type="button" onClick={() => { void loadPreview(); }}>重试</button>
        </div>
      )}
      {status === "unsupported" && preview && (
        <div className="file-preview-unsupported" style={PREVIEW_STATE_LAYOUT} role="status">
          <strong>该文件无法以文本预览</strong>
          <p>{preview.error ?? "该文件不是受支持的 UTF-8 文本。"}</p>
        </div>
      )}
      {status === "ready" && preview && (
        <div className="file-preview-ready" style={PREVIEW_READY_LAYOUT}>
          <div className="file-preview-meta">
            <span>{preview.file_type || "text"}</span>
            {preview.truncated && <strong role="status">内容已截断，仅显示安全预览范围。</strong>}
          </div>
          <pre
            className="diff-view file-preview-content"
            style={PREVIEW_CONTENT_LAYOUT}
            aria-label={`${preview.path} 的只读内容`}
          >
            {preview.content}
          </pre>
        </div>
      )}
    </section>
  );
}
