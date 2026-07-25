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
cargo test --locked --package metheus --lib --no-default-features --profile core-dev acceptance::tests -- --test-threads=1
cargo test --locked --package metheus --lib --no-default-features --profile core-dev quality_gate::tests -- --test-threads=1
cargo test --locked --package metheus --lib --no-default-features --profile core-dev evidence_ -- --test-threads=1
cargo test --locked --package metheus --lib --no-default-features --profile core-dev recovery::tests -- --test-threads=1

cd "${REPO_ROOT}"
npm run typecheck
npm run test:autopilot-policy
