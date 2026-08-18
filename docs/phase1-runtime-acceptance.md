# v0.0.4 第一阶段真实运行验收协议

## 1. 目的与结论分层

本协议只覆盖 v0.0.4 第一阶段真实运行收尾，不进入 Hermes、ML、成本分级或并行执行。它把技术自动化收口和真实体验收口分开：

- `TECHNICAL_PASS`：定向自动化契约、状态/账本/资源/owner/窗口边界均通过。
- `REAL_EXPERIENCE_PASS`：经批准的六任务受控真机/Grok 长链路通过，并且每个任务的执行、恢复、质量、验收和确认事实一致。
- `PHASE1_CLOSEOUT_PASS`：仅当 `TECHNICAL_PASS` 和 `REAL_EXPERIENCE_PASS` 同时成立。
- 缺少 R2 批准、存在未解释资源/进程归属、验收缺失、状态分裂或任一必要证据未知时，只能记录 `BLOCKED`，不能宣告真实体验收尾。

“恢复动作已完成”不等于执行成功；“执行成功”不等于质量通过；“质量通过”不等于验收 ledger 通过；Git 确认、心跳、基线恢复和用户接受偏差均不能单独产生完成态。

## 2. 固定六任务场景

验收对象是六个串行叶子任务，必须使用同一 `run_id`、稳定的 `task_id` 和不重复的 `execution_id/generation` 记录：

1. 任务一至任务五：执行、质量门禁和 Git 确认已知为成功，但仍需在本轮账本中逐项可追溯。
2. 任务六：必须覆盖输出截断、一次 continuation/当前任务受限重规划、业务进展 warning、自动修复、视觉复测失败或通过、再次执行、资源压力/被杀、心跳、窗口关闭和终端退出。
3. 任务六的任一恢复成功不得覆盖前一次执行结果、复测失败、ledger 项或 Git 事务。
4. 任何一次重试必须绑定原任务、次数、墙钟、资源边界和副作用边界；不得通过新 run 隐藏失败或重复已确认事务。

### 场景索引

- 场景 1：六任务使用稳定的 `run_id`、`task_id`、`execution_id` 和 generation。
- 场景 2：前五个任务分别记录执行、质量、验收 ledger 和 Git 确认事实。
- 场景 3：第六个任务的输出截断映射为独立失败类别。
- 场景 4：截断最多允许一次 continuation，并保留原执行结果。
- 场景 5：当前任务受限重规划最多执行一次，不重新生成整阶段计划。
- 场景 6：业务进展、心跳、warning、stalled 和 hard-timeout 分别记录。
- 场景 7：视觉复测或自动修复复测失败进入人工边界，不伪造通过。
- 场景 8：资源 warning、hard-stop、不可测和终止来源分别记录。
- 场景 9：内置 future、插件 child、恢复 worker、托管 job 和外部监督器分别归属。
- 场景 10：断线、陈旧快照和陈旧心跳关闭危险写操作并保留同步动作。
- 场景 11：窗口关闭、Tauri 退出和重开对账保留执行身份，不强杀未知 PID。
- 场景 12：恢复、重试和重开不重复已确认 Git 事务或清空 ledger。
- 场景 13：技术自动化与真实桌面/Grok R2 结论分栏，未批准 R2 保持 BLOCKED。

Core 代码封板门禁：2026-08-01 已通过；10 项人工终态安全 Rust 测试通过；46 项测试通过。
真实桌面烟雾：未执行；当前环境没有可复用桌面程序，等待经批准的 R2 长链路。

## 3. 每次运行的必填证据

验收记录至少要有以下字段；未知值必须写 `Unknown` 并说明原因，禁止以空值表示通过：

