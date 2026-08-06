#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly GROK_TARGET_DIR="${REPO_ROOT}/.build/grok-full"
readonly CORE_MAX_JOBS=2
readonly GROK_MAX_JOBS=1

cd "${REPO_ROOT}"

echo "[adaptive] static policy checks"
if rg -n 'ProjectMode|project\.mode|proj\.mode' src src-tauri/src; then
  echo "生产代码仍包含已删除的 ProjectMode 契约" >&2
  exit 1
fi
if rg -n '3-5个大阶段|2-5个中阶段|2-8个|MAX_DEFAULT_SPLIT_DEPTH' \
  src src-tauri/src; then
  echo "仍包含固定阶段数量或默认八层 split 策略" >&2
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
  matched=false
  for allowed in "${allowed_fork_paths[@]}"; do
    if [[ "${difference}" == *"third_party/grok-build-fork/${allowed}"* ]]; then
      matched=true
      break
    fi
    allowed_dir="${allowed%/*}"
    allowed_name="${allowed##*/}"
    if [[ "${allowed_dir}" == "${allowed}" ]]; then
      expected="Only in third_party/grok-build-fork: ${allowed_name}"
    else
      expected="Only in third_party/grok-build-fork/${allowed_dir}: ${allowed_name}"
    fi
    if [[ "${difference}" == "${expected}" ]]; then
      matched=true
      break
    fi
  done
  if [[ "${matched}" != true ]]; then
    echo "未登记的 Grok fork 差异：${difference}" >&2
    exit 1
  fi
done < <(diff -qr third_party/grok-build third_party/grok-build-fork || true)

for revision_file in \
  third_party/grok-build-fork/FORK_SOURCE.md \
  third_party/grok-build-fork/PATCHSET.md \
  third_party/grok-build-fork/UPSTREAM_SOURCE.md \
  third_party/grok-build-fork/crates/codegen/xai-grok-shell/src/metheus_embedded.rs \
  src-tauri/crates/metheus-grok-engine/src/lib.rs; do
  if ! rg -q 'metheus\.3' "${revision_file}"; then
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
npx --no-install vitest run \
  src/PlanApprovalPanel.test.tsx \
  src/planTargetPolicy.test.ts \
  src/ExecutionTree.test.tsx \
  src/V1ExecutionPanel.test.tsx \
  src/console/ExecutionPlanStep.test.tsx \
  src/TaskInspector.test.tsx

echo "[adaptive] Grok adapter track (.build/grok-full, jobs=1, one Cargo task)"
"${SCRIPT_DIR}/resource-preflight.sh" grok
export CARGO_TARGET_DIR="${GROK_TARGET_DIR}"
export CARGO_BUILD_JOBS="${GROK_MAX_JOBS}"
cd "${REPO_ROOT}/src-tauri"
cargo test --locked --workspace --no-default-features \
  --features metheus/builtin-grok --profile grok-check \
  adaptive_grok_contract -- --test-threads=1

cd "${REPO_ROOT}"
git diff --check
echo "[adaptive] closeout verification passed"
