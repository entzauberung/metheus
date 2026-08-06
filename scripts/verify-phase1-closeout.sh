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

readonly RUNTIME_COMMAND_DIRECTORY="src-tauri/src/commands"
readonly TIMEOUT_POLICY_SOURCE="src/utils/invokeWithTimeout.ts"
readonly EXPLICIT_TIMEOUT_RUNTIME_COMMANDS=(
  "apply_task_control_action_runtime"
  "set_task_control_mode_runtime"
)
readonly STREAMING_CHANNEL_RUNTIME_COMMANDS=(
  "chat_with_role_stream_runtime"
  "regenerate_chat_reply_stream_runtime"
)

is_listed_runtime_command() {
  local candidate="$1"
  shift
  local listed_command
  for listed_command in "$@"; do
    if [[ "${candidate}" == "${listed_command}" ]]; then
      return 0
    fi
  done
  return 1
}

mapfile -t RUNTIME_MUTATION_COMMANDS < <(
  rg --glob '*.rs' --no-filename -o \
    'pub\(crate\) async fn [a-z0-9_]+_runtime' "${RUNTIME_COMMAND_DIRECTORY}" \
    | awk '{ print $NF }' \
    | sort -u
)

if [[ "${#RUNTIME_MUTATION_COMMANDS[@]}" -eq 0 ]]; then
  echo "未能从 Rust 命令源提取运行时命令" >&2
  exit 1
fi

base_policy_count=0
exact_policy_count=0
explicit_exception_count=0
streaming_exception_count=0
EXACT_POLICY_COMMANDS=()

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

  if is_listed_runtime_command "${runtime_command}" \
      "${EXPLICIT_TIMEOUT_RUNTIME_COMMANDS[@]}"; then
    explicit_exception_count=$((explicit_exception_count + 1))
    continue
  fi

  if is_listed_runtime_command "${runtime_command}" \
      "${STREAMING_CHANNEL_RUNTIME_COMMANDS[@]}"; then
    streaming_exception_count=$((streaming_exception_count + 1))
    continue
  fi

  if rg -q "^[[:space:]]{2}${runtime_command}:" "${TIMEOUT_POLICY_SOURCE}"; then
    exact_policy_count=$((exact_policy_count + 1))
    EXACT_POLICY_COMMANDS+=("${runtime_command}")
    continue
  fi

  if rg -q "^[[:space:]]{2}${base_command}:" "${TIMEOUT_POLICY_SOURCE}"; then
    base_policy_count=$((base_policy_count + 1))
    continue
  fi

  echo "有界运行时命令缺少超时策略或已登记例外：${runtime_command}（基础命令 ${base_command}）" >&2
  exit 1
done

for exception_command in \
  "${EXPLICIT_TIMEOUT_RUNTIME_COMMANDS[@]}" \
  "${STREAMING_CHANNEL_RUNTIME_COMMANDS[@]}"; do
  if ! is_listed_runtime_command "${exception_command}" "${RUNTIME_MUTATION_COMMANDS[@]}"; then
    echo "超时例外未对应已提取的 Rust 运行时命令：${exception_command}" >&2
    exit 1
  fi
done

if ! rg -U -q '(?s)"apply_task_control_action_runtime".{0,1200}\}, 900_000\);' \
  src/hooks/useTaskControlWorkspace.ts; then
  echo "任务控制动作必须保留 900_000 毫秒显式超时" >&2
  exit 1
fi

if ! rg -U -q '(?s)"set_task_control_mode_runtime".{0,1200}\}, 15_000\);' \
  src/hooks/useTaskControlWorkspace.ts; then
  echo "任务控制模式切换必须保留 15_000 毫秒显式超时" >&2
  exit 1
fi

for stream_command in "${STREAMING_CHANNEL_RUNTIME_COMMANDS[@]}"; do
  if ! rg -q "\"${stream_command}\"" src/chatStreamController.ts; then
    echo "流式运行时命令缺少 Channel 调用登记：${stream_command}" >&2
    exit 1
  fi
done

if ! rg -U -q '(?s)invokeStream\(command, args\).{0,240}return invoke<RuntimeMutationResult>\(command, args\);' \
  src/chatStreamController.ts; then
  echo "聊天流式运行时命令必须继续通过 Tauri Channel invoke 等待终态" >&2
  exit 1
fi

if [[ "${#EXACT_POLICY_COMMANDS[@]}" -eq 0 ]]; then
  exact_policy_summary="none"
else
  exact_policy_summary="$(IFS=,; echo "${EXACT_POLICY_COMMANDS[*]}")"
fi

echo "超时策略审计通过：runtime_commands=${#RUNTIME_MUTATION_COMMANDS[@]} base_policies=${base_policy_count} exact_policies=${exact_policy_count} explicit_exceptions=${explicit_exception_count} streaming_exceptions=${streaming_exception_count} exact_commands=${exact_policy_summary}"

if ! rg -U -q \
  '(?s)impl Drop for ActivityGuard\s*\{.{0,200}fn drop\(&mut self\).{0,400}SETTINGS_STORE\.get\(\).{0,400}store\.state\.lock\(\).{0,400}release_activity\(\s*&mut state,\s*self\.kind\s*\);' \
  src-tauri/src/settings.rs; then
  echo "设置活动租约必须保持 ActivityGuard::drop 到 release_activity(&mut state, self.kind) 的 RAII 接线" >&2
  exit 1
fi

echo "设置活动租约 RAII 接线审计通过：ActivityGuard::drop -> release_activity(state, self.kind)"

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
