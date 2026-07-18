// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
use serde::{Deserialize, Serialize};

/// 项目来源类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProjectEntryKind {
    /// 从零开始新项目
    NoProject,
    /// 改造已有项目
    HalfProject,
}

impl Default for ProjectEntryKind {
    fn default() -> Self {
        ProjectEntryKind::NoProject
    }
}

/// 工作流顶层阶段
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TopLevelPhase {
    /// Before：项目入口
    Before,
    /// FirstDiscussion：首次讨论和方案批准
    FirstDiscussion,
    /// Console：控制台规划和执行
    Console,
    /// Completed：项目完成
    Completed,
}

impl Default for TopLevelPhase {
    fn default() -> Self {
        TopLevelPhase::Before
    }
}

/// 工作流当前步骤（详细到按钮级别的步骤标识）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkflowStep {
    /// 等待选择入口类型
    WaitingEntry,
    /// 已有项目分析（Half Project）
    ExistingAnalysis,
    /// 基线审批（Half Project）
    BaselineApproval,
    /// 讨论中（First Discussion 或分支讨论）
    Discussion,
    /// 三项检查
    ThreeChecks,
    /// 方案审批（已批准但尚未进入 Console）
    PlanApproval,
    /// 大阶段生成阶段
    MilestoneGeneration,
    /// 大阶段草稿检查
    MilestoneCheck,
    /// 大阶段草稿审批
    MilestoneApproval,
    /// 大阶段选择（中阶段尚未生成或检查）
    MilestoneSelection,
    /// 中阶段生成阶段
    MidStageGeneration,
    /// 中阶段检查
    MidStageCheck,
    /// 中阶段审批
    MidStageApproval,
    /// 中阶段选择（执行计划尚未生成）
    MidStageSelection,
    /// 执行计划生成
    PlanGeneration,
    /// 执行计划检查
    PlanCheck,
    /// 执行计划审批
    PlanApproving,
    /// 执行中（Pending 任务执行中）
    Execution,
    /// 暂停决策（In Stop 或 ED Stop）
    PauseDecision,
    /// 回退预览
    RollbackPreview,
    /// 分支讨论（FixPast 或 AdjustFuture 讨论中）
    BranchDiscussion,
    /// 未来计划审批（C 分支草稿待批准）
    FuturePlanApproval,
    /// 大阶段审阅（A/B/C 分支选择）
    MilestoneReview,
    /// 项目完成
    Completed,
}

/// 暂停原因
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PauseReason {
    /// 无暂停
    None,
    /// 立即暂停（In Stop）
    InStop,
    /// 当前小阶段完成后暂停（ED Stop）
    EDStop,
}

impl Default for PauseReason {
    fn default() -> Self {
        PauseReason::None
    }
}

/// 讨论范围类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiscussionScope {
    /// 首次讨论（First Discussion）
    FirstDiscussion,
    /// 暂停调整讨论
    PauseAdjustment,
    /// B 分支 - 修正过去讨论
    FixPast,
    /// C 分支 - 调整未来讨论
    AdjustFuture,
}

impl Default for DiscussionScope {
    fn default() -> Self { DiscussionScope::FirstDiscussion }
}

/// 统一工作流状态 — 前端显示和按钮权限的唯一判断来源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    pub top_level_phase: TopLevelPhase,
    pub current_step: WorkflowStep,
    pub pause_reason: PauseReason,
    pub data_revision: u64,
    /// 当前讨论范围
    #[serde(default)]
    pub discussion_scope: DiscussionScope,
    /// 当前待审阅节点标识（大阶段或中阶段 ID）
    #[serde(default)]
    pub review_node_id: String,
    /// 最后合法转换时间
    #[serde(default)]
    pub last_transition_at: String,
    /// 大阶段自动驾驶是否激活（可见、可监督、可中断）
    #[serde(default)]
    pub autopilot_active: bool,
    /// autopilot 当前目标大阶段 ID（空字符串表示未设置）
    #[serde(default)]
    pub autopilot_target_milestone_id: String,
    /// autopilot 运行状态快照（持久化，用于刷新恢复）
    #[serde(default)]
    pub autopilot_state: Option<AutopilotState>,
}

impl Default for WorkflowState {
    fn default() -> Self {
        WorkflowState {
            top_level_phase: TopLevelPhase::Before,
            current_step: WorkflowStep::WaitingEntry,
            pause_reason: PauseReason::None,
            data_revision: 0,
            discussion_scope: DiscussionScope::FirstDiscussion,
            review_node_id: String::new(),
            last_transition_at: String::new(),
            autopilot_active: false,
            autopilot_target_milestone_id: String::new(),
            autopilot_state: None,
        }
    }
}

/// autopilot 运行状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AutopilotRunStatus {
    /// 运行中，正在自动推进
    Running,
    /// 已暂停（用户手动暂停，不在执行中）
    Paused,
    /// 等待大阶段审阅（到达 MilestoneReview 边界）
    WaitingMilestoneReview,
    /// 出错停止（需人工介入）
    ErrorStopped,
}

