import { PanelRightOpen } from "lucide-react";
import type { ReactNode } from "react";

interface Props {
  syncStatus: ReactNode;
  inspectorOpen: boolean;
  onOpenInspector: () => void;
  settings: ReactNode;
}

export function ConsoleUtilityBar({ syncStatus, inspectorOpen, onOpenInspector, settings }: Props) {
  return (
    <div className="console-utility-bar" data-console-region="utility" role="region" aria-label="Console 工具栏">
      <div className="console-utility-sync" role="status" aria-live="polite" aria-atomic="true">
        {syncStatus}
      </div>
      <div className="console-utility-tools">
        <button
          type="button"
          className={`icon-button${inspectorOpen ? " active" : ""}`}
          onClick={onOpenInspector}
          title={inspectorOpen ? "任务检查器已打开" : "打开任务检查器"}
          aria-label={inspectorOpen ? "任务检查器已打开" : "打开任务检查器"}
          aria-expanded={inspectorOpen}
          aria-controls="task-inspector"
        >
          <PanelRightOpen size={16} />
        </button>
        {settings}
      </div>
    </div>
  );
}
