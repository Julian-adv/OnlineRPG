use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::driver::{LlmBackend, RunDir};

/// Configuration for the Claude CLI integration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClaudeConfig {
    /// Model to use (default: "sonnet")
    #[serde(default = "default_model")]
    pub model: String,
    /// Path to system prompt file (default: "data/system_prompt.txt")
    #[serde(default = "default_system_prompt_file")]
    pub system_prompt_file: String,
}

fn default_model() -> String {
    "sonnet".to_string()
}
fn default_system_prompt_file() -> String {
    "data/system_prompt.txt".to_string()
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            system_prompt_file: default_system_prompt_file(),
        }
    }
}

/// JSON output from `claude -p --output-format json`.
#[derive(Debug, Deserialize)]
struct JsonOutput {
    result: Option<String>,
    session_id: Option<String>,
}

/// Invokes `claude -p` per prompt, using `--resume` for conversation continuity.
/// First call captures session_id, subsequent calls resume that session.
pub struct ClaudeInvoker {
    config: ClaudeConfig,
    system_prompt: String,
    session_id: Mutex<Option<String>>,
    run_dir: RunDir,
}

impl ClaudeInvoker {
    pub fn new(config: &ClaudeConfig, system_prompt: String) -> anyhow::Result<Self> {
        info!("Claude invoker ready (model={})", config.model);
        Ok(Self {
            config: config.clone(),
            system_prompt,
            session_id: Mutex::new(None),
            run_dir: RunDir::create()?,
        })
    }
}

#[async_trait]
impl LlmBackend for ClaudeInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        debug!(">>> TO CLAUDE ({} bytes):\n{}", content.len(), content);

        let session_id = self.session_id.lock().await.clone();

        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--model")
            .arg(&self.config.model);

        if let Some(ref sid) = session_id {
            cmd.arg("--resume").arg(sid);
        } else {
            cmd.arg("--system-prompt").arg(&self.system_prompt);
        }

        // Player chat is untrusted: empty per-run cwd, no MCP servers, no
        // tools at all; `--` keeps the prompt out of flag parsing
        // (doc/REMOTE_AGENT_CLIENT.md).
        cmd.arg("--strict-mcp-config")
            .arg("--disallowedTools")
            .arg("Task,Bash,Glob,Grep,Read,Edit,Write,NotebookEdit,WebFetch,WebSearch")
            .arg("--")
            .arg(content)
            .current_dir(self.run_dir.path())
            .env_remove("CLAUDECODE")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let (mut child, mut group) = crate::process_group::spawn_in_group(&mut cmd, "claude")?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture claude stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture claude stderr"))?;

        // Log stderr in background
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "claude_stderr", "{}", line);
            }
        });

        // Read entire stdout (single JSON object)
        let mut raw = String::new();
        let mut reader = BufReader::new(stdout);
        tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut raw).await?;

        // Wait for process to finish
        let status = child.wait().await?;
        group.disarm();
        if !status.success() {
            warn!("Claude process exited with status: {status}");
        }

        // Parse JSON output
        let output: JsonOutput = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("Failed to parse claude JSON output: {e}\nRaw: {raw}"))?;

        let full_text = output.result.unwrap_or_default();

        // Store session_id for future --resume calls
        if let Some(sid) = output.session_id {
            let mut stored = self.session_id.lock().await;
            if stored.is_none() {
                info!("Claude session established: {sid}");
                *stored = Some(sid);
            }
        }

        debug!(
            "<<< FROM CLAUDE ({} bytes):\n{}",
            full_text.len(),
            full_text
        );
        Ok(full_text)
    }
}
