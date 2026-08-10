import { useEffect, useRef } from "react";

export type ModelServiceTarget = "decision" | "builtin-grok" | "vision";

interface Props {
  value: ModelServiceTarget;
  onChange: (target: ModelServiceTarget) => void;
  focusRequest?: number;
}

const ITEMS: Array<{ id: ModelServiceTarget; label: string }> = [
  { id: "decision", label: "决策模型" },
  { id: "builtin-grok", label: "内置 Grok Build" },
  { id: "vision", label: "视觉模型" },
];

export function ModelServiceNavigation({ value, onChange, focusRequest = 0 }: Props) {
  const activeButtonRef = useRef<HTMLButtonElement>(null);
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const focusButton = (index: number) => {
    buttonRefs.current[index]?.focus();
  };

  useEffect(() => {
    if (focusRequest > 0) activeButtonRef.current?.focus();
  }, [focusRequest]);

  return (
    <nav className="model-service-navigation" aria-label="模型服务子页面">
      <div role="tablist" aria-label="模型服务">
        {ITEMS.map((item, index) => (
          <button
            ref={(button) => {
              buttonRefs.current[index] = button;
              if (item.id === value) activeButtonRef.current = button;
            }}
            id={`model-service-tab-${item.id}`}
            type="button"
            role="tab"
            aria-selected={value === item.id}
            aria-controls={`model-service-panel-${item.id}`}
            tabIndex={value === item.id ? 0 : -1}
            className={value === item.id ? "selected" : ""}
            key={item.id}
            onClick={() => onChange(item.id)}
            onKeyDown={(event) => {
              let nextIndex = index;
              if (event.key === "ArrowLeft") {
                nextIndex = (index + ITEMS.length - 1) % ITEMS.length;
              } else if (event.key === "ArrowRight") {
                nextIndex = (index + 1) % ITEMS.length;
              } else if (event.key === "Home") {
                nextIndex = 0;
              } else if (event.key === "End") {
                nextIndex = ITEMS.length - 1;
              } else {
                return;
              }
              event.preventDefault();
              onChange(ITEMS[nextIndex].id);
              focusButton(nextIndex);
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
    </nav>
  );
}
