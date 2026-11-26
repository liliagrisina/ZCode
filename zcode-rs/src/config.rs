use std::path::PathBuf;

/// 进程启动时读到的运行配置。密钥只来自环境变量，不写入会话文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub data_dir: PathBuf,
    pub workspace: PathBuf,
    pub max_steps: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigValues {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub fallback_api_key: Option<String>,
    pub model: Option<String>,
    pub data_base: Option<String>,
    pub home: Option<String>,
    pub workspace: Option<String>,
    pub cwd: Option<PathBuf>,
    pub max_steps: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_values(ConfigValues {
            api_base: std::env::var("ZCODE_API_BASE").ok(),
            api_key: std::env::var("ZCODE_API_KEY").ok(),
            fallback_api_key: std::env::var("OPENAI_API_KEY").ok(),
            model: std::env::var("ZCODE_MODEL").ok(),
            data_base: std::env::var("ZCODE_DATA_BASE_DIR").ok(),
            home: std::env::var("HOME").ok(),
            workspace: std::env::var("ZCODE_WORKSPACE").ok(),
            cwd: std::env::current_dir().ok(),
            max_steps: std::env::var("ZCODE_MAX_STEPS").ok(),
        })
    }

    pub fn from_values(values: ConfigValues) -> Self {
        let api_base = normalize_base(
            values
                .api_base
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("https://api.openai.com/v1"),
        );
        let api_key = non_empty(values.api_key).or_else(|| non_empty(values.fallback_api_key));
        let model = non_empty(values.model).unwrap_or_else(|| "gpt-4o-mini".to_string());
        let cwd = values.cwd.unwrap_or_else(|| PathBuf::from("."));
        let data_dir = if let Some(base) = non_empty(values.data_base) {
            PathBuf::from(base).join(".zcode-rs")
        } else if let Some(home) = non_empty(values.home) {
            PathBuf::from(home).join(".zcode-rs")
        } else {
            cwd.join(".zcode-rs")
        };
        let workspace = non_empty(values.workspace)
            .map(PathBuf::from)
            .unwrap_or(cwd);
        let max_steps = values
            .max_steps
            .as_deref()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(16)
            .clamp(1, 32);

        Self {
            api_base,
            api_key,
            model,
            data_dir,
            workspace,
            max_steps,
        }
    }

    pub fn key_is_set(&self) -> bool {
        self.api_key.as_ref().is_some_and(|key| !key.is_empty())
    }
}

pub fn chat_completions_url(api_base: &str) -> String {
    let base = normalize_base(api_base);
    if base.ends_with("/chat/completions") {
        base
    } else {
        format!("{base}/chat/completions")
    }
}

fn normalize_base(api_base: &str) -> String {
    api_base.trim().trim_end_matches('/').to_string()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let config = Config::from_values(ConfigValues {
            api_base: Some("https://example.test/v1/".into()),
            api_key: Some("  ".into()),
            fallback_api_key: Some("from-openai".into()),
            model: Some("demo-model".into()),
            data_base: Some("/tmp/zcode-data".into()),
            workspace: Some("/work/app".into()),
            max_steps: Some("99".into()),
            ..ConfigValues::default()
        });
        assert_eq!(config.api_base, "https://example.test/v1");
        assert_eq!(config.api_key.as_deref(), Some("from-openai"));
        assert_eq!(config.model, "demo-model");
        assert_eq!(config.data_dir, PathBuf::from("/tmp/zcode-data/.zcode-rs"));
        assert_eq!(config.workspace, PathBuf::from("/work/app"));
        assert_eq!(config.max_steps, 32);
        assert_eq!(
            chat_completions_url(&config.api_base),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://example.test/v1/chat/completions/"),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn prefers_zcode_key() {
        let config = Config::from_values(ConfigValues {
            api_key: Some("zcode-key".into()),
            fallback_api_key: Some("other".into()),
            home: Some("/home/dev".into()),
            ..ConfigValues::default()
        });
        assert_eq!(config.api_key.as_deref(), Some("zcode-key"));
        assert_eq!(config.data_dir, PathBuf::from("/home/dev/.zcode-rs"));
        assert_eq!(config.model, "gpt-4o-mini");
        assert_eq!(config.max_steps, 16);
    }
}
