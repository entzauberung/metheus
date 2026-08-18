import { describe, expect, it } from "vitest";
import { resolveNativeReadiness } from "./PreflightPanel";
import type { NativeEvidenceKind, NativeEvidenceReference, NativeReadinessStatus } from "./types";

const requiredKinds: NativeEvidenceKind[] = [
  "operator",
  "channel",
  "screenshot",
  "input_probe",
  "close_reopen",
  "stop_control",
];

function evidence(kind: NativeEvidenceKind, source: NativeEvidenceReference["source"] = "human"): NativeEvidenceReference {
  return { kind, reference: `${kind}-record`, source, recorded_at: "2026-08-18T00:00:00Z" };
}

function completeRecord() {
  return {
    status: "PENDING" as NativeReadinessStatus,
    operator: "operator-1",
    channel: "native-channel-1",
    evidence_references: requiredKinds.map((kind) => evidence(kind)),
  };
}

describe("native readiness", () => {
  it("defaults legacy or missing records to pending with every evidence gap", () => {
    const view = resolveNativeReadiness();
    expect(view.status).toBe("PENDING");
    expect(view.missingRequirements).toHaveLength(6);
    expect(view.summary).toContain("Vite 页面");
  });

  it("rejects Vite evidence and malformed READY claims", () => {
    const record = completeRecord();
    record.status = "READY";
    record.evidence_references[2] = evidence("screenshot", "vite");
    const view = resolveNativeReadiness(record);
    expect(view.status).toBe("BLOCKED");
    expect(view.missingRequirements).toContain("原生窗口截图");
    expect(view.summary).toContain("声明 READY");
  });

  it("shows ready only after every native evidence reference is valid", () => {
    const view = resolveNativeReadiness(completeRecord());
    expect(view.status).toBe("READY");
    expect(view.missingRequirements).toEqual([]);
  });

  it("preserves an explicit backend block even when references are present", () => {
    const view = resolveNativeReadiness({ ...completeRecord(), status: "BLOCKED", next_action: "等待人工" });
    expect(view.status).toBe("BLOCKED");
    expect(view.nextAction).toBe("等待人工");
  });
});