| 字段组 | 必填字段 |
| --- | --- |
| 运行身份 | `run_id`、`task_id`、criterion index/text、`execution_id`、`generation`、开始/结束时间 |
| owner | owner 类型（内置 future、插件 child、恢复 worker、provider、外部执行器或开发监督器）、claim 时间、最后心跳、最后业务进展、owner 终态 |
| 执行/恢复 | 执行结果类别、恢复动作类别、重试/continuation 次数、墙钟、取消来源、是否有活跃 owner |
| 质量/验收 | quality gate 结果和证据、ledger 原因或逐项状态、已证明 criterion、Unknown/Unsatisfied/Contradictory 项 |
| 确认 | Git/人工确认事务、目标阶段、事务 ID、是否重复或与旧 generation 冲突 |
| 资源 | 采样可测性、cgroup 限额/使用（若可用）、进程 RSS 峰值、warning/hard-stop 时间、终止分类、峰值摘要 |
| 窗口/终端 | 窗口关闭决策、Tauri 生命周期、终端/开发监督器、内置 future、插件 child 的观察结果和退出顺序 |
| R2 资格 | `R2_APPROVAL`、批准人、批准时间、批准范围、`run_id`、停止权限、实际执行时间；未批准必须写 `NOT_GRANTED` |

资源采样不能测量时记录 `Unknown/不可测`，不能推导安全；PID 存活不能证明归属，必须有 execution ID/generation 和可用进程身份。

## 4. 技术自动化分栏

技术自动化只验证合约和可重复的定向门禁，不冒充真实桌面/Grok 长链路：

| 编号 | 门禁 | 技术 PASS 条件 | BLOCKED 条件 |
| --- | --- | --- | --- |
| A-001 | 唯一完成态 | 后端同时满足执行成功、恢复收敛、质量通过、ledger 合法、确认达标和人工边界完成 | 任一层 Unknown、Unsatisfied、Contradictory、Pending 或无确认事务 |
| A-002 | ledger | 有标准的任务在执行前有稳定 criterion index/text ledger；恢复、重试、重开保留已证明项 | ledger 缺失、无法关联、重排、清空或把空账本解释为通过 |
| A-003 | 截断/恢复 | 最多一次 continuation 和一次当前任务受限重规划，复测失败进入明确人工边界 | 无限重试、整阶段重生成、截断被记为成功或复测失败被覆盖 |
| A-004 | owner/时间 | owner claim、generation、心跳、业务进展、warning、stalled、hard-timeout 分离 | 心跳冒充业务进展、陈旧 worker 写入、超时后继续无界修改 |
| A-005 | 资源 | 有 warning、hard-stop、不可测三态；终态记录峰值/阈值跨越/资源终止类别 | 固定 8GB 通用阈值、无归属 kill、外部 kill 后才保存唯一事实或吞错 |
| A-006 | 同步/UI | 前端只消费后端 presentation/mutation；陈旧快照和断线有安全写操作门禁 | 用错误文本推导运行态，恢复文案暗示完成，陈旧心跳仍允许危险写操作 |
| A-007 | 窗口/PID | 关闭策略与实际语义一致；重开按执行身份验证后清理 | 未知 PID 强杀、把窗口关闭说成后台保活、混淆 Tauri/监督器/child |

技术自动化全部通过时，只能得到 `TECHNICAL_PASS`。如果六任务人工复验尚未执行，`REAL_EXPERIENCE_PASS` 仍为 `NOT_RUN`，整体不通过。

## 5. 受控人工分栏

人工步骤用于补齐自动化无法证明的真实边界，必须逐项记录，不得用人工接受偏差替代失败证据：

| 人工阶段 | 必须观察 | 通过条件 | 未通过处理 |
| --- | --- | --- | --- |
| 六任务夹具准备 | 固定顺序、任务身份、基线改动、criterion ledger、停止点 | 六项均有唯一身份，基线和用户改动可区分 | 停止；不得开始下一项，记录夹具缺项 |
| 无网络模拟长链路 | 截断、恢复、review、ledger、owner、资源和同步事件 | 每个异常有唯一终态和可执行下一步，无状态分裂 | `BLOCKED`，保留现场和失败类别 |
| 窗口/终端对账 | 关闭窗口后 Tauri、Tokio future、插件 child、Vite/Cargo 和终端监督器 | 每个进程有 owner/身份/终态；未知 PID 不清理 | 进入人工边界，不重开、不重复执行 |
| 真实体验观察 | 用户文案、业务进展、心跳、资源 warning/hard-stop、ledger 展示 | 文案准确，完成公式和实际观察一致 | 真实体验 `BLOCKED`，不得以“已恢复”收尾 |

## 6. R2 真实桌面/Grok 分栏

