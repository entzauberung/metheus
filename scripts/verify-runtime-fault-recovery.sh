#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly CORE_MAX_JOBS=2

"${SCRIPT_DIR}/resource-preflight.sh" runtime-fault

FAULT_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/metheus-runtime-fault.XXXXXX")"
readonly FAULT_ROOT
cleanup() {
  if [[ -n "${FAULT_ROOT:-}" && -d "${FAULT_ROOT}" ]]; then
    rm -rf -- "${FAULT_ROOT}"
  fi
}
trap cleanup EXIT

git -C "${REPO_ROOT}" status --porcelain=v1 --untracked-files=all \
  > "${FAULT_ROOT}/repo-state.before"

node - "${FAULT_ROOT}" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const root = process.argv[2];
const staleStartedAt = "2000-01-01T00:00:00Z";
const staleHeartbeatAt = "2000-01-01T00:00:05Z";

function lease(actionId, owner, actionKind, taskId) {
  return {
    action_id: actionId,
    owner_process_start_id: owner,
    action_kind: actionKind,
    task_id: taskId,
    started_at: staleStartedAt,
    heartbeat_at: staleHeartbeatAt,
    expected_max_duration_secs: 900,
  };
}

const fixtures = {
  "old-process-owner": {
    name: "runtime-fault-old-process-owner",
    task_control: {
      active_action_id: "old-owner-action",
      active_action_lease: lease(
        "old-owner-action",
        "retired-process-start-id",
        "execute",
        "task-old-owner",
      ),
    },
  },
  "heartbeat-timeout": {
    name: "runtime-fault-heartbeat-timeout",
    task_control: {
      active_action_id: "expired-heartbeat-action",
      active_action_lease: lease(
        "expired-heartbeat-action",
        "fixture-current-process",
        "repair",
        "task-expired-heartbeat",
      ),
    },
  },
  "unfinished-git-confirmation": {
    name: "runtime-fault-unfinished-git-confirmation",
    task_control: {
      active_action_id: "interrupted-git-confirm-action",
      active_action_lease: lease(
        "interrupted-git-confirm-action",
        "retired-process-start-id",
        "git_confirm",
        "task-git-confirm",
      ),
    },
    execution_session: {
      status: "confirming",
      confirmation_transaction_id: "transaction-interrupted",
      confirmation_phase: "CommitCreated",
      confirmation_commit: "commit-interrupted",
    },
  },
  "legacy-string-lock": {
    name: "runtime-fault-legacy-string-lock",
    task_control: {
      active_action_id: "legacy-action",
    },
  },
};

for (const [fixtureName, fixture] of Object.entries(fixtures)) {
  const fixtureDir = path.join(root, fixtureName);
  fs.mkdirSync(fixtureDir, { recursive: true });
  fs.writeFileSync(
    path.join(fixtureDir, "project.json"),
    `${JSON.stringify(fixture, null, 2)}\n`,
    { flag: "wx" },
  );
}

if (Object.keys(fixtures).length !== 4) {
  throw new Error("必须构造四类控制锁故障夹具");
}
NODE

readonly REQUIRED_RUNTIME_TESTS=(
  "runtime_fault_stale_lock_reconciliation_clears_and_audits_expired_lease"
  "runtime_fault_lock_lease_expired_heartbeat_is_stale"
  "runtime_fault_stale_lock_reconciliation_preserves_git_transaction_facts"
  "runtime_fault_stale_lock_reconciliation_legacy_lock_requires_human_audit"
  "runtime_fault_json_contract_compresses_object_string_after_one_repair"
  "runtime_fault_json_contract_normalizes_unknown_enum_safely"
  "runtime_fault_json_contract_stops_after_same_error_without_progress"
)

for test_name in "${REQUIRED_RUNTIME_TESTS[@]}"; do
  if ! rg -q "fn ${test_name}" "${REPO_ROOT}/src-tauri/src"; then
    echo "运行期故障验证缺少定向测试：${test_name}" >&2
    exit 1
  fi
done

export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"

cd "${REPO_ROOT}/src-tauri"
cargo fmt --package metheus -- --check
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev runtime_fault -- --test-threads=1

cd "${REPO_ROOT}"
npx --no-install vitest run \
  src/components/AutopilotControlBar.test.tsx \
  src/TaskInspector.test.tsx
npx --no-install tsc --noEmit
git diff --check

git status --porcelain=v1 --untracked-files=all > "${FAULT_ROOT}/repo-state.after"
if ! cmp -s "${FAULT_ROOT}/repo-state.before" "${FAULT_ROOT}/repo-state.after"; then
  echo "故障注入验证修改了主仓库文件状态，拒绝通过。" >&2
  diff -u "${FAULT_ROOT}/repo-state.before" "${FAULT_ROOT}/repo-state.after" >&2 || true
  exit 1
fi

echo "运行期故障恢复验证通过：四类锁夹具均位于临时目录，Rust/前端/TypeScript 门禁通过，主仓库文件状态未变化。"
