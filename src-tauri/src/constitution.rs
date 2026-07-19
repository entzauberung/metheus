use crate::project;
use serde::{Deserialize, Serialize};

/// 校验 AI 更新的宪法内容是否合法
///
/// 检查三个维度：
/// 1. 第 1 部分是否被修改（防 AI 越界修改）
/// 2. 第 2 部分结构是否完整
/// 3. 返回内容是否为空或过短
pub(crate) fn validate_constitution_update(before: &str, after: &str) -> ValidationResult {
    // 第 1 层：空内容检查
    if after.trim().len() < 100 {
        return ValidationResult::Empty(format!("返回内容仅 {} 字符，过短", after.trim().len()));
    }

    // 提取"更新前"第 1 部分
    fn extract_part1(text: &str) -> Option<&str> {
        let start = text.find("## 第 1 部分")?;
        let after_start = &text[start..];
        let end = after_start.find("## 第 2 部分")?;
        Some(&after_start[..end])
    }

    let before_part1 = extract_part1(before);
    let after_part1 = extract_part1(after);

    // 第 2 层：第 1 部分比对
    match (before_part1, after_part1) {
        (Some(b), Some(a)) => {
            // 标准化：统一换行、去除首尾空白
            let norm_b: String = b
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let norm_a: String = a
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");

            if norm_b != norm_a {
                // 构造差异描述
                let diff_desc = if norm_b.len().abs_diff(norm_a.len()) > 100 {
                    format!(
                        "第 1 部分长度变化：{} → {} 字符",
                        norm_b.len(),
                        norm_a.len()
                    )
                } else {
                    // 找第一个不同的字符位置
                    let mut diff_pos: usize = 0;
                    for (cb, ca) in norm_b.chars().zip(norm_a.chars()) {
                        if cb != ca {
                            break;
                        }
                        diff_pos += 1;
                    }
                    let ctx_start = diff_pos.saturating_sub(30);
                    format!(
                        "第 1 部分在偏移 {} 处出现差异：...{}...",
                        diff_pos,
                        &norm_a[ctx_start..norm_a.len().min(ctx_start + 200)]
                    )
                };
                return ValidationResult::Part1Modified(diff_desc);
            }
        }
        (Some(_), None) => {
            return ValidationResult::Part1Modified("AI 返回中缺少第 1 部分".to_string());
        }
        (None, Some(_)) => {
            // 之前没有第 1 部分（首次）——放行，由调用方处理
        }
        (None, None) => {
            // 都没有第 1 部分——放行
        }
    }

    // 第 3 层：第 2 部分结构检查
    match after.find("## 第 2 部分") {
        Some(pos) => {
            let part2 = &after[pos..];
            // 检查是否至少有一个 ### 子标题
            if !part2.contains("###") {
                return ValidationResult::StructureDamaged(
                    "第 2 部分缺少子标题（###）".to_string(),
                );
            }
        }
        None => {
            return ValidationResult::StructureDamaged("缺少第 2 部分标记".to_string());
        }
    }

    ValidationResult::Passed
}

