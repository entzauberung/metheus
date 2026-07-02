use crate::project;

/// 解析 git diff 的完整 stdout，提取变更摘要
///
/// 扫描 diff 输出中的文件增删改、函数增删改、依赖变更，
/// 自动跳过 node_modules/、target/、__pycache__/、.git/ 和 .lock 文件。
/// 纯文本解析，不涉及 I/O 或 AI 调用。
pub(crate) fn extract_diff_summary(diff_stdout: &str) -> project::DiffSummary {
    let mut summary = project::DiffSummary {
        new_files: Vec::new(),
        modified_files: Vec::new(),
        deleted_files: Vec::new(),
        new_functions: Vec::new(),
        modified_functions: Vec::new(),
        deleted_functions: Vec::new(),
        changed_dependencies: Vec::new(),
    };

    if diff_stdout.trim().is_empty() {
        return summary;
    }

    // 需要跳过的目录和文件模式
    let skip_patterns = ["node_modules/", "target/", "__pycache__/", ".git/"];
    let is_skipped = |path: &str| -> bool {
        if path.ends_with(".lock") {
            return true;
        }
        for pat in &skip_patterns {
            if path.contains(pat) {
                return true;
            }
        }
        false
    };

    // 收集文件名集合用于去重
    let mut new_files_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deleted_files_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut modified_files_set: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut new_funcs_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deleted_funcs_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut modified_funcs_set: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut deps_set: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 依赖文件名集合
    let dep_files: std::collections::HashSet<&str> = [
        "package.json",
        "Cargo.toml",
        "go.mod",
        "requirements.txt",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
    ]
    .iter()
    .cloned()
    .collect();

    let lines: Vec<&str> = diff_stdout.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // 检测新增文件
        if line.starts_with("new file mode") {
            // 下一行可能包含路径
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    new_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测删除文件
        if line.starts_with("deleted file mode") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("--- a/") {
                let path = lines[i + 1]
                    .strip_prefix("--- a/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    deleted_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 --- /dev/null（新增文件，无 new file mode 前缀时）
        if line.starts_with("--- /dev/null") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    new_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 +++ /dev/null（删除文件）
        if line.starts_with("+++ /dev/null") {
            if i >= 1 && lines[i - 1].starts_with("--- a/") {
                let path = lines[i - 1]
                    .strip_prefix("--- a/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    deleted_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 --- a/ 和 +++ b/ 同时出现 → 修改文件
        if line.starts_with("--- a/") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let old_path = line.strip_prefix("--- a/").unwrap_or("").replace('\\', "/");
                let new_path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                let path = if !new_path.is_empty() {
                    new_path
                } else {
                    old_path
                };
                if !path.is_empty() && !is_skipped(&path) {
                    modified_files_set.insert(path.clone());

                    // 检测是否为依赖文件变更
                    if let Some(filename) = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                    {
                        if dep_files.contains(filename) {
                            // 扫描该 diff 块内的 +/- 行
                            let mut j = i + 2;
                            while j < lines.len() {
                                let l = lines[j];
                                if l.starts_with("diff --git") || l.starts_with("--- a/") {
                                    break;
                                }
                                let content = if l.starts_with('+') && !l.starts_with("+++") {
                                    Some(l[1..].trim())
                                } else if l.starts_with('-') && !l.starts_with("---") {
                                    Some(l[1..].trim())
                                } else {
                                    None
                                };
                                if let Some(c) = content {
                                    if !c.is_empty() && c != "---" && c != "+++" {
                                        deps_set.insert(c.to_string());
                                    }
                                }
                                j += 1;
                            }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        // 提取新增函数（以 + 开头的行）
        if line.starts_with('+') && !line.starts_with("+++") {
            let content = &line[1..];
            if let Some(func_sig) = extract_function_signature(content) {
                new_funcs_set.insert(func_sig);
            }
        }

        // 提取删除函数（以 - 开头的行）
        if line.starts_with('-') && !line.starts_with("---") {
            let content = &line[1..];
            if let Some(func_sig) = extract_function_signature(content) {
                deleted_funcs_set.insert(func_sig);
            }
        }

        // 从 @@ 上下文行中提取可能被修改的函数名
        if line.starts_with("@@") {
            // 守卫：只处理包含已知语言函数定义关键字的 @@ 行
            let lang_keywords = ["fn ", "def ", "func ", "function ", "class "];
            let has_lang_keyword = lang_keywords.iter().any(|kw| line.contains(kw));
            if has_lang_keyword {
                if let Some(at_end) = line.rfind("@@") {
                    let ctx = &line[at_end + 2..];
                    let mut start = 0;
                    while start < ctx.len() {
                        if let Some(rest) = ctx.get(start..) {
                            if let Some(paren) = rest.find('(') {
                                let before = &rest[..paren];
                                // 向前找函数名起始（仅允许 ASCII 字母数字下划线）
                                if let Some(func_start) =
                                    before.rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                                {
                                    let fname = before[func_start + 1..].to_string();
                                    // 长度过滤：2-80 字符，且以字母或下划线开头
                                    if fname.len() >= 2
                                        && fname.len() <= 80
                                        && fname
                                            .chars()
                                            .next()
                                            .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
                                    {
                                        modified_funcs_set.insert(fname);
                                    }
                                } else if !before.is_empty()
                                    && before.len() >= 2
                                    && before.len() <= 80
                                    && before
                                        .chars()
                                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                                {
                                    modified_funcs_set.insert(before.to_string());
                                }
                                start += paren + 1;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        i += 1;
    }

    // 从 modified_files 中排除已归类为 new/deleted 的文件
    for f in &new_files_set {
        modified_files_set.remove(f);
    }
    for f in &deleted_files_set {
        modified_files_set.remove(f);
    }

    // 填充结果
    summary.new_files = new_files_set.into_iter().collect();
    summary.new_files.sort();
    summary.deleted_files = deleted_files_set.into_iter().collect();
    summary.deleted_files.sort();
    summary.modified_files = modified_files_set.into_iter().collect();
    summary.modified_files.sort();
    summary.new_functions = new_funcs_set.into_iter().collect();
    summary.new_functions.sort();
    summary.deleted_functions = deleted_funcs_set.into_iter().collect();
    summary.deleted_functions.sort();
    summary.modified_functions = modified_funcs_set.into_iter().collect();
    summary.modified_functions.sort();
    summary.changed_dependencies = deps_set.into_iter().collect();
    summary.changed_dependencies.sort();

    summary
}

/// 从一行代码中提取函数/方法签名
/// 支持 Rust / TypeScript / JavaScript / Python / Go / C++ / Java
/// 返回 None 表示该行不包含函数定义

pub(crate) fn extract_function_signature(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // 跳过注释行
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
        return None;
    }

    // Rust: fn / pub fn / pub async fn / unsafe fn
    if let Some(rest) = trimmed
        .strip_prefix("pub async fn ")
        .or_else(|| trimmed.strip_prefix("pub fn "))
        .or_else(|| trimmed.strip_prefix("async fn "))
        .or_else(|| trimmed.strip_prefix("unsafe fn "))
        .or_else(|| trimmed.strip_prefix("fn "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            let end = sig[..end].find(';').unwrap_or(end);
            return Some(format!("fn {}", sig[..end].trim()));
        }
    }

    // TypeScript/JS: function / export function / async function
    if let Some(rest) = trimmed
        .strip_prefix("export async function ")
        .or_else(|| trimmed.strip_prefix("export function "))
        .or_else(|| trimmed.strip_prefix("async function "))
        .or_else(|| trimmed.strip_prefix("function "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            return Some(format!("function {}", sig[..end].trim()));
        }
    }

    // TypeScript 箭头函数: const name = (...) =>
    if trimmed.starts_with("const ")
        && trimmed.contains('=')
        && (trimmed.contains("=>") || trimmed.contains(": ("))
    {
        let after_const = &trimmed[6..].trim();
        if let Some(eq) = after_const.find('=') {
            let name = after_const[..eq].trim();
            let name = name.split(':').next().unwrap_or(name).trim();
            if !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_alphabetic() || c == '_')
            {
                return Some(format!("const {} = (...) => {{...}}", name));
            }
        }
    }

    // Python: def / async def
    if let Some(rest) = trimmed
        .strip_prefix("async def ")
        .or_else(|| trimmed.strip_prefix("def "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find(':').unwrap_or(sig.len());
            return Some(format!("def {}", sig[..end].trim()));
        }
    }

    // Go: func / func (
    if let Some(rest) = trimmed.strip_prefix("func ") {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            return Some(format!("func {}", sig[..end].trim()));
        }
    }

    // Java: public/private/protected/static 后跟 (
    let java_modifiers = ["public ", "private ", "protected "];
    for modifier in &java_modifiers {
        if trimmed.starts_with(modifier) && trimmed.contains('(') {
            let rest = &trimmed[modifier.len()..];
            // 确保不是 class/interface/enum 声明
            if !rest.trim().starts_with("class ")
                && !rest.trim().starts_with("interface ")
                && !rest.trim().starts_with("enum ")
            {
                let end = rest.find('{').unwrap_or(rest.len());
                return Some(format!("{}{}", modifier, rest[..end].trim()));
            }
        }
    }

    // C++: ClassName::methodName(...) 模式
    if trimmed.contains("::") && trimmed.contains('(') && !trimmed.starts_with("//") {
        let end = trimmed.find('{').unwrap_or(trimmed.len());
        let end = trimmed[..end].find(';').unwrap_or(end);
        let candidate = trimmed[..end].trim();
        if candidate.contains('(') && candidate.contains("::") {
            return Some(candidate.to_string());
        }
    }

    None
}
