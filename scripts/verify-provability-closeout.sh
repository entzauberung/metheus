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
  --profile core-dev provability_closeout_ -- --test-threads=2

cd "${REPO_ROOT}"
npx --no-install tsc --noEmit
npx --no-install vitest run src/TaskInspector.test.tsx

for required in \
  'acceptance_criteria_meta' \
  'preferred_mode_with_label' \
  'evidence_source_history' \
  'enter_human_review_boundary'; do
  if ! rg -q "${required}" src-tauri/src src/types.ts; then
    echo "可证明性链路缺少关键坐标：${required}" >&2
    exit 1
  fi
done

if ! rg -q '视觉、样式一致、体验、美观' src-tauri/src/prompts.rs; then
  echo "规划提示词缺少 HumanReview 边界" >&2
  exit 1
fi

if rg -ni 'websocket|web-socket' package.json src-tauri/Cargo.toml; then
  echo "本专项禁止引入 WebSocket 依赖" >&2
  exit 1
fi

git diff --check
