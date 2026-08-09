#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly CORE_MAX_JOBS=2

static_only=false
if [[ "${1:-}" == "--static-only" ]]; then
  static_only=true
elif [[ -n "${1:-}" ]]; then
  echo "用法：$0 [--static-only]" >&2
  exit 2
fi
readonly static_only

readonly RUST_2021_FILES=(
  "src-tauri/src/api.rs"
  "src-tauri/src/commands/task_control.rs"
  "src-tauri/src/cost_ledger.rs"
  "src-tauri/src/engine/builtin.rs"
  "src-tauri/src/engine/service.rs"
  "src-tauri/src/pipeline.rs"
  "src-tauri/src/project.rs"
  "src-tauri/src/project_state_bus.rs"
  "src-tauri/src/recovery.rs"
  "src-tauri/src/task_compiler.rs"
)
readonly RUST_2024_FILES=(
  "src-tauri/crates/metheus-grok-engine/src/config.rs"
  "src-tauri/crates/metheus-grok-engine/src/event_bridge.rs"
  "src-tauri/crates/metheus-grok-engine/src/lib.rs"
  "src-tauri/crates/metheus-grok-engine/src/runtime.rs"
)

cd "${REPO_ROOT}"
rustfmt --edition 2021 --check "${RUST_2021_FILES[@]}"
rustfmt --edition 2024 --check "${RUST_2024_FILES[@]}"

if [[ "${static_only}" == false ]]; then
  "${SCRIPT_DIR}/resource-preflight.sh" core
  export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
  export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"
  cd "${REPO_ROOT}/src-tauri"
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev runtime_fix_ -- --test-threads=1
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev api::tests -- --test-threads=1
  cd "${REPO_ROOT}"
fi

readonly EVENT_TEST_DIR="$(mktemp -d)"
rustc --edition 2024 --test \
  src-tauri/crates/metheus-grok-engine/src/event_bridge.rs \
  -o "${EVENT_TEST_DIR}/event_bridge_tests"
"${EVENT_TEST_DIR}/event_bridge_tests" --test-threads=1

npx --no-install tsc --noEmit
npx --no-install vitest run \
  src/projectSyncPolicy.test.ts \
  src/hooks/useProjectStateSync.test.tsx

if rg -n 'current_head != session\.base_commit' src-tauri/src/pipeline.rs; then
  echo "受管改动识别仍错误依赖 HEAD 等于会话基线" >&2
  exit 1
fi
if rg -n 'response\.json\(\)\.await' src-tauri/src/api.rs; then
  echo "OpenAI Compatible 普通响应仍使用不透明 response.json 解码" >&2
  exit 1
fi
for split_source in \
  src-tauri/src/task_compiler.rs \
  src-tauri/src/commands/task_control.rs; do
  if sed '/^#\[cfg(test)\]/,$d' "${split_source}" \
    | rg -n 'required_identifiers|acceptance_identifiers|extract_backtick_tokens'; then
    echo "split 生产代码仍把标识符作为拆分维度：${split_source}" >&2
    exit 1
  fi
done
if rg -n 'MAX_CHILD_TASKS_PER_SPLIT' src-tauri/src; then
  echo "split 叶子上限仍使用过时常量名" >&2
  exit 1
fi

for contract in \
  'MAX_SPLIT_LEAVES: usize = 4' \
  'record_execution_call' \
  'GrokBuildRuntimeEvent::TokenUsage' \
  'runtime_dirty' \
  'TEXT_AGGREGATION_WINDOW'; do
  if ! rg -q "${contract}" src-tauri/src src-tauri/crates/metheus-grok-engine/src src; then
    echo "缺少运行期收口契约：${contract}" >&2
    exit 1
  fi
done

for constitution_rule in \
  '依赖引入原则' \
  '自适应执行原则' \
  '任务控制模式事实' \
  '桌面进程通信原则' \
  '执行器参数边界' \
  '受管改动语义' \
  'split 按独立产物拆分' \
  '新增执行路径收口' \
  'Grok Build 内置 token 可见' \
  '流式执行事件聚合'; do
  if ! rg -q "${constitution_rule}" CONSTITUTION.md; then
    echo "宪法缺少规则：${constitution_rule}" >&2
    exit 1
  fi
done

if rg -n 'dangerously-skip-permissions|当前安装的依赖（本轮不新增）|不在 MVP 阶段引入 WebSocket' \
  CONSTITUTION.md; then
  echo "宪法仍含现行过时硬约束" >&2
  exit 1
fi
# 宪法同步日期会随已验收事实推进；本脚本校验上面的运行期规则内容，
# 不再用历史交付日冻结后续合法修订。

if [[ -n "$(git diff --name-only -- Cargo.lock package-lock.json pnpm-lock.yaml yarn.lock src-tauri/Cargo.lock src-tauri/Cargo.toml package.json)" ]]; then
  echo "检测到依赖清单或锁文件改动" >&2
  exit 1
fi

git diff --check