/// 兜底机械更新：不调用 AI，直接将 DiffSummary 的信息追加到宪法第 2 部分
///
/// 在 AI 连续失败时的降级方案。在「变更历史」段落标注 [机械更新]。
pub(crate) fn mechanical_update_constitution(
    current_constitution: &str,
    diff: &project::DiffSummary,
) -> Result<String, String> {
    let mut result = current_constitution.to_string();
    let timestamp = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();

    // 确保第 2 部分存在
    if !result.contains("## 第 2 部分") {
        result.push_str("\n\n## 第 2 部分：项目当前状态\n");
    }

    // 确保三个子段落存在
    let ensure_section = |text: &mut String, section_title: &str| {
        if !text.contains(section_title) {
            // 在 "## 第 2 部分" 之后插入
            if let Some(pos) = text.find("## 第 2 部分") {
                let insert_pos = text[pos..]
                    .find('\n')
                    .map(|n| pos + n + 1)
                    .unwrap_or(text.len());
                text.insert_str(insert_pos, &format!("\n{}\n", section_title));
            }
        }
    };
    ensure_section(&mut result, "### 项目结构");
    ensure_section(&mut result, "### 函数/接口定义");
    ensure_section(&mut result, "### 变更历史");

    // 处理新增文件
    for f in &diff.new_files {
        let entry = format!("\n- [新增] {}", f);
        if !result.contains(&entry) {
            if let Some(section_pos) = result.find("### 项目结构") {
                // 在项目结构段落末尾插入
                let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                match next_section {
                    Some(ins_pos) => result.insert_str(ins_pos, &entry),
                    None => result.push_str(&entry),
                }
            }
        }
    }

    // 处理删除文件
    for f in &diff.deleted_files {
        let search = format!("- [新增] {}", f);
        let replace_with = format!("- [已删除] {}", f);
        if result.contains(&search) {
            result = result.replace(&search, &replace_with);
        } else {
            let entry = format!("\n- [已删除] {}", f);
            if !result.contains(&entry) {
                if let Some(section_pos) = result.find("### 项目结构") {
                    let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                    match next_section {
                        Some(ins_pos) => result.insert_str(ins_pos, &entry),
                        None => result.push_str(&entry),
                    }
                }
            }
        }
    }

    // 处理修改文件
    for f in &diff.modified_files {
        let search_new = format!("- [新增] {}", f);
        let search_del = format!("- [已删除] {}", f);
        let _marker = format!("- {}（已修改）", f);
        if !result.contains("（已修改）") && !result.contains(f) {
            // 找到该文件的条目，追加修改标记
            let file_entry = format!("- {}", f);
            if let Some(entry_pos) = result.find(&file_entry) {
                let line_end = result[entry_pos..]
                    .find('\n')
                    .unwrap_or(result[entry_pos..].len());
                let existing = &result[entry_pos..entry_pos + line_end];
                if !existing.contains("（已修改）") {
                    let new_entry = format!("- {}（已修改）", f);
                    result.replace_range(entry_pos..entry_pos + line_end, &new_entry);
                }
            } else {
                let entry = format!("\n- {}（已修改）", f);
                result.push_str(&entry);
            }
        }
        // 移除可能触发的 unused warning
        let _ = search_new;
        let _ = search_del;
    }

    // 处理新增函数
    for func in &diff.new_functions {
        let entry = format!("\n- [新增] {}", func);
        if !result.contains(&entry) {
            if let Some(section_pos) = result.find("### 函数/接口定义") {
                let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                match next_section {
                    Some(ins_pos) => result.insert_str(ins_pos, &entry),
                    None => result.push_str(&entry),
                }
            }
        }
    }

    // 处理删除函数
    for func in &diff.deleted_functions {
        let search = format!("- [新增] {}", func);
        let replace_with = format!("- [已删除] {}", func);
        if result.contains(&search) {
            result = result.replace(&search, &replace_with);
        } else {
            let entry = format!("\n- [已删除] {}", func);
            if !result.contains(&entry) {
                if let Some(section_pos) = result.find("### 函数/接口定义") {
                    let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                    match next_section {
                        Some(ins_pos) => result.insert_str(ins_pos, &entry),
                        None => result.push_str(&entry),
                    }
                }
            }
        }
    }

    // 追加变更历史条目
    let history_entry = format!(
        "\n- [机械更新] 小阶段自动更新，AI 更新失败后降级处理 — {}",
        timestamp
    );
    if let Some(section_pos) = result.find("### 变更历史") {
        let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
        match next_section {
            Some(ins_pos) => result.insert_str(ins_pos, &history_entry),
            None => result.push_str(&history_entry),
        }
    } else {
        result.push_str(&history_entry);
    }

    Ok(result)
}

