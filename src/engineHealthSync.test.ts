/* @vitest-environment happy-dom */

import { describe, expect, it, vi } from "vitest";
import {
  BUILT_IN_GROK_BUILD_HEALTH_TARGET,
  invalidateEngineHealth,
  matchesEngineHealthTarget,
  subscribeEngineHealthInvalidation,
} from "./engineHealthSync";

describe("engine health invalidation", () => {
  it("delivers one shared target to multiple listeners and cleans them independently", () => {
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = subscribeEngineHealthInvalidation(first);
    const unsubscribeSecond = subscribeEngineHealthInvalidation(second);

    invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    expect(first).toHaveBeenCalledWith({ runtime: "BuiltIn", provider: "GrokBuild" });
    expect(second).toHaveBeenCalledWith({ runtime: "BuiltIn", provider: "GrokBuild" });

    unsubscribeFirst();
    invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(2);

    unsubscribeSecond();
    invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    expect(second).toHaveBeenCalledTimes(2);
  });

  it("matches both runtime and provider", () => {
    expect(matchesEngineHealthTarget({
      runtime: "BuiltIn",
      provider: "GrokBuild",
      permission_profile: "Unattended",
      profile_revision: 1,
    }, BUILT_IN_GROK_BUILD_HEALTH_TARGET)).toBe(true);
    expect(matchesEngineHealthTarget({
      runtime: "Plugin",
      provider: "GrokBuild",
      permission_profile: "Unattended",
      profile_revision: 1,
    }, BUILT_IN_GROK_BUILD_HEALTH_TARGET)).toBe(false);
  });
});
