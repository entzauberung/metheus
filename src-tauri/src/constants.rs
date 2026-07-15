/// sanitize_json_response 的兜底值：当清洗结果为空时返回最小合法 JSON 对象
pub(crate) const SANITIZE_FALLBACK_JSON: &str = "{}";

/// DeepSeek API HTTP 请求超时秒数，防止网络故障导致永久阻塞
pub(crate) const DEEPSEEK_API_TIMEOUT_SECS: u64 = 120;

/// 当前所有 DeepSeek 工作流统一使用的模型。
pub(crate) const DEEPSEEK_WORKFLOW_MODEL: &str = "deepseek-v4-flash";

/// DeepSeek Chat Completions API 地址。
pub(crate) const DEEPSEEK_API_URL: &str = "https://api.deepseek.com/v1/chat/completions";

/// Claude Code 子进程整体执行超时秒数，防止子进程卡死
pub(crate) const CLAUDE_CODE_TIMEOUT_SECS: u64 = 600;

#[allow(dead_code)]
pub(crate) const GIT_INIT_FAILED: &str = "自动初始化 Git 仓库失败";
#[allow(dead_code)]
pub(crate) const GIT_AUTO_INIT_COMMIT_MSG: &str = "初始提交（由 Metheus 自动创建）";

/// 触发宪法第 2 部分压缩的 token 阈值（估计值）
pub(crate) const COMPACTION_TRIGGER_TOKENS: f64 = 3000.0;
