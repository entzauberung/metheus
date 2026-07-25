#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly GROK_TARGET_DIR="${REPO_ROOT}/.build/grok-full"
readonly GROK_MAX_JOBS=1

"${SCRIPT_DIR}/resource-preflight.sh" grok
export CARGO_TARGET_DIR="${GROK_TARGET_DIR}"
export CARGO_BUILD_JOBS="${GROK_MAX_JOBS}"

cd "${REPO_ROOT}/src-tauri"
cargo check --locked --package metheus --lib --no-default-features --features builtin-grok --profile grok-check
