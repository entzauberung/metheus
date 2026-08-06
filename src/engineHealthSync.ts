import type { ExecutionProfile, ExecutionProvider, ExecutionRuntime } from "./types";

export const ENGINE_HEALTH_INVALIDATED_EVENT = "metheus:engine-health-invalidated";

export interface EngineHealthInvalidationTarget {
  runtime: ExecutionRuntime;
  provider: ExecutionProvider;
}

export const BUILT_IN_GROK_BUILD_HEALTH_TARGET: EngineHealthInvalidationTarget = {
  runtime: "BuiltIn",
  provider: "GrokBuild",
};

export function matchesEngineHealthTarget(
  profile: ExecutionProfile,
  target: EngineHealthInvalidationTarget,
): boolean {
  return profile.runtime === target.runtime && profile.provider === target.provider;
}

export function invalidateEngineHealth(target: EngineHealthInvalidationTarget): void {
  window.dispatchEvent(new CustomEvent<EngineHealthInvalidationTarget>(
    ENGINE_HEALTH_INVALIDATED_EVENT,
    { detail: target },
  ));
}

export function subscribeEngineHealthInvalidation(
  listener: (target: EngineHealthInvalidationTarget) => void,
): () => void {
  const handleInvalidation = (event: Event) => {
    listener((event as CustomEvent<EngineHealthInvalidationTarget>).detail);
  };
  window.addEventListener(ENGINE_HEALTH_INVALIDATED_EVENT, handleInvalidation);
  return () => window.removeEventListener(ENGINE_HEALTH_INVALIDATED_EVENT, handleInvalidation);
}