历史版本曾将 `WO-009-ST-003` 定为唯一 R2；在当前蓝图中，唯一真实 R2 改由 `WO-006-ST-002` 承担。两者都必须在执行前停止并等待明确批准，且批准范围必须明确包含真实桌面/Grok 六任务长链路。历史坐标只用于事件追溯，不得复用其租约或证据。

R2 资格记录：

```text
R2_APPROVAL: NOT_GRANTED | GRANTED
APPROVER: <explicit human approver>
APPROVED_AT: <timestamp>
APPROVED_SCOPE: <desktop/Grok six-task long chain>
RUN_ID: <single approved run>
STOP_AUTHORITY: <named human/operator>
```

R2 PASS 必须同时满足：六项均按顺序完成或进入明确失败终态；第六项的截断/恢复/复测/资源/心跳/窗口/终端记录完整；没有未解释 OOM、进程归属、重复执行、状态分裂或验收缺失；每个需要验收的任务 ledger 无 `Unknown`、`Unsatisfied`、`Contradictory`；任务完成公式全部成立。

R2 未批准不是 PASS，也不是“稍后自动执行”；记录为 `BLOCKED: R2_NOT_GRANTED`，停止在该 ST，等待用户明确批准。

## 7. PASS/BLOCKED 判定

单个任务的 `PASS` 必须满足唯一完成公式：

```text
execution == Succeeded
AND recovery in {NotRequired, ActionSucceeded}
AND no active recovery owner
AND quality == Passed
AND ledger in {NoCriteria, Verified}
AND required confirmation reached
AND required manual boundary complete
```

四种空 ledger 原因必须区分：

- `NoCriteria`：任务合同明确无验收标准；不是“验收通过”文案。
- `WaitingInitialization`：有标准但账本尚未创建；阻断。
- `AwaitingVerification`：账本已创建但逐项验证未完成；阻断。
- `StateAnomaly/Unknown`：应有账本却缺失、损坏、无法关联或与 generation 冲突；阻断并人工核对。

阶段判定：

| 判定 | 条件 |
| --- | --- |
| `TECHNICAL_PASS` | A-001 至 A-007 全部通过，且没有未解释的静态契约冲突 |
| `REAL_EXPERIENCE_PASS` | 已获 R2 批准，六任务长链路和人工分栏全部通过 |
| `PHASE1_CLOSEOUT_PASS` | 前两项同时通过 |
| `BLOCKED` | 任一必要证据 Unknown、任一门禁失败、R2 未批准、资源/进程归属不明、重复执行风险或状态/报告/ledger 矛盾 |

如果只有技术自动化通过，结论必须写为“技术自动化收口，真实体验尚未收尾”，不能写“第一阶段完成”。

## 7A. 当前状态/事实/动作矩阵（WO-001-ST-003）

前端显示必须消费后端 `RuntimeSnapshot`、`RecoveryPresentation`、ledger 和同步状态。四类事实状态保持独立：`Unknown` 表示事实缺失或不可归属，`Pending` 表示事实正在形成，`Blocked` 表示有明确失败/冲突，`Passed` 表示该事实已满足；没有验收/质量要求时只能使用后端明确的 `NoCriteria`/`NotRequired`。

| 状态 | 显示 | 必要事实 | 允许动作 | 禁止动作 |
| --- | --- | --- | --- | --- |
| `unknown` | 事实不足 | 快照、任务身份或关键事实缺失/未对账 | 同步、重试、读取 | 执行、确认、恢复、接受偏差、完成宣告 |
| `idle` | 等待执行 | 无活跃执行/恢复且同步新鲜 | 后端授权的准备、执行或自动驾驶入口 | 绕过能力门写项目、重复启动 |
| `executing` | 执行中 | pipeline/session 运行中，execution 未收口 | 同步、后端授权的暂停/停止 | 确认、验收、重复执行 |
| `recovering` | 恢复中 | recovery active，阶段、owner/心跳和停止边界可见 | 同步、后端提供的恢复动作 | 把恢复当执行成功、重复恢复、直接确认 |
| `validating` | 验证中 | 执行有结果，质量或 ledger 仍 Pending | 等待、同步、后端授权的验证/复测 | 完成宣告、跳过 Unknown、整阶段重生成 |
| `awaiting_confirmation` | 待确认 | 执行/质量/验收事实已有，confirmation required | 后端授权的确认、驳回或人工边界 | 将待确认写成完成、重复确认 |
| `quality_blocked` | 质量受阻 | quality/acceptance 明确 Blocked | 同步、修复、复测或人工处理 | 用恢复文案覆盖失败、直接确认 |
| `completed` | 已完成 | 完成公式全部成立 | 读取历史、新任务入口 | 覆盖结果、重复确认、清空 ledger |
| `failed` | 执行失败 | 明确失败终态 | 同步、授权恢复/重试、人工边界 | 把失败当待确认或完成、清理未知 PID |
| `waiting_human` | 等待人工 | 后台停止，人工选项/原因/任务明确 | 同步、填写授权人工决策 | 自动重跑、跳过未知 criterion、伪造确认 |