impl Default for AutopilotRunStatus {
    fn default() -> Self {
        AutopilotRunStatus::Running
    }
}

/// autopilot 持久化状态（写入 WorkflowState，用于刷新恢复）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotState {
    /// 是否激活
    pub active: bool,
    /// 目标大阶段 ID
    pub target_milestone_id: String,
    /// 运行状态
    pub run_status: AutopilotRunStatus,
    /// 最近一次自动动作说明（人类可读）
    pub last_action: String,
    /// 最近一次自动动作时间（ISO 8601）
    pub last_action_at: String,
    /// 出错时的错误信息
    pub error_message: String,
}

impl Default for AutopilotState {
    fn default() -> Self {
        AutopilotState {
            active: false,
            target_milestone_id: String::new(),
            run_status: AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
        }
    }
}

///项目整体状态（保留用于旧数据迁移，新界面不得直接依赖）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProjectStatus {
    ///空闲，未开始
    Idle,
    ///讨论中
    Discussing,
    ///方案已确认，产品经理拆解大阶段中
    Planning,
    ///大阶段拆分完成，等待执行
    MilestoneReady,
    ///执行中
    Executing,
    ///暂停中
    Paused,
    ///项目完成
    Completed,
}

impl Default for ProjectStatus {
    fn default() -> Self {
        ProjectStatus::Idle
    }
}
///小阶段状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubtaskStatus {
    ///待执行
    Pending,
    ///执行中
    Executing,
    ///执行完成，待人工确认
    AwaitingConfirmation,
    ///已通过
    Passed,
    ///已驳回
    Rejected,
    ///已回退
    RolledBack,
}
///大阶段状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MilestoneStatus {
    ///待开始
    Pending,
    ///进行中
    InProgress,
    ///已完成
    Completed,
    ///已暂停
    Paused,
}
///大阶段执行模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StageMode {
    ///快速模式：大阶段直接包含小阶段（两级）
    Quick,
    ///专业模式：大阶段包含中阶段（三级）
    Professional,
}
///项目整体执行模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProjectMode {
    ///快速模式
    Quick,
    ///专业模式
    Professional,
}

