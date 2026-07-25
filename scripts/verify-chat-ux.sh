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
cargo test --locked --package metheus --lib --no-default-features --profile core-dev api::tests -- --test-threads=1
cargo test --locked --package metheus --lib --no-default-features --profile core-dev chat -- --test-threads=1

cd "${REPO_ROOT}"
npm run test:chat-policy
npm run typecheck
npm run build
git diff --check

echo "聊天专项验证通过：已覆盖 Rust API/持久化与前端策略/生命周期；未启用 builtin-grok，未编译 Grok Build，未运行 Tauri dev/build，未发送真实模型请求。"
