# Metheus v0.0.4 第一阶段运行时验收协议

本协议只验证第一阶段正式默认启用与最终体验封板，不重建控制算法、同步总线或恢复主链。自动化轨道固定使用 `.build/core`、`--no-default-features` 和最多两个 Cargo 任务；禁止 Tauri dev/build、Grok Build、真实模型请求、无过滤 Rust 测试、依赖变更和 Git 写操作。

## 自动化契约场景

| 场景 | 契约 | 证据入口 |
|---|---|---|
| 场景 1： | 新项目默认进入 `SerialTakeover`，缺字段旧项目仍为 `Legacy` | `task_control::tests::phase1_runtime_contract_new_projects_default_to_serial_without_migrating_legacy` |
| 场景 2： | `Shadow` 只记录对照，不派发控制动作 | `commands::workflow::tests::phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits` |
| 场景 3： | `SerialTakeover` 派发 `execute_control_action` | `commands::workflow::tests::phase1_runtime_contract_serial_takeover_dispatches_control_action` |
| 场景 4： | 后端阻断经 Channel/统一快照出现，无需 Reload | `useProjectStateSync.test.tsx`、`AutopilotControlBar.test.tsx` |
| 场景 5： | Channel 不可用时低频快照兜底保持状态可恢复 | `useProjectStateSync.test.tsx`、`SyncStatusIndicator.test.tsx` |
| 场景 6： | 执行终态延迟时显示后台收尾同步状态 | `executionSyncPolicy.test.ts`、`SyncStatusIndicator.test.tsx` |
| 场景 7： | 详细任务控制快照乱序、跨进程或动作不一致时拒绝 | `useTaskControlWorkspace.test.tsx` |
| 场景 8： | 恢复完成后旧恢复状态立即清除 | `pipeline::tests::phase1_runtime_contract_successful_retest_clears_recovery_immediately`、`RecoveryNotice.test.tsx` |
| 场景 9： | 叶子串行收口后父节点聚合，Git 确认事务不会触发重复执行 | `task_aggregation::tests::phase1_runtime_contract_two_leaf_closeout_aggregates_parent_and_advances`、`pipeline::tests::phase1_runtime_contract_git_confirmation_claim_reconciles_without_reexecution` |
| 场景 10： | 显式确认回退到 `Shadow` 后旧任务流水线继续可用，且原因可审计 | `commands::workflow::tests::phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits`、`taskControlPolicy.test.ts` |

执行命令：`./scripts/verify-phase1-runtime-contract.sh`。只做静态契约核对时使用 `--static-only`。

## 最终封板定向清单

- 人工通过、接受偏差和跳过任务统一消费后端 `human_action_policy`；未执行、执行失败、父任务、非当前任务与陈旧请求均拒绝。
- 每个任务节点由后端下发能力、禁用原因与可操作验收项；前端缺少详细快照时保持只读，不本地补权。
- 详情独立轮询仅在 Channel 重连/断开、原子详情不可用或超时、连续同步失败时启用；响应继续接受同游标校验。
- `ProjectSyncState` 测试统一使用完整工厂，生产字段不降级为可选。

## 真实桌面烟雾协议

桌面验收前必须先运行 `./scripts/resource-preflight.sh desktop`，并且只复用已经存在的 Core 桌面二进制。候选必须位于 `.build/core/core-dev/metheus` 或 `.build/core/debug/metheus`，不早于当前桌面源码，并带有同名 `.build-meta`：`track=core`、`default_features=false` 和当前源码指纹必须全部匹配。任一条件不满足时记录“未执行”，不得自动触发构建。

人工步骤：

1. 新建项目，确认模式显示为串行接管。
2. 使用假执行或免费环境执行一个简单任务。
3. 制造后台阻断，不 Reload，记录恢复按钮出现延迟。
4. 最小化窗口，确认标题或系统提醒能发现新阻断。
5. 执行后端给出的唯一恢复动作，确认结果横幅和旧恢复状态清理。
6. 打开任务检查器，确认顶部、执行面板与详细快照的阶段、原因、心跳和质量摘要一致。
7. 暂停活动作业，显式回退到 `Shadow`，填写原因并确认模式说明。

记录字段：二进制路径与时间戳、恢复按钮延迟、通知是否丢失、状态是否分裂、峰值内存/CPU、每步结果、未执行原因。

## 当前执行记录

- Core 自动化契约：2026-07-31 已通过。资源预检和 Rust 格式通过；`.build/core`、最多两个 Cargo 任务、`--no-default-features` 下 6 项定向 Rust 测试通过、360 项过滤，仅有 13 条既有未使用代码告警；TypeScript、11 个指定前端测试文件共 60 项测试、静态命令覆盖和 `git diff --check` 通过。
- Core 代码封板门禁：2026-08-01 已通过。资源预检与 Rust 格式通过；`.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下 10 项人工终态安全 Rust 测试通过、365 项过滤；TypeScript 无输出检查、5 个指定前端测试文件共 46 项测试、静态旁路门禁与 `git diff --check` 通过。仅有既有未使用代码告警。
- 真实桌面烟雾：未执行。资源预检通过，但 `.build/core` 中没有可复用桌面程序；唯一候选 `src-tauri/target/debug/metheus` 生成于 2026-07-31 21:17:54，早于本轮 22:42–22:54 的恢复、快照和正式接管源码，且不属于 Core 轨道，因此未启动该过期程序，也未触发构建。
- 2026-08-01 再次复核：上述候选时间戳仍未变化，且继续早于最新运行时与前端源码；本轮仍不得把它用于长链路验收，也未自动触发重建。
- 2026-08-01 最终条件检查：`./scripts/resource-preflight.sh desktop` 的资源检查通过，但返回 `DESKTOP_SMOKE_ELIGIBLE=no`；`.build/core` 没有桌面程序，`src-tauri/target/debug/metheus` 被明确判定为非 Core 候选，且不存在匹配当前源码指纹与关闭默认特性的 `.build-meta`。因此真实桌面烟雾保持“未执行”，没有启动候选程序或触发构建。
- 桌面记录字段：恢复按钮延迟、后台通知丢失、状态分裂、峰值内存/CPU 和人工步骤结果均为 `N/A（未执行）`。
- 剩余风险：无需 Reload 的恢复按钮、窗口后台提醒、顶部/执行面板/任务检查器一致性、恢复结果横幅和显式回退仍需使用当前桌面二进制人工验收。静态与定向测试不能替代这部分证据。
- 未执行：全量构建、无过滤 Rust 全量测试、Clippy、Tauri dev/build 与打包、Grok Build、真实模型和付费 CLI。未执行任何 Git 写操作。