/// 宪法更新主函数
///
/// 接收当前宪法全文和变更摘要，调用 AI 更新第 2 部分。
/// 流程：检查 → AI 调用 → 校验 → 重试 → 兜底
#[tauri::command]
pub(crate) async fn update_constitution(
    constitution_content: String,
    diff_summary: project::DiffSummary,
) -> Result<String, String> {
    // 第一步：所有字段为空 → 跳过 AI 调用
    if diff_summary.new_files.is_empty()
        && diff_summary.modified_files.is_empty()
        && diff_summary.deleted_files.is_empty()
        && diff_summary.new_functions.is_empty()
        && diff_summary.modified_functions.is_empty()
        && diff_summary.deleted_functions.is_empty()
        && diff_summary.changed_dependencies.is_empty()
    {
        return Ok(constitution_content);
    }

    // 第二步：构造 user message
    let mut change_desc = String::new();
    if !diff_summary.new_files.is_empty() {
        change_desc.push_str("### 新增文件\n");
        for f in &diff_summary.new_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.modified_files.is_empty() {
        change_desc.push_str("### 修改文件\n");
        for f in &diff_summary.modified_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.deleted_files.is_empty() {
        change_desc.push_str("### 删除文件\n");
        for f in &diff_summary.deleted_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.new_functions.is_empty() {
        change_desc.push_str("### 新增函数\n");
        for f in &diff_summary.new_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.modified_functions.is_empty() {
        change_desc.push_str("### 修改函数\n");
        for f in &diff_summary.modified_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.deleted_functions.is_empty() {
        change_desc.push_str("### 删除函数\n");
        for f in &diff_summary.deleted_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.changed_dependencies.is_empty() {
        change_desc.push_str("### 依赖变更\n");
        for d in &diff_summary.changed_dependencies {
            change_desc.push_str(&format!("- {}\n", d));
        }
    }

    let user_message = format!(
        "【当前宪法】\n{}\n\n【本次变更】\n{}\n\n严格约束：你只能修改第 2 部分。第 1 部分一个字都不要动。",
        constitution_content, change_desc
    );

    // 第三步：调用 AI（Flash 模型，低 temperature，纯文本模式）
    let ai_result = match crate::api::call_deepseek_api_inner(
        crate::prompts::CONSTITUTION_UPDATE_PROMPT,
        &user_message,
        false,
        0.1,
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            // AI 调用失败 → 直接兜底
            eprintln!("[constitution] AI 调用失败，降级为机械更新：{}", e);
            return mechanical_update_constitution(&constitution_content, &diff_summary);
        }
    };

    // 第四步：校验
    let validation = validate_constitution_update(&constitution_content, &ai_result);
    match validation {
        ValidationResult::Passed => {
            eprintln!("[constitution] 宪法更新成功");
            return Ok(ai_result);
        }
        ref result @ _ => {
            let err_desc = match result {
                ValidationResult::Part1Modified(desc) => desc.clone(),
                ValidationResult::StructureDamaged(desc) => desc.clone(),
                ValidationResult::Empty(desc) => desc.clone(),
                ValidationResult::Passed => unreachable!(),
            };
            eprintln!("[constitution] 第一次校验不通过：{}，进入重试", err_desc);

            // 第五步：重试
            let retry_message = format!(
                "{}\n\n你上一次更新宪法时出现了以下错误：{}\n请修正后重新输出。务必严格遵守约束：只修改第 2 部分。",
                user_message, err_desc
            );

            match crate::api::call_deepseek_api_inner(
                crate::prompts::CONSTITUTION_UPDATE_PROMPT,
                &retry_message,
                false,
                0.1,
            )
            .await
            {
                Ok(retry_reply) => {
                    let validation2 =
                        validate_constitution_update(&constitution_content, &retry_reply);
                    match validation2 {
                        ValidationResult::Passed => {
                            eprintln!("[constitution] 宪法更新成功（重试后）");
                            return Ok(retry_reply);
                        }
                        ref result2 @ _ => {
                            let err_desc2 = match result2 {
                                ValidationResult::Part1Modified(desc) => desc.clone(),
                                ValidationResult::StructureDamaged(desc) => desc.clone(),
                                ValidationResult::Empty(desc) => desc.clone(),
                                ValidationResult::Passed => unreachable!(),
                            };
                            eprintln!(
                                "[constitution] 宪法更新降级（机械更新），原因：{}",
                                err_desc2
                            );
                            return mechanical_update_constitution(
                                &constitution_content,
                                &diff_summary,
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[constitution] AI 调用失败，降级为机械更新：{}", e);
                    return mechanical_update_constitution(&constitution_content, &diff_summary);
                }
            }
        }
    }
}

/// 估算文本的 token 数量
///
/// 中文字符按 1.0 token，ASCII 可打印字符按 0.25 token，其他按 0.5 token。
/// 纯计算函数，无 I/O。
pub(crate) fn estimate_tokens(text: &str) -> f64 {
    let mut tokens = 0.0;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c.is_ascii_punctuation() || c == ' ' || c == '\n' {
            tokens += 0.25;
        } else if matches!(c,
            '\u{4e00}'..='\u{9fff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{2a6df}'
            | '\u{2a700}'..='\u{2b73f}'
            | '\u{2b740}'..='\u{2b81f}'
            | '\u{2b820}'..='\u{2ceaf}'
            | '\u{3000}'..='\u{303f}'
        ) {
            tokens += 1.0;
        } else {
            tokens += 0.5;
        }
    }
    tokens
}

#[tauri::command]
pub(crate) async fn compact_constitution(constitution_content: String) -> Result<String, String> {
    // 第一步：提取第 2 部分
    let part2_start = match constitution_content.find("## 第 2 部分") {
        Some(pos) => pos,
        None => {
            eprintln!("[constitution] 宪法中缺少第 2 部分，跳过剪枝");
            return Ok(constitution_content);
        }
    };
    let part2 = &constitution_content[part2_start..];

    // 第二步：阈值检查（基于 token 估算）
    let estimated_tokens = estimate_tokens(part2);
    if estimated_tokens < crate::constants::COMPACTION_TRIGGER_TOKENS {
        eprintln!(
            "[constitution] 宪法第 2 部分未超过阈值（估算 {:.0} < {:.0} token），跳过剪枝",
            estimated_tokens,
            crate::constants::COMPACTION_TRIGGER_TOKENS
        );
        return Ok(constitution_content);
    }

    // 第三步：构造 AI 调用消息
    let user_message = format!(
        "【当前宪法】\n{}\n\n【压缩指令】\n\
        压缩第 2 部分，操作规则：\n\
        1. 保留最新的项目结构（文件树）\n\
        2. 保留所有仍然有效的函数/接口定义（删除已被后续覆盖的过时条目）\n\
        3. 如果旧函数名已被新函数替代，只保留最新的函数定义\n\
        4. 变更历史：保留最近 5 条完整记录，更早的合并为一行概述\n\
        5. 保持 Markdown 结构和标题层级不变\n\
        6. 压缩后第 2 部分的目标：约 1500 token\n\
        7. 直接输出完整的 CONSTITUTION.md 文件内容\n\
        \n严格约束：你只能修改第 2 部分。第 1 部分一个字都不要动。",
        constitution_content
    );

    // 第四步：调用 AI
    let ai_result = match crate::api::call_deepseek_api_inner(
        crate::prompts::COMPACT_CONSTITUTION_PROMPT,
        &user_message,
        false,
        0.1,
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("[constitution] 宪法剪枝 AI 调用失败：{}，保留膨胀版本", e);
            return Err(format!("AI 调用失败：{}", e));
        }
    };

    // 第五步：校验
    let validation = validate_constitution_update(&constitution_content, &ai_result);
    match validation {
        ValidationResult::Passed => {
            eprintln!("[constitution] 宪法剪枝成功");
            return Ok(ai_result);
        }
        ref result @ _ => {
            let err_desc = match result {
                ValidationResult::Part1Modified(desc) => desc.clone(),
                ValidationResult::StructureDamaged(desc) => desc.clone(),
                ValidationResult::Empty(desc) => desc.clone(),
                ValidationResult::Passed => unreachable!(),
            };
            eprintln!(
                "[constitution] 宪法剪枝第一次校验不通过：{}，进入重试",
                err_desc
            );

            // 第六步：重试（仅 1 次）
            let retry_message = format!(
                "{}\n\n你上一次剪枝宪法时出现了以下错误：{}\n请修正后重新输出。务必严格遵守约束：只修改第 2 部分。",
                user_message, err_desc
            );

            match crate::api::call_deepseek_api_inner(
                crate::prompts::COMPACT_CONSTITUTION_PROMPT,
                &retry_message,
                false,
                0.1,
            )
            .await
            {
                Ok(retry_reply) => {
                    let validation2 =
                        validate_constitution_update(&constitution_content, &retry_reply);
                    match validation2 {
                        ValidationResult::Passed => {
                            eprintln!("[constitution] 宪法剪枝成功（重试后）");
                            return Ok(retry_reply);
                        }
                        ref result2 @ _ => {
                            let err_desc2 = match result2 {
                                ValidationResult::Part1Modified(desc) => desc.clone(),
                                ValidationResult::StructureDamaged(desc) => desc.clone(),
                                ValidationResult::Empty(desc) => desc.clone(),
                                ValidationResult::Passed => unreachable!(),
                            };
                            eprintln!("[constitution] 宪法剪枝失败，保留膨胀版本：{}", err_desc2);
                            return Err(format!("剪枝校验两次不通过：{}", err_desc2));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[constitution] 宪法剪枝重试 AI 调用失败：{}，保留膨胀版本",
                        e
                    );
                    return Err(format!("重试 AI 调用失败：{}", e));
                }
            }
        }
    }
}

/// 读取项目目录下的 CONSTITUTION.md 文件，返回完整内容。
/// 文件不存在或为空时返回友好提示（Ok），而非报错。
#[tauri::command]
pub(crate) async fn read_constitution(project_path: String) -> Result<String, String> {
    use std::fs;
    use std::path::Path;

    let file_path = Path::new(&project_path).join("CONSTITUTION.md");

    match fs::read(&file_path) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok("项目宪法为空。".to_string());
            }
            match String::from_utf8(bytes) {
                Ok(content) => Ok(content),
                Err(_) => Err("项目宪法文件编码异常，无法读取。".to_string()),
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok("项目宪法尚未生成，请先完成一个阶段的任务。".to_string())
            } else {
                Err(format!("项目宪法文件读取失败：{}", e))
            }
        }
    }
}

/// 获取宪法摘要信息
///
/// 从 CONSTITUTION.md 第 2 部分中提取项目状态快照，包括：
/// - 项目结构简述
/// - 公开函数数量
/// - 最近变更列表（最多 5 条）
/// - 第 2 部分的 token 估算值
/// 宪法不存在或缺少第 2 部分时返回空字段结构体，不报错。
#[tauri::command]
pub(crate) async fn get_constitution_summary(
    project_path: String,
) -> Result<project::ConstitutionSummary, String> {
    use std::fs;
    use std::path::Path;

    let empty_summary = project::ConstitutionSummary {
        structure_description: String::new(),
        function_count: 0,
        recent_changes: vec![],
        total_tokens: 0.0,
    };

    // 读取 CONSTITUTION.md
    let file_path = Path::new(&project_path).join("CONSTITUTION.md");
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(empty_summary);
            }
            eprintln!("[get_constitution_summary] 读取宪法文件失败: {}", e);
            return Ok(empty_summary);
        }
    };

    if content.trim().is_empty() {
        return Ok(empty_summary);
    }

    // 定位第 2 部分
    let part2_start = match content.find("## 第 2 部分") {
        Some(pos) => pos,
        None => {
            eprintln!("[get_constitution_summary] 宪法中缺少第 2 部分");
            return Ok(empty_summary);
        }
    };
    let part2 = &content[part2_start..];

    // 辅助函数：提取子标题之间的文本内容
    fn extract_section(text: &str, heading: &str, next_headings: &[&str]) -> String {
        let start = match text.find(heading) {
            Some(pos) => pos + heading.len(),
            None => return String::new(),
        };
        let section = &text[start..];

        // 找到下一个最近的标题（### 或 ##）
        let mut end = section.len();
        for h in next_headings {
            if let Some(pos) = section.find(h) {
                if pos < end {
                    end = pos;
                }
            }
        }
        // 也查找任何 ### 标题
        if let Some(pos) = section.find("\n### ") {
            if pos < end {
                end = pos;
            }
        }
        section[..end].trim().to_string()
    }

    // 提取 structure_description：第 2 部分中的第一个 ### 子标题内容
    // 蓝图要求从 "### 项目结构" 提取
    let structure_description = extract_section(
        part2,
        "### 项目结构",
        &["### 函数/接口定义", "### 变更历史"],
    );

    // 解析 function_count：从 "### 函数/接口定义" 统计含 ( 的行
    let func_section = extract_section(
        part2,
        "### 函数/接口定义",
        &["### 变更历史", "### 项目结构"],
    );
    let function_count: u32 = func_section
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            (trimmed.starts_with("- [新增]")
                || trimmed.starts_with("- fn ")
                || trimmed.starts_with("- function ")
                || trimmed.starts_with("- pub "))
                && trimmed.contains('(')
        })
        .count() as u32;

    // 解析 recent_changes：从 "### 变更历史" 提取以 "- " 开头的行，最多 5 条
    let changes_section = extract_section(
        part2,
        "### 变更历史",
        &["### 项目结构", "### 函数/接口定义"],
    );
    let recent_changes: Vec<String> = changes_section
        .lines()
        .filter(|line| line.trim().starts_with("- "))
        .take(5)
        .map(|line| line.trim().trim_start_matches("- ").to_string())
        .collect();

    // 计算 total_tokens：对第 2 部分全文调用 estimate_tokens
    let total_tokens = estimate_tokens(part2);

    Ok(project::ConstitutionSummary {
        structure_description,
        function_count,
        recent_changes,
        total_tokens,
    })
}

