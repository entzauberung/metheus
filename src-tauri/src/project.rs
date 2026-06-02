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
}
///小阶段（claude code可执行的最小单元）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub status: SubtaskStatus,
    pub test_report: String,
    // === 新增字段 ===
    #[serde(default)]
    pub execution_result: Option<ExecutionResult>,
    #[serde(default)]
    pub test_result: Option<TestResult>,
    #[serde(default)]
    pub retry_count: u32,
}
///中阶段（域负责人拆解的技术实现模块）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidStage {
    ///唯一标识
    pub id: String,
    ///版本号（eg: v0.1.1, v1.1.2)
    pub version: String,
    ///中阶段标题
    pub title: String,
    ///描述
    pub description: String,
    ///技术重点
    pub tech_focus: String,
    ///当前状态
    pub status: MidStageStatus,
    ///包含的小阶段列表
    pub subtasks: Vec<Subtask>,
    ///测试报告
    pub test_report: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub passed: bool,
    pub issues: Vec<String>,
    pub suggestion: String,
}
///开发工程师动态生成下一个小阶段
#[derive(Debug, Clone, Serialize, Deserialize)]
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
