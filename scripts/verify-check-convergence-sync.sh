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
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev check_convergence_ -- --test-threads=1

cd "${REPO_ROOT}"
npx --no-install tsc --noEmit
npx --no-install vitest run \
  src/projectSyncPolicy.test.ts \
  src/hooks/useProjectStateSync.test.tsx

if rg -n 'serde_json::from_str\(&response.content\)' \
  src-tauri/src/commands/milestone.rs; then
  echo "大阶段、中阶段或执行计划检查仍存在响应裸解析" >&2
  exit 1
fi

for contract in \
  MILESTONE_CHECK_JSON_CONTRACT \
  MID_STAGE_CHECK_JSON_CONTRACT \
  EXECUTION_PLAN_CHECK_JSON_CONTRACT; do
  if ! rg -q "${contract}" src-tauri/src/json_utils.rs src-tauri/src/commands/milestone.rs; then
    echo "缺少检查 JSON 契约：${contract}" >&2
    exit 1
  fi
done

if ! rg -q 'check_execution_plan\(&mid.subtasks\)' \
  src-tauri/src/commands/milestone.rs \
  || ! rg -q 'mod plan_deterministic_checks' src-tauri/src/lib.rs; then
  echo "执行计划确定性预检未接到检查入口" >&2
  exit 1
fi

if ! rg -q 'PROJECT_SYNC_CONNECTED_FALLBACK_MS = 60_000' src/projectSyncPolicy.ts \
  || ! rg -q 'shouldRequestRuntimeSnapshot' src/hooks/useProjectStateSync.ts; then
  echo "同步修订门禁或健康 Channel 低频兜底缺失" >&2
  exit 1
fi

if rg -ni 'websocket|web-socket' package.json src-tauri/Cargo.toml; then
  echo "本专项禁止引入 WebSocket 依赖" >&2
  exit 1
fi

git diff --check