impl Default for ProjectMode {
    fn default() -> Self {
        ProjectMode::Professional
    }
}
///中阶段状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MidStageStatus {
    ///待拆解
    Pending,
    ///已拆解,待执行
    Ready,
    ///执行中
    InProgress,
    ///已完成
    Completed,
    ///已驳回
    Rejected,
    Approved,
    // 4.1.1a 该中阶段已被回退（代码和执行树都回到了更早的版本）
    RolledBack,
}
///小阶段（claude code可执行的最小单元）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub status: SubtaskStatus,
    #[serde(default)]
    pub test_report: String,
    // === 新增字段 ===
    #[serde(default)]
    pub execution_result: Option<ExecutionResult>,
    #[serde(default)]
    pub test_result: Option<TestResult>,
    #[serde(default)]
    pub retry_count: u32,
    /// 小阶段执行完成后的 Git tag 名，格式 metheus/auto/v0.1.1/task-0
    #[serde(default)]
    pub auto_tag: Option<String>,
    // === V1 结构化任务字段 ===
    /// 执行顺序号
    #[serde(default)]
    pub order: u32,
    /// 单一目标（一句话描述本任务要达成什么）
    #[serde(default)]
    pub goal: String,
    /// 允许修改的相对文件路径范围
    #[serde(default)]
    pub allowed_file_paths: Vec<String>,
    /// 允许新建文件的相对路径范围
    #[serde(default)]
    pub new_file_paths: Vec<String>,
    /// 必须读取的证据文件路径
    #[serde(default)]
    pub evidence_files: Vec<String>,
    /// 精确上下文摘要（注入给模型的背景信息）
    #[serde(default)]
    pub context_summary: String,
    /// 验收标准列表
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// 不可跨越的边界
    #[serde(default)]
    pub stop_rules: Vec<String>,
    /// 面向当前统一 DeepSeek 工作流模型的最终执行提示
    #[serde(default)]
    pub execution_prompt: String,
    // === V1 人工确认字段 ===
    /// 用户是否已确认结果
    #[serde(default)]
    pub confirmed_by_user: Option<bool>,
    /// 确认时间
    #[serde(default)]
    pub confirmed_at: Option<String>,
    /// 确认备注
    #[serde(default)]
    pub confirmation_notes: Option<String>,
}
///中阶段（域负责人拆解的技术实现模块）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidStage {
    pub id: String,
    pub title: String,
    pub version: String, // 如 "v0.1.1"
    pub order: Option<i32>,
    pub status: MidStageStatus,
    pub subtasks: Vec<Subtask>,
    pub domain: Option<String>, // 专业模式：域负责人
    pub test_log: Option<String>,
    pub created_at: String,
    pub description: String,
    pub tech_focus: String,
    #[serde(default)]
    pub test_report: String,
    pub completed_at: Option<String>,
    pub approved_at: Option<String>,
    #[serde(default)]
    pub git_tag: String, // Git tag 名，如 "metheus/v0.1.1"
    /// 执行计划检查结果
    #[serde(default)]
    pub plan_check_result: Option<StagePlanCheckResult>,
    /// 执行计划批准时间
    #[serde(default)]
    pub plan_approved_at: Option<String>,
    /// 计划修订号（检测计划是否被修改）
    #[serde(default)]
    pub plan_revision: u64,
    /// 当前执行计划草稿修订号（每次成功生成递增）
    #[serde(default)]
    pub plan_draft_revision: u64,
    /// 当前执行计划草稿生成时间
    #[serde(default)]
    pub plan_generated_at: Option<String>,
    /// 执行计划成功重新生成次数
    #[serde(default)]
    pub plan_regeneration_count: u32,
}
/// 子任务执行错误类型
/// 区分"用户主动暂停"和"真正的执行失败"
/// 序列化格式与前端 types.ts 的 SubTaskError 对齐
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SubTaskError {
    // 用户主动暂停 → 流水线优雅暂停，保留进度
    UserPaused,
    // 执行失败 → 按现有逻辑处理
    ExecutionFailed {
        // 错误信息
        message: String,
    },
    // 超时（预留）
    Timeout,
}
///执行结果（claude code执行后的输出)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: String,
    pub error_log: String,
    pub file_changes: Vec<String>,
}
///测试工程师的检查结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestResult {
    pub passed: bool,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub suggestion: String,
    /// 诊断/警告信息（非阻塞），用于向后端调用方和前端传递非致命的诊断信息
    #[serde(default)]
    pub warnings: Vec<String>,
}
///开发工程师动态生成下一个小阶段
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct GeneratedSubtask {
    pub title: String,
    pub prompt: String,
}
///大阶段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    ///唯一标识
    pub id: String,
    ///版本号（eg: v0.1, v0.2）
    pub version: String,
    ///大阶段标题
    pub title: String,
    ///描述
    pub description: String,
    ///技术栈
    pub tech_stack: String,
    ///当前状态
    pub status: MilestoneStatus,
    ///执行模式 （快速/专业）
    pub mode: StageMode,
    ///中阶段列表（专业模式使用）
    #[serde(default)]
    pub mid_stages: Vec<MidStage>,
    ///小阶段列表
    #[serde(default)]
    pub subtasks: Vec<Subtask>,
    ///需求质检结果（None=尚未质检）
    #[serde(default)]
    pub qa_result: Option<QAResult>,
    ///Git 提交哈希（完成后记录）
    #[serde(default)]
    pub git_commit_hash: String,
    ///拆解检查结果
    #[serde(default)]
    pub decomposition_check: Option<String>,
    ///大阶段审阅状态
    #[serde(default)]
    pub review_status: Option<String>, // "pending_review" | "approved" | "needs_fix" | "future_adjusted"
    ///审阅结论（A/B/C 分支选择结果）
    #[serde(default)]
    pub review_conclusion: Option<String>,
    ///批准时间
    #[serde(default)]
    pub approved_at: Option<String>,
    /// 目标（V1 结构化大阶段要求）
    #[serde(default)]
    pub goal: String,
    /// 范围边界
    #[serde(default)]
    pub scope: String,
    /// 依赖项
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 预期输出
    #[serde(default)]
    pub expected_output: String,
    /// 验收方向
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}
///单条聊天消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    ///唯一标识
    pub id: String,
    ///角色（"user"/"策略产品经理"/“产品经理”/“全栈技术顾问”/“测试工程师”）
    pub role: String,
    ///消息内容
    pub content: String,
    ///时间戳(毫秒)
    pub timestamp: u64,
    ///消息类型（如 "version_plan"），用于前端区分渲染方式
    #[serde(default)]
    pub msg_type: Option<String>,
    ///版本方案是否已批准
    #[serde(default)]
    pub approved: Option<bool>,
    ///版本方案是否已驳回
    #[serde(default)]
    pub rejected: Option<bool>,
    ///关联的大阶段 ID（仅 msg_type="milestone_summary" 时使用）
    #[serde(default)]
    pub milestone_id: Option<String>,
}
///讨论线程
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionThread {
    ///唯一标识
    pub id: String,
    ///线程标题
    pub title: String,
    ///的在挂载的节点ID（版本方案/大阶段ID）
    pub node_id: String,
    ///消息列表
    pub messages: Vec<Message>,
}
/// 已有项目基线
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExistingProjectBaseline {
    pub project_summary: String,
    pub tech_stack: String,
    pub architecture_evidence: String,
    pub completed_capabilities: Vec<String>,
    pub pending_capabilities: Vec<String>,
    pub risks: Vec<String>,
    pub uncertainties: Vec<String>,
    pub scanned_files: Vec<String>,
    pub scan_complete: bool,
    pub evidence_summary: String,
    pub generated_at: String,
    pub approved: bool,
    pub approved_at: Option<String>,
    /// Already 项目宪法文件路径（独立于工作 CONSOLUTION.md）
    #[serde(default)]
    pub already_constitution_path: String,
    /// Already 宪法摘要（注入工作宪法第一部分"已有信息"）
    #[serde(default)]
    pub already_constitution_summary: String,
}

