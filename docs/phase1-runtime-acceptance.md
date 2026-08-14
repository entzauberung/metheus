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

- 2026-08-09 当前门禁基线（取代下方历史记录作为本轮指针）：`WO-001` 至 `WO-008` 的定向子任务与 `WO-009-ST-001/002/003` 已完成，三条 R1 集成门禁均在当前源码上退出 0。`npm run verify:adaptive-execution-closeout` 为 Core 61 项、13 个前端文件 70 项、Grok adapter 8 项、host continuation 9 项和 fork embedded runtime 12 项通过，墙钟 `629.44s`（用户批准上限 `800s`）；`npm run verify:phase1-runtime-contract` 为 Rust 15 项、24 个前端文件 161 项通过，墙钟 `12.41s`；`npm run verify:grok-check` 独立 Grok check 通过，墙钟 `104.00s`。当前源码 R2 为全量前端 218/218、主包 Rust 561/561、生产构建通过；Core fingerprint `65523426f71927f4e1cfbbb0eaffa9a95dc55beffff1c80e3d28bb8344fb00f5` 通过桌面资格检查，本地 Tauri CLI 已重建 `.deb`、`.rpm`、`.AppImage`。13 步真实桌面烟雾与真实 provider 生命周期保留为待人工审核，不计入自动化失败，也不得伪造为已执行。
- 2026-08-08 工作树分类快照：staged 0；unstaged tracked 78（包含 tracked 修改和删除）；untracked 4,536，其中 `src-tauri/.build/**` 生成缓存 4,482，非生成缓存路径 54。非生成缓存未跟踪路径包含 43 个 `AGENTS.md`/`ai/**`/`opencode.json` 蓝图与本地控制文件，以及 11 个必须进入验收清单的功能文件：`src-tauri/src/vision_review.rs`、`src/ConsoleWorkspace.css`、`src/MilestoneHumanReviewDialog{,.test}.tsx`、`src/MilestoneReviewPanel.test.tsx`、`src/components/ConsoleBottomPanel.tsx`、`ConsoleNavigator.tsx`、`ConsoleWorkspace{,.test}.tsx`、`src/consoleWritePolicy{,.test}.ts`。验收脚本会输出 staged、unstaged、非缓存 untracked 完整路径，并单独记录生成缓存数量，既不把 `git diff` 当作全部工作树，也不让缓存洪泛掩盖功能文件。
- 2026-08-08 依赖与未执行边界：当前 tracked 状态包含 `src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock` 修改，门禁只审计并报告，不在本工单改写依赖；本 ST 未运行测试、构建、服务、网络、桌面、Tauri、发布或真实模型命令。下方所有 2026-08-07 及更早的数量和结果均为历史证据。
- Review 唯一写入门禁与 Git 证据收尾（2026-08-07）：`verify-adaptive-execution-closeout.sh` 现在遍历 `src-tauri/src/**/*.rs`，只扫描每个文件首个 `#[cfg(test)]` 之前的生产部分，以跨行空白归一化匹配要求 `current_step = project::WorkflowStep::MilestoneReview` 恰好出现一次、位于 `workflow_resolution.rs`，并确认 `apply_milestone_review_boundary` 存在；违规时列出文件和行号并退出 1。Shell 语法、生产调用链审计和完整专项执行均通过，没有发现 select、continue、通用 transition、migration、startup 或 pipeline 新旁路。验证前实际读取 `git status --short`、`git diff --cached --name-status` 和 `git diff --name-only`，cached diff 为空，不存在 staged `ONSTITUTION.md` 或其他 staged 路径；现有交付文件均为未暂存修改，专项脚本是开工前即存在且继续保持未跟踪的文件。
- 2026-08-07 最终四轨复验：四条主命令和两类 diff check 全部退出 0。`npm run verify:adaptive-execution-closeout` 中 Core 60 项通过、455 项过滤（Cargo 0.37 秒、测试 0.27 秒、18 条 unused-code 告警），6 个前端文件 22 项通过（Vitest 1.41 秒），Grok 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 39 秒；lib 报告 25 条告警，test 目标报告 14 条且其中 11 条重复），整条命令墙钟 229.23 秒，新 Review 唯一写入门禁、静态边界、资源预检与 diff 门禁通过。`npm run verify:phase1-runtime-contract` 为 Rust 15 项通过、500 项过滤（Cargo 1.25 秒、测试 0.14 秒、18 条告警），19 个前端文件 115 项通过（Vitest 3.40 秒），墙钟 11.05 秒；`npm run verify:grok-check` 为 Cargo 1 分 28 秒、墙钟 88.58 秒、25 条告警；`CARGO_TARGET_DIR=.build/grok-full CARGO_BUILD_JOBS=1 cargo test --manifest-path third_party/grok-build-fork/Cargo.toml --locked -p xai-grok-shell --features metheus-embedded --test metheus_embedded_runtime` 的 11 项测试通过（Cargo 2 分 09 秒、测试 2.24 秒、墙钟 131.52 秒）。随后 `git diff --check`（0.03 秒）与 `git diff --cached --check`（0.00 秒）通过。
- 2026-08-07 未执行边界：未运行桌面 smoke、桌面资源资格复核、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、真实模型或付费请求；未执行 Git 提交、暂存、取消暂存、标签、清理或推送。本轮结果不证明真实桌面刷新/恢复、真实模型调用或发布级资源稳定性。
- Review 旁路修复后最终封板（2026-08-06）：当前 `MilestoneReview` 事实只由同步、无 I/O 且不自行修改 revision 的 `apply_milestone_review_boundary` 写入。正式进入命令、select/continue、Passed/AcceptedDeviation/Skipped 的 pipeline 收敛、workflow closure migration、启动对账及 Discussion B/C 重返均复用该边界，统一建立 Milestone `Completed`、`review_status=pending_review`、`review_conclusion=None`、正确 `review_node_id` 和 active autopilot 的 `WaitingMilestoneReview`；Discussion 线程与历史事件保留。缺失或过期画像在迁移、推进和审阅操作中通过 `Result` 明确阻断且不保存半状态，普通项目加载仍保持兼容；每次持久化转换只增加一次 revision，AcceptDeviation 的 CAS 与动作租约不变。
- Review 修复后四轨复验（2026-08-06）：使用 `/usr/bin/time -p` 实际运行 `npm run verify:adaptive-execution-closeout` 并退出 0；Core `adaptive_execution_contract` 60 项通过、455 项过滤（Cargo 0.35 秒、测试 0.20 秒），6 个前端文件 22 项通过（Vitest 1.38 秒），Grok `adaptive_grok_contract` 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 45 秒），整条命令墙钟 232.46 秒，静态边界、pristine 基线、PATCHSET、`metheus.3` revision、资源预检与 diff 门禁通过，仅报告 25 条未使用代码告警。随后 `npm run verify:phase1-runtime-contract` 退出 0：Rust 15 项通过、500 项过滤（Cargo 1.62 秒、测试 0.12 秒），19 个前端文件 115 项通过（Vitest 2.75 秒），墙钟 10.81 秒；`npm run verify:grok-check` 退出 0（Cargo 1 分 29 秒、墙钟 89.51 秒，仅有 25 条未使用代码告警）；`CARGO_TARGET_DIR=.build/grok-full CARGO_BUILD_JOBS=1 cargo test --manifest-path third_party/grok-build-fork/Cargo.toml --locked -p xai-grok-shell --features metheus-embedded --test metheus_embedded_runtime` 的 11 项测试全部通过（Cargo 2 分 05 秒、测试 2.39 秒、墙钟 128.11 秒）。最后 `git diff --check` 与 `git diff --cached --check` 均退出 0，Git index 为空。
- Review 修复后未执行边界（2026-08-06）：未运行桌面 smoke、桌面资源资格复核、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、真实模型或付费请求；未执行 Git 提交、暂存、标签、清理或推送。Grok 受控工具集合仍严格为 `read_file/search_replace/list_dir/grep`；本轮结果不证明真实桌面刷新/恢复、真实模型调用或发布级资源稳定性。
- 历史自适应执行封板（2026-08-06，已由上述 Review 旁路修复后复验取代）：使用 `/usr/bin/time -p` 记录墙钟并实际依次运行 `npm run verify:adaptive-execution-closeout`、`npm run verify:phase1-runtime-contract`、`npm run verify:grok-check`，以及 `CARGO_TARGET_DIR=.build/grok-full CARGO_BUILD_JOBS=1 cargo test --manifest-path third_party/grok-build-fork/Cargo.toml --locked -p xai-grok-shell --features metheus-embedded --test metheus_embedded_runtime`，四个命令均退出 0。专项门禁通过静态边界、pristine 基线、PATCHSET、`metheus.3` revision、资源预检与 `git diff --check`；Core `adaptive_execution_contract` 40 项通过、457 项过滤（Cargo 0.70 秒、测试 0.27 秒），6 个前端文件 22 项通过（Vitest 2.57 秒），Grok `adaptive_grok_contract` 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 46 秒），首条命令墙钟 238.16 秒。Phase 1 复验为 Rust 15 项通过、482 项过滤（Cargo 1.46 秒、测试 0.05 秒），19 个前端文件 115 项通过（Vitest 3.60 秒），墙钟 11.53 秒。Grok check Cargo 1 分 31 秒、墙钟 91.79 秒，仅有 25 条未使用代码告警；fork 本地 fake-SSE/embedded runtime 11 项通过（Cargo 2 分 26 秒、测试 2.54 秒、墙钟 149.35 秒）。最后 `git diff --check` 与 `git diff --cached --check` 均退出 0。
- 自适应执行修复事实（2026-08-06）：`contract_snapshot` 当前为严格 `task-contract-v2` 派生缓存，已知 v1 只失效缓存，未知版本和损坏 v2 拒绝；缺失/过期画像项目可只读加载但控制能力与 autopilot 明确阻断。进入 MilestoneReview 统一经正式入口建立完成态与审阅事实，AcceptDeviation 经正式 dispatcher 收敛且不重复递增修订。Adapter 的两层 sampler 配置共同遵守 transport 预算；transport retry 为 0 的 Responses fake-SSE 证明 Doom 预算 1 恰好两次请求并只接受 clean response，预算 0 恰好一次请求并保留原响应。工具集合仍严格为 `read_file/search_replace/list_dir/grep`。
- 自适应执行未执行边界（2026-08-06）：未运行桌面 smoke、桌面资源资格复核、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、Grok 全工作区测试、真实模型或付费请求；未开放 MCP、subagent、plugin、skills、terminal、memory、hook 或 web 工具面，未执行 Git 提交、暂存、标签、清理或推送。本轮结果不证明真实桌面刷新/恢复、真实模型调用或发布级资源稳定性。
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

