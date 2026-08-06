#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly CORE_MAX_JOBS=2
readonly VERIFY_SCOPE="${1:-full}"

if [[ "${VERIFY_SCOPE}" != "--static-only" ]]; then
  "${SCRIPT_DIR}/resource-preflight.sh" core
  export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
  export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"

  cd "${REPO_ROOT}/src-tauri"
  cargo fmt --package metheus -- --check
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev phase1_runtime_contract -- --test-threads=1

  cd "${REPO_ROOT}"
  npx --no-install tsc --noEmit
  npx --no-install vitest run \
    src/utils/invokeWithTimeout.test.ts \
    src/engineHealthSync.test.ts \
    src/projectSyncPolicy.test.ts \
    src/hooks/useProjectStateSync.test.tsx \
    src/executionSyncPolicy.test.ts \
    src/hooks/useTaskControlWorkspace.test.tsx \
    src/components/ApplicationSettings.test.tsx \
    src/components/ExecutionEngineSelector.test.tsx \
    src/components/AutopilotControlBar.test.tsx \
    src/components/RecoveryDecisionDialog.test.tsx \
    src/RecoveryImpactDialog.test.tsx \
    src/components/RecoveryResultBanner.test.tsx \
    src/components/RecoveryNotice.test.tsx \
    src/components/SyncStatusIndicator.test.tsx \
    src/V1ExecutionPanel.test.tsx \
    src/TaskConsole.test.tsx \
    src/TaskInspector.test.tsx \
    src/autopilotPolicy.test.ts \
    src/taskControlPolicy.test.ts
fi

cd "${REPO_ROOT}"

if ! rg -q 'PHASE1_DEFAULT_TASK_CONTROL_MODE: TaskControlMode = TaskControlMode::SerialTakeover' \
  src-tauri/src/task_control.rs; then
  echo "新项目默认模式必须为 SerialTakeover" >&2
  exit 1
fi

if ! rg -q 'command: "execute_control_action"' src-tauri/src/commands/workflow.rs; then
  echo "SerialTakeover 必须派发 execute_control_action" >&2
  exit 1
fi

if ! rg -q 'source_control_action_id' src-tauri/src/control_snapshot.rs \
  || ! rg -q 'isTaskControlSnapshotCurrent' src/hooks/useTaskControlWorkspace.ts \
  || ! rg -q 'waitingForAtomicSnapshot' src/hooks/useTaskControlWorkspace.ts \
  || ! rg -q 'taskControlFallbackDecision' src/hooks/useTaskControlWorkspace.ts; then
  echo "任务控制详细快照必须具备完整运行时游标保护" >&2
  exit 1
fi

if ! rg -q 'HumanTerminalAction::AcceptDeviation' src-tauri/src/control_action_executor.rs \
  || ! rg -q 'validate_recorded_human_acceptance' src-tauri/src/pipeline.rs \
  || ! rg -q 'capabilities: Vec<String>' src-tauri/src/control_snapshot.rs; then
  echo "人工终态动作和节点能力必须由后端统一裁决" >&2
  exit 1
fi

if rg -n 'getRecoveryStatusLabel|isValidationRecovery|getAutopilotErrorActions|getGitConfirmationBlockPresentation' \
  src --glob '!*.test.ts' --glob '!*.test.tsx'; then
  echo "恢复说明和动作裁决不得在前端策略中重复推导" >&2
  exit 1
fi

if rg -n '恢复前将先展示影响范围|后台重试进行中|最终状态可能延迟，请等待统一同步|恢复到提交：|后台作业：(已重新启动|未自动启动)' \
  src --glob '!*.test.ts' --glob '!*.test.tsx'; then
  echo "恢复动态文案必须来自后端展示模型" >&2
  exit 1
fi

for scenario in {1..13}; do
  if ! rg -q "场景 ${scenario}：" docs/phase1-runtime-acceptance.md; then
    echo "运行时验收文档缺少场景 ${scenario}" >&2
    exit 1
  fi
done

if rg -n 'Core 自动化契约：尚未|真实桌面烟雾：尚未执行；待' \
  docs/phase1-runtime-acceptance.md; then
  echo "运行时验收文档仍包含待定执行记录" >&2
  exit 1
fi

if ! rg -q '真实桌面烟雾：(未执行|已执行)' docs/phase1-runtime-acceptance.md; then
  echo "运行时验收文档必须明确记录桌面烟雾结果" >&2
  exit 1
fi

"${SCRIPT_DIR}/verify-phase1-closeout.sh" --static-only
git diff --check
