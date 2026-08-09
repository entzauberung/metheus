#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly GROK_TARGET_DIR="${REPO_ROOT}/.build/grok-full"
readonly CORE_MAX_JOBS=2
readonly GROK_MAX_JOBS=1
readonly -a ADAPTIVE_FRONTEND_TESTS=(
  src/PlanApprovalPanel.test.tsx
  src/planTargetPolicy.test.ts
  src/ExecutionTree.test.tsx
  src/V1ExecutionPanel.test.tsx
  src/console/ExecutionPlanStep.test.tsx
  src/TaskInspector.test.tsx
  src/components/ConsoleWorkspace.test.tsx
  src/TaskConsole.test.tsx
  src/MilestoneHumanReviewDialog.test.tsx
  src/MilestoneReviewPanel.test.tsx
  src/consoleWritePolicy.test.ts
  src/components/ApplicationSettings.test.tsx
  src/logPolicy.test.ts
)
readonly -a REQUIRED_CLOSEOUT_FILES=(
  src-tauri/src/vision_review.rs
  src/ConsoleWorkspace.css
  src/MilestoneHumanReviewDialog.test.tsx
  src/MilestoneHumanReviewDialog.tsx
  src/MilestoneReviewPanel.test.tsx
  src/components/ConsoleBottomPanel.tsx
  src/components/ConsoleNavigator.tsx
  src/components/ConsoleWorkspace.test.tsx
  src/components/ConsoleWorkspace.tsx
  src/consoleWritePolicy.test.ts
  src/consoleWritePolicy.ts
)

cd "${REPO_ROOT}"

rust_before_tests() {
  awk '
    /^[[:space:]]*#[[:space:]]*\[[[:space:]]*cfg[[:space:]]*\([[:space:]]*test[[:space:]]*\)[[:space:]]*\]/ {
      exit
    }
    { print }
  ' "$1"
}