## 2026-08-13 运行时与桌面生命周期补充协议

本节是对第一阶段收尾的补充，不改变前文历史证据。它专门覆盖本轮暴露的窗口消失、后台存活、模型失败、恢复停滞和托管入口问题。静态代码、定向自动化、人工桌面和真实模型证据必须分开记录；未执行或证据不合格不得填写通过。

### 补充验收矩阵

| 验收项 | 静态证据 | 定向自动化证据 | 人工桌面证据 | 真实模型/外部边界证据 | 通过条件与停止条件 |
| --- | --- | --- | --- | --- | --- |
| 启动门禁 | 启动托管只在允许工作流步骤创建 `active=true, Running` 和新的 job identity；连接自检与执行会话分离。 | 覆盖缺失密钥、连接失败、状态不变和不得伪装执行的定向测试。 | 使用合格二进制启动项目，记录二进制路径、源码指纹、进程启动标识和 execution/job identity。 | 记录实际 provider、model、endpoint 指纹和是否真的发出请求；不得把“Grok Build 内置”当作 provider 证据。 | 无密钥/连接失败不得进入成功 Running；无法确认启动进程归属时 BLOCKED。 |
| 模型连接失败 | `MissingSecret`、认证、额度、限流、网络、供应商不可用和超时保持独立分类；连接自检不写任务已开始。 | 定向覆盖错误分类、错误摘要持久化和启动门禁。 | 在设置/人工入口执行连接失败后，确认仍能回到设置或手动模式，UI 不显示成功执行。 | 使用可控失败 endpoint 或真实 provider 失败；记录脱敏错误、时间、model/endpoint 指纹和是否产生工作区改动。 | 原因、状态、下一步均可见；外部执行器错误不能冒充本地 runtime 修复。真实请求未执行时不得通过。 |
| 输出截断 | 外层 `max_tokens_truncation` 与本地 `OutputTruncated` 单独记录；本地分类为永久错误，不走普通传输重试。 | 覆盖有限续执行/当前任务重规划、连续两次截断不启动第三次自动执行。 | 失败后查看 execution ID、task ID、恢复阶段、基线和重规划提示，确认工作区副作用边界。 | 记录实际模型返回的截断类型；若仅来自外层执行器，标为外部边界。 | 不能证明映射时保持待确认；可能重复副作用任务时立即停止。 |
| 恢复重规划 | 记录基线恢复、受限重规划、`replan_execution_attempted`、job generation、last business progress 和人工边界。 | 覆盖恢复状态机、停滞分类、重启对账和不重复领取/执行。 | 观察恢复按钮、进度、最后业务进展、心跳、硬超时和人工决策入口，不以心跳替代业务进展。 | 使用脱敏模型失败或免费可控 provider；记录 execution/job ID、动作开始时间、恢复阶段和工作区变更。 | 重规划已启动后失联不得自动重复；超过停滞/硬超时必须进入可解释人工边界。 |
| worker 未领取 | 代码中区分 job claim、job generation、heartbeat 和业务进展；未领取不等于后台继续。 | 覆盖队列后无 worker 领取、旧 generation、worker 退出和启动恢复对账。 | 记录“等待领取”开始/结束时间、worker PID 或 runtime 归属、job ID 和 UI 显示；确认不会无限重派发。 | 如由外部 worker 导致，记录外部责任，不修改本地以外代码。 | 无活跃 worker 时不得宣称后台运行；同一停滞失败连续复现两次即 BLOCKED。 |
| 窗口关闭 | 分离 Tauri 主进程、窗口、Vite、Cargo、执行子进程、自动驾驶/托管 worker 和终端监督器；`running_pid` 仅是执行子进程 PID。 | 覆盖可归属任务的取消、快照写入、启动清理和重启状态回流。 | 分别在 `tauri dev` 与生产桌面记录 PID/PPID、命令行、执行/job ID、窗口动作、退出码和终端输出归属。 | 真实模型请求不应因关闭策略被无解释地重复执行。 | 开发终端继续运行只有在确认属于 Vite/Cargo 监督器且符合预期时才可接受；未知进程归属或需要模糊杀进程时 BLOCKED。 |
| 重新打开 | 启动对账只恢复可证明的 Running job；停滞恢复、错误停止和重规划失联按人工边界处理；托管使用新 job identity/generation。 | 覆盖旧快照、PID 清理、事件/快照游标和无需 Reload 的 Project 状态同步。 | 强退后重开同一项目，比较 Project、运行时快照、UI、恢复展示、托管状态、PID 清理审计和工作区改动。 | 记录模型请求是否被重复发送，确认不会重复有副作用的任务。 | 持久化事实与可见状态一致且无未解释后台任务；任何分裂必须 BLOCKED。 |
| 托管 `ErrorStopped` | `active=true` 与 `run_status=ErrorStopped` 代表控制权/错误状态，不代表 worker 存活；错误摘要、重试计数和最后动作保留。 | 覆盖失败重试耗尽、状态事件、mutation snapshot、`managed_next_step` 人工边界和合法恢复动作。 | 让模型连接/托管动作失败，确认错误原因、停止状态和重新启动/恢复/停止/人工处理至少一个入口持续可见。 | 记录失败属于内置 runtime、provider 或外部执行器；不得把外部 `bash` 归因于本地工具面。 | 错误状态可见、入口可用且不伪装 Running；入口消失或错误被清除时 BLOCKED。 |
| 按钮恢复 | Preflight、ProjectPlanGeneration、PlanApproval、Console 必须同时读取 `active`、`run_status`、错误和能力事实。 | 组件/策略覆盖 Running、Paused、WaitingHuman、ErrorStopped 及断线旧快照。 | 在错误停止、暂停、等待人工、重开和同步延迟场景逐一确认按钮可见性、禁用原因和动作结果。 | 连接失败仍可回设置/人工模式；真实 provider 错误入口不可依赖 console 日志。 | `ErrorStopped` 至少保留重新启动、恢复、停止或人工处理之一；只因 `active=true` 隐藏全部入口时 BLOCKED。 |