/// 三项检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheckResult {
    pub check_type: String,         // "goal_completeness" | "reality_consistency" | "task_executability"
    pub passed: bool,
    pub summary: String,
    pub issues: Vec<String>,
    pub suggestions: Vec<String>,
    pub discussion_revision: u64,
    pub checked_at: String,
    /// 是否已过期（用户发送新需求后标记）
    #[serde(default)]
    pub stale: bool,
    /// 过期时间
    #[serde(default)]
    pub expired_at: Option<String>,
}

/// 方案草稿
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDraft {
    /// 唯一草稿标识
    #[serde(default = "default_draft_id")]
    pub draft_id: String,
    /// 草稿生命周期状态（默认 Pending）
    #[serde(default)]
    pub draft_status: DraftStatus,
    /// 方案正文
    #[serde(default)]
    pub plan_content: String,
    /// 宪法第一部分草稿
    #[serde(default)]
    pub constitution_part1_draft: String,
    /// 生成时的讨论修订号
    #[serde(default)]
    pub generation_revision: u64,
    /// 生成时的项目数据修订号
    #[serde(default)]
    pub data_revision_at_generation: u64,
    /// AI 自检结果或驳回反馈
    #[serde(default)]
    pub self_check_result: String,
    /// 生成时间
    #[serde(default)]
    pub generated_at: String,
    /// [deprecated] 旧兼容字段，新代码使用 draft_status
    #[serde(default)]
    pub approved: bool,
    /// 批准时间
    #[serde(default)]
    pub approved_at: Option<String>,
    /// 批准时的讨论修订号
    #[serde(default)]
    pub approved_at_discussion_revision: Option<u64>,
    /// 驳回反馈
    #[serde(default)]
    pub rejection_feedback: Option<String>,
    /// 驳回时间
    #[serde(default)]
    pub rejected_at: Option<String>,
    /// 过期时间
    #[serde(default)]
    pub expired_at: Option<String>,
    /// 被替代时间（用户主动重新讨论已批准方案时设置）
    #[serde(default)]
    pub superseded_at: Option<String>,
}

fn default_draft_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Default for PlanDraft {
    fn default() -> Self {
        PlanDraft {
            draft_id: default_draft_id(),
            draft_status: DraftStatus::Pending,
            plan_content: String::new(),
            constitution_part1_draft: String::new(),
            generation_revision: 0,
            data_revision_at_generation: 0,
            self_check_result: String::new(),
            generated_at: String::new(),
            approved: false,
            approved_at: None,
            approved_at_discussion_revision: None,
            rejection_feedback: None,
            rejected_at: None,
            expired_at: None,
            superseded_at: None,
        }
    }
}

/// 执行计划检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePlanCheckResult {
    pub passed: bool,
    pub omissions: Vec<String>,       // 遗漏项
    pub out_of_scope: Vec<String>,    // 越界项
    pub not_executable: Vec<String>,  // 不可执行项
    pub suggestions: Vec<String>,     // 建议
    pub checked_at: String,
}

/// 暂停上下文
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PauseContext {
    pub pause_type: String,           // "in_stop" | "ed_stop"
    pub current_subtask_id: String,
    pub last_passed_subtask_id: String,
    pub stable_tag: String,
    pub paused_at: String,
    pub discussion_start_revision: u64,
    pub pending_action: String,       // 待选择动作
}

/// 回退影响范围
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RollbackImpact {
    pub target_checkpoint: String,
    pub retained_nodes: Vec<String>,
    pub discarded_nodes: Vec<String>,
    pub deleted_tags: Vec<String>,
    pub regeneration_scope: String,
    pub includes_code_rollback: bool,
}

/// 大阶段审阅分支决策
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiscussionBranchType {
    /// A：正常继续
    Continue,
    /// B：修正过去
    FixPast,
    /// C：保留过去调整未来
    AdjustFuture,
}

/// 项目方案草稿生命周期状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DraftStatus {
    /// 待审批
    Pending,
    /// 已批准
    Approved,
    /// 已驳回
    Rejected,
    /// 已过期（用户发送新需求导致）
    Expired,
    /// 已被替代（用户主动重新讨论已批准方案）
    Superseded,
}

impl Default for DraftStatus {
    fn default() -> Self {
        DraftStatus::Pending
    }
}

/// 大阶段草稿生命周期状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MilestoneDraftStatus {
    /// 待检查
    Pending,
    /// 检查未通过
    CheckFailed,
    /// 已批准（候选大阶段已复制到正式 milestones）
    Approved,
}

impl Default for MilestoneDraftStatus {
    fn default() -> Self {
        MilestoneDraftStatus::Pending
    }
}

