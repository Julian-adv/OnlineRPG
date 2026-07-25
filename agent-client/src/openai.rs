use std::sync::LazyLock;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::driver::LlmBackend;

/// Configuration for a generic OpenAI-compatible chat completions endpoint
/// (e.g. a LiteLLM proxy, vLLM, Ollama, or any self-hosted gateway).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    /// Endpoint origin plus whatever path prefix the provider needs; the code
    /// appends `/chat/completions` (e.g. "http://0.0.0.0:4000/v1").
    pub base_url: String,
    /// Bearer token (can also be set via the OPENAI_COMPAT_API_KEY env var).
    pub api_key: String,
    /// Model identifier, passed through to the endpoint as-is.
    pub model: String,
    /// Path to system prompt file
    pub system_prompt_file: String,
    /// Max tokens for the response
    pub max_tokens: u32,
    pub temperature: f32,
    /// Reasoning effort passed through as-is. Defaults to "none" so the token
    /// budget goes to the reply; "" omits the field for models that reject an
    /// unrecognized value.
    pub reasoning_effort: String,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            system_prompt_file: "data/system_prompt.txt".to_string(),
            max_tokens: 1024,
            temperature: 0.7,
            reasoning_effort: "none".to_string(),
        }
    }
}

/// Configured key wins, else the named env var. Backends name their own var:
/// codex holds the real OpenAI key in OPENAI_API_KEY, and these endpoints are
/// usually somebody else's.
pub(crate) fn resolve_api_key(configured: &str, env_key: &str) -> String {
    if configured.is_empty() {
        std::env::var(env_key).unwrap_or_default()
    } else {
        configured.to_string()
    }
}

impl OpenAiConfig {
    pub fn endpoint(&self) -> anyhow::Result<Endpoint> {
        if self.base_url.is_empty() {
            anyhow::bail!("openai.base_url is not set");
        }
        if self.model.is_empty() {
            anyhow::bail!("openai.model is not set");
        }

        Ok(Endpoint {
            name: "OpenAI-compatible",
            url: format!("{}/chat/completions", self.base_url.trim_end_matches('/')),
            api_key: resolve_api_key(&self.api_key, "OPENAI_COMPAT_API_KEY"),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            reasoning_effort: (!self.reasoning_effort.is_empty())
                .then(|| self.reasoning_effort.clone()),
        })
    }
}

/// A resolved chat completions endpoint. OpenRouter is one of these too, so
/// `openrouter.rs` builds an `Endpoint` instead of repeating the invoker.
pub struct Endpoint {
    pub name: &'static str,
    pub url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub reasoning_effort: Option<String>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
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
    endpoint: Endpoint,
    system_prompt: String,
    messages: Mutex<Vec<ChatMessage>>,
}

/// Shared by every invoker: it carries no per-endpoint settings (the call cap
/// is `TimeoutBackend`'s job), so NPCs on the same endpoint pool connections
/// instead of each holding their own.
static HTTP: LazyLock<Client> = LazyLock::new(Client::new);

impl OpenAiInvoker {
    pub fn new(endpoint: Endpoint, system_prompt: String) -> Self {
        info!(
            "{} invoker ready (url={}, model={})",
            endpoint.name, endpoint.url, endpoint.model
        );

        Self {
            endpoint,
            system_prompt,
            messages: Mutex::new(Vec::new()),
        }
    }

    /// One round trip.
    async fn complete(&self, request: &ChatRequest) -> anyhow::Result<String> {
        let name = self.endpoint.name;
        let mut req = HTTP.post(&self.endpoint.url).json(request);
        if !self.endpoint.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.endpoint.api_key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{name} API request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read {name} response body: {e}"))?;

        if !status.is_success() {
            anyhow::bail!("{name} API error (HTTP {status}): {body}");
        }

        let chat_response: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse {name} response: {e}\nRaw: {body}"))?;

        Ok(chat_response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default())
    }
}

#[async_trait]
impl LlmBackend for OpenAiInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        let name = self.endpoint.name;
        debug!(">>> TO {name} ({} bytes):\n{content}", content.len());

        let mut messages = self.messages.lock().await;

        if messages.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: self.system_prompt.clone(),
            });
        }

        // Built aside and committed only once the call comes back. Undoing the
        // user turn in an error arm instead would miss a cancelled call — the
        // scheduler's timeout drops this future — and leave two user messages
        // in a row, which stricter chat templates reject.
        let mut turn = messages.clone();
        turn.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });

        // Trim conversation history if it gets too long (keep system + last 20 turns)
        const MAX_MESSAGES: usize = 41; // system + 20 user/assistant pairs
        if turn.len() > MAX_MESSAGES {
            let system = turn[0].clone();
            let keep_from = turn.len() - (MAX_MESSAGES - 1);
            turn = std::iter::once(system)
                .chain(turn[keep_from..].iter().cloned())
                .collect();
            warn!(
                "{name}: trimmed conversation history to {} messages",
                turn.len()
            );
        }

        let request = ChatRequest {
            model: self.endpoint.model.clone(),
            messages: turn,
            max_tokens: self.endpoint.max_tokens,
            temperature: self.endpoint.temperature,
            reasoning_effort: self.endpoint.reasoning_effort.clone(),
        };

        let result = self.complete(&request).await?;

        let mut turn = request.messages;
        turn.push(ChatMessage {
            role: "assistant".to_string(),
            content: result.clone(),
        });
        *messages = turn;

        debug!("<<< FROM {name} ({} bytes):\n{result}", result.len());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(base_url: &str) -> OpenAiConfig {
        OpenAiConfig {
            base_url: base_url.to_string(),
            model: "some-model".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn endpoint_appends_path_once() {
        assert_eq!(
            config("http://host:4000/v1/").endpoint().unwrap().url,
            "http://host:4000/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_requires_base_url_and_model() {
        assert!(config("").endpoint().is_err());
        assert!(OpenAiConfig {
            base_url: "http://host/v1".to_string(),
            ..Default::default()
        }
        .endpoint()
        .is_err());
    }

    #[test]
    fn empty_reasoning_effort_omits_the_field() {
        let mut cfg = config("http://host/v1");
        assert_eq!(
            cfg.endpoint().unwrap().reasoning_effort.as_deref(),
            Some("none")
        );
        cfg.reasoning_effort = String::new();
        assert!(cfg.endpoint().unwrap().reasoning_effort.is_none());
    }
}
