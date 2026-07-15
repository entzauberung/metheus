// src-tauri/src/commands/project_analysis.rs — Half Project 已有项目分析
use crate::project;

const MAX_DIR_DEPTH: usize = 8;
const MAX_SINGLE_FILE_LINES: usize = 200;
const MAX_TOTAL_CONTEXT_CHARS: usize = 15000;
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "__pycache__", "dist",
    ".next", "build", "coverage", ".venv", "env", ".env",
];
const SENSITIVE_FILES: &[&str] = &[
    ".env", ".env.local", ".env.production", "id_rsa", "id_ed25519",
    "*.key", "*.pem", "*.cert", "*.p12", "keystore",
];

/// Internal scan (sync, reusable)
fn scan_internal(project_path: &str) -> Result<project::ExistingProjectBaseline, String> {
    let path = std::path::Path::new(&project_path);
    if !path.exists() || !path.is_dir() {
        return Err("项目路径不存在或不是目录".to_string());
    }

    let mut scanned_files: Vec<String> = Vec::new();
    let mut readme_content = String::new();
    let mut manifest_files: Vec<(String, String)> = Vec::new();
    let mut source_files: Vec<(String, String)> = Vec::new();
    let mut total_chars = 0;

    for entry in walkdir::WalkDir::new(&project_path)
        .max_depth(MAX_DIR_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path() == path {
            continue;
        }

        let rel_path = entry.path()
            .strip_prefix(&project_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();

        // Skip excluded directories
        let is_skipped = rel_path.split('/')
            .any(|component| SKIP_DIRS.contains(&component));
        if is_skipped {
            continue;
        }

        if entry.file_type().is_dir() {
            continue;
        }

        scanned_files.push(rel_path.clone());

        // Check for sensitive files
        let file_name = entry.file_name().to_string_lossy().to_string();
        if SENSITIVE_FILES.iter().any(|sf| file_name.contains(sf.trim_start_matches('*'))) {
            continue;
        }

        // Determine file type
        let ext = entry.path().extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let is_text = matches!(
            ext.as_str(),
            "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java"
            | "c" | "cpp" | "h" | "hpp" | "cs" | "swift" | "kt"
            | "md" | "txt" | "yml" | "yaml" | "json" | "toml"
            | "css" | "html" | "vue" | "svelte" | "sql" | "rb" | "php"
            | "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat"
            | "dockerfile" | "cmake" | "gradle" | "xml"
        );

        // Read relevant files for content analysis
        if is_text && total_chars < MAX_TOTAL_CONTEXT_CHARS {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                let lines: Vec<&str> = content.lines().collect();
                let excerpt = if lines.len() > MAX_SINGLE_FILE_LINES {
                    let head: Vec<String> = lines.iter().take(MAX_SINGLE_FILE_LINES / 2).map(|s| s.to_string()).collect();
                    let mut tail: Vec<String> = lines.iter().rev().take(MAX_SINGLE_FILE_LINES / 2).map(|s| s.to_string()).collect();
                    tail.reverse();
                    format!("{}\n... ({} lines truncated) ...\n{}",
                        head.join("\n"), lines.len() - MAX_SINGLE_FILE_LINES, tail.join("\n"))
                } else {
                    content.clone()
                };

                let excerpt_chars = excerpt.chars().count().min(MAX_TOTAL_CONTEXT_CHARS - total_chars);

                // Classify files
                let lower_rel = rel_path.to_lowercase();
                if lower_rel.contains("readme") {
                    readme_content = excerpt.chars().take(excerpt_chars).collect();
                } else if lower_rel.contains("package.json") || lower_rel.contains("cargo.toml")
                    || lower_rel.contains("go.mod") || lower_rel.contains("pyproject.toml")
                    || lower_rel.contains("gemfile") || lower_rel.contains("cmakelists.txt")
                    || lower_rel.contains("pom.xml") || lower_rel.contains("build.gradle")
                {
                    manifest_files.push((rel_path.clone(), excerpt.chars().take(excerpt_chars).collect()));
                } else {
                    source_files.push((rel_path.clone(), excerpt.chars().take(excerpt_chars).collect()));
                }

                total_chars += excerpt_chars;
            }
        }
    }

    // Build a summary
    let tech_stack = detect_tech_stack(&manifest_files, &source_files);
    let summary = format!(
        "项目包含 {} 个源文件。\n技术栈：{}\nReadme：{}\n",
        scanned_files.len(),
        tech_stack,
        if readme_content.chars().count() > 200 { format!("{}...", readme_content.chars().take(200).collect::<String>()) } else { readme_content.clone() }
    );

    // Sort: manifests first, then source
    let file_count = scanned_files.len();
    let evidence_count = manifest_files.len() + source_files.len();
    scanned_files.sort();

    Ok(project::ExistingProjectBaseline {
        project_summary: summary,
        tech_stack,
        architecture_evidence: format!("总文件数：{}，已读取 {} 个文件的核心内容（约 {} 字符）", file_count, evidence_count, total_chars),
        completed_capabilities: vec![],
        pending_capabilities: vec![],
        risks: vec![],
        uncertainties: vec![],
        scanned_files,
        scan_complete: true,
        evidence_summary: format!("扫描 {} 个文件，读取 {} 个清单和核心文件（约 {} 字符）",
            file_count, evidence_count, total_chars),
        generated_at: chrono::Utc::now().to_rfc3339(),
        approved: false,
        approved_at: None,
        already_constitution_path: String::new(),
        already_constitution_summary: String::new(),
    })
}

