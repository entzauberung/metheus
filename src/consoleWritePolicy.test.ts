import { describe, expect, it } from "vitest";
import { getConsoleWritePolicy } from "./consoleWritePolicy";

describe("getConsoleWritePolicy", () => {
  it("allows writes only for a fully reconciled console snapshot", () => {
    expect(getConsoleWritePolicy(true, {
      status: "synced",
      subscriptionStatus: "connected",
      pendingRevision: null,
    }).writable).toBe(true);
  });

  it.each([
    ["syncing", "connected", null],
    ["delayed", "connected", null],
    ["disconnected", "reconnecting", null],
    ["synced", "connected", 12],
  ] as const)("blocks stale facts (%s, %s, %s)", (status, subscriptionStatus, pendingRevision) => {
    const policy = getConsoleWritePolicy(true, { status, subscriptionStatus, pendingRevision });
    expect(policy.writable).toBe(false);
    expect(policy.reason).not.toBe("");
  });

  it("does not gate non-console phases", () => {
    expect(getConsoleWritePolicy(false, {
      status: "disconnected",
      subscriptionStatus: "reconnecting",
      pendingRevision: 3,
    }).writable).toBe(true);
  });
});
