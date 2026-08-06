# Metheus v0.0.4 第一阶段运行时验收协议

本协议只验证第一阶段正式默认启用与最终体验封板，不重建控制算法、同步总线或恢复主链。Core 自动化轨道固定使用 `.build/core`、`--no-default-features` 和最多两个 Cargo 任务；Grok 专项只允许 `.build/grok-full`、单任务、`builtin-grok` 的库目标 `cargo check`。禁止 Tauri dev/build、Grok 完整构建、真实模型请求、无过滤 Rust 测试、依赖变更和 Git 写操作。

## 自动化契约场景

| 场景 | 契约 | 证据入口 |
|---|---|---|
| 场景 1： | 新项目默认进入 `SerialTakeover`，缺字段旧项目仍为 `Legacy` | `task_control::tests::phase1_runtime_contract_new_projects_default_to_serial_without_migrating_legacy` |
| 场景 2： | `Shadow` 只记录对照，不派发控制动作 | `commands::workflow::tests::phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits` |
| 场景 3： | 验收文本先推断为 `SemanticReview`，`SerialTakeover` 再派发确定的 `TargetedValidate` 控制动作 | `commands::workflow::tests::phase1_runtime_contract_serial_takeover_dispatches_control_action` |
| 场景 4： | 后端阻断经 Channel/统一快照出现，无需 Reload | `useProjectStateSync.test.tsx`、`AutopilotControlBar.test.tsx` |
| 场景 5： | Channel 不可用时低频快照兜底保持状态可恢复 | `useProjectStateSync.test.tsx`、`SyncStatusIndicator.test.tsx` |
| 场景 6： | 执行终态延迟时显示后台收尾同步状态 | `executionSyncPolicy.test.ts`、`SyncStatusIndicator.test.tsx` |
| 场景 7： | 详细任务控制快照乱序、跨进程或动作不一致时拒绝 | `useTaskControlWorkspace.test.tsx` |
| 场景 8： | 恢复完成后旧恢复状态立即清除 | `pipeline::tests::phase1_runtime_contract_successful_retest_clears_recovery_immediately`、`RecoveryNotice.test.tsx` |
| 场景 9： | 叶子串行收口后父节点聚合，Git 确认事务不会触发重复执行 | `task_aggregation::tests::phase1_runtime_contract_two_leaf_closeout_aggregates_parent_and_advances`、`pipeline::tests::phase1_runtime_contract_git_confirmation_claim_reconciles_without_reexecution` |
| 场景 10： | 显式确认回退到 `Shadow` 后旧任务流水线继续可用，且原因可审计 | `commands::workflow::tests::phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits`、`taskControlPolicy.test.ts` |
| 场景 11： | 有界 `_runtime` 命令按显式覆盖、完整命令、单层基础名和默认值解析，不再误落 30 秒 | `invokeWithTimeout.test.ts`、`verify-phase1-closeout.sh --static-only` |
| 场景 12： | 决策与执行活动租约存续时阻断设置修改，终态释放后恢复修改且计数保持饱和 | 两项 settings Rust 单测验证阻断与释放算法；`verify-phase1-closeout.sh --static-only` 验证 `ActivityGuard::drop` 将当前 `kind` 接入 `release_activity` |
| 场景 13： | Grok Build 内置自检返回后，所有匹配的已挂载健康消费者无需 Reload 即可刷新，旧响应不能覆盖新状态 | `engineHealthSync.test.ts`、`ApplicationSettings.test.tsx`、`ExecutionEngineSelector.test.tsx` |

Core 执行命令：`npm run verify:phase1-runtime-contract`。只做静态契约核对时使用 `./scripts/verify-phase1-runtime-contract.sh --static-only`；Grok 特性边界使用 `npm run verify:grok-check`。

## 最终封板定向清单

- 人工通过、接受偏差和跳过任务统一消费后端 `human_action_policy`；未执行、执行失败、父任务、非当前任务与陈旧请求均拒绝。
- 每个任务节点由后端下发能力、禁用原因与可操作验收项；前端缺少详细快照时保持只读，不本地补权。
- 详情独立轮询仅在 Channel 重连/断开、原子详情不可用或超时、连续同步失败时启用；响应继续接受同游标校验。
- `ProjectSyncState` 测试统一使用完整工厂，生产字段不降级为可选。

## 运行期故障注入门禁

执行 `./scripts/verify-runtime-fault-recovery.sh`。脚本先运行 `runtime-fault` 资源预检，仅在 `mktemp` 创建的临时测试项目目录中构造旧进程持有且心跳已过期、心跳超时、未完成 Git 确认事务和旧格式字符串锁四类故障；随后使用 `.build/core`、`--no-default-features`、`CARGO_BUILD_JOBS=2` 运行 `runtime_fault` Rust 定向测试、锁恢复前端测试、TypeScript 检查和 `git diff --check`。脚本在退出时删除夹具，并比较验证前后的主仓库文件状态。

