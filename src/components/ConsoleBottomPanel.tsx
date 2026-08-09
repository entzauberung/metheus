import { ChevronDown, ChevronUp } from "lucide-react";
import { useState, type ReactNode } from "react";

export function ConsoleBottomPanel({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className={`console-bottom-panel${open ? " open" : ""}`}>
      <button
        aria-expanded={open}
        className="console-bottom-toggle"
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        {open ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        控制台
      </button>
      {open && <div className="console-bottom-content">{children}</div>}
    </section>
  );
}
