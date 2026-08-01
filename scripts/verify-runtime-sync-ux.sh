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

readonly RUST_FILTERS=(
  "project_state_bus::tests"
  "runtime_snapshot::tests"
  "recovery_presentation::tests"
  "recovery_preview"
  "recovery_rejects_preview_after_workspace_changes"
  "session_lost_acknowledge_restores_baseline"
  "concurrent_automatic_recovery_is_idempotent_and_clears_pipeline"
)

for filter in "${RUST_FILTERS[@]}"; do
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev "${filter}" -- --test-threads=1
done

cd "${REPO_ROOT}"
npx --no-install tsc --noEmit
npx --no-install vitest run \
  src/projectSyncPolicy.test.ts \
  src/hooks/useProjectStateSync.test.tsx \
  src/executionSyncPolicy.test.ts \
  src/hooks/useTaskControlWorkspace.test.tsx \
  src/components/AutopilotControlBar.test.tsx \
  src/components/RecoveryDecisionDialog.test.tsx \
  src/V1ExecutionPanel.test.tsx \
  src/RecoveryImpactDialog.test.tsx \
  src/components/SyncStatusIndicator.test.tsx \
  src/components/RecoveryNotice.test.tsx

git diff --check