## 真机暴露的运行期缺陷与覆盖边界

2026-08-02 根据真实桌面运行反馈补录三类缺陷：持久化控制动作锁只有字符串标识，进程中途退出后会永久残留；锁占用被错误混入代码恢复展示，导致出现无对应 `recovery_state` 的人工恢复入口，并且阻断说明被多层重复拼接；强类型 JSON 仍沿用无目标契约的通用修复，字段类型错误会重复同一无效请求并被误算为代码修复失败。

本轮已分别补上租约持有者、心跳、最长时长和启动对账；独立的锁占用恢复展示与后端清理动作；Schema 感知的单次协议修复、确定性归一化和协议失败分类。静态测试可以注入残留磁盘事实，但不能证明桌面进程被操作系统强制终止、重启和真实 Git 工作区衔接时的完整生命周期，因此下面的强退复验仍是必测项，定向测试通过不得替代桌面稳定性验收。

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
8. 启动一个长时间控制动作，确认锁占用展示包含动作类型、已持续时间和最后心跳；占用有效时只能等待，不得提供清理或人工恢复决策。
9. 在动作执行中强制关闭应用，再重新打开同一项目，确认陈旧锁由后端自动对账释放，无需手工编辑项目文件。
10. 确认陈旧锁清理已写入执行历史，锁清理后节点能力和控制模式切换立即恢复，旧锁按钮随状态刷新消失。
11. 在 Git 确认事务中断场景重复强制关闭与重开，比较提交标识，确认既有幂等事务续跑且没有重复提交。
12. 确认锁占用阻断和代码恢复是不同展示：陈旧锁只有清理动作，无 `recovery_state` 时不显示“选择人工恢复处理方式”。
13. 清理完成后执行一个正常控制动作，确认恢复入口可用且新动作能重新建立有效占用。

记录字段：二进制路径与时间戳、进程启动标识、动作标识、最后心跳、陈旧判定与清理原因、清理审计事件、Git 提交标识前后值、恢复按钮延迟、通知是否丢失、状态是否分裂、峰值内存/CPU、每步结果、未执行原因。

## 当前执行记录

