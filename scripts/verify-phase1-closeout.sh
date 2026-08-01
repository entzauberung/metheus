#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly CORE_TARGET_DIR="${REPO_ROOT}/.build/core"
readonly CORE_MAX_JOBS=2
readonly VERIFY_SCOPE="${1:-full}"

if [[ "${VERIFY_SCOPE}" != "--static-only" ]]; then
  "${SCRIPT_DIR}/resource-preflight.sh" core
  export CARGO_TARGET_DIR="${CORE_TARGET_DIR}"
  export CARGO_BUILD_JOBS="${CORE_MAX_JOBS}"

  cd "${REPO_ROOT}/src-tauri"
  cargo fmt --package metheus -- --check
  cargo test --locked --package metheus --lib --no-default-features \
    --profile core-dev phase1_human_action_safety -- --test-threads=1

  cd "${REPO_ROOT}"
  npx --no-install tsc --noEmit
  npx --no-install vitest run \
    src/projectSyncPolicy.test.ts \
    src/hooks/useProjectStateSync.test.tsx \
    src/hooks/useTaskControlWorkspace.test.tsx \
    src/components/SyncStatusIndicator.test.tsx \
    src/TaskInspector.test.tsx
fi

cd "${REPO_ROOT}"

readonly RUNTIME_COMMAND_SOURCES=(
  "src-tauri/src/commands/runtime_mutations.rs"
  "src-tauri/src/commands/task_control.rs"
  "src-tauri/src/commands/chat.rs"
)

mapfile -t RUNTIME_MUTATION_COMMANDS < <(
  rg -o 'pub\(crate\) async fn [a-z0-9_]+_runtime' "${RUNTIME_COMMAND_SOURCES[@]}" \
    | awk '{ print $NF }' \
    | sort -u
)

for runtime_command in "${RUNTIME_MUTATION_COMMANDS[@]}"; do
  if ! rg -q "::${runtime_command}," src-tauri/src/lib.rs; then
    echo "运行时状态变更命令未注册：${runtime_command}" >&2
    exit 1
  fi

  base_command="${runtime_command%_runtime}"
  if rg -n --glob '!*.test.ts' --glob '!*.test.tsx' \
    "invoke[^()]*(\\(|<[^>]+>\\()[\"']${base_command}[\"']" src; then
    echo "主前端不得直接调用旧状态变更命令：${base_command}" >&2
    exit 1
  fi
done

readonly FRONTEND_PROJECT_MUTATION_FILES=(
  src/App.tsx
  src/ConsoleWorkflowPanel.tsx
  src/PreflightPanel.tsx
  src/ExistingBaselinePanel.tsx
  src/components/ExecutionEngineSettings.tsx
  src/hooks/useTaskControlWorkspace.ts
  src/chatStreamController.ts
)

if rg -n 'invokeWithTimeout<(Project|PipelineState|ExecutionWorkspaceStatus)>\("(select_|generate_|analyze_|approve_|reject_|start_|stop_|request_|resolve_|confirm_|retry_|toggle_|autopilot_|reconcile_|execute_|prepare_|refresh_|update_)' \
  "${FRONTEND_PROJECT_MUTATION_FILES[@]}"; then
  echo "关键状态变更命令不得绕过统一运行时返回模型" >&2
  exit 1
fi

if ! rg -q 'summarize_milestone_runtime' src/App.tsx; then
  echo "大阶段总结必须原子返回成本账本所在的运行时快照" >&2
  exit 1
fi

if rg -n "invoke[^()]*(\\(|<[^>]+>\\()[\\\"']summarize_milestone[\\\"']" src/App.tsx; then
  echo "大阶段总结不得继续使用只返回字符串的旧命令" >&2
  exit 1
fi

readonly MODE_DOCUMENTS=(README.md CONSTITUTION.md)

if rg -n '(新建|新项目)[^。\n]{0,40}默认(继续)?(进入|使用)[[:space:]]*`?(Shadow|影子)|(新建|新项目)[^。\n]{0,40}默认[[:space:]]*`?(Shadow|影子)|SerialTakeover[^。\n]{0,48}(不是|并非|没有被设置为)默认' \
  "${MODE_DOCUMENTS[@]}"; then
  echo "产品文档不得继续声明新项目默认 Shadow" >&2
  exit 1
fi

for document in "${MODE_DOCUMENTS[@]}"; do
  if ! rg -q 'SerialTakeover.*(正式默认|默认.*正式)|正式默认.*SerialTakeover' "${document}"; then
    echo "产品文档缺少 SerialTakeover 正式默认说明：${document}" >&2
    exit 1
  fi

  if ! rg -q '旧项目.*(不自动迁移|不会自动迁移|不被自动迁移|已有模式不自动迁移)' "${document}"; then
    echo "产品文档缺少旧项目不自动迁移说明：${document}" >&2
    exit 1
  fi
done

if ! rg -q 'Core 代码封板门禁：2026-08-01 已通过.*10 项人工终态安全 Rust 测试通过.*46 项测试' \
  docs/phase1-runtime-acceptance.md; then
  echo "第一阶段验收文档必须记录本轮真实封板测试数量" >&2
  exit 1
fi

if ! rg -q '真实桌面烟雾：未执行.*没有可复用桌面程序' \
  docs/phase1-runtime-acceptance.md; then
  echo "第一阶段验收文档必须准确记录桌面烟雾未执行风险" >&2
  exit 1
fi

if ! rg -q 'mod human_action_policy;' src-tauri/src/lib.rs \
  || ! rg -q 'validate_recorded_human_acceptance' src-tauri/src/pipeline.rs \
  || ! rg -q 'save_project_if_revision' src-tauri/src/control_action_executor.rs; then
  echo "人工终态动作必须统一经过策略、审计复核和条件写盘" >&2
  exit 1
fi

if rg -n 'isCurrentTask[[:space:]]*\?[^;]*(accept_deviation)|:[[:space:]]*true\);' \
  src/TaskInspector.tsx src/TaskInspectorHeader.tsx; then
  echo "前端不得为非当前节点本地放宽终态能力" >&2
  exit 1
fi

if ! rg -q 'fallbackDecision.active' src/hooks/useTaskControlWorkspace.ts \
  || rg -n 'fallbackPollingEnabled:[[:space:]]*inspectorOpen' src/App.tsx; then
  echo "任务详情独立轮询必须只由异常兜底策略启用" >&2
  exit 1
fi

readonly TODAY="$(date +%F)"
while IFS= read -r recorded_date; do
  if [[ "${recorded_date}" > "${TODAY}" ]]; then
    echo "验收文档包含未来日期：${recorded_date}（当前 ${TODAY}）" >&2
    exit 1
  fi
done < <(rg -o '20[0-9]{2}-[0-9]{2}-[0-9]{2}' docs/phase1-runtime-acceptance.md | sort -u)

git diff --check
