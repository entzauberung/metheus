import type {
  AcceptanceLedgerItem,
  AutopilotRecoveryAction,
  AutopilotRunStatus,
  GitConfirmationFailureKind,
  PipelineState,
  RecoveryState,
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
  const codeReview = reviewServiceFailure || test?.review_evidence_status === "Unavailable"
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

export function isValidationRecovery(recovery: RecoveryState | undefined): boolean {
  return recovery !== undefined && [
    "ReviewTransientFailure",
    "ReviewProtocolFailure",
    "ReviewServiceBlocked",
    "AutomatedTestUnavailable",
    "TestUnavailable",
  ].includes(recovery.error_kind);
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

export function getRecoveryStatusLabel(recovery: RecoveryState): string {
  const lastAttempt = recovery.attempt_history?.length
    ? recovery.attempt_history[recovery.attempt_history.length - 1]
    : undefined;
  switch (recovery.phase) {
    case "Diagnosing":
      return recovery.active_issues?.length
        ? `正在分析 ${recovery.active_issues.length} 个未满足验收项`
        : "正在诊断错误";
    case "Repairing":
      return recovery.replan_attempted
        ? "正在执行重规划后的当前任务"
        : `正在执行第 ${recovery.attempt}/${recovery.max_attempts} 次修复`;
    case "Retesting":
      if (recovery.error_kind === "ReviewTransientFailure") {
        return recovery.next_validation_retry_at
          ? `等待 AI 审查验证重试（${recovery.validation_retry_count}/${recovery.max_validation_retries}）`
          : `正在重新请求 AI 审查（${recovery.validation_retry_count}/${recovery.max_validation_retries}）`;
      }
      if (recovery.error_kind === "ReviewProtocolFailure") {
        return recovery.next_validation_retry_at
          ? `等待审查协议重试（${recovery.validation_retry_count}/${recovery.max_validation_retries}）`
          : "正在恢复审查协议";
      }
      if (recovery.error_kind === "EvidenceInsufficient") {
        return recovery.evidence_rebuild_attempts > 0
          ? `正在补充验收证据（${recovery.evidence_rebuild_attempts}/2）`
          : "正在准备补充验收证据";
      }
      return lastAttempt
        ? `正在重新测试；上一轮解决 ${lastAttempt.resolved_issue_ids.length} 项，剩余 ${lastAttempt.remaining_issue_ids.length + lastAttempt.regressed_issue_ids.length} 项`
        : "正在重新测试";
    case "Replanning": return "常规修复耗尽，正在重新规划当前任务";
    case "WaitingEngine": return "执行引擎阻断，代码恢复已停止";
    case "Recovered": return "恢复成功，继续执行";
    case "WaitingHuman":
      switch (recovery.error_kind) {
        case "EvidenceInsufficient": return "验收证据仍不足，等待人工处理";
        case "ContractContradiction": return "验收结论与阻断证据冲突，等待人工判断";
        case "ValidationOscillation": return "验收结论反复变化，等待人工判断";
        case "AutomatedTestUnavailable": return "自动化测试环境不可用，等待人工处理";
        case "ReviewServiceBlocked": return "AI 审查认证或额度异常，等待设置后重新验证";
        case "ReviewProtocolFailure": return "审查结果格式持续异常，等待人工处理";
        case "ReviewTransientFailure": return "AI 审查服务连续不可用，等待人工处理";
        case "TestUnavailable": return "旧验证结果不可用，等待人工处理";
        case "ExecutionError":
        case "TestFailure":
        case "ReviewFailure":
          return recovery.attempt >= recovery.max_attempts
            ? "自动恢复已耗尽，等待人工处理"
            : "代码质量问题等待人工处理";
        default: return "等待人工处理";
      }
  }
}

export function executionPollingOwnsNextAdvance(state: PipelineState): boolean {
  return state.status === "Running";
}

export interface GitConfirmationBlockPresentation {
  canRetry: boolean;
  hint: string;
}

export function getGitConfirmationBlockPresentation(
  failureKind: GitConfirmationFailureKind | undefined,
): GitConfirmationBlockPresentation {
  switch (failureKind) {
    case "LegacyV1TagConflict":
    case "CommitFailed":
    case "TagFailed":
    case "ProjectFinalizationFailed":
    case "GitMetadataUnavailable":
      return {
        canRetry: true,
        hint: "修复 Git 环境后可续跑同一确认事务，不会重新执行代码或质量检查。",
      };
    case "ScopeViolation":
      return {
        canRetry: false,
        hint: "请先人工处理任务范围外的工作区变更。",
      };
    case "V2TagIntegrityConflict":
    case "TagIdentityConflict":
    default:
      return {
        canRetry: false,
        hint: "请人工核对 V2 不可变标签与确认提交；系统不会覆盖、移动或删除标签。",
      };
  }
}

export interface AutopilotErrorActions {
  canResume: boolean;
  canRetryAdvance: boolean;
  canRegeneratePlan: boolean;
  canPrepareWorkspace: boolean;
  canRefreshWorkspace: boolean;
  canRetryGitConfirmation: boolean;
  canClose: boolean;
}

export function getAutopilotErrorActions(
  runStatus: AutopilotRunStatus,
  recoveryAction: AutopilotRecoveryAction,
): AutopilotErrorActions {
  const isStopped = runStatus === "Paused" || runStatus === "ErrorStopped";
  return {
    canResume:
      isStopped
      && recoveryAction !== "RestoreExecutionBaseline"
      && recoveryAction !== "WaitHumanDecision"
      && recoveryAction !== "RetryAutopilotAdvance"
      && recoveryAction !== "SyncAndClose"
      && recoveryAction !== "RegenerateExecutionPlan"
      && recoveryAction !== "PrepareExecutionWorkspace"
      && recoveryAction !== "ResolveWorkspaceChanges"
      && recoveryAction !== "RunAutomaticRecovery"
      && recoveryAction !== "RetryGitConfirmation",
    canRetryAdvance:
      runStatus === "ErrorStopped" && recoveryAction === "RetryAutopilotAdvance",
    canRegeneratePlan:
      runStatus === "ErrorStopped" && recoveryAction === "RegenerateExecutionPlan",
    canPrepareWorkspace:
      runStatus === "ErrorStopped" && recoveryAction === "PrepareExecutionWorkspace",
    canRefreshWorkspace:
      runStatus === "ErrorStopped" && recoveryAction === "ResolveWorkspaceChanges",
    canRetryGitConfirmation:
      runStatus === "ErrorStopped" && recoveryAction === "RetryGitConfirmation",
    canClose: isStopped,
  };
}
