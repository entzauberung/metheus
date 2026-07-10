use std::env;
use crate::project;

#[tauri::command]
pub(crate) async fn generate_milestones(
    version_plan: String,
    mode: String,
) -> Result<Vec<project::Milestone>, String> {
    //1. 读API  读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    //拼好提示词
    //2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        crate::prompts::PM_PROMPT, mode
    );
    //3. 构造 API 消息
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!("请根据以下版本方案拆解为3-5个大阶段：\n{}", version_plan)}
            ]
    });
    //叫AI干活
    //4. 发送请求
    //创建HTTP客户端
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });
    //发出请求等待回复
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    //从HTPP响应里提取JSON格式的正文
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    //从回复JSON抽出AI回答原文
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    //（上面的文字）JSON数组转化为Rust数组,json改动替换
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    //创建一个空的、可变的、专门用来存放 project::Milestone 结构体的数组，变量名叫 milestones
    let mut milestones: Vec<project::Milestone> = Vec::new();
    //把这个数组里面的每个对象，转化为Rust里Milestone结构体（补上id,状态...）
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            //vec!创建空的Vec数组= vec::new()
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            //创建空的可变的字符串
            git_commit_hash: "".to_string(),
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[generate_milestones] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response =
        match crate::api::call_deepseek_api_inner(crate::prompts::QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("[generate_milestones] 质检 API 调用失败：{}，跳过质检", e);
                return Ok(milestones);
            }
        };

    // 步骤 4：使用 parse_json_with_retry 解析 AI 返回的 QAResult JSON
    let qa_result = match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await {
        Ok(mut result) => {
            result.checked_at = chrono::Utc::now().to_rfc3339();
            result
        }
        Err(e) => {
            eprintln!(
                "[generate_milestones] 质检 JSON 解析失败：{}，默认判定为不通过",
                e
            );
            project::QAResult {
                passed: false,
                reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                    .to_string(),
                details: vec![],
                attention_points: vec![],
                checked_at: chrono::Utc::now().to_rfc3339(),
                warnings: vec![format!("质检 JSON 解析失败：{}", e)],
            }
        }
    };

    // 步骤 5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///根据质检驳回的反馈，重新让产品经理拆解大阶段
///与 generate_milestones 的区别：user_message 中包含驳回原因，引导 AI 修正
#[tauri::command]
pub(crate) async fn regenerate_milestones_with_feedback(
    version_plan: String,
    mode: String,
    feedback: String,
) -> Result<Vec<project::Milestone>, String> {
    // 1. 读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    // 2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        crate::prompts::PM_PROMPT, mode
    );
    // 3. 构造 API 消息（包含驳回反馈）
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!(
                    "上次拆解被需求质检驳回，原因：\n{}\n\n请根据此反馈，重新根据以下版本方案拆解为3-5个大阶段：\n{}",
                    feedback, version_plan
                )}
            ]
    });
    // 4. 发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    // 5. 解析 JSON 数组
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    // 6. 构造 Milestone 结构体
    let mut milestones: Vec<project::Milestone> = Vec::new();
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: "".to_string(),
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 7.1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[regenerate_milestones_with_feedback] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 7.2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 7.3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response =
        match crate::api::call_deepseek_api_inner(crate::prompts::QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_with_feedback] 质检 API 调用失败：{}，跳过质检",
                    e
                );
                return Ok(milestones);
            }
        };

    // 步骤 7.4：使用 parse_json_with_retry 解析 AI 返回的 QAResult JSON
    let qa_result = match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await {
        Ok(mut result) => {
            result.checked_at = chrono::Utc::now().to_rfc3339();
            result
        }
        Err(e) => {
            eprintln!("[regenerate_milestones_with_feedback] 质检 JSON 解析失败：{}，默认判定为不通过", e);
            project::QAResult {
                passed: false,
                reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                    .to_string(),
                details: vec![],
                attention_points: vec![],
                checked_at: chrono::Utc::now().to_rfc3339(),
                warnings: vec![format!("质检 JSON 解析失败：{}", e)],
            }
        }
    };

    // 步骤 7.5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///中阶段控制