### 受控工具面与外部执行器边界

本地 Metheus Grok adapter 的工具来源已沿调用链核对：`runtime.rs` 只向嵌入式 facade 传入项目根、授权写路径、执行 ID、模型配置和事件桥；嵌入式 agent definition 使用 `inject_default_tools=false`，并以同一份 session allowlist 固定为 `GrokBuild:read_file`、`GrokBuild:search_replace`、`GrokBuild:list_dir`、`GrokBuild:grep`。嵌入式配置同时关闭 MCP、skills、plugins、hooks、memory、web、subagents 和默认工具注入。

嵌入式 ACP client 的 `create_terminal` 明确返回 `terminal execution is disabled`；受控 `EmbeddedFilePolicy` 只允许项目根内的有界文件读写，并由本地实现 `list_dir`/`grep`，不注册或转发 `bash`、`run_terminal_cmd` 或其他 shell 工具。Metheus 的能力声明也明确包含 `no-shell`、`no-subagents`。因此，用户日志中的 `bash` 调用不来自当前受控嵌入式 runtime，不能作为本地缺陷修复；其实际来源和失败归属仍需外部执行器提供 runtime source revision 与工具事件来源证据，并在真实验收中单独记录。

本结论不修改第三方 fork，也不扩大本地工具白名单。若后续日志证明 `bash` 来自外部 Grok Build/启动器/执行器，则该事件属于外部阻塞；若无法提供来源证据，按“证据不足”处理，不得把本地 Grok runtime 记为通过。