/// 大阶段草稿种类（区分普通草稿与未来规划草稿）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MilestoneDraftKind {
    /// 普通大阶段草稿（首次生成或重新生成）
    Normal,
    /// C 分支"只改未来"草稿
    FutureOnly,
}

impl Default for MilestoneDraftKind {
    fn default() -> Self {
        MilestoneDraftKind::Normal
    }
}

/// 大阶段草稿
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneDraft {
    /// 唯一草稿标识
    #[serde(default = "default_draft_id")]
    pub draft_id: String,
    /// 草稿生命周期状态
    #[serde(default)]
    pub status: MilestoneDraftStatus,
    /// 草稿种类（Normal 或 FutureOnly）
    #[serde(default)]
    pub draft_kind: MilestoneDraftKind,
    /// 候选大阶段列表
    #[serde(default)]
    pub candidate_milestones: Vec<Milestone>,
    /// 质量检查结果摘要
    #[serde(default)]
    pub check_result: Option<String>,
    /// 生成时的讨论修订号
    #[serde(default)]
    pub generation_revision: u64,
    /// 来源方案修订号（生成时的 data_revision）
    #[serde(default)]
    pub source_plan_revision: u64,
    /// 生成时间
    #[serde(default)]
    pub generated_at: String,
    /// 批准时间
    #[serde(default)]
    pub approved_at: Option<String>,
    /// 成功重新生成的次数
    #[serde(default)]
    pub regeneration_count: u32,
    /// 被当前草稿替换的上一个草稿标识
    #[serde(default)]
    pub previous_draft_id: Option<String>,
    /// 最近一次重新生成采用的反馈
    #[serde(default)]
    pub last_regeneration_reason: Option<String>,
    /// 最近一次成功重新生成时间
    #[serde(default)]
    pub last_regenerated_at: Option<String>,

    // === C 分支"只改未来"元数据（所有字段均带 serde(default)，普通草稿可不填） ===
    /// 分割点大阶段 ID（在此之后的大阶段会被替换）
    #[serde(default)]
    pub split_after_milestone_id: Option<String>,
    /// 保留的大阶段 ID 列表（已完成部分，不可更改）
    #[serde(default)]
    pub retained_milestone_ids: Vec<String>,
    /// 未来候选大阶段 ID 列表（新生成部分）
    #[serde(default)]
    pub future_candidate_ids: Vec<String>,
    /// AI 原始输出的版本号（批准前参考用）
    #[serde(default)]
    pub original_ai_versions: Vec<String>,
    /// 系统归一化后的正式版本号（批准后使用）
    #[serde(default)]
    pub normalized_versions: Vec<String>,
    /// 版本归一化是否成功完成
    #[serde(default)]
    pub versions_normalized: bool,
    /// 原始剩余大阶段数量（分割点之后、替换之前）
    #[serde(default)]
    pub original_remaining_count: Option<usize>,
    /// 新生成的未来大阶段数量
    #[serde(default)]
    pub new_future_count: Option<usize>,
    /// 数量是否显著膨胀（新数量 > 原剩余 * 1.5 且差值 > 1）
    #[serde(default)]
    pub count_expansion_warning: bool,
    /// 粒度校验是否全部通过
    #[serde(default)]
    pub granularity_check_passed: bool,
    /// 粒度问题列表
    #[serde(default)]
    pub granularity_issues: Vec<String>,
}

impl Default for MilestoneDraft {
    fn default() -> Self {
        MilestoneDraft {
            draft_id: default_draft_id(),
            status: MilestoneDraftStatus::Pending,
            draft_kind: MilestoneDraftKind::Normal,
            candidate_milestones: vec![],
            check_result: None,
            generation_revision: 0,
            source_plan_revision: 0,
            generated_at: String::new(),
            approved_at: None,
            regeneration_count: 0,
            previous_draft_id: None,
            last_regeneration_reason: None,
            last_regenerated_at: None,
            split_after_milestone_id: None,
            retained_milestone_ids: vec![],
            future_candidate_ids: vec![],
            original_ai_versions: vec![],
            normalized_versions: vec![],
            versions_normalized: false,
            original_remaining_count: None,
            new_future_count: None,
            count_expansion_warning: false,
            granularity_check_passed: false,
            granularity_issues: vec![],
        }
    }
}

/// 中阶段草稿生命周期状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MidStageDraftStatus {
    /// 待检查
    Pending,
    /// 检查未通过
    CheckFailed,
    /// 已批准
    Approved,
}

impl Default for MidStageDraftStatus {
    fn default() -> Self {
        MidStageDraftStatus::Pending
    }
}

/// 中阶段草稿
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidStageDraft {
    /// 唯一草稿标识
    #[serde(default = "default_draft_id")]
    pub draft_id: String,
    /// 所属大阶段 ID
    #[serde(default)]
    pub milestone_id: String,
    /// 草稿生命周期状态
    #[serde(default)]
    pub status: MidStageDraftStatus,
    /// 候选中阶段列表
    #[serde(default)]
    pub candidate_mid_stages: Vec<MidStage>,
    /// 质量检查结果摘要
    #[serde(default)]
    pub check_result: Option<String>,
    /// 生成时的讨论修订号
    #[serde(default)]
    pub generation_revision: u64,
    /// 生成时间
    #[serde(default)]
    pub generated_at: String,
    /// 批准时间
    #[serde(default)]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub regeneration_count: u32,
    #[serde(default)]
    pub previous_draft_id: Option<String>,
    #[serde(default)]
    pub last_regeneration_reason: Option<String>,
    #[serde(default)]
    pub source_data_revision: u64,
}