#[tauri::command]
pub(crate) async fn generate_mid_stages(
    _milestone_id: String,
    milestone_title: String,
    milestone_description: String,
    version_plan: String,
    mode: String,
    attention_points: Vec<String>,
) -> Result<Vec<project::MidStage>, String> {
    // 1. 读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    // 2. 构造 system prompt
    let mut system_prompt = format!(
        "{}\n\n当前项目模式：{}。请根据版本方案，将大阶段拆解为 3-6 个中阶段。\
         每个中阶段是一个垂直切片。",
        crate::prompts::DOMAIN_LEAD_PROMPT, mode
    );
    // 注入 attention_points（若不为空）
    if !attention_points.is_empty() {
        system_prompt.push_str("\n【需求关注点】\n该大阶段在需求对齐检查中确认了以下要点，请在拆分中阶段时确保覆盖：\n");
        for point in &attention_points {
            system_prompt.push_str(&format!("- {}\n", point));
        }
    }
    // 3. 构造请求体
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": format!(
                "请根据版本方案，为大阶段「{} - {}」拆解中阶段：\n{}",
                milestone_title, milestone_description, version_plan
            )}
        ]
    });
    // 4. 发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    // 5. 解析 JSON,json 解析改动
    let raw_mid_stages: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析中阶段 JSON 失败：{}", e))?;
    // 6. 转换成 MidStage 结构体
    let mut mid_stages: Vec<project::MidStage> = Vec::new();
    for (i, raw) in raw_mid_stages.iter().enumerate() {
        mid_stages.push(project::MidStage {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_focus: raw["tech_focus"].as_str().unwrap_or("").to_string(),
            order: Some((i + 1) as i32),
            status: project::MidStageStatus::Pending,
            subtasks: vec![],
            test_report: "".to_string(),
            domain: None,
            test_log: None,
            created_at: chrono::Utc::now().to_rfc3339(), // 或 String::new()
            completed_at: None,
            approved_at: None,
            git_tag: String::new(),
        });
    }
    Ok(mid_stages)
}

// src-tauri/src/lib.rs




/// 校验项目路径的前端可调用命令
/// 生成下一个子任务的提示词
#[tauri::command]
pub(crate) async fn generate_next_prompt(
    mid_stage_title: String,
    mid_stage_description: String,
    previous_subtask_title: String,
    previous_subtask_result: String,
    file_changes: Vec<String>,
    test_result: String,
    is_retry: bool,
    retry_reason: String,
) -> Result<project::GeneratedSubtask, String> {
    // 构建开发工程师 prompt 的 user_message
    let user_message = format!(
        "## 当前中阶段\n标题：{}\n描述：{}\n\n## 上一个小阶段\n标题：{}\n执行结果：{}\n\n## 改动文件\n{}\n\n## 测试结果\n{}\n\n## 是否重试\n{}\n\n## 打回原因\n{}\n\n## 项目技术栈约束\n本项目是一个 Tauri 桌面应用，必须遵守以下技术栈约束：\n- 后端使用 Rust 语言，文件位于 src-tauri/src/ 目录\n- 前端使用 React + TypeScript，文件位于 src/ 目录\n- 不使用任何数据库（无 MySQL、MongoDB、PostgreSQL、SQLite 等）\n- 不使用任何 ORM（无 Mongoose、Prisma、TypeORM 等）\n- 不使用 Express、Koa、Next.js 等服务端框架\n- 如果提示词需要创建文件，文件路径必须在上述目录范围内\n- 提示词中不得出现与项目技术栈无关的技术名词",
        mid_stage_title,
        mid_stage_description,
        previous_subtask_title,
        previous_subtask_result,
        file_changes.join("\n"),
        test_result,
        if is_retry { "是" } else { "否" },
        retry_reason
    );
    // 调用 AI
    let reply = crate::api::call_deepseek_api_json(crate::prompts::TECH_PROMPT, &user_message).await?;
    // 解析 JSON 响应
    let generated: project::GeneratedSubtask = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析生成结果失败：{}", e))?;
    Ok(generated)
}