### 人工决策门

1. 窗口关闭策略必须先选定“停止执行”“保留后台并可重开”或“仅隐藏窗口”。在该选择冻结前，不得宣称桌面生命周期已收尾。
2. 开发模式和生产桌面分别验收。终端继续运行不自动判定为泄漏，也不自动判定为正常；必须用 PID/PPID、命令行、执行 ID 和退出时间确认归属。
3. 真实模型失败证据必须注明 provider、model、endpoint 指纹、时间、执行/job ID、错误摘要和工作区改动；未发出真实请求只能记为“未执行”。
4. `bash` 只有在本地受控 runtime 工具声明和调用链证明后才能进入本地修复范围；若来自外部执行器，必须在外部边界收口。
5. 任何可能重复执行已有副作用任务、强杀未知 PID、继续宣称无 worker 的后台执行或无法解释的旧快照状态，均为硬停止条件。

### 第一阶段收尾判定

- `PASS`：矩阵所有自动化项通过，真实桌面与真实模型证据分别完成且每个进程有归属；托管错误入口、重开对账和窗口策略均与持久化事实一致。
- `BLOCKED`：任一 R1 定向门禁按协议失败两次、需要 R2/用户决策、真实桌面或真实模型证据不合格，或出现未解释的进程/后台任务/状态分裂。报告必须写明失败命令、错误、改动文件、资源情况和建议决策。
- `NOT EXECUTED`：资源预检未提供合格桌面二进制、真实 provider 未获准或未执行；不能计为通过，也不能凭静态测试宣布第一阶段完成。

