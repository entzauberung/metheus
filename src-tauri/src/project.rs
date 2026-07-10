// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
use serde::{Deserialize, Serialize};
///项目整体状态
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
///小阶段状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubtaskStatus {
    ///待执行
    Pending,
    ///执行中
    Executing,
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
    pub git_tag: String, // ← 新增：Git tag 名，如 "metheus/v0.1.1"
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
    pub mid_stages: Vec<MidStage>,
    ///小阶段列表
    pub subtasks: Vec<Subtask>,
    ///需求质检结果（None=尚未质检）
    #[serde(default)]
    pub qa_result: Option<QAResult>,
    ///Git 提交哈希（完成后记录）
    pub git_commit_hash: String,
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
///项目根结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    ///项目名称
    pub name: String,
    ///项目状态
    pub status: ProjectStatus,
    ///项目模式
    pub mode: ProjectMode,
    ///当前大阶段 ID
    pub current_milestone_id: String,
    ///当前中阶段ID
    pub current_mid_stage_id: String,
    ///版本方案 (Markdown)
    pub version_plan: String,
    ///大阶段列表
    pub milestones: Vec<Milestone>,
    ///讨论线程列表
    pub discussion_threads: Vec<DiscussionThread>,
    //第三周新增
    #[serde(default)]
    pub project_path: String,
}
impl Project {
    /// 创建一个新的空项目，不含任何预定义的大阶段。
    /// 大阶段将由策略产品经理与用户在聊天讨论中动态生成。
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
            mode: ProjectMode::Professional,
            current_milestone_id: "".to_string(),
            current_mid_stage_id: "".to_string(),
            version_plan: "".to_string(),
            milestones: vec![], // 空列表，等待策略产品经理在讨论中定义
            discussion_threads: vec![initial_thread],
            project_path: "".to_string(), // 暂时给空，等实际使用时再设置
        }
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
pub struct GitTagInfo {
    /// tag 名称（如 "metheus/v0.1.1"）
    pub name: String,
    /// 创建日期（如 "2026-01-15"）
    pub date: String,
    /// commit 主题行
    pub subject: String,
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