impl Default for MidStageDraft {
    fn default() -> Self {
        MidStageDraft {
            draft_id: default_draft_id(),
            milestone_id: String::new(),
            status: MidStageDraftStatus::Pending,
            candidate_mid_stages: vec![],
            check_result: None,
            generation_revision: 0,
            generated_at: String::new(),
            approved_at: None,
            regeneration_count: 0,
            previous_draft_id: None,
            last_regeneration_reason: None,
            source_data_revision: 0,
        }
    }
}

/// 分支决策详情
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchDecision {
    pub branch_type: Option<DiscussionBranchType>,
    pub discussion_start_revision: u64,
    pub user_feedback: String,
    pub suggested_checkpoint: String,
    pub impact_scope: String,
    pub confirmed: bool,
}

///项目根结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    ///项目名称
    pub name: String,
    ///项目状态（保留用于旧数据兼容，新界面使用 workflow_state）
    #[serde(default)]
    pub status: ProjectStatus,
    ///项目来源
    #[serde(default)]
    pub entry_kind: ProjectEntryKind,
    ///统一工作流状态 — 前端显示和按钮权限的唯一判断来源
    #[serde(default)]
    pub workflow_state: WorkflowState,
    ///项目模式
    #[serde(default)]
    pub mode: ProjectMode,
    ///当前大阶段 ID
    #[serde(default)]
    pub current_milestone_id: String,
    ///当前中阶段ID
    #[serde(default)]
    pub current_mid_stage_id: String,
    ///版本方案 (Markdown)
    #[serde(default)]
    pub version_plan: String,
    ///已有项目基线（Half Project）
    #[serde(default)]
    pub existing_baseline: Option<ExistingProjectBaseline>,
    ///三项检查结果
    #[serde(default)]
    pub preflight_results: Vec<PreflightCheckResult>,
    ///讨论修订号（每次用户新增消息递增）
    #[serde(default)]
    pub discussion_revision: u64,
    ///当前方案草稿（批准前暂存）
    #[serde(default)]
    pub plan_draft: Option<PlanDraft>,
    ///草稿历史列表（被驳回/过期的草稿移入此处）
    #[serde(default)]
    pub draft_history: Vec<PlanDraft>,
    ///大阶段列表
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    ///讨论线程列表
    #[serde(default)]
    pub discussion_threads: Vec<DiscussionThread>,
    ///宪法变更历史（按确认时间排列，最新在末尾）
    #[serde(default)]
    pub constitution_change_history: Vec<ConstitutionChangeEntry>,
    ///代码变更历史（按确认时间排列，最新在末尾）
    #[serde(default)]
    pub change_history: Vec<ChangeHistoryEntry>,
    ///暂停上下文
    #[serde(default)]
    pub pause_context: Option<PauseContext>,
    ///大阶段草稿（批准前暂存）
    #[serde(default)]
    pub milestone_draft: Option<MilestoneDraft>,
    ///中阶段草稿（批准前暂存）
    #[serde(default)]
    pub mid_stage_draft: Option<MidStageDraft>,
    ///分支决策
    #[serde(default)]
    pub branch_decision: Option<BranchDecision>,
    ///项目路径
    #[serde(default)]
    pub project_path: String,
    /// 当前执行会话（用于刷新恢复与状态同步）
    #[serde(default)]
    pub execution_session: Option<ExecutionSession>,
    /// 执行操作历史（持久化，刷新不丢）
    #[serde(default)]
    pub execution_history: Vec<ExecutionHistoryEntry>,
}
impl Project {
    /// 创建一个新的空项目。
    pub fn new(name: &str) -> Self {
        let initial_thread = DiscussionThread {
            id: "thread-init".to_string(),
            title: "初始讨论".to_string(),
            node_id: "root".to_string(),
            messages: vec![],
        };

        Project {
            name: name.to_string(),
            status: ProjectStatus::Idle,
            entry_kind: ProjectEntryKind::NoProject,
            workflow_state: WorkflowState::default(),
            mode: ProjectMode::Professional,
            current_milestone_id: "".to_string(),
            current_mid_stage_id: "".to_string(),
            version_plan: "".to_string(),
            existing_baseline: None,
            preflight_results: vec![],
            discussion_revision: 0,
            plan_draft: None,
            draft_history: vec![],
            milestones: vec![],
            discussion_threads: vec![initial_thread],
            milestone_draft: None,
            mid_stage_draft: None,
            change_history: vec![],
            constitution_change_history: vec![],
            pause_context: None,
            branch_decision: None,
            project_path: "".to_string(),
            execution_session: None,
            execution_history: vec![],
        }
    }

