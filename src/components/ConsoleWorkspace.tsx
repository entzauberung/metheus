import type { ReactNode } from "react";

export const CONSOLE_LAYOUT_CONTRACT = {
  compactMaxWidth: 420,
  singleColumnMaxWidth: 600,
  floatingLayerMaximum: 100,
  inspectorBackdropLayer: 110,
  inspectorLayer: 120,
  inspectorResizeLayer: 125,
} as const;

interface ConsoleWorkspaceProps {
  commandBar: ReactNode;
  navigator: ReactNode;
  bottom?: ReactNode;
  children: ReactNode;
}

export function ConsoleWorkspace({ commandBar, navigator, bottom, children }: ConsoleWorkspaceProps) {
  return (
    <section
      className="console-workspace"
      aria-label="Console 工作台"
      data-console-layout="responsive-grid"
      data-console-compact-max-width={CONSOLE_LAYOUT_CONTRACT.compactMaxWidth}
      data-console-single-column-max-width={CONSOLE_LAYOUT_CONTRACT.singleColumnMaxWidth}
    >
      <div
        className="console-workspace-command"
        data-console-region="command"
        role="region"
        aria-label="Console 命令区"
      >
        {commandBar}
      </div>
      <aside
        className="console-workspace-navigator"
        data-console-region="navigator"
        aria-label="Console 导航区"
      >
        {navigator}
      </aside>
      <div
        className="console-workspace-main"
        data-console-region="main"
        role="region"
        aria-label="Console 主工作区"
        tabIndex={0}
      >
        {children}
      </div>
      {bottom && (
        <div
          className="console-workspace-bottom"
          data-console-region="bottom"
          role="region"
          aria-label="Console 底部面板"
        >
          {bottom}
        </div>
      )}
    </section>
  );
}

export function ConsoleCommandBar({ children }: { children: ReactNode }) {
  return <div className="console-command-bar" aria-label="Console 命令栏">{children}</div>;
}

export function ConsoleRuntimeRow({ children }: { children: ReactNode }) {
  return (
    <div
      className="console-runtime-row"
      data-console-region="runtime"
      role="region"
      aria-label="Console 运行控制"
    >
      {children}
    </div>
  );
}
