import { useState, type ReactNode } from "react";

export function ConsoleNavigator({
  taskTree,
  fileTree,
}: {
  taskTree: ReactNode;
  fileTree: ReactNode;
}) {
  const [tab, setTab] = useState<"tasks" | "files">("tasks");
  return (
    <div className="console-navigator">
      <div className="console-navigator-tabs" role="tablist" aria-label="Console 导航">
        <button
          aria-selected={tab === "tasks"}
          onClick={() => setTab("tasks")}
          role="tab"
          type="button"
        >任务</button>
        <button
          aria-selected={tab === "files"}
          onClick={() => setTab("files")}
          role="tab"
          type="button"
        >文件</button>
      </div>
      <div className="console-navigator-body" role="tabpanel">
        {tab === "tasks" ? taskTree : fileTree}
      </div>
    </div>
  );
}