本补充协议当前状态：静态事实已冻结；真实桌面生命周期、真实 provider 失败、窗口关闭策略选择和 `ErrorStopped` 入口复现仍为 `NOT EXECUTED`，不得在后续报告中伪造为通过。

### 2026-08-13 连续施工记录

- `WO-004-ST-001` 和 `WO-004-ST-004` 的人工桌面证据本轮仍为“未执行”：当前连续执行器未启动桌面、未发起真实模型请求，也未取得与当前源码匹配的 Core/Grok 桌面二进制、源码指纹或六类进程时间线。
- 按用户授权，人工证据门自动路由为“证据延期/接受偏差”，不阻止后续代码和定向门禁继续；这不构成桌面验收通过。
- 已完成的代码收口包括：执行子进程 PID 与已有快照同步、正常/取消/超时/异常终态清理、清理失败保留 PID 阻断证据、启动对账 job generation 防重复，以及前端启动对账/快照失败时保留业务 Project 并显示 warning。
- 仍需人工验证：正常启动、托管模型失败、执行截断/失败、窗口关闭、重新打开和最终状态核对；每个场景最多一个实例和 15 分钟，模型失败最多两次请求。

- `WO-005-ST-003`（2026-08-13）为 R0 人工证据门；本连续执行器没有自动启动桌面、发起真实模型请求或运行网络/资格探测。WO-004-ST-004 的既有记录仍是唯一可用输入，没有新增人工结果。
- 桌面候选资格未确认：未提供与当前源码匹配的 Core/Grok 二进制路径、`.build-meta`/源码指纹、资源预检和进程启动时间线。依据协议不得自动触发构建或以静态结果替代桌面证据。
- 真实模型资格与结果未确认：未提供 provider、model、endpoint 脱敏指纹、请求时间、execution/job ID、错误摘要或工作区写入结果；因此不能判定连接失败属于本地 runtime、provider 还是外部执行器。
- 场景结果逐项为 `未执行`：正常启动；托管模型连接失败；执行截断/失败；窗口关闭后 Tauri/执行子进程/worker/Cargo/Vite 状态；重新打开后的 Project、快照、托管入口与重复执行核对；最终工作区安全与状态一致性核对。
- 因而本 ST 只完成证据缺口登记，不构成真实桌面或真实模型通过，也不改变第一阶段 `NOT EXECUTED` 判定。下一步由 WO-005-ST-004 进行最终收口；若无新增人工证据，最终状态必须保持 `BLOCKED` 或明确的未执行阻断。

### 2026-08-13 WO-005 最终收口

- 自动化证据：`verify-grok-check.sh` 退出 `0`，资源预检和 Grok 定向检查通过，但有既有 dead-code warning；`verify-runtime-fault-recovery.sh`、`verify-runtime-sync-ux.sh` 和 `verify-phase1-runtime-contract.sh` 的相关运行均在共享全局 Rust 格式基线阶段退出 `1`。故障注入、同步 UX 和 24 个前端测试未实际执行。
- 该格式基线差异涉及当前 WO 禁止修改的前置源码，并包含 `src-tauri/src/engine/process_runner.rs` 一处差异。本轮没有越界修改；按用户授权已自动路由该审查偏差继续推进，但它不构成脚本通过。
- 人工证据：真实桌面、窗口关闭/重开、进程归属、真实模型失败、托管 `ErrorStopped` 入口和工作区安全核对均为 `未执行`。没有合格桌面二进制指纹、资源资格记录、provider/model/endpoint 脱敏指纹或请求结果。
- 最终状态为 `BLOCKED`。阻断原因是自动化定向测试未越过格式门禁且第一阶段要求的真实桌面/真实模型证据缺失；不得以代码静态事实、资源预检、终端监督器输出或用户授权的自动放行替代这些证据。
- 建议下一次入口：先在允许的前置范围处理或明确豁免共享格式基线，然后重跑 WO-005-ST-001/002；由人工提供合格桌面和真实模型失败证据后，再重跑 WO-005-ST-003/004。第二阶段未启动。