同步写门是独立事实：无快照、`idle`、`syncing`、`delayed`、重连中和 `disconnected` 均禁止普通危险写；只有存在当前快照且状态为 `synced` 才允许普通写。安全同步动作可以保留，但停止/关闭动作必须由后端明确证明当前执行身份可控，不能只靠按钮存在推断。

## 8. 本协议的执行边界

本文件只定义验收契约和证据字段。它不授权运行命令、启动桌面、启动服务、发起 provider 请求、执行 R2 或修改源码；这些动作必须由对应 WO/ST 的允许范围和验证等级单独授权。

## 9. TabZero 六任务复验 Runbook

本 runbook 只冻结 R2 前的可重复步骤，不表示真实桌面/Grok 已执行。复验必须使用独立测试项目和单一 `run_id`，不得在主仓库工作区、主仓库 Git 事务或用户已有改动上运行。

### 9.1 固定夹具与身份

- 项目：独立 TabZero 测试项目；主仓库只作为验收记录载体，不作为被修改对象。
- 顺序：六个串行叶子任务，固定为 `T1` 至 `T6`，不得并行、跳项或更换顺序。
- 身份：每项记录唯一 `task_id`、`execution_id`、`generation`、owner claim、开始/结束时间；重试只能复用原 `task_id` 并递增受控次数。
- 基线：记录测试项目初始 Git head、工作区文件摘要和用户改动；每项结束保存文件摘要与最终 Git 状态。

### 9.2 六项任务合同

| 任务 | 必须观察的合同 | 必须保留的终态证据 |
| --- | --- | --- |
| T1-T5 | 执行、质量门、Git 确认和逐项 ledger | execution/job ID、owner、测试结果、criterion 状态、确认事务 |
| T6-A | 输出截断、一次 continuation、当前任务受限重规划 | 截断类别、continuation 次数、replan 次数、原结果与新结果 |
| T6-B | 90 秒业务进展 warning、300 秒 stalled、恢复硬上限 | 业务进展时间戳、心跳、warning/stalled/hard-timeout 事实 |
| T6-C | 自动修复和视觉复测 | 复测命令、视觉证据、失败或通过结论；不得用“已恢复”代替质量结果 |
| T6-D | 资源压力/被杀、窗口关闭、终端退出、重开对账 | RSS/cgroup 峰值、warning/hard-stop、kill 来源、PID 身份、窗口/终端结果 |

T6 的验收 ledger 必须包含视觉 criterion：`主题切换背景渐变具有平滑过渡`。该 criterion 在独立验证前保持 `Unknown`，视觉复测失败必须为 `Unsatisfied` 或进入人工边界，不得删除、重排或覆盖已有项。

### 9.3 每项记录表

每个任务必须记录以下字段；不可测字段写 `Unknown` 并附原因，禁止用空值表示通过：

```text
run_id:
task_id:
execution_id:
generation:
ledger: criterion_index / criterion / status / evidence / evidence_references / updated_at
owner: type / claim_at / last_heartbeat_at / last_business_progress_at / terminal_state
execution: result_kind / retry_count / continuation_count / wall_clock_secs
recovery: action_kind / replan_count / active_owner / terminal_state
resource: source / cgroup_limit / current_rss / peak_rss / warning_at / hard_stop_at / kill_kind / kill_source
process: pid / executable / process_start_identity / ownership_check / cleanup_decision
window_terminal: window_decision / tauri_state / tokio_state / child_state / supervisor_state / reopen_result
workspace: baseline_head / user_change_boundary / final_head / final_status
final_decision: PASS | FAIL | STOP
```