- 第一阶段运行期三项收口（2026-08-06）：实际运行 `npm run verify:phase1-runtime-contract`，命令退出状态为 0、总耗时约 8.7 秒；资源预检、Rust 格式、TypeScript 无输出检查、运行时命令注册/旧命令旁路/超时覆盖静态门禁及 `git diff --check` 通过。`.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下 15 项 `phase1_runtime_contract` Rust 测试通过、422 项过滤，仅报告 15 条既有未使用代码告警；19 个指定前端测试文件共 113 项通过。超时门禁动态审计 61 个命令：56 个基础策略、1 个精确策略、2 个显式超时例外和 2 个 Channel 例外，精确策略明确包含 `test_grok_build_runtime`。settings Rust 单测覆盖两类活动计数的阻断与释放算法，静态门禁覆盖 `ActivityGuard::drop` 到释放函数的接线；`workflow.rs` 同一测试先证明夹具为 `SemanticReview`，再证明派发 `TargetedValidate`；Grok 自检健康刷新与请求序号保护也已覆盖。该自动化证据不等同于真实网络请求成功、失败或取消的生命周期测试。
- Grok 专项（2026-08-06）：实际运行 `npm run verify:grok-check`，命令退出状态为 0且资源预检通过；`.build/grok-full`、`CARGO_BUILD_JOBS=1`、`--no-default-features --features builtin-grok` 下单任务库目标 `cargo check` 通过，Cargo 报告耗时 1 分 34 秒，仅报告 21 条未使用代码告警；未执行最终链接、桌面构建或真实模型请求。
- 桌面资格复核（2026-08-06）：实际运行 `./scripts/resource-preflight.sh desktop`，资源检查通过，但仅发现非 Core 候选 `src-tauri/target/debug/metheus`；没有同时满足 Core 路径、关闭默认特性的构建元数据、当前源码指纹和时间戳的桌面二进制，结果为 `DESKTOP_SMOKE_ELIGIBLE=no`。未启动候选程序，也未触发重建，因此本轮自动化只证明 Core/Grok 类型与前端契约，不宣称真实桌面自检刷新、强退恢复或发布级稳定性已经验收。

- 验证可证明性收尾（2026-08-03）：已实现五档可证明性元数据、规划显式打标与本地保守校准、旧项目纯本地迁移、标签优先分拣、逐项人工确认入口、证据来源指纹熔断，以及按动作类型收紧的租约窗口。实际运行 `scripts/verify-provability-closeout.sh`：资源预检、Rust 格式、TypeScript、静态链路门禁与 `git diff --check` 通过；`.build/core`、最多两个 Cargo 任务、`--no-default-features` 下 14 项 Rust 定向测试通过（409 项过滤），任务检查器 8 项通过，仅有 14 条既有未使用代码告警。

- 检查收敛与同步专项：2026-08-02 已通过本轮定向代码门禁。`.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下 `check_convergence_` 11 项 Rust 测试通过、399 项过滤；TypeScript 无输出检查通过，`projectSyncPolicy` 与 `useProjectStateSync` 共 17 项前端测试通过。覆盖建议不阻断、建议误分类回收、硬阻断指纹、三类检查契约与协议失败、确定性预检、重生成阈值、修订未前进不拉全量及健康 Channel 60 秒兜底。
- 本专项桌面验收：未执行。未运行 Tauri dev/build、Grok Build、真实模型、无过滤 Rust 测试、Clippy 或发布构建；因此只证明 Core 协议与前端策略，不证明真实长链路桌面顿挫已经完成发布级验收。
- 本专项验证过程说明：首次定向 Cargo 命令漏设 `CARGO_TARGET_DIR`，约 1 秒后被中断并计入两个 Cargo 任务之一；随后唯一一次完整 Cargo 任务显式使用 `.build/core` 并通过。中断任务触及既有 `src-tauri/target` 缓存，未作为验收证据，也未清理或覆盖用户缓存。
- Core 自动化契约：2026-07-31 已通过。资源预检和 Rust 格式通过；`.build/core`、最多两个 Cargo 任务、`--no-default-features` 下 6 项定向 Rust 测试通过、360 项过滤，仅有 13 条既有未使用代码告警；TypeScript、11 个指定前端测试文件共 60 项测试、静态命令覆盖和 `git diff --check` 通过。
- Core 代码封板门禁：2026-08-01 已通过。资源预检与 Rust 格式通过；`.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下 10 项人工终态安全 Rust 测试通过、365 项过滤；TypeScript 无输出检查、5 个指定前端测试文件共 46 项测试、静态旁路门禁与 `git diff --check` 通过。仅有既有未使用代码告警。
- 运行期故障注入门禁：2026-08-02 已通过。四类故障夹具全部位于临时目录；22 项 `runtime_fault` Rust 定向测试通过、379 项过滤，2 个指定前端测试文件共 21 项测试通过；TypeScript、Rust 格式、`git diff --check` 和主仓库文件状态前后比较通过。仅有 14 条未使用代码告警，未发送真实模型请求。
- 真实桌面烟雾：未执行。资源预检通过，但 `.build/core` 中没有可复用桌面程序；唯一候选 `src-tauri/target/debug/metheus` 生成于 2026-07-31 21:17:54，早于本轮 22:42–22:54 的恢复、快照和正式接管源码，且不属于 Core 轨道，因此未启动该过期程序，也未触发构建。
- 2026-08-01 再次复核：上述候选时间戳仍未变化，且继续早于最新运行时与前端源码；本轮仍不得把它用于长链路验收，也未自动触发重建。
- 2026-08-01 最终条件检查：`./scripts/resource-preflight.sh desktop` 的资源检查通过，但返回 `DESKTOP_SMOKE_ELIGIBLE=no`；`.build/core` 没有桌面程序，`src-tauri/target/debug/metheus` 被明确判定为非 Core 候选，且不存在匹配当前源码指纹与关闭默认特性的 `.build-meta`。因此真实桌面烟雾保持“未执行”，没有启动候选程序或触发构建。
- 2026-08-02 运行期复核：`./scripts/resource-preflight.sh desktop` 的资源检查通过，但仍返回 `DESKTOP_SMOKE_ELIGIBLE=no`；仅发现非 Core 候选 `src-tauri/target/debug/metheus`，没有满足当前源码指纹、关闭默认特性和时间戳条件的 Core 桌面二进制。本轮未启动该候选、未自动重建，真实桌面强退与重启烟雾继续记为“未执行”。
- 桌面记录字段：进程启动标识、动作标识、最后心跳、陈旧判定与清理原因、清理审计事件、Git 提交标识前后值、恢复按钮延迟、后台通知丢失、状态分裂、峰值内存/CPU 和人工步骤结果均为 `N/A（未执行）`。
- 剩余风险：执行中强制关闭应用后的锁自动释放、Git 确认不重复提交、锁占用独立恢复展示，以及无需 Reload 的恢复按钮、窗口后台提醒、顶部/执行面板/任务检查器一致性、恢复结果横幅和显式回退，仍需使用当前源码对应的桌面二进制人工验收。静态与定向测试不能替代这部分证据。
- 未执行：全量构建、无过滤 Rust 全量测试、Clippy、Tauri dev/build 与打包、Grok Build、真实模型和付费 CLI。未执行任何 Git 写操作。