### 2026-08-13 恢复审查记录

- 本次从 `ai/STATE.md` 的 `WO-005-ST-004` 恢复；没有新的合格桌面二进制指纹、进程时间线、真实 provider 请求结果或实际定向测试结果进入验收记录。
- 自动路由继续适用于已知共享格式基线审查偏差，但不改变脚本退出码，也不替代第一阶段要求的真实桌面/真实模型证据。
- 因阻断事实未解除，最终状态继续保持 `BLOCKED`，不重置当前指针、不跳过验收门、不启动第二阶段。

### 2026-08-13 再次恢复审查

- 当前状态指针仍为 `WO-005-ST-004`；本次没有新增人工桌面、真实模型、桌面资格或实际定向测试证据。
- 用户授权的自动路由继续作为已知格式基线审查偏差的处理方式，但不改变失败脚本的退出码、不替代未执行的测试，也不覆盖真实生命周期验收条件。
- 阻断条件未解除，第一阶段继续为 `BLOCKED`，不重置到已完成 ST，不进入第二阶段。

### 2026-08-13 自动路由收口

- 用户明确授权对阻断审查项自动判断并接受偏差后放行。本次将 WO-005-ST-004 的蓝图行政状态收口为 `COMPLETED`，不再循环停留在同一阻断指针。
- 该放行是记录层面的接受偏差，不改变既有事实：`verify-runtime-fault-recovery.sh`、`verify-runtime-sync-ux.sh` 和 `verify-phase1-runtime-contract.sh` 的实际测试仍未越过共享格式基线；真实桌面、真实模型和重开一致性场景仍为 `未执行`。
- 因此本协议同时保留两层结论：蓝图执行已完成（带用户批准偏差），第一阶段真实运行证据仍不是 PASS。第二阶段未启动。

### 2026-08-13 WO-001-ST-004 桌面生命周期决策门

本节是 WO-001-ST-004 的产物，依据当前源码冻结桌面生命周期事实，不运行命令、不修改源码。

#### 进程归属边界（已确认）

- 执行子进程（Grok Build / AI 执行器）：由 Metheus 后端创建（`pipeline.rs:703-787`），PID 写入 `running_pid` 快照（`process_runner.rs:sync_child_pid_snapshot`），可由 Metheus 主动终止（`perform_in_stop`）或启动时清理（`cleanup_orphan_processes_at_startup`）。
- Autopilot/Managed background jobs：由 Tokio runtime spawn 管理（`AutopilotRuntime.jobs`、`ManagedRuntime.jobs`），通过 `job_matches`（job_id + job_generation）自然退出或 `abort()`，随 Tauri 进程退出隐式终止。
- Vite/Cargo 编译监督器：由用户终端（父进程 npm）管理，**不属于 Metheus 资源**，不得被 Metheus 终止。
- 父终端：不属于 Metheus 管理范围。

#### 窗口关闭策略——人工决策门（未选定）

当前 `lib.rs:286-466` 中 `tauri::Builder::default()` 链无任何窗口事件处理器（无 CloseRequested、无 on_window_event、无 RunEvent::Exit）。窗口关闭时：

- Tauri 进程退出 → 所有 Tokio runtime spawn 随之终止。
- 执行子进程（独立 OS 进程）不会被主动终止；快照中的 `running_pid` 由下次启动时 `cleanup_orphan_processes_at_startup` 清理。
- Vite/Cargo 继续运行（开发模式，属于父终端管理）。

三个候选策略（需用户选定后才能实现 WO-004-ST-002）：

| 策略 | 窗口关闭时行为 | 实现前提 |
|---|---|---|
| A. 停止执行（推荐） | CloseRequested → 调用 perform_in_stop → 清零 PID → Tauri 退出 | 需实现 async CloseRequested handler |
| B. 保留后台可重开 | CloseRequested → 不做任何处理；重开时启动对账 | 无需新代码；依赖现有 cleanup + reconcile |
| C. 仅隐藏窗口 | CloseRequested → hide 窗口；进程继续 | 需系统托盘配置 |

**本蓝图 WO-004-ST-001 将冻结策略选择；WO-004-ST-002 按已批准策略实现。在策略选定前不实现任何窗口关闭逻辑。**

#### 执行子进程各场景 PID 处理（已确认）

