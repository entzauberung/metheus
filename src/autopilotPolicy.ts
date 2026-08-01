import type {
  AcceptanceLedgerItem,
  PipelineState,
  TestResult,
  VerificationStage,
} from "./types";

export interface QualityStatusPresentation {
  key: "automated-test" | "code-review" | "review-protocol" | "acceptance-evidence";
  label: string;
  tone: "success" | "warning" | "error" | "neutral";
}

export function getQualityStatusPresentation(
  test: TestResult | undefined,
  ledger: AcceptanceLedgerItem[],
): QualityStatusPresentation[] {
  const automated = (() => {
    switch (test?.automated_test_status) {
      case "Passed": return { label: "自动化测试：通过", tone: "success" as const };
      case "Failed": return { label: "自动化测试：失败", tone: "error" as const };
      case "NotConfigured": return { label: "自动化测试：未配置", tone: "neutral" as const };
      case "Unavailable": return { label: "自动化测试：不可用", tone: "warning" as const };
      default: return { label: "自动化测试：状态未知", tone: "neutral" as const };
    }
  })();
  const hasBlockingReview = test?.review_issues?.some(issue => issue.severity === "Blocking") === true;
  const reviewServiceFailure = test?.review_failure_kind !== undefined
    && !["InvalidJson", "FieldTypeMismatch"].includes(test.review_failure_kind);
  const codeReview = test?.verification_kind === "DeterministicLocal"
    ? test.passed
      ? { label: "本地确定性验证：通过", tone: "success" as const }
      : { label: "本地确定性验证：未通过", tone: "error" as const }
    : reviewServiceFailure || test?.review_evidence_status === "Unavailable"
    ? { label: "代码审查：不可用", tone: "warning" as const }
    : test?.review_passed === true
      ? { label: "代码审查：通过", tone: "success" as const }
      : hasBlockingReview
        ? { label: "代码审查：存在阻断问题", tone: "error" as const }
        : { label: "代码审查：待确认", tone: "neutral" as const };
  const protocolFailure = test?.review_failure_kind === "InvalidJson"
    || test?.review_failure_kind === "FieldTypeMismatch";
  const protocolInProgress = test?.review_status === "InProgress"
    || ["ParsingReview", "DeterministicNormalization", "ProtocolRepair", "ReviewRetry"]
      .includes(test?.verification_stage ?? "NotStarted");
  const protocol = protocolFailure
    ? { label: "审查协议：格式异常", tone: "error" as const }
    : protocolInProgress
      ? { label: "审查协议：处理中", tone: "warning" as const }
      : test?.review_status === "Completed"
        ? { label: "审查协议：有效", tone: "success" as const }
        : test?.review_status === "Failed"
          ? { label: "审查协议：未取得结果", tone: "warning" as const }
          : { label: "审查协议：未请求", tone: "neutral" as const };
  const evidence = ledger.some(item => item.status === "Contradictory")
    ? { label: "验收证据：结论冲突", tone: "error" as const }
    : ledger.some(item => item.status === "Unknown")
      ? { label: "验收证据：不足", tone: "warning" as const }
      : ledger.some(item => item.status === "Unsatisfied")
        ? { label: "验收证据：存在阻断项", tone: "error" as const }
        : ledger.length > 0
          ? { label: "验收证据：充分", tone: "success" as const }
          : test?.review_evidence_status === "Unavailable"
            ? { label: "验收证据：不可用", tone: "warning" as const }
            : { label: "验收证据：无逐项标准", tone: "neutral" as const };

  return [
    { key: "automated-test", ...automated },
    { key: "code-review", ...codeReview },
    { key: "review-protocol", ...protocol },
    { key: "acceptance-evidence", ...evidence },
  ];
}

const VERIFICATION_STAGE_LABELS: Record<VerificationStage, string> = {
  NotStarted: "等待验证",
  AutomatedTests: "运行自动化测试",
  PreparingEvidence: "准备验收证据",
  RequestingReview: "请求 AI 审查",
  ParsingReview: "解析审查结果",
  DeterministicNormalization: "确定性归一化",
  ProtocolRepair: "修复审查协议",
  ReviewRetry: "重新请求 AI 审查",
  TargetedEvidence: "定向补充证据",
  Completed: "验证完成",
};

export function getVerificationStageLabel(stage: VerificationStage | undefined): string {
  return VERIFICATION_STAGE_LABELS[stage ?? "NotStarted"];
}

export function isHeartbeatStale(
  heartbeatAt: string | undefined,
  active: boolean,
  now = Date.now(),
): boolean {
  if (!active || !heartbeatAt) return false;
  const heartbeat = Date.parse(heartbeatAt);
  return Number.isFinite(heartbeat) && now - heartbeat > 15_000;
}

export function executionPollingOwnsNextAdvance(state: PipelineState): boolean {
  return state.status === "Running";
}