    /// 创建 Half Project 项目（含路径和来源标识）
    pub fn new_half(name: &str, path: &str) -> Self {
        let mut p = Project::new(name);
        p.entry_kind = ProjectEntryKind::HalfProject;
        p.project_path = path.to_string();
        p.workflow_state.current_step = crate::project::WorkflowStep::ExistingAnalysis;
        p
    }
}

/// 需求质检：单条偏差记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QADetail {
    /// 偏差类型：遗漏/多余/偏离
    pub issue_type: String,
    /// 具体偏差描述
    pub description: String,
    /// 关联的原始需求描述（引用版本方案原文）
    pub related_requirement: String,
}

/// 需求质检：完整的检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QAResult {
    /// 是否通过质检
    pub passed: bool,
    /// 总结原因（通过时="全部对齐"，驳回时写概述）
    pub reason: String,
    /// 偏差明细列表
    pub details: Vec<QADetail>,
    /// 后续实现需关注的要点
    pub attention_points: Vec<String>,
    /// 质检时间（ISO 8601 格式）
    pub checked_at: String,
    /// 诊断/警告信息（非阻塞），用于向后端调用方和前端传递非致命的诊断信息
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// git diff 解析结果摘要
/// 由 extract_diff_summary 函数解析 git diff 输出后填充
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    /// 新增文件列表（相对路径）
    #[serde(default)]
    pub new_files: Vec<String>,
    /// 修改文件列表（相对路径）
    #[serde(default)]
    pub modified_files: Vec<String>,
    /// 删除文件列表（相对路径）
    #[serde(default)]
    pub deleted_files: Vec<String>,
    /// 新增函数/方法签名列表
    #[serde(default)]
    pub new_functions: Vec<String>,
    /// 修改函数/方法签名列表
    #[serde(default)]
    pub modified_functions: Vec<String>,
    /// 删除函数/方法签名列表
    #[serde(default)]
    pub deleted_functions: Vec<String>,
    /// 依赖变更条目列表（从 package.json / Cargo.toml 等文件中提取）
    #[serde(default)]
    pub changed_dependencies: Vec<String>,
}

/// 宪法摘要信息
/// 从 CONSTITUTION.md 第 2 部分提取的项目状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionSummary {
    /// 项目结构简述（文件树部分文本）
    #[serde(default)]
    pub structure_description: String,
    /// 公开函数数量
    #[serde(default)]
    pub function_count: u32,
    /// 变更历史中最近 5 条
    #[serde(default)]
    pub recent_changes: Vec<String>,
    /// 当前宪法第 2 部分的 token 估算值
    #[serde(default)]
    pub total_tokens: f64,
}

/// Git tag 信息
/// 从 git tag 列表中解析的单条 tag 记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GitTagInfo {
    /// tag 名称（如 "metheus/v0.1.1"）
    pub name: String,
    /// 创建日期（如 "2026-01-15"）
    pub date: String,
    /// commit 主题行
    pub subject: String,
}

// === Git 标签树结构（大阶段 → 中阶段 → 小阶段） ===

/// Git 标签树根节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitTagTree {
    pub milestones: Vec<MilestoneTagNode>,
}

/// 大阶段标签节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneTagNode {
    pub milestone_id: String,
    pub milestone_title: String,
    pub milestone_version: String,
    pub milestone_status: String,
    pub mid_stages: Vec<MidStageTagNode>,
}

/// 中阶段标签节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidStageTagNode {
    pub mid_stage_id: String,
    pub mid_stage_title: String,
    pub mid_stage_version: String,
    pub mid_stage_tag: String,
    pub mid_stage_status: String,
    pub subtasks: Vec<SubtaskTagNode>,
}

/// 小阶段标签节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskTagNode {
    pub subtask_id: String,
    pub subtask_title: String,
    pub subtask_index: u32,
    pub subtask_tag: String,
    pub subtask_status: String,
}

/// 项目文件/目录条目
/// 从 walkdir 遍历得到的单条文件或目录记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// 相对路径（相对于 project_path）
    pub path: String,
    /// 是否为目录
    pub is_dir: bool,
    /// 文件扩展名（如 "rs"、"tsx"，目录为空字符串）
    #[serde(default)]
    pub file_type: String,
}

/// 路径校验结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathValidationResult {
    pub is_valid: bool,
    pub exists: bool,
    pub is_directory: bool,
    pub is_git_repo: bool,
    pub error_message: String,
}

/// 执行工作区状态 — 进入 Execution 步骤后的 Git 就绪探测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionWorkspaceStatus {
    /// 项目路径是否存在
    pub path_exists: bool,
    /// 项目路径是否为目录
    pub is_directory: bool,
    /// 是否为 Git 仓库（.git 存在）
    pub is_git_repo: bool,
    /// 是否至少有一次提交
    pub has_commits: bool,
    /// Git user.name 是否可用
    pub git_user_available: bool,
    /// Git user.email 是否可用
    pub git_email_available: bool,
    /// 是否满足执行前置条件
    pub ready: bool,
    /// 给前端显示的状态说明
    pub status_message: String,
}