/// 获取宪法第二部分变更历史与当前 token 预测
#[tauri::command]
pub(crate) async fn get_constitution_change_history(
    project_name: String,
    project_path: String,
) -> Result<project::ConstitutionChangeHistory, String> {
    let proj = crate::load_project(&project_name)?;
    let entries = proj.constitution_change_history.clone();

    // 读取当前宪法并估算 Part 2 token
    let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
    let current_token_estimate = if constitution_path.exists() {
        let content = std::fs::read_to_string(&constitution_path).unwrap_or_default();
        // 提取 Part 2
        let part2 = if let Some(pos) = content.find("## 第 2 部分") {
            content[pos..].to_string()
        } else if let Some(pos) = content.find("## Part 2") {
            content[pos..].to_string()
        } else {
            String::new()
        };
        estimate_tokens(&part2)
    } else {
        0.0
    };

    let compaction_threshold = crate::constants::COMPACTION_TRIGGER_TOKENS as f64;

    Ok(project::ConstitutionChangeHistory {
        entries,
        current_token_estimate,
        compaction_threshold,
        needs_compaction: current_token_estimate > compaction_threshold,
    })
}

/// 宪法更新校验结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationResult {
    /// 校验通过
    Passed,
    /// 第 1 部分被修改，携带差异描述
    Part1Modified(String),
    /// 第 2 部分结构损坏，携带错误描述
    StructureDamaged(String),
    /// 返回内容为空或过短，携带原因描述
    Empty(String),
}

