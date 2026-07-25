import { describe, expect, it } from "vitest";
import {
  canSubmitChatMessage,
  CHAT_COMPOSER_MAX_HEIGHT_PX,
  CHAT_COMPOSER_MIN_HEIGHT_PX,
  clampComposerHeight,
  shouldSendFromComposer,
} from "./chatComposerPolicy";

describe("chat composer policy", () => {
  it("sends on Enter but inserts a newline on Shift+Enter", () => {
    expect(shouldSendFromComposer({ key: "Enter", shiftKey: false, isComposing: false })).toBe(true);
    expect(shouldSendFromComposer({ key: "Enter", shiftKey: true, isComposing: false })).toBe(false);
  });

  it("does not send while an input method editor is composing", () => {
    expect(shouldSendFromComposer({ key: "Enter", shiftKey: false, isComposing: true })).toBe(false);
  });

  it("clamps auto height between one and six lines", () => {
    expect(clampComposerHeight(0)).toBe(CHAT_COMPOSER_MIN_HEIGHT_PX);
    expect(clampComposerHeight(84)).toBe(84);
    expect(clampComposerHeight(999)).toBe(CHAT_COMPOSER_MAX_HEIGHT_PX);
  });

  it("rejects blank or concurrent submissions", () => {
    expect(canSubmitChatMessage("   ", false)).toBe(false);
    expect(canSubmitChatMessage("next draft", true)).toBe(false);
    expect(canSubmitChatMessage("next draft", false)).toBe(true);
  });
});