audit_worktree() {
  local staged_count=0
  local unstaged_count=0
  local untracked_count=0
  local generated_untracked_count=0
  local status_code path status_line
  local -a delivery_untracked_paths=()

  while IFS= read -r status_line; do
    [[ -z "${status_line}" ]] && continue
    status_code="${status_line:0:2}"
    path="${status_line:3}"
    if [[ "${status_code}" == "??" ]]; then
      untracked_count=$((untracked_count + 1))
      if [[ "${path}" == src-tauri/.build/* ]]; then
        generated_untracked_count=$((generated_untracked_count + 1))
      else
        delivery_untracked_paths+=("${path}")
      fi
      continue
    fi
    if [[ "${status_code:0:1}" != " " ]]; then
      staged_count=$((staged_count + 1))
    fi
    if [[ "${status_code:1:1}" != " " ]]; then
      unstaged_count=$((unstaged_count + 1))
    fi
  done < <(git status --porcelain=v1 --untracked-files=all)

  echo "[adaptive] worktree audit: staged=${staged_count} unstaged=${unstaged_count} untracked=${untracked_count} generated_untracked=${generated_untracked_count} delivery_untracked=${#delivery_untracked_paths[@]}"
  echo "[adaptive] staged paths"
  git diff --cached --name-status
  echo "[adaptive] unstaged tracked paths"
  git diff --name-status
  echo "[adaptive] dependency manifest status"
  git status --short -- \
    package.json package-lock.json \
    src-tauri/Cargo.toml src-tauri/Cargo.lock \
    third_party/grok-build-fork/Cargo.toml \
    third_party/grok-build-fork/Cargo.lock
  echo "[adaptive] untracked delivery paths (generated src-tauri/.build entries counted above)"
  if [[ "${#delivery_untracked_paths[@]}" -eq 0 ]]; then
    echo "  (none)"
  else
    printf '  %s\n' "${delivery_untracked_paths[@]}"
  fi
}

require_files() {
  local required_file
  for required_file in "$@"; do
    if [[ ! -f "${required_file}" ]]; then
      echo "验收清单缺少文件：${required_file}" >&2
      exit 1
    fi
  done
}

echo "[adaptive] static policy checks"
audit_worktree
require_files "${ADAPTIVE_FRONTEND_TESTS[@]}"
require_files "${REQUIRED_CLOSEOUT_FILES[@]}"
echo "[adaptive] selected frontend test files: ${#ADAPTIVE_FRONTEND_TESTS[@]}"
echo "[adaptive] required closeout files: ${#REQUIRED_CLOSEOUT_FILES[@]}"
if rg -n 'ProjectMode|project\.mode|proj\.mode' src src-tauri/src; then
  echo "生产代码仍包含已删除的 ProjectMode 契约" >&2
  exit 1
fi
if rg -n '3-5个大阶段|2-5个中阶段|2-8个|MAX_DEFAULT_SPLIT_DEPTH' \
  src src-tauri/src; then
  echo "仍包含固定阶段数量或默认八层 split 策略" >&2
  exit 1
fi
if rg -n -- 'METHEUS_MODEL|--model' <<<"$(rust_before_tests src-tauri/src/engine/claude_code.rs)"; then
  echo "Claude Code adapter 不得注入模型覆盖" >&2
  exit 1
fi
if rg -n -- '--yolo' <<<"$(rust_before_tests src-tauri/src/engine/kimi_cli.rs)"; then
  echo "Kimi CLI adapter 仍包含冲突的 --yolo 参数" >&2
  exit 1
fi
if rg -n 'environment_remove[^;]*UPSTREAM_GROK_API_KEY_ENV' <<<"$(rust_before_tests src-tauri/src/engine/grok_cli.rs)"; then
  echo "外部 Grok CLI 不得移除用户 XAI_API_KEY" >&2
  exit 1
fi
execution_profile_definition="$(
  awk '
    /^[[:space:]]*pub[[:space:]]+struct[[:space:]]+ExecutionProfile[[:space:]]*\{/ {
      inside = 1
    }
    inside {
      print
      if ($0 ~ /^[[:space:]]*\}[[:space:]]*$/) {
        exit
      }
    }
  ' src-tauri/src/project.rs
)"
if rg -n '^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?[[:alnum:]_]*model[[:alnum:]_]*[[:space:]]*:' \
  <<<"${execution_profile_definition}"; then
  echo "ExecutionProfile 不得增加外部插件模型字段" >&2
  exit 1
fi
if [[ -z "${execution_profile_definition}" ]]; then
  echo "无法定位 ExecutionProfile 定义" >&2
  exit 1
fi

if rg -n '(error|message|failure|reason).*(includes|startsWith|match)[[:space:]]*\([^)]*(恢复|同步|重试|超时)' \
  src/App.tsx src/components/AutopilotControlBar.tsx \
  src/consoleWritePolicy.ts; then
  echo "Console 恢复能力不得通过错误文本推导" >&2
  exit 1
fi
if rg -n 'pause_managed_flow|resume_managed_flow|stop_managed_flow' \
  src/ConsoleWorkflowPanel.tsx; then
  echo "托管暂停、恢复和停止入口只能位于顶部命令栏" >&2
  exit 1
fi
if rg -n 'onSync|onPause|onResume|TaskConsole|执行日志' \
  src/V1ExecutionPanel.tsx; then
  echo "执行面板不得重复提供同步、暂停、恢复或日志入口" >&2
  exit 1
fi

mapfile -t milestone_review_writes < <(
  while IFS= read -r rust_file; do
    awk -v path="${rust_file}" '
      /^[[:space:]]*#[[:space:]]*\[[[:space:]]*cfg[[:space:]]*\([[:space:]]*test[[:space:]]*\)[[:space:]]*\]/ {
        exit
      }
      {
        compact = $0
        gsub(/[[:space:]]/, "", compact)
        window = window compact
        while (match(window, /current_step=project::WorkflowStep::MilestoneReview/)) {
          printf "%s:%d\n", path, NR
          window = substr(window, RSTART + RLENGTH)
        }
        if (length(window) > 512) {
          window = substr(window, length(window) - 511)
        }
      }
    ' "${rust_file}"
  done < <(rg --files src-tauri/src -g '*.rs')
)

if [[ "${#milestone_review_writes[@]}" -ne 1 ]] \
  || [[ "${milestone_review_writes[0]%:*}" != "src-tauri/src/workflow_resolution.rs" ]]; then
  echo "MilestoneReview 生产直接写入必须恰好一处且位于 workflow_resolution.rs；实际匹配：" >&2
  if [[ "${#milestone_review_writes[@]}" -eq 0 ]]; then
    echo "  （无）" >&2
  else
    printf '  %s\n' "${milestone_review_writes[@]}" >&2
  fi
  exit 1
fi
if ! rg -q \
  '^[[:space:]]*pub\(crate\)[[:space:]]+fn[[:space:]]+apply_milestone_review_boundary[[:space:]]*\(' \
  src-tauri/src/workflow_resolution.rs; then
  echo "workflow_resolution.rs 缺少 apply_milestone_review_boundary 共享边界" >&2
  exit 1
fi

while IFS= read -r path; do
  if [[ "${path}" != "src-tauri/src/engine/builtin.rs" ]]; then
    echo "业务模块不得直接引用 metheus-grok-engine：${path}" >&2
    exit 1
  fi
done < <(rg -l 'metheus_grok_engine' src-tauri/src || true)

if [[ -n "$(git diff --name-only -- third_party/grok-build)" ]]; then
  echo "pristine Grok Build 审计基线被修改" >&2
  exit 1
fi

mapfile -t allowed_fork_paths < <(
  sed -n 's/^| `\([^`]*\)` |.*/\1/p' third_party/grok-build-fork/PATCHSET.md
)
while IFS= read -r difference; do
  [[ -z "${difference}" ]] && continue
  fork_path=""
  if [[ "${difference}" == "Files third_party/grok-build/"* ]]; then
    comparison="${difference#Files third_party/grok-build/}"
    pristine_path="${comparison%% and third_party/grok-build-fork/*}"
    fork_path="${comparison#* and third_party/grok-build-fork/}"
    if [[ "${fork_path}" == "${comparison}" || "${fork_path}" != *" differ" ]]; then
      echo "无法解析 Grok fork 差异：${difference}" >&2
      exit 1
    fi
    fork_path="${fork_path% differ}"
    if [[ "${pristine_path}" != "${fork_path}" ]]; then
      echo "Grok fork 差异路径不一致：${difference}" >&2
      exit 1
    fi
  elif [[ "${difference}" == "Only in third_party/grok-build-fork"* ]]; then
    location_and_name="${difference#Only in third_party/grok-build-fork}"
    if [[ "${location_and_name}" != *": "* ]]; then
      echo "无法解析 Grok fork 独有路径：${difference}" >&2
      exit 1
    fi
    location="${location_and_name%%: *}"
    name="${location_and_name#*: }"
    location="${location#/}"
    if [[ -n "${location}" ]]; then
      fork_path="${location}/${name}"
    else
      fork_path="${name}"
    fi
  else
    echo "未登记或无法映射到 fork 的 Grok 差异：${difference}" >&2
    exit 1
  fi
  matched=false
  for allowed in "${allowed_fork_paths[@]}"; do
    if [[ "${fork_path}" == "${allowed}" ]]; then
      matched=true
      break
    fi
  done
  if [[ "${matched}" != true ]]; then
    echo "未登记的 Grok fork 差异：${difference}" >&2
    exit 1
  fi
done < <(diff -qr third_party/grok-build third_party/grok-build-fork || true)

host_continuation_test_count="$(
  awk '
    /^[[:space:]]*#\[(tokio::)?test/ {
      if (getline <= 0) {
        next
      }
      name = $0
      sub(/^[[:space:]]*(async[[:space:]]+)?fn[[:space:]]+/, "", name)
      sub(/\(.*/, "", name)
      if (name ~ /^continuation_/) {
        count++
      }
    }
    END { print count + 0 }
  ' src-tauri/crates/metheus-grok-engine/src/runtime.rs
)"
if [[ "${host_continuation_test_count}" -ne 9 ]]; then
  echo "Host continuation 测试数量必须为 9；实际 ${host_continuation_test_count}" >&2
  exit 1
fi
fork_runtime_test_count="$(rg -c '^#\[(tokio::)?test' \
  third_party/grok-build-fork/crates/codegen/xai-grok-shell/tests/metheus_embedded_runtime.rs)"
if [[ "${fork_runtime_test_count}" -ne 12 ]]; then
  echo "Fork embedded runtime 测试数量必须为 12；实际 ${fork_runtime_test_count}" >&2
  exit 1
fi
if ! rg -q \
  'fn adaptive_grok_contract_interruptions_preserve_usage_files_and_output' \
  src-tauri/src/engine/builtin.rs; then
  echo "BuiltIn Timeout/Cancelled 事实传播测试未纳入 adaptive_grok_contract" >&2
  exit 1
fi
if ! rg -q \
  'fn adaptive_execution_contract_builtin_interruptions_reach_cost_ledger_with_facts' \
  src-tauri/src/pipeline.rs; then
  echo "Pipeline Timeout/Cancelled 成本账本传播测试未纳入 adaptive_execution_contract" >&2
  exit 1
fi

for revision_file in \
  third_party/grok-build-fork/FORK_SOURCE.md \
  third_party/grok-build-fork/PATCHSET.md \
  third_party/grok-build-fork/UPSTREAM_SOURCE.md \
  third_party/grok-build-fork/crates/codegen/xai-grok-shell/src/metheus_embedded.rs \
  src-tauri/crates/metheus-grok-engine/src/lib.rs; do
  if ! rg -q 'metheus\.4' "${revision_file}"; then
    echo "Grok fork 修订身份不一致：${revision_file}" >&2
    exit 1
  fi
done

echo "[adaptive] core track (.build/core, jobs=2, two Cargo tasks)"
"${SCRIPT_DIR}/resource-preflight.sh" core
export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"
cd "${REPO_ROOT}/src-tauri"
core_tree="$(cargo tree --locked --package metheus --no-default-features --edges normal)"
if rg -q 'metheus-grok-engine|xai-grok-' <<<"${core_tree}"; then
  echo "Core no-default-features 依赖图解析了 Grok Build" >&2
  exit 1
fi
cargo test --locked --package metheus --lib --no-default-features \
  --profile core-dev adaptive_execution_contract -- --test-threads=1

echo "[adaptive] frontend contract track"
cd "${REPO_ROOT}"
npx --no-install tsc --noEmit
npx --no-install vitest run "${ADAPTIVE_FRONTEND_TESTS[@]}"

echo "[adaptive] Grok adapter track (.build/grok-full, jobs=1, one Cargo task)"
"${SCRIPT_DIR}/resource-preflight.sh" grok
export CARGO_TARGET_DIR="${GROK_TARGET_DIR}"
export CARGO_BUILD_JOBS="${GROK_MAX_JOBS}"
cd "${REPO_ROOT}/src-tauri"
cargo test --locked --workspace --no-default-features \
  --features metheus/builtin-grok --profile grok-check \
  adaptive_grok_contract -- --test-threads=1
cd "${REPO_ROOT}"
cargo test --locked \
  --manifest-path src-tauri/crates/metheus-grok-engine/Cargo.toml \
  --lib continuation_ -- --test-threads=1
cargo test --locked \
  --manifest-path third_party/grok-build-fork/Cargo.toml \
  -p xai-grok-shell --features metheus-embedded \
  --test metheus_embedded_runtime

cd "${REPO_ROOT}"
git diff --check
echo "[adaptive] closeout verification passed"