/// 在保留已完成大阶段的前提下，根据用户反馈重新生成后续大阶段
///
/// 与 generate_milestones（首次全量生成）的区别：
/// - 只生成 after_milestone_id 之后的大阶段，不修改已完成的
/// - 接受用户反馈作为修正方向
/// - 将已完成大阶段的摘要作为上下文传给 AI
///
/// 1. 加载项目，定位 after_milestone_id 作为分割点
/// 2. 构造包含已完成摘要和用户反馈的 AI 请求
/// 3. 调用 AI 生成后续大阶段
/// 4. QA 质检 → 不通过则返回错误，不修改 project.json
/// 5. 填充 UUID / 时间戳，拼接新旧 milestones，持久化
#[tauri::command]
pub(crate) async fn regenerate_milestones_from_point(
    project_name: String,
    after_milestone_id: String,
    version_plan: String,
    mode: String,
    feedback: String,
    completed_summary: String,
) -> Result<String, String> {
    // 1. 加载项目
    let mut project = crate::load_project(&project_name)?;

    // 2. 定位分割点：找到 after_milestone_id 的索引
    let split_idx = if after_milestone_id.is_empty() {
        // 没有已完成的大阶段 → 退化为全量生成
        None
    } else {
        let mut found: Option<usize> = None;
        for (i, m) in project.milestones.iter().enumerate() {
            if m.id == after_milestone_id {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(idx) => Some(idx),
            None => return Err(format!("未找到指定的大阶段: {}", after_milestone_id)),
        }
    };

    // 3. 收集已完成大阶段的上下文信息
    let completed_milestones: Vec<&project::Milestone> = match split_idx {
        Some(idx) => project.milestones[..=idx].iter().collect(),
        None => vec![],
    };

    let completed_titles: Vec<String> = completed_milestones
        .iter()
        .map(|m| format!("- {} ({})", m.title, m.version))
        .collect();
    let completed_titles_str = if completed_titles.is_empty() {
        "（暂无已完成的大阶段）".to_string()
    } else {
        completed_titles.join("\n")
    };

    let next_version_hint = if let Some(last) = completed_milestones.last() {
        format!(
            "\n\n已有大阶段的最后一个版本是 {}，新生成的大阶段版本号应从 {} 之后开始。",
            last.version, last.version
        )
    } else {
        String::new()
    };

    // 4. 构造 AI 请求
    let system_prompt = crate::prompts::REGENERATE_MILESTONES_PROMPT.to_string();

    let user_message = format!(
        "版本方案：\n{}\n\n项目模式：{}\n\n已完成大阶段摘要：\n{}\n\n已完成大阶段列表：\n{}{}\n\n用户反馈：\n{}\n\n请根据以上信息，生成后续的大阶段（milestones）JSON 数组。",
        version_plan,
        mode,
        completed_summary,
        completed_titles_str,
        next_version_hint,
        if feedback.is_empty() { "（用户未提供额外反馈）" } else { &feedback }
    );

    // 5. 读取 API 密钥
    let api_key = std::env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;

    // 6. 构造请求体（与 generate_milestones 相同的模式，不强制 json_object 以便返回数组）
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message}
        ]
    });

    // 7. 发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {}", e))?;

    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;

    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();

    // 8. 解析 AI 返回的 JSON 数组
    let raw_milestones: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry(&content)
            .await
            .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;

    // 9. 构造新的 Milestone 结构体
    let mut new_milestones: Vec<project::Milestone> = Vec::new();
    for raw in raw_milestones {
        new_milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
        });
    }

    // 10. QA 质检
    if !new_milestones.is_empty() {
        let milestones_json = match serde_json::to_string(&new_milestones) {
            Ok(json) => json,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_from_point] 大阶段 JSON 序列化失败：{}，跳过质检",
                    e
                );
                // 序列化失败不阻塞流程，跳过质检
                let merged = merge_milestones(completed_milestones, new_milestones);
                project.milestones = merged;
                crate::save_project(&project)?;
                let json_str = serde_json::to_string_pretty(&project)
                    .map_err(|e| format!("序列化项目文件失败: {}", e))?;
                return Ok(json_str);
            }
        };

        let qa_user_message = format!(
            "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
            version_plan, milestones_json
        );

        let qa_response = match crate::api::call_deepseek_api_inner(
            crate::prompts::QA_CHECK_PROMPT,
            &qa_user_message,
            false,
            0.1,
        )
        .await
        {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_from_point] 质检 API 调用失败：{}，跳过质检",
                    e
                );
                let merged = merge_milestones(completed_milestones, new_milestones);
                project.milestones = merged;
                crate::save_project(&project)?;
                let json_str = serde_json::to_string_pretty(&project)
                    .map_err(|e| format!("序列化项目文件失败: {}", e))?;
                return Ok(json_str);
            }
        };

        let qa_result = match crate::json_utils::parse_json_with_retry::<project::QAResult>(&qa_response).await {
            Ok(mut result) => {
                result.checked_at = chrono::Utc::now().to_rfc3339();
                result
            }
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_from_point] 质检 JSON 解析失败：{}，默认判定为不通过",
                    e
                );
                project::QAResult {
                    passed: false,
                    reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                        .to_string(),
                    details: vec![],
                    attention_points: vec![],
                    checked_at: chrono::Utc::now().to_rfc3339(),
                    warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                }
            }
        };

        // 质检不通过 → 返回错误，不修改 project.json
        if !qa_result.passed {
            return Err(format!(
                "质检不通过：{}\n\n详细偏差：\n{}",
                qa_result.reason,
                qa_result
                    .details
                    .iter()
                    .map(|d| format!(
                        "- [{}] {}（相关需求：{}）",
                        d.issue_type, d.description, d.related_requirement
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        // 质检通过 → 写入每个新 milestone
        for milestone in &mut new_milestones {
            milestone.qa_result = Some(qa_result.clone());
        }
    }

    // 11. 拼接：保留已完成 + 新生成的
    let merged = merge_milestones(completed_milestones, new_milestones);
    project.milestones = merged;

    // 12. 持久化
    crate::save_project(&project)?;

    // 13. 返回完整 Project JSON
    let json_str = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("序列化项目文件失败: {}", e))?;

    Ok(json_str)
}

/// 将已完成的大阶段和新生成的大阶段拼接为一个列表
fn merge_milestones(
    completed: Vec<&project::Milestone>,
    new: Vec<project::Milestone>,
) -> Vec<project::Milestone> {
    let mut result: Vec<project::Milestone> = Vec::new();
    for m in completed {
        result.push(m.clone());
    }
    for m in new {
        result.push(m);
    }
    result
}

/// 在回退后，根据分割点重新生成后续 subtask 的执行计划
///
/// 保留分割点之前的已完成 subtask（含分割点自身），
/// 调用 AI 批量生成分割点之后的后续 subtask，并持久化到 project.json。
///
/// 与 handleGeneratePlanForMidStage 中逐个 generate_next_prompt 的区别：
/// - 本命令一次性生成多个后续 subtask，而非逐个生成
/// - 提供已完成 subtask 的上下文和 git diff，确保逻辑连贯
///
/// 1. 加载项目，定位 milestone → mid_stage → split subtask
/// 2. 收集已完成 subtask 上下文 + git diff
/// 3. 调用 AI 批量生成后续 subtask（JSON 数组）
/// 4. 拼接新旧 subtask，持久化
/// 5. 返回更新后的 mid_stage JSON
#[tauri::command]
pub(crate) async fn regenerate_plan_from_checkpoint(
    project_name: String,
    project_path: String,
    milestone_id: String,
    mid_stage_id: String,
    subtask_id: String,
) -> Result<String, String> {
    // 1. 加载项目
    let mut project = crate::load_project(&project_name)?;

    // 2. 定位目标 mid_stage
    let milestone = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or(format!("未找到大阶段: {}", milestone_id))?;
    let mid_stage = milestone
        .mid_stages
        .iter()
        .find(|ms| ms.id == mid_stage_id)
        .ok_or(format!("未找到中阶段: {}", mid_stage_id))?;

    let mid_stage_title = mid_stage.title.clone();
    let mid_stage_description = mid_stage.description.clone();

    // 3. 定位分割点：找到 subtask_id 对应的索引
    let split_idx = mid_stage
        .subtasks
        .iter()
        .position(|st| st.id == subtask_id)
        .ok_or(format!("未找到小阶段: {}", subtask_id))?;

    let total_count = mid_stage.subtasks.len();
    let remaining_count = total_count.saturating_sub(split_idx + 1);

    // 如果没有后续 subtask 需要生成，直接返回当前 mid_stage JSON
    if remaining_count == 0 {
        let json_str = serde_json::to_string_pretty(&mid_stage)
            .map_err(|e| format!("序列化中阶段失败: {}", e))?;
        return Ok(json_str);
    }

    // 4. 收集已完成 subtask 的上下文
    let completed_subtasks: Vec<String> = mid_stage.subtasks[..=split_idx]
        .iter()
        .map(|st| {
            let result_summary = match (&st.execution_result, &st.test_result) {
                (Some(exec), Some(test)) => {
                    if test.passed {
                        format!("通过 — {}", exec.output.chars().take(100).collect::<String>())
                    } else {
                        format!("未通过 — {}", test.suggestion)
                    }
                }
                (Some(exec), None) => {
                    format!("已执行 — {}", exec.output.chars().take(100).collect::<String>())
                }
                _ => "待执行".to_string(),
            };
            format!("- {}（结果：{}）", st.title, result_summary)
        })
        .collect();

    let completed_context = if completed_subtasks.is_empty() {
        "（暂无已完成的小阶段）".to_string()
    } else {
        completed_subtasks.join("\n")
    };

    // 5. 获取 git diff
    let git_diff = match std::process::Command::new("git")
        .args(["diff"])
        .current_dir(&project_path)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                let diff_str = String::from_utf8_lossy(&output.stdout).to_string();
                if diff_str.trim().is_empty() {
                    "（工作区干净，无未提交变更）".to_string()
                } else {
                    diff_str
                }
            } else {
                "（无法获取 git diff）".to_string()
            }
        }
        Err(_) => "（无法获取 git diff）".to_string(),
    };

    // 6. 构造 AI 请求
    let user_message = format!(
        "中阶段标题：{}\n\
         中阶段描述：{}\n\n\
         已完成小阶段：\n{}\n\n\
         分割点：已完成 {} 个小阶段，需要从第 {} 个小阶段开始生成。\n\n\
         需要生成数量：{}\n\n\
         当前项目代码变更（git diff）：\n{}",
        mid_stage_title,
        mid_stage_description,
        completed_context,
        split_idx + 1,
        split_idx + 2,
        remaining_count,
        git_diff
    );

    // 7. 调用 AI
    let reply = crate::api::call_deepseek_api_json(
        crate::prompts::REGENERATE_SUBTASKS_PROMPT,
        &user_message,
    )
    .await
    .map_err(|e| format!("AI 调用失败: {}", e))?;

    // 8. 解析 AI 返回的 JSON 数组
    let raw_subtasks: Vec<serde_json::Value> =
        crate::json_utils::parse_json_with_retry(&reply)
            .await
            .map_err(|e| format!("解析小阶段 JSON 失败：{}", e))?;

    // 9. 构建新的 subtask 列表
    let mut new_subtasks: Vec<project::Subtask> = Vec::new();

    // 保留已完成的 subtask（克隆原始数据）
    for st in mid_stage.subtasks[..=split_idx].iter() {
        new_subtasks.push(st.clone());
    }

    // 追加 AI 生成的新 subtask
    for raw in raw_subtasks {
        new_subtasks.push(project::Subtask {
            id: uuid::Uuid::new_v4().to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            prompt: raw["prompt"].as_str().unwrap_or("").to_string(),
            status: project::SubtaskStatus::Pending,
            test_report: String::new(),
            execution_result: None,
            test_result: None,
            retry_count: 0,
            auto_tag: None,
        });
    }

    // 10. 更新 project 中的 mid_stage subtasks
    {
        let ms = project
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or("更新时找不到大阶段".to_string())?;
        let mid = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == mid_stage_id)
            .ok_or("更新时找不到中阶段".to_string())?;
        mid.subtasks = new_subtasks;
    }

    // 11. 持久化
    crate::save_project(&project)?;

    // 12. 序列化并返回更新后的 mid_stage
    let updated_mid_stage = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .and_then(|ms| ms.mid_stages.iter().find(|m| m.id == mid_stage_id))
        .ok_or("序列化时找不到中阶段".to_string())?;

    let json_str = serde_json::to_string_pretty(updated_mid_stage)
        .map_err(|e| format!("序列化中阶段失败: {}", e))?;

    Ok(json_str)
}

