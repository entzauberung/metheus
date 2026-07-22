import { describe, expect, it } from "vitest";
import { engineChangeBlockedReason, engineHealthBlocksExecution } from "./enginePolicy";
import type { EngineHealth, PipelineState, Project } from "./types";

function project(): Project {
  return {
    execution_profile: {
      runtime: "Plugin",
      provider: "ClaudeCode",
      permission_profile: "Unattended",
      profile_revision: 1,
    },
    workflow_state: {
      recovery_state: undefined,
      autopilot_state: undefined,
      managed_flow_state: undefined,
    },
    execution_session: undefined,
  } as unknown as Project;
}

function pipeline(status: PipelineState["status"]): PipelineState {
  return { status } as PipelineState;
}

describe("execution engine change policy", () => {
  it("blocks known unusable engines but permits unknown probes", () => {
    const health = (status: EngineHealth["status"]) => ({ status } as EngineHealth);
    expect(engineHealthBlocksExecution(health("NotInstalled"))).toBe(true);
    expect(engineHealthBlocksExecution(health("Unauthenticated"))).toBe(true);
    expect(engineHealthBlocksExecution(health("Disabled"))).toBe(true);
    expect(engineHealthBlocksExecution(health("Available"))).toBe(false);
    expect(engineHealthBlocksExecution(health("Unknown"))).toBe(false);
  });

  it("allows changes at a stable boundary", () => {
    expect(engineChangeBlockedReason(project(), null)).toBeNull();
  });

  it("blocks a running process or persisted session", () => {
    expect(engineChangeBlockedReason(project(), pipeline("Running"))).toBe("执行正在运行");
    const withSession = project();
    withSession.execution_session = { active: true } as Project["execution_session"];
    expect(engineChangeBlockedReason(withSession, null)).toBe("存在活跃执行会话");
  });

  it("blocks recovery and automatic advancement", () => {
    const recovering = project();
    recovering.workflow_state.recovery_state = { phase: "Repairing" } as NonNullable<
      Project["workflow_state"]["recovery_state"]
    >;
    expect(engineChangeBlockedReason(recovering, null)).toBe("错误恢复正在进行");

    const autopilot = project();
    autopilot.workflow_state.autopilot_state = {
      active: true,
      run_status: "Running",
    } as NonNullable<Project["workflow_state"]["autopilot_state"]>;
    expect(engineChangeBlockedReason(autopilot, null)).toBe("自动驾驶正在推进");
  });
});
