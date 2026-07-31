use std::sync::LazyLock;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::driver::LlmBackend;
use crate::openai::{resolve_api_key, Endpoint as OpenAiEndpoint, OpenAiInvoker};

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniMaxRegion {
    #[default]
    GlobalEn,
    CnZh,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MiniMaxProtocol {
    #[default]
    Anthropic,
    Openai,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MiniMaxThinking {
    Adaptive,
    Disabled,
}

impl MiniMaxThinking {
    fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct MiniMaxConfig {
    pub api_key: String,
    pub model: String,
    pub region: MiniMaxRegion,
    pub protocol: MiniMaxProtocol,
    pub global_openai_base_url: String,
    pub global_anthropic_base_url: String,
    pub cn_openai_base_url: String,
    pub cn_anthropic_base_url: String,
    pub system_prompt_file: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub thinking: Option<MiniMaxThinking>,
}

impl Default for MiniMaxConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "MiniMax-M3".to_string(),
            region: MiniMaxRegion::GlobalEn,
            protocol: MiniMaxProtocol::Anthropic,
            global_openai_base_url: "https://api.minimax.io/v1".to_string(),
            global_anthropic_base_url: "https://api.minimax.io/anthropic".to_string(),
            cn_openai_base_url: "https://api.minimaxi.com/v1".to_string(),
            cn_anthropic_base_url: "https://api.minimaxi.com/anthropic".to_string(),
            system_prompt_file: "data/system_prompt.txt".to_string(),
            max_tokens: 1024,
            temperature: 0.7,
            thinking: None,
        }
    }
}

struct ResolvedEndpoint {
    protocol: MiniMaxProtocol,
    url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    thinking: Option<MiniMaxThinking>,
}

impl MiniMaxConfig {
    fn endpoint(&self) -> anyhow::Result<ResolvedEndpoint> {
        if self.model.is_empty() {
            anyhow::bail!("minimax.model is not set");
        }

        let api_key = resolve_api_key(&self.api_key, "MINIMAX_API_KEY");
        if api_key.is_empty() {
            anyhow::bail!(
                "MiniMax API key not set. Set minimax.api_key in config or MINIMAX_API_KEY env var"
            );
        }

        let (base_url, path) = match (self.region, self.protocol) {
            (MiniMaxRegion::GlobalEn, MiniMaxProtocol::Openai) => {
                (&self.global_openai_base_url, "chat/completions")
            }
            (MiniMaxRegion::GlobalEn, MiniMaxProtocol::Anthropic) => {
                (&self.global_anthropic_base_url, "v1/messages")
            }
            (MiniMaxRegion::CnZh, MiniMaxProtocol::Openai) => {
                (&self.cn_openai_base_url, "chat/completions")
            }
            (MiniMaxRegion::CnZh, MiniMaxProtocol::Anthropic) => {
                (&self.cn_anthropic_base_url, "v1/messages")
            }
        };
        if base_url.is_empty() {
            anyhow::bail!("MiniMax base URL is not set for the selected region and protocol");
        }

        Ok(ResolvedEndpoint {
            protocol: self.protocol,
            url: format!("{}/{}", base_url.trim_end_matches('/'), path),
            api_key,
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            thinking: self.thinking,
        })
    }
}

pub enum MiniMaxInvoker {
    Openai(OpenAiInvoker),
    Anthropic(AnthropicInvoker),
}

impl MiniMaxInvoker {
    pub fn new(config: &MiniMaxConfig, system_prompt: String) -> anyhow::Result<Self> {
        let endpoint = config.endpoint()?;
        Ok(match endpoint.protocol {
            MiniMaxProtocol::Openai => Self::Openai(OpenAiInvoker::new(
                OpenAiEndpoint {
                    name: "MiniMax",
                    url: endpoint.url,
                    api_key: endpoint.api_key,
                    model: endpoint.model,
                    max_tokens: None,
                    max_completion_tokens: Some(endpoint.max_tokens),
                    temperature: endpoint.temperature,
                    reasoning_effort: None,
                    thinking: endpoint.thinking.map(|value| value.as_str().to_string()),
                    reasoning_split: Some(true),
                },
                system_prompt,
            )),
            MiniMaxProtocol::Anthropic => {
                Self::Anthropic(AnthropicInvoker::new(endpoint, system_prompt))
            }
        })
    }
}

#[async_trait]
impl LlmBackend for MiniMaxInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        match self {
            Self::Openai(invoker) => invoker.send_message(content).await,
            Self::Anthropic(invoker) => invoker.send_message(content).await,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<Value>),
}

#[derive(Clone, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    system: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingRequest>,
}

#[derive(Clone, Serialize)]
struct ThinkingRequest {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<Value>,
}

