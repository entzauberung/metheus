import { describe, expect, it } from "vitest";
import {
  distanceFromBottom,
  isNearChatBottom,
  nextUnreadState,
} from "./chatScrollPolicy";

describe("chat scroll policy", () => {
  it("treats an empty or exactly-bottom viewport as pinned", () => {
    expect(isNearChatBottom({ scrollTop: 0, scrollHeight: 0, clientHeight: 400 })).toBe(true);
    expect(isNearChatBottom({ scrollTop: 600, scrollHeight: 1000, clientHeight: 400 })).toBe(true);
  });

  it("keeps history reading detached beyond the bottom threshold", () => {
    const metrics = { scrollTop: 300, scrollHeight: 1000, clientHeight: 400 };
    expect(distanceFromBottom(metrics)).toBe(300);
    expect(isNearChatBottom(metrics)).toBe(false);
  });

  it("collapses any number of detached updates into one unread state", () => {
    expect(nextUnreadState(true, true)).toBe(false);
    expect(nextUnreadState(false, false)).toBe(true);
    expect(nextUnreadState(true, false)).toBe(true);
    expect(nextUnreadState(false, false, false)).toBe(false);
  });
});