/// 大阶段完成后的 AI 自然语言总结
///
/// 基于大阶段的执行统计数据（中阶段完成情况、测试通过率、Git 标签等），
/// 调用 AI 生成一段自然语言总结和下一步建议。
/// 纯文本输出，与第一层前端统计表格配合使用。
///
/// 1. 加载项目，定位目标 milestone
/// 2. 收集中阶段/子任务统计数据
/// 3. 调用 AI 生成自然语言总结
/// 4. 返回纯文本总结
#[tauri::command]
pub(crate) async fn summarize_milestone(
    project_name: String,
    milestone_id: String,
) -> Result<String, String> {
    // 1. 加载项目
    let project = crate::load_project(&project_name)?;

    // 2. 定位目标 milestone
    let milestone = project
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or(format!("未找到指定的大阶段: {}", milestone_id))?;

    let title = &milestone.title;
    let version = &milestone.version;

    // 3. 收集中阶段统计数据
    let mid_stages = &milestone.mid_stages;
    let total_mid_stages = mid_stages.len();
    let completed_count = mid_stages
        .iter()
        .filter(|ms| ms.status == project::MidStageStatus::Completed)
        .count();
    let failed_count = mid_stages
        .iter()
        .filter(|ms| ms.status == project::MidStageStatus::Rejected)
        .count();

    // Git 标签列表
    let tags: Vec<&str> = mid_stages
        .iter()
        .filter_map(|ms| {
            if ms.git_tag.is_empty() {
                None
            } else {
                Some(ms.git_tag.as_str())
            }
        })
        .collect();
    let tags_line = if tags.is_empty() {
        "无".to_string()
    } else {
        tags.join("、")
    };

    // 4. 收集子任务测试通过率
    let mut total_subtasks: usize = 0;
    let mut passed_subtasks: usize = 0;
    for mid in mid_stages {
        for st in &mid.subtasks {
            total_subtasks += 1;
            if let Some(ref test_result) = st.test_result {
                if test_result.passed {
                    passed_subtasks += 1;
                }
            }
        }
    }
    let pass_rate = if total_subtasks > 0 {
        format!(
            "{}%（{}/{}）",
            ((passed_subtasks as f64 / total_subtasks as f64) * 100.0).round() as u32,
            passed_subtasks,
            total_subtasks
        )
    } else {
        "N/A".to_string()
    };

    // 5. 项目剩余大阶段数
    let milestone_idx = project
        .milestones
        .iter()
        .position(|m| m.id == milestone_id)
        .unwrap_or(0);
    let remaining = project.milestones.len().saturating_sub(milestone_idx + 1);

    // 6. 构造 user message
    let user_message = format!(
        "大阶段：{}（{}）\n\n\
         中阶段统计：\n\
         - 总数：{}\n\
         - 已完成：{}\n\
         - 失败：{}\n\
         - Git 标签：{}\n\n\
         子任务测试通过率：{}\n\n\
         项目剩余大阶段数：{} 个",
        title, version, total_mid_stages, completed_count, failed_count, tags_line, pass_rate, remaining
    );

    // 7. 调用 AI（纯文本模式，低 temperature = 0.3，语气中性）
    let summary = crate::api::call_deepseek_api_inner(
        crate::prompts::SUMMARIZE_MILESTONE_PROMPT,
        &user_message,
        false,
        0.3,
    )
    .await
    .map_err(|e| format!("AI 调用失败: {}", e))?;

    Ok(summary)
}
