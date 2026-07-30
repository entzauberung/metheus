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

readonly RUST_FILTERS=(
  "workflow_resolution_"
  "mid_stage_initial_contract_"
  "workflow_closure_migration_"
  "workflow_closure_e2e_"
  "managed_legacy_empty_plan_approval_generates_instead_of_waiting"
  "managed_generation_reuses_existing_valid_plan_draft"
  "managed_milestone_check_"
  "managed_runtime_supports_milestone_regeneration_action"
  "startup_reconciliation_assigns_a_new_backend_job_identity"
  "backend_runtime_finishes_a_reached_target_without_a_ui_loop"
  "future_draft_source_requires_same_thread_revision_and_project_facts"
  "future_discussion_keeps_but_expires_the_existing_draft"
  "planning_constraints_recursively_scope_dynamic_task_facts"
  "future_planning_context_"
  "managed_action_timeout_"
  "branches_b_and_c_pause_autopilot_and_reject_duplicate_submit"
)

for filter in "${RUST_FILTERS[@]}"; do
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev "${filter}" -- --nocapture
done

cd "${REPO_ROOT}"
npx tsc --noEmit
npx vitest run src/managedFlowPolicy.test.ts src/FuturePlanningWorkspace.test.tsx

if rg -q 'runManagedCycle|managed_next_step' src --glob '*.ts' --glob '*.tsx'; then
  echo "前端不得包含托管动作选择或命令循环。" >&2
  exit 1
fi

if rg -q 'handleTransition\("MidStageGeneration"\)|handleTransition\("PlanGeneration"\)' \
  src --glob '*.ts' --glob '*.tsx'; then
  echo "前端不得硬编码中阶段事实路由。" >&2
  exit 1
fi

if ! rg -q 'continue_current_milestone' src/ConsoleWorkflowPanel.tsx; then
  echo "手动继续必须调用后端事实解析命令。" >&2
  exit 1
fi

if ! rg -q 'FuturePlanApproval.*BranchDiscussion|BranchDiscussion.*FuturePlanApproval' src/App.tsx; then
  echo "未来讨论和审批必须路由到同一工作区。" >&2
  exit 1
fi

if ! rg -q '已有执行中或已完成的中阶段，禁止整表替换' \
  src-tauri/src/commands/milestone.rs; then
  echo "中阶段整表替换安全保护缺失。" >&2
  exit 1
fi

git diff --check
