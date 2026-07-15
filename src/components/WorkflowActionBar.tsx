import { ReactNode } from "react";

export function WorkflowActionBar({ children }: { children: ReactNode }) {
  return <div className="workflow-action-bar">{children}</div>;
}