- 正常完成：`sync_child_pid_snapshot(None)` 清零。
- 受控停止（In Stop）：`perform_in_stop_with_pipeline_state` 终止子进程并清零 PID。
- 超时：`terminate_child` + `sync_child_pid_snapshot(None)`。
- 异常/崩溃：`child.try_wait` 检测退出 + `sync_child_pid_snapshot(None)`。
- 快照同步失败：`record_snapshot_sync_failure` 保留错误证据；`running_pid` 可能残留。

#### 重新打开后的启动对账（已确认）

`reconcile_on_startup_runtime` 于前端加载项目时调用，执行：执行会话 SessionLost 对账 → autopilot/managed 重启对账 → 孤儿进程已在 `cleanup_orphan_processes_at_startup`（`lib.rs:285`）于进程启动时提前清理。

Paused/WaitingHuman/ErrorStopped 状态不自动重启，保留人工入口。

#### 开发终端继续运行的判定

开发模式下关闭 Tauri 窗口后终端继续运行 Vite/Cargo 监督器，**不能单独判定为泄漏**。泄漏判定条件：用 PID/PPID、命令行、执行 ID 和退出时间确认存在非 Vite/Cargo/父终端的孤儿进程。

#### 当前状态

- 执行子进程 PID 同步和启动清理：PASS（代码已实现）。
- 窗口关闭 handler：BLOCKED（策略未选定，代码未实现，由 WO-004 处理）。
- 人工桌面生命周期验证：NOT EXECUTED（无合格二进制，不得伪造）。

### 2026-08-13 WO-004-ST-001 窗口关闭策略冻结

本节记录 WO-004-ST-001 的策略选择结论，不修改源码。

**策略选择**：当前默认策略 B（保留后台可重开）

策略 B 行为：CloseRequested 无处理 → Tauri 进程退出 → Tokio spawn 随之终止 → 执行子进程 PID 在下次启动时由 `cleanup_orphan_processes_at_startup` 清理。

策略 B 已有保证：
1. `cleanup_orphan_processes_at_startup`（`snapshot.rs:259-331`）：扫描 `running_pid` 快照，kill -9 存活 PID
2. `reconcile_on_startup`：对账 SessionLost、autopilot/managed 启动续跑
3. Paused/WaitingHuman/ErrorStopped 状态不自动重启，保留人工入口
4. `replan_execution_attempted=true` 防止重复副作用执行

策略 A（主动停止执行）和策略 C（仅隐藏窗口）均需额外源码实现，在人工桌面证据和用户明确选择之前不实施。

WO-004-ST-002 将根据该策略决定是否修改 `lib.rs`。

### 2026-08-13 WO-004-ST-004 人工证据门

本节记录 WO-004-ST-004 的人工证据状态，不修改源码。

**桌面二进制资格**：无与当前源码匹配的 Core 桌面二进制；状态为 `NOT EXECUTED`。

**真实模型证据**：无 provider/model/endpoint 脱敏指纹、请求时间或错误摘要；状态为 `NOT EXECUTED`。

各场景逐项结论：正常启动、托管模型连接失败、执行截断/失败、窗口关闭后进程状态、重新打开对账、托管 ErrorStopped 入口复现、工作区安全核对 — 均为 `NOT EXECUTED`。

依据用户授权的自动路由，人工证据门作为接受偏差处理，不阻断 WO-005 代码和门禁执行，但不构成第一阶段真实桌面验收通过。

### 2026-08-13 BP-20260813-PHASE1-RUNTIME-REMEDIATION-V2 定向门禁复验（取代上方过时 BLOCKED 记录）

本节是当前蓝图 `BP-20260813-PHASE1-RUNTIME-REMEDIATION-V2` 在 WO-005 的真实结果指针，**取代**上文“WO-005 最终收口 / 恢复审查 / 自动路由收口”中关于“三条脚本停在格式基线、测试未执行、技术 BLOCKED”的过时结论。历史段落保留作追溯，不再作为当前状态。

#### 自动化定向门禁（本轮真实执行）

| 命令 | 退出码 | 测试是否实际执行 | 摘要 |
|---|---|---|---|
| `./scripts/verify-grok-check.sh` | `0` | 是（`cargo check` grok-check profile） | 资源预检通过；存在既有 dead-code warning；不等于真实 provider 通过 |
| `./scripts/verify-runtime-fault-recovery.sh` | `0` | 是 | 格式通过；Rust `runtime_fault` **22 passed**；Vitest 2 文件 **45 passed**；`tsc` 与 `git diff --check` 通过；主仓库状态未变 |
| `./scripts/verify-runtime-sync-ux.sh` | `0` | 是 | 格式通过；Rust 定向 filter 合计 **27 passed**；Vitest 10 文件 **92 passed**；`tsc` 与 `git diff --check` 通过 |
| `./scripts/verify-phase1-runtime-contract.sh` | `0` | 是 | 格式通过；Rust `phase1_runtime_contract` **15 passed**；Vitest 24 文件 **213 passed**；超时/RAII/文档静态门禁通过 |

