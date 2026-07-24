/// sanitize_json_response 的兜底值：当清洗结果为空时返回最小合法 JSON 对象
pub(crate) const SANITIZE_FALLBACK_JSON: &str = "{}";

/// DeepSeek API HTTP 请求超时秒数，防止网络故障导致永久阻塞
pub(crate) const DEEPSEEK_API_TIMEOUT_SECS: u64 = 120;

/// 当前所有 DeepSeek 工作流统一使用的模型。
pub(crate) const DEEPSEEK_WORKFLOW_MODEL: &str = "deepseek-v4-flash";

/// DeepSeek Chat Completions API 地址。
pub(crate) const DEEPSEEK_API_URL: &str = "https://api.deepseek.com/v1/chat/completions";

/// 应用级非敏感设置存储位置，位于 `~/.metheus/config/` 下以避免项目名冲突。
pub(crate) const APP_SETTINGS_RELATIVE_PATH: &str = ".metheus/config/app-settings.json";

/// 决策层 API Key 的首选与兼容环境变量名。
pub(crate) const DECISION_API_KEY_ENV: &str = "METHEUS_DECISION_API_KEY";
pub(crate) const LEGACY_DECISION_API_KEY_ENV: &str = "API_KEY";

/// 预装 Grok Build API Key 的首选与兼容环境变量名。
pub(crate) const BUILTIN_GROK_BUILD_API_KEY_ENV: &str = "METHEUS_BUILTIN_GROK_BUILD_API_KEY";
pub(crate) const LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV: &str = "METHEUS_BUILTIN_GROK_API_KEY";
pub(crate) const UPSTREAM_GROK_API_KEY_ENV: &str = "XAI_API_KEY";

pub(crate) const CREDENTIAL_SERVICE: &str = "com.bruce.metheus";
pub(crate) const DECISION_CREDENTIAL_ACCOUNT: &str = "decision-model-api-key";
pub(crate) const BUILTIN_GROK_BUILD_CREDENTIAL_ACCOUNT: &str = "built-in-grok-build-api-key";

pub(crate) const DEFAULT_BUILTIN_GROK_BUILD_API_BASE_URL: &str = "https://api.x.ai/v1";
pub(crate) const DEFAULT_BUILTIN_GROK_BUILD_MODEL: &str = "grok-4.5";
pub(crate) const DEFAULT_BUILTIN_GROK_BUILD_MAX_TURNS: u32 = 50;

/// Claude Code 子进程整体执行超时秒数，防止子进程卡死
pub(crate) const EXECUTION_ENGINE_TIMEOUT_SECS: u64 = 600;

#[allow(dead_code)]
pub(crate) const GIT_INIT_FAILED: &str = "自动初始化 Git 仓库失败";
#[allow(dead_code)]
pub(crate) const GIT_AUTO_INIT_COMMIT_MSG: &str = "初始提交（由 Metheus 自动创建）";

/// 触发宪法第 2 部分压缩的 token 阈值（估计值）
pub(crate) const COMPACTION_TRIGGER_TOKENS: f64 = 3000.0;