### 9.4 人工步骤与判定

1. 创建独立项目，冻结基线和六项 ledger；任何身份或基线缺失立即 `STOP`。
2. 依次运行 T1-T5；每项必须先确认执行结果、质量门、ledger 和 Git 事务，再进入下一项；任一 Unknown/Unsatisfied/Contradictory 为 `FAIL`。
3. 对 T6 注入已批准的截断/复测/资源观察夹具，记录每个边界和停止点；不得真实制造 OOM，不得杀父进程、未知 PID 或系统服务。
4. 观察窗口关闭与重新打开对账；无法证明 owner、PID 身份、execution ID/generation 或终止来源时为 `STOP`，不得重开或重复执行。
5. 只有六项均满足唯一完成公式、ledger 无阻断状态、资源/进程/窗口证据完整且最终工作区符合基线时才记录 `PASS`。
6. 任一任务失败记录 `FAIL` 并保留现场；资源不可测、费用不可控、权限越界、状态分裂或需要人工授权时记录 `STOP`，不得宣告真实体验收尾。

R2 未获明确批准时，以上步骤状态固定为 `NOT_RUN`；本节不授予 R2 执行权。

## 10. 接续施工矩阵与停止门

本节是 `BP-20260815-PHASE1-RUNTIME-CLOSEOUT-V2` 的历史接续基线。表中的 WO-008/009/010 坐标描述历史事件，不覆盖当前 `BP-20260818-V004-PHASE1-UX-FUNCTIONAL-CLOSEOUT-V1` 的状态指针。行政上的人工放宽只能允许继续审查和施工，不能把缺失的技术结果写成 `PASS`，也不能替代 R2 批准、原生窗口控制面或六任务证据。

| 门 | 前置事实 | 通过条件 | 当前结论/失败动作 |
| --- | --- | --- | --- |
| G0 基线 | 上一轮 R2 原生控制面不可观测；历史技术摘要缺少逐命令结果 | 当前事实、报告和状态一致 | 已记录；未知证据保留 Unknown |
| G1 资源语义 | `Continue`、`Warning`、`HardStop`、`Unknown` 必须互斥且可追溯 | `Continue` 不再持久化为 `Unknown`，旧数据缺失仍为 Unknown | 未通过时停在 WO-002，禁止宣称资源安全 |
| G2 生产接线 | 内置 future 和插件 child 都必须低频调用 `runtime_resource::observe` 并更新 guard | 生产采样可取消、有界，HardStop 只取消当前执行 | 未通过时停在 WO-003，夹具测试不能替代接线 |
| G3 六任务 fixture | long-chain 必须创建 fake provider、T1-T6、owner、ledger、故障和唯一终态 | 输出结构化六任务事实且断言无重复执行 | 未通过时停在 WO-004，包装脚本不能算通过 |
| G4 完成/恢复/UI | 执行、恢复、质量、ledger、确认、owner、同步和 UI 文案独立保存 | 空 ledger、stale、复测失败和恢复动作均有明确边界 | 未通过时停在 WO-005/006 |
| G5 原生控制面 | 必须看到并操作 Tauri 原生窗口，且能区分 Tauri、future、child、监督器和终端 | 控制面资格通过、PID 身份可核对、未知 PID 不清理 | 未通过时停在 WO-007，不得启动昂贵会话 |
| G6 R1 技术证据 | WO-008 每条允许命令必须实际进入目标测试并记录 exit、数量和结构化断言 | 资源、long-chain、恢复、同步、phase1、Grok 门禁均有独立结果 | 缺任一结果则保持技术未收口 |
| G7 R2 输入 | 独立项目、单一 run_id、脱敏 provider、预算、批准范围和停止权限齐全 | WO-009 记录完整，缺字段不得编译/启动 R2 | 缺字段时 `BLOCKED: R2_INPUT_INCOMPLETE` |
| G8 人工批准 | 批准必须明确桌面/Grok 六任务、网络、费用、30 分钟和停止人 | `R2_APPROVAL=GRANTED` 且批准范围匹配 | 未批准时 `R2_NOT_GRANTED`，不自动执行 |
| G9 唯一 R2 | 仅 `WO-010-ST-003` 可执行真实 Tauri/Grok 长链路 | 六任务证据完整，且无未解释资源/进程/状态问题 | 失败保留现场并 `BLOCKED`，禁止自动重跑 |

