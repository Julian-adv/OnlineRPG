use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::driver::{LlmBackend, RunDir};

/// Configuration for the Codex CLI integration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CodexConfig {
    /// Model to use (default: "o4-mini")
    #[serde(default = "default_model")]
    pub model: String,
    /// Path to system prompt file (default: "data/system_prompt.txt")
    #[serde(default = "default_system_prompt_file")]
    pub system_prompt_file: String,
}

fn default_model() -> String {
    "o4-mini".to_string()
}
fn default_system_prompt_file() -> String {
    "data/system_prompt.txt".to_string()
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            system_prompt_file: default_system_prompt_file(),
        }
    }
}

/// JSONL event from `codex exec --json`.
/// Events have varying shapes; we extract text from `item.completed` events
/// and error details from `error` / `turn.failed` events.
#[derive(Debug, Deserialize)]
struct CodexEvent {
    #[serde(rename = "type")]
    event_type: String,
    /// Present on "item.completed" events
    item: Option<CodexItem>,
    /// Present on "error" events — raw error string (often nested JSON)
    message: Option<String>,
    /// Present on "turn.failed" events
    error: Option<CodexErrorBody>,
}

#[derive(Debug, Deserialize)]
struct CodexItem {
    #[serde(rename = "type")]
    item_type: Option<String>,
    /// Text content for agent_message items
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexErrorBody {
    message: Option<String>,
}

/// Try to peel the `error.message` from an outer JSON wrapper like
/// `{"type":"error","status":400,"error":{"message":"..."}}`. Returns the raw
/// string if it doesn't match that shape.
fn unwrap_error_message(raw: &str) -> String {
    #[derive(Deserialize)]
    struct Outer {
        error: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        message: Option<String>,
    }
    if let Ok(Outer {
        error: Some(Inner {
            message: Some(msg), ..
        }),
        ..
    }) = serde_json::from_str::<Outer>(raw)
    {
        msg
    } else {
        raw.to_string()
    }
}

/// Invokes `codex exec` per prompt in full-auto JSONL mode.
/// Prompt is piped via stdin (using `-` argument).
pub struct CodexInvoker {
    config: CodexConfig,
    system_prompt: String,
    run_dir: RunDir,
}

impl CodexInvoker {
    pub fn new(config: &CodexConfig, system_prompt: String) -> anyhow::Result<Self> {
        info!("Codex invoker ready (model={})", config.model);
        Ok(Self {
            config: config.clone(),
            system_prompt,
            run_dir: RunDir::create()?,
        })
    }
}

/// npm installs the CLI as a `.cmd` shim, which `Command` cannot spawn, so
/// look for a real `codex.exe` and otherwise run the package's node
/// entrypoint ourselves. Resolved once: the answer cannot change while we run.
#[cfg(windows)]
fn codex_command() -> Command {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    static PROGRAM: OnceLock<(PathBuf, Option<PathBuf>)> = OnceLock::new();

    let (program, entrypoint) = PROGRAM.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();

        if let Some(executable) = std::env::split_paths(&path)
            .map(|directory| directory.join("codex.exe"))
            .find(|candidate| candidate.is_file())
        {
            return (executable, None);
        }

        for directory in std::env::split_paths(&path) {
            let entrypoint = directory.join("node_modules/@openai/codex/bin/codex.js");
            if !entrypoint.is_file() {
                continue;
            }
            let bundled_node = directory.join("node.exe");
            let node = if bundled_node.is_file() {
                bundled_node
            } else {
                PathBuf::from("node.exe")
            };
            return (node, Some(entrypoint));
        }

        (PathBuf::from("codex.exe"), None)
    });

    let mut command = Command::new(program);
    if let Some(entrypoint) = entrypoint {
        command.arg(entrypoint);
    }
    command
}

#[cfg(not(windows))]
fn codex_command() -> Command {
    Command::new("codex")
}

#[async_trait]
impl LlmBackend for CodexInvoker {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        info!(">>> TO CODEX ({} bytes):\n{}", content.len(), content);

        let full_prompt = format!("{}\n\n{}", self.system_prompt, content);

        let mut cmd = codex_command();
        cmd.arg("exec")
            .arg("--sandbox")
            .arg("read-only")
            .arg("--json")
            .arg("--ephemeral")
            .arg("--skip-git-repo-check")
            .arg("-m")
            .arg(&self.config.model)
            .arg("-") // read prompt from stdin
            .current_dir(self.run_dir.path()) // empty per-run cwd, no AGENTS.md
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn codex CLI: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(full_prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture codex stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture codex stderr"))?;

        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "codex_stderr", "{}", line);
            }
        });

        // Capture error/turn.failed messages so they can be surfaced if no
        // agent_message arrives.
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut last_text = String::new();
        let mut last_error: Option<String> = None;

        while let Ok(Some(line)) = lines.next_line().await {
            debug!(target: "codex_stdout", "{}", line);
            let Ok(event) = serde_json::from_str::<CodexEvent>(&line) else {
                continue;
            };
            match event.event_type.as_str() {
                "item.completed" => {
                    if let Some(text) = event
                        .item
                        .filter(|i| i.item_type.as_deref() == Some("agent_message"))
                        .and_then(|i| i.text)
                    {
                        last_text = text;
                    }
                }
                "error" => {
                    if let Some(raw) = event.message {
                        last_error = Some(unwrap_error_message(&raw));
                    }
                }
                "turn.failed" => {
                    if let Some(raw) = event.error.and_then(|e| e.message) {
                        last_error = Some(unwrap_error_message(&raw));
                    }
                }
                _ => {}
            }
        }

        let status = child.wait().await?;
        if !status.success() {
            warn!("Codex process exited with status: {status}");
        }

        let response = last_text.trim().to_string();
        if response.is_empty() {
            if let Some(err) = last_error {
                return Err(anyhow::anyhow!("Codex error: {err}"));
            }
            return Err(anyhow::anyhow!(
                "Codex produced no agent_message (exit: {status})"
            ));
        }
        info!("<<< FROM CODEX ({} bytes):\n{}", response.len(), response);
        Ok(response)
    }
}