pub struct AnthropicInvoker {
    endpoint: ResolvedEndpoint,
    system_prompt: String,
    messages: Mutex<Vec<AnthropicMessage>>,
}

static HTTP: LazyLock<Client> = LazyLock::new(Client::new);

impl AnthropicInvoker {
    fn new(endpoint: ResolvedEndpoint, system_prompt: String) -> Self {
        info!(
            "MiniMax invoker ready (url={}, model={})",
            endpoint.url, endpoint.model
        );
        Self {
            endpoint,
            system_prompt,
            messages: Mutex::new(Vec::new()),
        }
    }

    async fn complete(&self, request: &AnthropicRequest) -> anyhow::Result<AnthropicResponse> {
        let response = HTTP
            .post(&self.endpoint.url)
            .bearer_auth(&self.endpoint.api_key)
            .json(request)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("MiniMax API request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read MiniMax response body: {e}"))?;

        if !status.is_success() {
            anyhow::bail!("MiniMax API error (HTTP {status}): {body}");
        }

        serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse MiniMax response: {e}\nRaw: {body}"))
    }
}

fn response_text(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect()
}

#[async_trait]
impl LlmBackend for AnthropicInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        debug!(">>> TO MINIMAX ({} bytes):\n{content}", content.len());

        let mut messages = self.messages.lock().await;
        let mut turn = messages.clone();
        turn.push(AnthropicMessage {
            role: "user".to_string(),
            content: AnthropicContent::Text(content.to_string()),
        });

        const MAX_PENDING_MESSAGES: usize = 39;
        if turn.len() > MAX_PENDING_MESSAGES {
            let keep_from = turn.len() - MAX_PENDING_MESSAGES;
            turn = turn[keep_from..].to_vec();
            warn!(
                "MiniMax: trimmed conversation history to {} messages",
                turn.len()
            );
        }

        let request = AnthropicRequest {
            model: self.endpoint.model.clone(),
            system: self.system_prompt.clone(),
            messages: turn,
            max_tokens: self.endpoint.max_tokens,
            temperature: self.endpoint.temperature,
            thinking: self.endpoint.thinking.map(|value| ThinkingRequest {
                kind: value.as_str().to_string(),
            }),
        };

        let response = self.complete(&request).await?;
        let result = response_text(&response.content);

        let mut turn = request.messages;
        turn.push(AnthropicMessage {
            role: "assistant".to_string(),
            content: AnthropicContent::Blocks(response.content),
        });
        *messages = turn;

        debug!("<<< FROM MINIMAX ({} bytes):\n{result}", result.len());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> MiniMaxConfig {
        MiniMaxConfig {
            api_key: "test-key".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn selects_region_and_protocol_endpoints() {
        let cases = [
            (
                MiniMaxRegion::GlobalEn,
                MiniMaxProtocol::Openai,
                "https://api.minimax.io/v1/chat/completions",
            ),
            (
                MiniMaxRegion::GlobalEn,
                MiniMaxProtocol::Anthropic,
                "https://api.minimax.io/anthropic/v1/messages",
            ),
            (
                MiniMaxRegion::CnZh,
                MiniMaxProtocol::Openai,
                "https://api.minimaxi.com/v1/chat/completions",
            ),
            (
                MiniMaxRegion::CnZh,
                MiniMaxProtocol::Anthropic,
                "https://api.minimaxi.com/anthropic/v1/messages",
            ),
        ];

        for (region, protocol, expected) in cases {
            let mut config = config();
            config.region = region;
            config.protocol = protocol;
            assert_eq!(config.endpoint().unwrap().url, expected);
        }
    }

    #[test]
    fn current_model_ids_are_passed_through() {
        for model in ["MiniMax-M3", "MiniMax-M2.7"] {
            let mut config = config();
            config.model = model.to_string();
            assert_eq!(config.endpoint().unwrap().model, model);
        }
    }

    #[test]
    fn parses_china_openai_configuration() {
        let config: MiniMaxConfig = toml::from_str(
            r#"
api_key = "test-key"
model = "MiniMax-M2.7"
region = "cn_zh"
protocol = "openai"
thinking = "disabled"
"#,
        )
        .unwrap();
        assert_eq!(config.region, MiniMaxRegion::CnZh);
        assert_eq!(config.protocol, MiniMaxProtocol::Openai);
        assert_eq!(config.thinking, Some(MiniMaxThinking::Disabled));
    }

    #[test]
    fn extracts_text_and_keeps_thinking_out_of_the_reply() {
        let blocks = vec![
            json!({"type": "thinking", "thinking": "private", "signature": "sig"}),
            json!({"type": "text", "text": "first"}),
            json!({"type": "text", "text": " second"}),
        ];
        assert_eq!(response_text(&blocks), "first second");
    }
}