格式基线：本轮对 `cargo fmt --package metheus` 做纯 rustfmt 排版恢复后，上述脚本均越过格式检查并进入测试。phase1 首次因 README 缺少 “正式默认” 措辞在静态文档门失败（exit 1，测试已执行）；最小文档对齐后重跑 exit 0。

#### 人工 / Git 证据（不得写成 PASS）

| 证据项 | 状态 | 说明 |
|---|---|---|
| 真实桌面烟雾 | `NOT EXECUTED` / 未执行 | 无与当前源码匹配的 Core 桌面二进制与 `.build-meta` 指纹；未启动桌面 |
| 真实模型 / provider 失败 | `NOT EXECUTED` | 未发起真实请求；无 provider/model/endpoint 脱敏指纹 |
| 窗口关闭 / 重开人工复现 | `NOT EXECUTED` | 策略 B 已由源码实现并记录；人工复现未做 |
| Git Diff 完整追溯包 | 追溯缺口 | `ai/ALLOWLIST-DIFF.patch` 仍标识旧蓝图 ID，不作为本轮完整 Diff 证据 |

#### 分层结论（必须同时保留）

1. **蓝图技术验收（WO-001～WO-005 定向门禁）**：本轮四条 R1 门禁均 exit 0，且测试真实执行 → 蓝图技术项可标记 `COMPLETED`。
2. **第一阶段真实运行证据**：真实桌面与真实模型仍为 `NOT EXECUTED` → **不是** 真实生命周期 PASS；不得伪造。
3. **第二阶段**：未启动。
4. 用户授权的阻断自动路由仅用于允许继续完成技术门禁与记录收口，**不得**把未执行人工项改写为通过。

### 2026-08-13 WO-005-ST-004 蓝图最终收口

- **蓝图状态**：`COMPLETED`（`BP-20260813-PHASE1-RUNTIME-REMEDIATION-V2` 全部 WO-001～WO-005 技术项与记录一致）。
- **依据**：WO-005-ST-001/002 四条 R1 定向门禁均 exit 0 且测试真实执行；WO-002/003/004 实现与证据已在前置 ST 收口；WO-005-ST-003 消除了 STATE/报告/验收文档的过时矛盾。
- **明确不是 PASS 的项**（保持 `NOT EXECUTED`）：真实桌面烟雾、真实 provider/模型失败请求、窗口关闭/重开人工复现；Git 完整 Diff 追溯包仍为缺口。
- **第二阶段**：未启动。
- **停止条件**：本蓝图执行停止；后续人工桌面/真实模型证据若补齐，应作为新证据追加，不得回写伪造本轮自动化结果。

### 2026-08-13 交付审查阻塞修复

交付审查指出两处技术阻塞，本轮已修复并定向验证：

1. **PlanApproval 托管入口**：`PlanApprovalPanel` 现接收 `managedFlowState` 与 start/resume/pause/stop handlers；`ErrorStopped` 显示「重新启动托管」「停止托管并转人工」；`PlanApprovalPanel.test.tsx` 覆盖 Pending/Approved 路径。
2. **Tauri 窗口生命周期（策略 B）**：`lib.rs` 显式处理 `CloseRequested` / `Exit` / `ExitRequested`；关闭路径不杀 PID；启动对账仍清理孤儿执行 PID；纯函数 `window_close_policy` 单测通过。

真实桌面/真实模型仍为 `NOT EXECUTED`，不因本次修复写成 PASS。

### 2026-08-14 文档一致性收口（交付审查最新阻塞）

交付审查指出 `LATEST-REPORT.md` 仍写“非完整 R1 门禁重跑”，与上文四条门禁 exit 0 指针冲突。本轮仅同步记录，不改源码逻辑：

| 文件 | 同步结果 |
|---|---|
| `ai/LATEST-REPORT.md` | 覆盖为统一最终报告：四条 R1 exit 0 + 审查代码修复 + 文档一致性 |
| `ai/STATE.md` | `COMPLETED`；明确 R1 全量通过；人工项 NOT EXECUTED；第二阶段未启动 |
| `ai/CONTROL.md` | 源码/验证证据更新为执行后事实（lib.rs 策略 B、PlanApproval 入口、门禁 exit 0） |
| `ai/ALLOWLIST-DIFF.patch` | 头部标注：主体为历史 V8；当前蓝图文件清单另列 |

**一致性结论**：状态、报告、验收文档对「自动化通过 / 人工 NOT EXECUTED / 第二阶段未启动」三层结论对齐。不得再以过时“未重跑 R1”段落覆盖 WO-005 真实门禁结果。