/// 扫描已有项目目录（Tauri 命令 — 保留用于前端手动调用）
#[tauri::command]
pub(crate) async fn scan_existing_project(project_path: String) -> Result<project::ExistingProjectBaseline, String> {
    scan_internal(&project_path)
}

/// 统一分析入口：扫描 + AI 生成基线 + 持久化
#[tauri::command]
pub(crate) async fn analyze_existing_project(
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    if project_path.is_empty() {
        return Err("项目路径为空，请先设置项目路径".to_string());
    }

    // Step 1: Scan project files
    let mut baseline = scan_internal(&project_path)?;

    // Step 2: Generate AI analysis
    let prompt = format!(
        "你是一个项目分析专家。请分析以下已有项目信息，识别项目的完成能力和待处理能力。\n\n\
        项目摘要：\n{}\n\n技术栈：{}\n\n架构证据：{}\n\n\
        文件列表（共 {} 个）：\n{}\n\n\
        请以 JSON 格式返回分析结果，包含以下字段：\n\
        - completed_capabilities: 已完成的功能/能力列表（字符串数组）\n\
        - pending_capabilities: 待完成/需要改进的功能列表（字符串数组）\n\
        - risks: 项目风险列表（字符串数组）\n\
        - uncertainties: 不确定项列表（字符串数组）\n\
        请基于已有证据评估，不要编造。不确定的项目写入 uncertainties。",
        baseline.project_summary,
        baseline.tech_stack,
        baseline.architecture_evidence,
        baseline.scanned_files.len(),
        baseline.scanned_files.join("\n")
    );

    let result_str = crate::api::call_deepseek_api_json(
        &crate::prompts::EXISTING_BASELINE_PROMPT,
        &prompt,
    ).await?;

    let result: serde_json::Value = serde_json::from_str(&result_str)
        .map_err(|e| format!("解析 AI 返回的 JSON 失败：{}", e))?;

    if let Some(capabilities) = result["completed_capabilities"].as_array() {
        baseline.completed_capabilities = capabilities.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(pending) = result["pending_capabilities"].as_array() {
        baseline.pending_capabilities = pending.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(risks) = result["risks"].as_array() {
        baseline.risks = risks.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(uncertainties) = result["uncertainties"].as_array() {
        baseline.uncertainties = uncertainties.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }

    // Step 3: Save to project
    let mut proj = crate::load_project(&project_name)?;
    baseline.approved = false;
    baseline.scan_complete = true;
    proj.existing_baseline = Some(baseline.clone());
    proj.workflow_state.current_step = project::WorkflowStep::BaselineApproval;
    proj.workflow_state.data_revision += 1;
    crate::save_project(&proj)?;

    Ok(proj)
}

/// 通过 AI 生成详细基线内容
#[tauri::command]
pub(crate) async fn generate_existing_baseline(
    _project_name: String,
    baseline_json: String,
) -> Result<project::ExistingProjectBaseline, String> {
    let mut baseline: project::ExistingProjectBaseline =
        serde_json::from_str(&baseline_json).map_err(|e| format!("解析基线数据失败：{}", e))?;

    let prompt = format!(
        "你是一个项目分析专家。请分析以下已有项目信息，识别项目的完成能力和待处理能力。\n\n\
        项目摘要：\n{}\n\n技术栈：{}\n\n架构证据：{}\n\n\
        文件列表（共 {} 个）：\n{}\n\n\
        请以 JSON 格式返回分析结果，包含以下字段：\n\
        - completed_capabilities: 已完成的功能/能力列表（字符串数组）\n\
        - pending_capabilities: 待完成/需要改进的功能列表（字符串数组）\n\
        - risks: 项目风险列表（字符串数组）\n\
        - uncertainties: 不确定项列表（字符串数组）\n\
        请基于已有证据评估，不要编造。不确定的项目写入 uncertainties。",
        baseline.project_summary,
        baseline.tech_stack,
        baseline.architecture_evidence,
        baseline.scanned_files.len(),
        baseline.scanned_files.join("\n")
    );

    let result_str = crate::api::call_deepseek_api_json(
        &crate::prompts::EXISTING_BASELINE_PROMPT,
        &prompt,
    ).await?;

    let result: serde_json::Value = serde_json::from_str(&result_str)
        .map_err(|e| format!("解析 AI 返回的 JSON 失败：{}", e))?;

    if let Some(capabilities) = result["completed_capabilities"].as_array() {
        baseline.completed_capabilities = capabilities.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(pending) = result["pending_capabilities"].as_array() {
        baseline.pending_capabilities = pending.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(risks) = result["risks"].as_array() {
        baseline.risks = risks.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(uncertainties) = result["uncertainties"].as_array() {
        baseline.uncertainties = uncertainties.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }

    Ok(baseline)
}

/// 批准已有项目基线 — 先写入宪法第二部分，再标记批准
#[tauri::command]
pub(crate) async fn approve_existing_baseline(
    project_name: String,
) -> Result<project::Project, String> {
    let mut project = crate::load_project(&project_name)?;

    let baseline = project
        .existing_baseline
        .as_ref()
        .ok_or("项目没有已有项目基线数据".to_string())?;

    if !baseline.scan_complete {
        return Err("基线扫描尚未完成，请等待分析完成后再批准。".to_string());
    }
    if baseline.approved {
        return Err("基线已经批准，无需重复操作。".to_string());
    }

    let project_path = project.project_path.clone();
    if project_path.is_empty() {
        return Err("项目路径为空，无法写入宪法文件。".to_string());
    }

    // 1. 生成独立的 Already 项目宪法（与工作 CONSTITUTION.md 隔离）
    let already_constitution_content = build_already_constitution(baseline);
    let already_path = std::path::Path::new(&project_path).join("ALREADY_CONSTITUTION.md");
    std::fs::write(&already_path, &already_constitution_content)
        .map_err(|e| format!("写入 Already 宪法失败：{}", e))?;

    // 2. 生成 Already 宪法摘要
    let already_summary = build_already_summary(baseline);

    // 3. 写入工作宪法第二部分
    crate::constitution::write_constitution_part2(&project_path, baseline)?;

    // 4. 将 Already 摘要注入工作宪法第一部分"已有信息"段落
    inject_already_summary_into_part1(&project_path, &already_summary)?;

    // 5. 标记基线已批准，记录 Already 宪法路径
    if let Some(ref mut b) = project.existing_baseline {
        b.approved = true;
        b.approved_at = Some(chrono::Utc::now().to_rfc3339());
        b.already_constitution_path = already_path.to_string_lossy().to_string();
        b.already_constitution_summary = already_summary;
    }

    // 推进到 Discussion
    project.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    project.workflow_state.current_step = project::WorkflowStep::Discussion;
    project.workflow_state.data_revision += 1;

    crate::save_project(&project)?;
    Ok(project)
}

/// 构建独立的 Already 项目宪法（隔离于工作 CONSTITUTION.md）
fn build_already_constitution(baseline: &project::ExistingProjectBaseline) -> String {
    format!(
        "# Already 项目宪法（低权重全局记忆）\n\n\
         > 本文档由 AI 读取已有项目文件自动生成，作为背景参考。\n\
         > 权重低于工作 CONSTITUTION.md 和当前讨论。仅供参考，不得覆盖当前决策。\n\n\
         ## 项目摘要\n{}\n\n\
         ## 技术栈\n{}\n\n\
         ## 架构证据\n{}\n\n\
         ## 已完成能力\n{}\n\n\
         ## 待完成能力\n{}\n\n\
         ## 风险\n{}\n\n\
         ## 不确定项\n{}\n\n\
         ## 扫描信息\n- 文件数：{}\n- 证据：{}\n- 生成时间：{}\n",
        baseline.project_summary,
        baseline.tech_stack,
        baseline.architecture_evidence,
        baseline.completed_capabilities.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n"),
        baseline.pending_capabilities.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n"),
        baseline.risks.iter().map(|r| format!("- {}", r)).collect::<Vec<_>>().join("\n"),
        baseline.uncertainties.iter().map(|u| format!("- {}", u)).collect::<Vec<_>>().join("\n"),
        baseline.scanned_files.len(),
        baseline.evidence_summary,
        baseline.generated_at,
    )
}

/// 构建 Already 宪法精炼摘要（注入工作宪法第一部分）
fn build_already_summary(baseline: &project::ExistingProjectBaseline) -> String {
    let capabilities_summary = if baseline.completed_capabilities.is_empty() {
        "暂无已识别能力".to_string()
    } else {
        baseline.completed_capabilities.iter()
            .take(5)
            .map(|c| format!("- {}", c))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let risk_summary = if baseline.risks.is_empty() {
        "暂无已识别风险".to_string()
    } else {
        baseline.risks.iter()
            .take(3)
            .map(|r| format!("- {}", r))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "已有项目技术栈：{}。\n\
         项目摘要：{}\n\
         已有能力：\n{}\n\
         主要风险：\n{}\n\
         详细参见：ALREADY_CONSTITUTION.md（低权重参考）",
        baseline.tech_stack,
        baseline.project_summary.chars().take(300).collect::<String>(),
        capabilities_summary,
        risk_summary,
    )
}

/// 将 Already 摘要注入工作宪法第一部分"已有信息"段落
fn inject_already_summary_into_part1(
    project_path: &str,
    summary: &str,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    let constitution_path = Path::new(project_path).join("CONSTITUTION.md");

    if !constitution_path.exists() {
        // 创建新宪法
        let content = format!(
            "## 第 1 部分：项目长期规则\n\n### 已有信息\n{}\n\n（项目方案将在批准时写入）\n\n---\n\n## 第 2 部分：项目当前状态\n",
            summary
        );
        fs::write(&constitution_path, &content)
            .map_err(|e| format!("创建 CONSTITUTION.md 失败：{}", e))?;
        return Ok(());
    }

    let existing = fs::read_to_string(&constitution_path)
        .map_err(|e| format!("读取 CONSTITUTION.md 失败：{}", e))?;

    let new_content = if let Some(part1_pos) = existing.find("## 第 1 部分") {
        // 仅在 Part 1 范围内搜索"已有信息"段落
        let part1_range = &existing[part1_pos..];
        if let Some(info_pos) = part1_range.find("### 已有信息") {
            // 在已有信息段落后追加
            let after_info = &existing[info_pos..];
            if let Some(next_section) = after_info.find("\n### ") {
                let insert_pos = info_pos + next_section;
                format!(
                    "{}\n\n### 已有信息（已更新）\n{}\n{}",
                    existing[..insert_pos].trim_end(),
                    summary,
                    existing[insert_pos..].trim_start(),
                )
            } else {
                // 没有后续子节，追加到 Part 1 末尾
                format!("{}\n\n### 已有信息\n{}\n", existing.trim_end(), summary)
            }
        } else if let Some(part1_end) = existing.find("## 第 2 部分") {
            // 在 Part 1 末尾（Part 2 之前）插入已有信息
            let insert_pos = part1_end;
            format!(
                "{}\n\n### 已有信息\n{}\n\n{}",
                existing[..insert_pos].trim_end(),
                summary,
                existing[insert_pos..].trim_start(),
            )
        } else {
            // 没有 Part 2 — 追加到末尾
            format!("{}\n\n### 已有信息\n{}\n", existing.trim_end(), summary)
        }
    } else {
        // 没有 Part 1 — 在前面添加
        format!(
            "## 第 1 部分：项目长期规则\n\n### 已有信息\n{}\n\n---\n\n{}",
            summary,
            existing.trim_start(),
        )
    };

    fs::write(&constitution_path, &new_content)
        .map_err(|e| format!("更新 CONSTITUTION.md 已有信息失败：{}", e))?;

    Ok(())
}

/// 检测技术栈
fn detect_tech_stack(
    manifests: &[(String, String)],
    sources: &[(String, String)],
) -> String {
    let mut techs: Vec<String> = Vec::new();

    for (path, _) in manifests {
        let lower = path.to_lowercase();
        if lower.contains("cargo.toml") { techs.push("Rust".to_string()); }
        if lower.contains("package.json") { techs.push("Node.js/JavaScript/TypeScript".to_string()); }
        if lower.contains("go.mod") { techs.push("Go".to_string()); }
        if lower.contains("pyproject.toml") || lower.contains("requirements.txt") {
            techs.push("Python".to_string());
        }
        if lower.contains("pom.xml") || lower.contains("build.gradle") {
            techs.push("Java/Kotlin".to_string());
        }
        if lower.contains("gemfile") { techs.push("Ruby".to_string()); }
        if lower.contains("cmakelists.txt") { techs.push("C/C++".to_string()); }
    }

    // Detect from source files
    for (path, _) in sources {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => { if !techs.iter().any(|t| t == "Rust") { techs.push("Rust".to_string()); }},
            "ts" | "tsx" => { if !techs.iter().any(|t| t.contains("TypeScript")) { techs.push("TypeScript".to_string()); }},
            "js" | "jsx" => { if !techs.iter().any(|t| t.contains("JavaScript")) { techs.push("JavaScript".to_string()); }},
            "py" => { if !techs.iter().any(|t| t == "Python") { techs.push("Python".to_string()); }},
            "go" => { if !techs.iter().any(|t| t == "Go") { techs.push("Go".to_string()); }},
            _ => {}
        }
    }

    if techs.is_empty() {
        "未检测到明确技术栈".to_string()
    } else {
        techs.join(", ")
    }
}
