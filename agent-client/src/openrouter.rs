use serde::Deserialize;

use crate::openai::{resolve_api_key, Endpoint, OpenAiConfig, OpenAiInvoker};

/// Configuration for OpenRouter API integration. Separate from `OpenAiConfig`
/// only because the URL is fixed and reasoning is left to the model.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct OpenRouterConfig {
    /// OpenRouter API key (can also be set via OPENROUTER_API_KEY env var)
    pub api_key: String,
    /// Model identifier (e.g. "google/gemini-2.0-flash-001", "meta-llama/llama-3-70b-instruct")
    pub model: String,
    /// Path to system prompt file
    pub system_prompt_file: String,
    /// Max tokens for the response
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        let shared = OpenAiConfig::default();
        Self {
            api_key: String::new(),
            model: "openrouter/hunter-alpha".to_string(),
            system_prompt_file: shared.system_prompt_file,
            max_tokens: shared.max_tokens,
            temperature: shared.temperature,
        }
    }
}

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// OpenRouter is a fixed-URL OpenAI-compatible endpoint, so it runs on the
/// invoker in `openai.rs`.
pub fn invoker(config: &OpenRouterConfig, system_prompt: String) -> anyhow::Result<OpenAiInvoker> {
    let api_key = resolve_api_key(&config.api_key, "OPENROUTER_API_KEY");
    if api_key.is_empty() {
        anyhow::bail!(
            "OpenRouter API key not set. Set openrouter.api_key in config or OPENROUTER_API_KEY env var"
        );
    }

    Ok(OpenAiInvoker::new(
        Endpoint {
            name: "OpenRouter",
            url: OPENROUTER_API_URL.to_string(),
            api_key,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            reasoning_effort: None,
        },
        system_prompt,
    ))
}
