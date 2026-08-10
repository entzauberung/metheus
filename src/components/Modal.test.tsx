/* @vitest-environment happy-dom */

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dialogState = vi.hoisted(() => ({
  onOpenChange: (_open: boolean) => {},
}));

vi.mock("@radix-ui/react-dialog", () => ({
  Root: ({ children, onOpenChange }: {
    children: ReactNode;
    onOpenChange: (open: boolean) => void;
  }) => {
    dialogState.onOpenChange = onOpenChange;
    return <div data-testid="dialog-root">{children}</div>;
  },
  Portal: ({ children }: { children: ReactNode }) => <>{children}</>,
  Overlay: () => <div data-testid="dialog-overlay" />,
  Content: ({ children, onEscapeKeyDown, onInteractOutside }: {
    children: ReactNode;
    onEscapeKeyDown?: (event: { preventDefault: () => void }) => void;
    onInteractOutside?: (event: { preventDefault: () => void }) => void;
  }) => {
    const dismiss = (handler?: (event: { preventDefault: () => void }) => void) => {
      let prevented = false;
      handler?.({ preventDefault: () => { prevented = true; } });
      if (!prevented) dialogState.onOpenChange(false);
    };
    return (
      <div data-testid="dialog-content">
        <button data-testid="escape-dismiss" onClick={() => dismiss(onEscapeKeyDown)} type="button">Esc</button>
        <button data-testid="outside-dismiss" onClick={() => dismiss(onInteractOutside)} type="button">Outside</button>
        {children}
      </div>
    );
  },
  Title: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
  Description: ({ children }: { children: ReactNode }) => <p>{children}</p>,
  Close: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

import { Modal } from "./Modal";

describe("Modal close contract", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  function renderModal(onClose: () => void, lockClose = false) {
    act(() => root.render(
      <Modal isOpen onClose={onClose} title="测试弹窗" lockClose={lockClose}>
        内容
      </Modal>,
    ));
  }

  it.each(["escape-dismiss", "outside-dismiss"])(
    "routes %s through the shared close callback",
    (testId) => {
      const onClose = vi.fn();
      renderModal(onClose);

      act(() => document.body.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`)?.click());

      expect(onClose).toHaveBeenCalledTimes(1);
      expect(document.body.querySelector('[aria-label="关闭"]')).not.toBeNull();
    },
  );

  it("preserves lockClose for confirmation dialogs", () => {
    const onClose = vi.fn();
    renderModal(onClose, true);

    act(() => document.body.querySelector<HTMLButtonElement>('[data-testid="escape-dismiss"]')?.click());
    act(() => document.body.querySelector<HTMLButtonElement>('[data-testid="outside-dismiss"]')?.click());
    act(() => dialogState.onOpenChange(false));

    expect(onClose).not.toHaveBeenCalled();
    expect(document.body.querySelector('[aria-label="关闭"]')).toBeNull();
  });
});