### 停止门优先级

1. 先完成 G1-G4 的代码和无网络技术证据，再核验 G5 原生窗口控制面。
2. G5 未通过时，不得编译或启动 R2；浏览器 Vite 页面不满足资格。
3. G6 未形成逐命令结果时，只能记录技术证据缺失，不能写 `TECHNICAL_PASS`。
4. G7/G8 任一缺失时，不得进入唯一 R2；不得用用户放宽、自动路由或脚本 `exit 0` 代替批准和现场证据。
5. 任一门发生未知 PID、重复副作用、状态分裂、资源不可归属或命令未实际执行，停止当前链路并保持现场。

## 11. BP-20260815-PHASE1-RUNTIME-CLOSEOUT-V2 R1 结果

本节记录本轮 `WO-008` 的最终逐命令证据。每条命令均按对应 ST 串行执行；未把脚本包装退出码作为唯一证据，目标测试数量和结构化断言同时记录。

| ST | 命令 | 最终结果 | 实际证据 |
| --- | --- | --- | --- |
| `WO-008-ST-001` | `./scripts/verify-runtime-outcome-contract.sh` | `PASS`, exit `0` | Rust `15 passed, 0 failed`；前端 `24 files, 219 passed, 0 failed` |
| `WO-008-ST-001` | `./scripts/verify-runtime-resource-safety.sh` | `PASS`, exit `0` | Rust `22 passed, 0 failed`；前端 `2 files, 48 passed, 0 failed`；runtime resource safety `PASS` |
| `WO-008-ST-002` | `./scripts/verify-runtime-long-chain-closeout.sh` | `PASS`, exit `0` | Rust `7 passed, 0 failed`；6 条 `LONG_CHAIN_TASK_FACT`；1 条 `LONG_CHAIN_SUMMARY`；`task_count=6`、`violation_count=0`、`reopen_violation_count=0` |
| `WO-008-ST-002` | `./scripts/verify-runtime-fault-recovery.sh` | `PASS`, exit `0` | Rust `22 passed, 0 failed`；前端 `2 files, 48 passed, 0 failed`；四类锁夹具、临时目录和工作区审计通过 |
| `WO-008-ST-002` | `./scripts/verify-runtime-sync-ux.sh` | `PASS`, exit `0` | 同步、快照、恢复和幂等性 Rust 分组均通过；前端 `10 files, 96 passed, 0 failed` |
| `WO-008-ST-003` | `./scripts/verify-phase1-runtime-contract.sh` | `PASS`, exit `0` | Rust `15 passed, 0 failed`；前端 `24 files, 219 passed, 0 failed`；超时策略和 `ActivityGuard::drop` 接线审计通过 |
| `WO-008-ST-003` | `./scripts/verify-grok-check.sh` | `PASS`, exit `0` | `grok-check` profile 受控边界检查完成；无 provider 请求、网络或桌面启动 |

长链命令首次目标测试为 `7 passed`，但脚本使用行首匹配把首条与 Rust 测试名同一行的结构化事实漏计为 5 条；已在当前 ST allowlist 内修正计数审计并重跑，最终 6 条事实和汇总审计均通过。该修复不改变 fixture 或业务语义。

### 分栏结论

- `TECHNICAL_PASS: YES`：G1-G4、G6 的定向自动化证据齐全，资源、账本、owner、恢复、同步和阶段/Grok 受控边界均有实际结果。
- `REAL_EXPERIENCE_PASS: NOT_RUN`：未执行唯一 R2；原生窗口控制资格仍为 `DESKTOP_CONTROL_ELIGIBLE=no`，不能把 Vite 页面或技术编译视为真实桌面证据。
- `PHASE1_CLOSEOUT_PASS: NO`：缺少真实桌面/Grok 六任务现场和 R2 批准记录，当前阶段仍未完成最终收口。
## WO-001-ST-001 技术基线冻结

本验收协议沿用前序 R1 矩阵对技术能力的通过结论，并将其与真实桌面体验明确分层：

