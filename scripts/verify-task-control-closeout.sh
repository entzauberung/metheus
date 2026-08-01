#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly CORE_MAX_JOBS=2

"${SCRIPT_DIR}/resource-preflight.sh" core
export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"

cd "${REPO_ROOT}/src-tauri"
cargo fmt --package metheus -- --check
cargo check --locked --package metheus --lib --no-default-features --profile core-dev

readonly MODULE_FILTERS=(
  "task_tree::tests"
  "task_aggregation::tests"
  "task_compiler::tests"
  "task_control::tests"
  "control_action_executor::tests"
  "control_scheduler::tests"
  "control_snapshot::tests"
  "validator_registry::tests"
  "review_evidence::tests"
  "automated_validation::tests"
  "acceptance::tests"
  "quality_gate::tests"
  "recovery::tests"
  "cost_ledger::tests"
  "api::tests"
)

for filter in "${MODULE_FILTERS[@]}"; do
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev "${filter}" -- --nocapture
done

cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev \
  commands::workflow::tests::phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits \
  -- --exact --nocapture
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev \
  commands::workflow::tests::phase1_runtime_contract_serial_takeover_dispatches_control_action \
  -- --exact --nocapture
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev \
  commands::workflow::tests::missing_target_and_rejected_subtask_persist_error_stopped \
  -- --exact --nocapture
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev \
  autopilot_runtime::tests::installing_same_job_identity_keeps_one_runtime_job \
  -- --exact --nocapture
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev \
  pipeline::tests::execution_failure_updates_only_nested_leaf \
  -- --exact --nocapture

cd "${REPO_ROOT}"
npm run typecheck
npm test

if ! rg -q '<TaskInspector' src/App.tsx; then
  echo "App must render the shared TaskInspector" >&2
  exit 1
fi

if rg -q 'value="control"|TaskControlCenter' src/TaskConsole.tsx; then
  echo "TaskConsole must not embed a second task-control page" >&2
  exit 1
fi

if rg -q 'TaskControlTree|task-control-tree' src --glob '*.tsx'; then
  echo "Only ExecutionTree may render the visible task tree" >&2
  exit 1
fi

if ! rg -q 'child_tasks' src/ExecutionTree.tsx; then
  echo "ExecutionTree must retain recursive child_tasks rendering" >&2
  exit 1
fi

if rg -q 'mid(Stage)?\?*\.subtasks\.(find|filter)' src/App.tsx; then
  echo "App execution and recovery paths must not use shallow subtask lookup" >&2
  exit 1
fi

if rg -q 'flatMap\([^)]*=>[^)]*\.subtasks' src/components/AutopilotControlBar.tsx; then
  echo "Autopilot recovery must not flatten only first-level subtasks" >&2
  exit 1
fi

targeted_validation_body="$(sed -n '/async fn run_targeted_validation/,/^fn merge_ledger_updates/p' src-tauri/src/control_action_executor.rs)"
if ! rg -q 'review_subtask_with_context_and_model' <<<"${targeted_validation_body}"; then
  echo "TargetedValidate must use the review-only path without rerunning project tests" >&2
  exit 1
fi
if rg -q 'check_subtask_with_context_and_model' <<<"${targeted_validation_body}"; then
  echo "TargetedValidate must not use the combined test-and-review path" >&2
  exit 1
fi

evidence_recovery_body="$(sed -n '/if recovery.error_kind == project::RecoveryErrorKind::EvidenceInsufficient/,/if matches!/p' src-tauri/src/recovery.rs)"
if ! rg -q 'RecoveryRetestKind::ReviewOnly' <<<"${evidence_recovery_body}"; then
  echo "EvidenceInsufficient recovery must reuse automated test facts" >&2
  exit 1
fi

if ! rg -q '"verify:task-control-closeout"' package.json; then
  echo "The task-control closeout script must be exposed through package scripts" >&2
  exit 1
fi

git diff --check
