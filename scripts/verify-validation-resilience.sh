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
for test_filter in \
  review_protocol::tests \
  quality_gate::tests \
  recovery::tests \
  autopilot_runtime::tests; do
  cargo test --locked --package metheus --lib --no-default-features --profile core-dev \
    "${test_filter}" -- --test-threads=1
done

cd "${REPO_ROOT}"
npm run typecheck
npx vitest run \
  src/autopilotPolicy.test.ts \
  src/components/AutopilotControlBar.test.tsx \
  src/logPolicy.test.ts
git diff --check