/// 安全写入宪法第二部分（已有项目基线）。
///
/// - 如果宪法文件不存在，创建新的，包含第一和第二部分的占位结构
/// - 如果宪法存在且已有「## 第 1 部分」，第一部分逐字保留
/// - 第二部分替换或追加 Metheus 管理的基线内容
/// - 无法安全解析宪法结构时返回错误
pub(crate) fn write_constitution_part2(
    project_path: &str,
    baseline: &crate::project::ExistingProjectBaseline,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    let constitution_path = Path::new(project_path).join("CONSTITUTION.md");
    let part2_content = format!(
        "### 已有项目基线\n\
         **项目摘要**：{}\n\n\
         **技术栈**：{}\n\n\
         **已完成能力**：\n{}\n\n\
         **待处理能力**：\n{}\n\n\
         **风险**：\n{}\n\n\
         **不确定项**：\n{}\n\n\
         **证据来源**：扫描 {} 个文件，{}\n",
        baseline.project_summary,
        baseline.tech_stack,
        baseline
            .completed_capabilities
            .iter()
            .map(|c| format!("- {}", c))
            .collect::<Vec<_>>()
            .join("\n"),
        baseline
            .pending_capabilities
            .iter()
            .map(|c| format!("- {}", c))
            .collect::<Vec<_>>()
            .join("\n"),
        baseline
            .risks
            .iter()
            .map(|r| format!("- {}", r))
            .collect::<Vec<_>>()
            .join("\n"),
        baseline
            .uncertainties
            .iter()
            .map(|u| format!("- {}", u))
            .collect::<Vec<_>>()
            .join("\n"),
        baseline.scanned_files.len(),
        baseline.evidence_summary,
    );

    let new_content = if constitution_path.exists() {
        let existing = fs::read_to_string(&constitution_path)
            .map_err(|e| format!("读取已有 CONSTITUTION.md 失败：{}", e))?;

        if existing.contains("## 第 1 部分") {
            // 安全分区：找到「## 第 2 部分」位置
            if let Some(part2_pos) = existing.find("## 第 2 部分") {
                // 保留第一部分，替换第二部分
                let part1 = &existing[..part2_pos];
                format!(
                    "{}{}\n\n{}\n",
                    part1.trim_end(),
                    "\n\n## 第 2 部分：项目当前状态",
                    part2_content
                )
            } else {
                // 有第一部分但没有第二部分 — 追加
                format!(
                    "{}\n\n## 第 2 部分：项目当前状态\n\n{}\n",
                    existing.trim_end(),
                    part2_content
                )
            }
        } else if existing.contains("## 第 2 部分") {
            // 只有第二部分（异常），整体替换第二部分
            if let Some(part2_pos) = existing.find("## 第 2 部分") {
                let before = &existing[..part2_pos];
                format!(
                    "{}{}\n\n{}\n",
                    before.trim_end(),
                    "\n\n## 第 2 部分：项目当前状态",
                    part2_content
                )
            } else {
                format!(
                    "{}\n\n## 第 2 部分：项目当前状态\n\n{}\n",
                    existing.trim_end(),
                    part2_content
                )
            }
        } else {
            // 没有标准分区 — 在已有内容末尾安全追加
            format!(
                "{}\n\n---\n\n## 第 2 部分：项目当前状态\n\n{}\n",
                existing.trim_end(),
                part2_content
            )
        }
    } else {
        // 宪法文件不存在 — 创建新的，包含第一和第二部分的占位
        format!(
            "## 第 1 部分：项目长期规则\n（在方案批准时写入）\n\n---\n\n## 第 2 部分：项目当前状态\n\n{}\n",
            part2_content
        )
    };

    fs::write(&constitution_path, &new_content)
        .map_err(|e| format!("写入 CONSTITUTION.md 失败：{}", e))?;

    Ok(())
}

/// 读取 Already 项目宪法作为低权重背景参考
/// 返回格式化的参考文本，或空字符串（如果 Already 宪法不存在）
///
/// 用于 AI 生成提示词注入（方案/大阶段/中阶段/执行计划/检查/聊天），
/// 权重低于工作宪法和当前讨论。
pub(crate) fn read_already_constitution_reference(project_path: &str) -> String {
    use std::path::Path;
    let already_path = Path::new(project_path).join("ALREADY_CONSTITUTION.md");
    if !already_path.exists() {
        return String::new();
    }
    match std::fs::read_to_string(&already_path) {
        Ok(content) => {
            // 截断到合理长度（低权重参考，不宜过长）
            let truncated: String = content.chars().take(2000).collect();
            format!(
                "## 低权重背景参考（仅作了解，不得覆盖当前决策）\n\
                 > 以下内容来自对已有项目文件的自动分析，权重低于工作宪法和当前讨论。\n\
                 > 如果与当前讨论或工作宪法冲突，以工作宪法和当前讨论为准。\n\n{}\n\n\
                 ---\n（Already 宪法参考结束）\n",
                truncated
            )
        }
        Err(_) => String::new(),
    }
}