| 分层 | 结论 | 证据边界 |
| --- | --- | --- |
| `TECHNICAL_PASS` | `YES` | 资源 guard/生产采样、长链 T1-T6 结构事实、完成态/ledger/恢复/重开、UI/同步及 `grok-adaptive` 资格的既有 R1 证据 |
| `NATIVE_CONTROL_PASS` | `NO/NOT_RUN` | 尚无命名操作员、实时窗口通道、截图/输入/关闭/重开回传演练结论 |
| `R2_PASS` | `NO/NOT_RUN` | 前次启动在 NATIVE_READY 前停止，provider 请求为 0，T1-T6 未运行；当前唯一真实 R2 坐标为 `WO-006-ST-002`，历史租约不可复用 |
| `REAL_EXPERIENCE_PASS` | `NO/NOT_RUN` | 不得以浏览器页、构建成功、自动化 R1 或旧行政完成替代原生桌面证据 |

以上冻结项不产生新的实现或测试要求；后续工单只补齐当前蓝图列出的原生通道、隔离项目、租约和唯一真实 R2 证据。
## 当前验收坐标裁决（WO-001-ST-002）

本节是当前蓝图对本协议旧坐标的统一修订，适用于第 6、10、11 节及 G5/G7/G8/G9：

| 坐标 | 当前要求 |
| --- | --- |
| 唯一真实 R2 | 仅 `WO-006-ST-002`；其他 WO/ST 只能产生准备、资格或历史记录，不得标记为唯一 R2 |
| `NATIVE_CONTROL_PASS` | 必须在任何编译或 Tauri 启动前成立；需要命名操作员、实时消息通道、截图回传、输入/关闭/重开能力及停止权限证据 |
| provider 请求门 | `NATIVE_READY`、原生窗口证据和运行期资源门同时满足后才允许请求；前次 provider 请求为 0 的停止事实不可覆盖 |
| G5 | 原生人工控制资格在编译前通过；浏览器页、自动化 R1 或用户宽免不构成通过 |
| G7 | 仅在原生资格和批准前资源资格均通过后进入隔离项目与租约准备 |
| G8 | 启动租约只授权一次 Tauri 启动；真实运行坐标固定为 `WO-006-ST-002`，不可复用已消费租约 |
| G9 | provider、T1-T6、资源、PID/owner/generation、ledger、关闭/重开及 workspace 证据齐全后才可裁决 R2/真实体验通过 |

历史文档中与上述坐标冲突的“唯一 R2”表述按事件时间线保留，但不再作为当前唯一 R2 坐标或通过证据；不得通过删除真实证据要求来消除冲突。
## WO-001 接续验收矩阵

| 分层/门 | 当前结论 | 责任 WO |
| --- | --- | --- |
| `TECHNICAL_PASS` | `YES`，由前序 R1 证据支持 | 已冻结，不重做 |
| 当前 Git Diff | `PENDING`，尚无当前完整 Diff/diff-check 封存 | `WO-007` |
| `NATIVE_CONTROL_PASS` | `NO/NOT_RUN`；原生通道和窗口/PID 证据缺失或 `Unknown` | `WO-005` |
| `R2_ENTRY_READY` | `PENDING`；资源、隔离项目、runbook、一次性租约未完成 | `WO-005` |
| `R2_PASS` / `REAL_EXPERIENCE_PASS` | `NO/NOT_RUN`；provider=0，T1-T6 未运行 | `WO-006` |
## 历史原生控制资格阻断（WO-003-ST-001）

在原生操作员、双向实时通道、截图、输入、关闭/重开和停止权限均有可审查证据前，`NATIVE_CONTROL_PASS` 必须保持 `NO/NOT_RUN`。当前这些字段均为 `UNKNOWN`，因此编译前资格门未通过；后续 R2 工单不可进入。

## WO-005-ST-002 Native 证据包与握手

本节只定义人工提交和审查契约，不产生原生资格或 R2 执行授权。初始状态必须为 `PENDING_EXTERNAL`；`UNKNOWN`、用户批准、Codex 对话、浏览器/Vite 页面和 readiness UI 都不是 Native 证据。

