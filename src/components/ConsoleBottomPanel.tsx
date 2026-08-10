import { ChevronDown, ChevronUp, FileText, History } from "lucide-react";
import { useState, type CSSProperties, type ReactNode } from "react";

export type ConsoleBottomView = "logs" | "preview";

const BOTTOM_CONTENT_LAYOUT: CSSProperties = {
  display: "grid",
  gridTemplateRows: "auto minmax(0, 1fr)",
  height: "100%",
  minWidth: 0,
  minHeight: 0,
  overflow: "hidden",
};

const BOTTOM_VIEW_LAYOUT: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  minWidth: 0,
  minHeight: 0,
  overflow: "hidden",
};

interface ConsoleBottomPanelProps {
  activeView?: ConsoleBottomView;
  children: ReactNode;
  onActiveViewChange?: (view: ConsoleBottomView) => void;
  onOpenChange?: (open: boolean) => void;
  open?: boolean;
  preview?: ReactNode;
}

export function ConsoleBottomPanel({
  activeView,
  children,
  onActiveViewChange,
  onOpenChange,
  open,
  preview,
}: ConsoleBottomPanelProps) {
  const [internalView, setInternalView] = useState<ConsoleBottomView>("logs");
  const [internalOpen, setInternalOpen] = useState(true);
  const selectedView = activeView ?? internalView;
  const panelOpen = open ?? internalOpen;
  const changeView = onActiveViewChange ?? setInternalView;
  const changeOpen = onOpenChange ?? setInternalOpen;

  return (
    <section className={`console-bottom-panel${panelOpen ? " open" : ""}`}>
      <button
        aria-controls="console-bottom-content"
        aria-expanded={panelOpen}
        className="console-bottom-toggle"
        onClick={() => changeOpen(!panelOpen)}
        type="button"
      >
        {panelOpen ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        {selectedView === "logs" ? "运行与测试日志" : "文件预览"}
      </button>
      {panelOpen && (
        <div
          className="console-bottom-content"
          id="console-bottom-content"
          style={BOTTOM_CONTENT_LAYOUT}
        >
          <div className="task-tabs" role="tablist" aria-label="底部面板">
            <button
              aria-controls="console-bottom-panel-view"
              aria-selected={selectedView === "logs"}
              className="task-tab"
              onClick={() => changeView("logs")}
              role="tab"
              type="button"
            >
              <History size={14} />运行与测试日志
            </button>
            <button
              aria-controls="console-bottom-panel-view"
              aria-selected={selectedView === "preview"}
              className="task-tab"
              onClick={() => changeView("preview")}
              role="tab"
              type="button"
            >
              <FileText size={14} />文件预览
            </button>
          </div>
          <div id="console-bottom-panel-view" role="tabpanel" style={BOTTOM_VIEW_LAYOUT}>
            {selectedView === "logs"
              ? children
              : preview ?? <p className="file-preview-empty">请从文件树选择要预览的文件。</p>}
          </div>
        </div>
      )}
    </section>
  );
}
