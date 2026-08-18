/* @vitest-environment happy-dom */

import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import AcceptanceLedgerPanel from "./AcceptanceLedgerPanel";

describe("AcceptanceLedgerPanel", () => {
  it("explains why a task with no criteria has no ledger", () => {
    const html = renderToStaticMarkup(<AcceptanceLedgerPanel items={[]} />);
    expect(html).toContain("当前任务没有验收标准");
    expect(html).not.toContain("尚无逐项验收记录");
  });

  it("distinguishes an uninitialized ledger from a task that is still executing", () => {
    const html = renderToStaticMarkup(
      <AcceptanceLedgerPanel
        items={[]}
        criteria={["页面可打开"]}
        taskStatus="Executing"
      />,
    );
    expect(html).toContain("任务等待验证");
    expect(html).toContain("尚未形成逐项验收记录");
    expect(html).toContain('data-ledger-empty-state="AwaitingVerification"');
  });

  it("distinguishes initialization, verification, and anomalous ledger gaps", () => {
    const initializing = renderToStaticMarkup(
      <AcceptanceLedgerPanel items={[]} criteria={["页面可打开"]} taskStatus="Pending" />,
    );
    expect(initializing).toContain('data-ledger-empty-state="WaitingInitialization"');
    expect(initializing).toContain("等待后端在验证阶段初始化");

    const anomaly = renderToStaticMarkup(
      <AcceptanceLedgerPanel items={[]} criteria={["页面可打开"]} taskStatus="Passed" />,
    );
    expect(anomaly).toContain('data-ledger-empty-state="StateAnomaly"');
    expect(anomaly).toContain("不应出现空账本");
  });

  it("keeps malformed ledger rows visible while marking the structure anomalous", () => {
    const html = renderToStaticMarkup(
      <AcceptanceLedgerPanel
        items={[{
          criterion_index: 1,
          criterion: "旧标准",
          status: "Unknown",
          evidence: "",
          evidence_references: [],
          confidence: 0,
          updated_at: "2026-08-18T00:00:00Z",
        }]}
        criteria={["当前标准"]}
        taskStatus="Passed"
      />,
    );
    expect(html).toContain('data-ledger-empty-state="StateAnomaly"');
    expect(html).toContain("重复/错配记录");
    expect(html).toContain("旧标准");
    expect(html).toContain("后端尚未提供足够证据");
  });

  it("does not treat an unexpected ledger as no-criteria", () => {
    const html = renderToStaticMarkup(
      <AcceptanceLedgerPanel
        items={[{
          criterion_index: 1,
          criterion: "不应存在",
          status: "Contradictory",
          evidence: "",
          evidence_references: [],
          confidence: 0,
          updated_at: "2026-08-18T00:00:00Z",
        }]}
        criteria={[]}
      />,
    );
    expect(html).toContain('data-ledger-empty-state="StateAnomaly"');
    expect(html).not.toContain("当前任务没有验收标准");
    expect(html).toContain("验收契约与当前证据冲突");
  });
});