/// 宪法变更历史条目 — 小阶段确认后宪法第二部分更新记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionChangeEntry {
    /// 记录时间（ISO 8601）
    pub timestamp: String,
    /// 关联小阶段 ID
    pub subtask_id: String,
    /// 关联小阶段标题
    pub subtask_title: String,
    /// 本轮第二部分变更摘要
    pub change_summary: String,
    /// 更新后第二部分的 token 估算值
    pub token_estimate: f64,
}

/// 宪法变更历史响应（含当前 token 预测）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionChangeHistory {
    /// 变更历史条目列表
    pub entries: Vec<ConstitutionChangeEntry>,
    /// 当前宪法第二部分 token 估算值
    pub current_token_estimate: f64,
    /// 剪枝触发阈值
    pub compaction_threshold: f64,
    /// 是否建议剪枝
    pub needs_compaction: bool,
}

/// 执行会话 — 记录当前正在执行或待确认的小阶段，用于刷新恢复
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSession {
    /// 是否活跃（有执行中的会话）
    pub active: bool,
    /// 当前大阶段 ID
    pub milestone_id: String,
    /// 当前中阶段 ID
    pub mid_stage_id: String,
    /// 当前小阶段 ID
    pub subtask_id: String,
    /// 小阶段标题
    pub subtask_title: String,
    /// 会话状态："executing" | "awaiting_confirmation"
    pub status: String,
    /// 会话开始时间（ISO 8601）
    pub started_at: String,
    /// 进入当前状态的时间
    pub state_entered_at: String,
    /// 计划修订号
    pub plan_revision: u64,
    /// 小阶段索引
    pub subtask_index: usize,
    /// 总小阶段数
    pub total_subtasks: usize,
}

impl Default for ExecutionSession {
    fn default() -> Self {
        ExecutionSession {
            active: false,
            milestone_id: String::new(),
            mid_stage_id: String::new(),
            subtask_id: String::new(),
            subtask_title: String::new(),
            status: String::new(),
            started_at: String::new(),
            state_entered_at: String::new(),
            plan_revision: 0,
            subtask_index: 0,
            total_subtasks: 0,
        }
    }
}

/// 执行事件类型 — 用于持久化执行操作历史
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionEventType {
    /// 用户点击准备执行环境
    WorkspacePrepare,
    /// Git 工作区准备成功
    WorkspaceReady,
    /// Git 工作区准备失败
    WorkspacePrepareFailed,
    /// 用户点击执行当前小阶段
    UserExecute,
    /// 小阶段进入执行中
    SubtaskExecuting,
    /// 执行器完成
    ExecutorComplete,
    /// 测试完成
    TestComplete,
    /// 进入待确认
    AwaitingConfirmation,
    /// 用户确认通过
    UserConfirm,
    /// 用户驳回
    UserReject,
    /// 用户点击立即暂停 (In Stop)
    UserInStop,
    /// 用户点击完成后暂停 (ED Stop)
    UserEdStop,
    /// 用户选择继续
    UserContinue,
    /// 用户选择调整后续
    UserAdjust,
    /// 用户选择回退更早稳定点
    UserRollback,
    /// 中阶段完成
    MidStageComplete,
    /// 推进到下一中阶段
    AdvanceNextMidStage,
    /// 推进到大阶段审阅
    AdvanceMilestoneReview,
    /// 系统自动推进
    SystemAdvance,
}

impl Default for ExecutionEventType {
    fn default() -> Self {
        ExecutionEventType::SystemAdvance
    }
}

/// 执行历史条目 — 持久化到 Project 中，刷新不丢
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionHistoryEntry {
    /// 事件时间（ISO 8601）
    pub timestamp: String,
    /// 事件级别：info / success / error / pause
    pub level: String,
    /// 事件类型
    pub event_type: ExecutionEventType,
    /// 事件描述文本
    pub text: String,
    /// 关联大阶段 ID（可选）
    #[serde(default)]
    pub milestone_id: Option<String>,
    /// 关联中阶段 ID（可选）
    #[serde(default)]
    pub mid_stage_id: Option<String>,
    /// 关联小阶段 ID（可选）
    #[serde(default)]
    pub subtask_id: Option<String>,
}

/// 执行历史上限
pub const MAX_EXECUTION_HISTORY: usize = 500;

/// 代码变更历史条目 — 小阶段确认时记录的 diff 快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeHistoryEntry {
    /// 关联小阶段 ID
    pub subtask_id: String,
    /// 关联小阶段标题
    pub subtask_title: String,
    /// 记录时间（ISO 8601）
    pub recorded_at: String,
    /// 变更涉及的文件列表
    pub files_changed: Vec<String>,
    /// diff 文本（可能被截断）
    pub diff_text: String,
    /// diff 是否已被截断
    pub diff_truncated: bool,
}