`NATIVE_READY` 的最小包必须绑定同一 `run_id`，并包含：命名操作员与控制模式、双向窗口通道、真实原生窗口句柄、截图引用、输入探针、关闭/重开结果、停止权限与停止结果、资格时间戳、项目路径、owner/job/generation、PID 身份和资源 profile。证据来源仅允许 `human` 或 `native_observer`；`vite`、`browser`、`conversation`、`approval` 和 `unknown` 一律拒绝。

握手时间界限：原生窗口启动后 5 分钟内、首个 provider 请求前完成 `WINDOW_PROBE`；所有引用齐全后发送 `NATIVE_READY`；任一缺失证据、通道丢失、未知 PID、资源硬停止或重复运行信号在 10 秒内发送 `STOP_ACK`。缺失 `NATIVE_READY` 时 provider 请求必须为 `0`，不得自动重试或复用租约。

当前裁决保持：`NATIVE_CONTROL_PASS=NO/NOT_RUN`、`R2_ENTRY=BLOCKED_UNTIL_NATIVE_READY`、`REAL_EXPERIENCE_PASS=NO/NOT_RUN`。历史 R2 停止记录和已消费租约不被本模板覆盖。

## WO-005-ST-003 资源、隔离项目与一次性租约

本节是 R2 输入请求模板，不是授权。当前必须保持 `R2_INPUT_STATUS=PENDING_NATIVE_READY`、`ALLOW_R2=NO`、`LEASE_STATUS=NOT_REQUESTED`。

| 输入 | 必要事实 | 缺失时 |
| --- | --- | --- |
| 资源 | 新鲜 `STANDARD`/`CONSTRAINED` profile、来源、时间戳、MeasuredSafe/headroom | 不消费租约，停止在资源门 |
| Native | `NATIVE_READY` 六类证据、操作员和停止权 | 不创建项目，不启动 Tauri |
| 隔离项目 | 唯一新 TabZero 项目、独立路径、现有项目不变 | `R2_INPUT_INCOMPLETE` |
| 运行身份 | 原子 `run_id`、execution/job/generation | `R2_INPUT_INCOMPLETE` |
| 预算 | 1 auth/health + 6 主请求 + 1 个 T6 continuation | 不启动 provider |
| 租约 | 20 分钟 TTL，一次 Tauri 启动消费 | 保持 `NOT_REQUESTED` |

批准范围固定为 `R2_SAFE_HUMAN_30M_ADAPTIVE_V1 / WO-006-ST-002`，单实例最长 30 分钟。资源预检失败、Native 未 READY、操作员缺失或隔离项目缺失均不得生成 `ALLOW_R2`；Tauri 启动才消费租约，任何失败/停止/通道丢失都不可自动重试或复用。

## WO-005-ST-004 R2_ENTRY_MATRIX

| 入口结论 | 必要事实 | 当前动作 | 自动修复 |
| --- | --- | --- | --- |
| `AUTO_TECHNICAL_CONTINUE` | 当前 ST 为 R0/R1，未触及 R2 | 继续执行 STATE 指针 | 允许，仅限当前 ST |
| `TECHNICAL_PASS_NATIVE_PENDING` | 技术 R1 通过，Native 为 PENDING/UNKNOWN | 显示技术完成与 Native 等待 | 不允许，需外部证据 |
| `R2_INPUT_PENDING` | Native、资源、隔离项目、run_id、预算、lease 未全部齐全 | 保持 `ALLOW_R2=NO` | 资源只可按明确 R1 重检 |
| `R2_READY_WAITING_APPROVAL` | 所有输入齐全但当前批准缺失/过期/已消费 | 等待当前精确批准 | 不允许 |
| `REAL_R2_RUNNING` | fresh approval、NATIVE_READY、MeasuredSafe、唯一 run/lease、停止权 | 单实例有界运行 | 不自动重试 |

阻断责任映射：Native 缺失/丢失由外部操作员补证并发送 `STOP_ACK`；资源未知由资源门新鲜重检，HardStop 不自动恢复；输入缺失由准备责任人补齐；批准缺失或 lease 已消费由用户提供新批准/新 lease；未知 PID、重复运行、超预算和运行失败由操作员停止并保留现场。任何分支均不能用 `TECHNICAL_PASS` 生成 `ALLOW_R2`。
