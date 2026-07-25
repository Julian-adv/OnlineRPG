use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::driver::LlmBackend;

/// Configuration for a generic OpenAI-compatible chat completions endpoint
/// (e.g. a LiteLLM proxy, vLLM, Ollama, or any self-hosted gateway).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiConfig {
    /// Endpoint origin, without a trailing slash or `/chat/completions`
    /// (e.g. "http://0.0.0.0:4000", or "https://host/v1" for servers that
    /// require the `/v1` prefix).
    #[serde(default)]
    pub base_url: String,
    /// Bearer token (can also be set via the OPENAI_API_KEY env var).
    #[serde(default)]
    pub api_key: String,
    /// Model identifier, passed through to the endpoint as-is.
    #[serde(default)]
    pub model: String,
    /// Path to system prompt file (default: "data/system_prompt.txt")
    #[serde(default = "default_system_prompt_file")]
    pub system_prompt_file: String,
    /// Max tokens for the response (default: 1024)
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Temperature (default: 0.7)
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_system_prompt_file() -> String {
    "data/system_prompt.txt".to_string()
}
fn default_max_tokens() -> u32 {
    1024
}
fn default_temperature() -> f32 {
    0.7
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            system_prompt_file: default_system_prompt_file(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
        }
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Invokes any OpenAI-compatible chat completions endpoint over HTTP.
/// Maintains conversation history for multi-turn context.
pub struct OpenAiInvoker {
    client: Client,
    config: OpenAiConfig,
    url: String,
    system_prompt: String,
    api_key: String,
    messages: Mutex<Vec<ChatMessage>>,
}

impl OpenAiInvoker {
    pub fn new(config: &OpenAiConfig, system_prompt: String) -> anyhow::Result<Self> {
        if config.base_url.is_empty() {
            anyhow::bail!("openai.base_url is not set");
        }
        if config.model.is_empty() {
            anyhow::bail!("openai.model is not set");
        }

        let api_key = if !config.api_key.is_empty() {
            config.api_key.clone()
        } else {
            std::env::var("OPENAI_API_KEY").unwrap_or_default()
        };

        let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
        info!(
            "OpenAI-compatible invoker ready (url={url}, model={})",
            config.model
        );

        Ok(Self {
            client: Client::new(),
            config: config.clone(),
            url,
            system_prompt,
            api_key,
            messages: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl LlmBackend for OpenAiInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        debug!(
            ">>> TO OPENAI-COMPAT ({} bytes):\n{}",
            content.len(),
            content
        );

        let mut messages = self.messages.lock().await;

        if messages.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: self.system_prompt.clone(),
            });
        }

        messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });

        // Trim conversation history if it gets too long (keep system + last 20 turns)
        const MAX_MESSAGES: usize = 41; // system + 20 user/assistant pairs
        if messages.len() > MAX_MESSAGES {
            let system = messages[0].clone();
            let keep_from = messages.len() - (MAX_MESSAGES - 1);
            *messages = std::iter::once(system)
                .chain(messages[keep_from..].iter().cloned())
                .collect();
            warn!(
                "OpenAI-compat: trimmed conversation history to {} messages",
                messages.len()
            );
        }

        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: messages.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
        };

        let mut req = self.client.post(&self.url).json(&request);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI-compatible API request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read OpenAI-compatible response body: {e}"))?;

        if !status.is_success() {
            anyhow::bail!("OpenAI-compatible API error (HTTP {status}): {body}");
        }

        let chat_response: ChatResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("Failed to parse OpenAI-compatible response: {e}\nRaw: {body}")
        })?;

        let result = chat_response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default();

        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: result.clone(),
        });

        debug!(
            "<<< FROM OPENAI-COMPAT ({} bytes):\n{}",
            result.len(),
            result
        );
        Ok(result)
    }
}
