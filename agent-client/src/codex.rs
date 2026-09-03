use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex, Notify};
use tracing::{debug, info, warn};

use crate::driver::{LlmBackend, RunDir};
use crate::process_group::GroupKill;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CodexConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_system_prompt_file")]
    pub system_prompt_file: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

fn default_model() -> String {
    "gpt-5.6-luna".to_string()
}

fn default_reasoning_effort() -> String {
    "low".to_string()
}

fn default_system_prompt_file() -> String {
    "data/system_prompt.txt".to_string()
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            system_prompt_file: default_system_prompt_file(),
            reasoning_effort: default_reasoning_effort(),
        }
    }
}

#[derive(Clone, Default)]
pub struct CodexAppServer {
    session: Arc<AsyncMutex<Option<Arc<AppServerSession>>>>,
}

impl CodexAppServer {
    pub fn new() -> Self {
        Self::default()
    }

    async fn session(&self) -> anyhow::Result<Arc<AppServerSession>> {
        let mut slot = self.session.lock().await;
        if let Some(session) = slot.as_ref().filter(|session| !session.is_dead()) {
            return Ok(Arc::clone(session));
        }

        let session = AppServerSession::spawn().await?;
        *slot = Some(Arc::clone(&session));
        Ok(session)
    }

    async fn send_turn(
        &self,
        config: &CodexConfig,
        cwd: &std::path::Path,
        prompt: String,
    ) -> anyhow::Result<String> {
        self.session().await?.run_turn(config, cwd, prompt).await
    }
}

type RpcReply = std::result::Result<Value, String>;
type AppServerInput = Box<dyn AsyncWrite + Unpin + Send>;

struct AppServerSession {
    input: AsyncMutex<AppServerInput>,
    pending: Mutex<HashMap<u64, oneshot::Sender<RpcReply>>>,
    turns: Mutex<HashMap<String, Arc<TurnRoute>>>,
    next_id: AtomicU64,
    dead: AtomicBool,
    process: Mutex<Option<(Child, GroupKill)>>,
    _run_dir: Option<RunDir>,
}

impl AppServerSession {
    async fn spawn() -> anyhow::Result<Arc<Self>> {
        let run_dir = RunDir::create()?;
        let mut command = codex_command();
        command
            .arg("app-server")
            .current_dir(run_dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let (mut child, group) =
            crate::process_group::spawn_in_group(&mut command, "codex app-server")?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture codex app-server stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture codex app-server stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture codex app-server stderr"))?;

        let session = Arc::new(Self {
            input: AsyncMutex::new(Box::new(input)),
            pending: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            dead: AtomicBool::new(false),
            process: Mutex::new(Some((child, group))),
            _run_dir: Some(run_dir),
        });

        tokio::spawn(read_stdout(Arc::downgrade(&session), output));
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "codex_stderr", "{line}");
            }
        });

        session
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "openmmo-agent-client",
                        "title": "OpenMMO Agent Client",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await
            .map_err(|error| anyhow::anyhow!("Failed to initialize codex app-server: {error}"))?;
        session.notify("initialized").await?;
        info!("Shared Codex app-server ready");
        Ok(session)
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    async fn request(self: &Arc<Self>, method: &str, params: Value) -> anyhow::Result<Value> {
        if self.is_dead() {
            return Err(anyhow::anyhow!("Codex app-server is not running"));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(id, sender);
        let _guard = PendingRequest {
            session: Arc::downgrade(self),
            id,
        };

        self.write_json(&json!({
            "method": method,
            "id": id,
            "params": params
        }))
        .await?;

        receiver
            .await
            .map_err(|_| anyhow::anyhow!("Codex app-server closed the {method} request"))?
            .map_err(|error| anyhow::anyhow!("Codex app-server {method} failed: {error}"))
    }

    async fn notify(&self, method: &str) -> anyhow::Result<()> {
        self.write_json(&json!({ "method": method })).await
    }

    async fn write_json(&self, value: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        let result = {
            let mut input = self.input.lock().await;
            input.write_all(&line).await
        };
        if let Err(error) = result {
            self.fail(format!("Codex app-server stdin failed: {error}"));
            return Err(error.into());
        }
        Ok(())
    }

    fn fail(&self, reason: String) {
        if self.dead.swap(true, Ordering::AcqRel) {
            return;
        }

        warn!("{reason}");
        for (_, sender) in lock(&self.pending).drain() {
            let _ = sender.send(Err(reason.clone()));
        }
        for (_, route) in lock(&self.turns).drain() {
            let _ = route.events.send(TurnEvent::Disconnected(reason.clone()));
        }

        if let Some((mut child, mut group)) = lock(&self.process).take() {
            if let Ok(Some(status)) = child.try_wait() {
                group.disarm();
                warn!("Codex app-server exited with status {status}");
            }
        }
    }

    fn route(&self, thread_id: &str) -> Option<Arc<TurnRoute>> {
        lock(&self.turns).get(thread_id).cloned()
    }

    async fn delete_thread(self: &Arc<Self>, thread_id: &str) {
        if self.is_dead() {
            return;
        }
        if let Err(error) = self
            .request("thread/delete", json!({ "threadId": thread_id }))
            .await
        {
            warn!("Codex app-server kept thread {thread_id}: {error}");
        }
    }

    fn remove_route(&self, thread_id: &str) {
        lock(&self.turns).remove(thread_id);
    }

    fn handle_message(self: &Arc<Self>, mut message: Value) {
        let Some(message) = message.as_object_mut() else {
            return;
        };
        if let Some(Value::String(method)) = message.remove("method") {
            if let Some(id) = message.remove("id") {
                self.reject_server_request(id, method);
            } else {
                self.handle_notification(&method, message.remove("params"));
            }
            return;
        }

        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(sender) = lock(&self.pending).remove(&id) else {
            return;
        };
        let reply = match message.remove("error") {
            Some(error) => Err(rpc_error(&error)),
            None => Ok(message.remove("result").unwrap_or(Value::Null)),
        };
        let _ = sender.send(reply);
    }

    fn reject_server_request(self: &Arc<Self>, id: Value, method: String) {
        let session = Arc::clone(self);
        tokio::spawn(async move {
            let _ = session
                .write_json(&json!({
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Unsupported server request: {method}")
                    }
                }))
                .await;
        });
    }

    fn handle_notification(self: &Arc<Self>, method: &str, params: Option<Value>) {
        if !matches!(method, "turn/started" | "item/completed" | "turn/completed") {
            return;
        }
        let Some(params) = params else {
            return;
        };
        let Some(thread_id) = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let Some(route) = self.route(&thread_id) else {
            return;
        };

        if method == "turn/started" {
            if let Some(turn_id) = params.pointer("/turn/id").and_then(Value::as_str) {
                route.set_turn_id(turn_id);
            }
            return;
        }

        let event = match method {
            "item/completed" => TurnEvent::Item(params),
            "turn/completed" => TurnEvent::Completed(params),
            _ => return,
        };
        if route.events.send(event).is_err() {
            self.remove_route(&thread_id);
        }
    }

    async fn run_turn(
        self: &Arc<Self>,
        config: &CodexConfig,
        cwd: &std::path::Path,
        prompt: String,
    ) -> anyhow::Result<String> {
        let thread_result = self
            .request("thread/start", thread_start_params(config, cwd))
            .await?;
        let thread_id = required_string(&thread_result, "/thread/id", "thread/start")?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        let route = Arc::new(TurnRoute {
            events: sender,
            turn_id: Mutex::new(None),
            turn_ready: Notify::new(),
        });
        lock(&self.turns).insert(thread_id.clone(), Arc::clone(&route));
        let mut guard = TurnGuard {
            session: Arc::clone(self),
            thread_id: thread_id.clone(),
            route,
            completed: false,
        };

        let turn_result = match self
            .request("turn/start", turn_start_params(config, &thread_id, prompt))
            .await
        {
            Ok(result) => result,
            Err(error) => {
                guard.finish();
                return Err(error);
            }
        };
        if let Some(turn_id) = turn_result.pointer("/turn/id").and_then(Value::as_str) {
            guard.route.set_turn_id(turn_id);
        }

        let mut last_text = String::new();
        capture_turn_messages(&turn_result, &mut last_text);
        while let Some(event) = receiver.recv().await {
            match event {
                TurnEvent::Item(params) => {
                    if let Some(item) = params.get("item") {
                        capture_agent_message(item, &mut last_text);
                    }
                }
                TurnEvent::Completed(params) => {
                    capture_turn_messages(&params, &mut last_text);
                    let status = params
                        .pointer("/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("failed");
                    let error = params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    guard.finish();
                    return completed_turn(status, last_text, error);
                }
                TurnEvent::Disconnected(reason) => return Err(anyhow::anyhow!(reason)),
            }
        }

        Err(anyhow::anyhow!(
            "Codex app-server stopped reporting thread {thread_id}"
        ))
    }
}

impl Drop for AppServerSession {
    fn drop(&mut self) {
        self.process
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

struct PendingRequest {
    session: Weak<AppServerSession>,
    id: u64,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(session) = self.session.upgrade() {
            lock(&session.pending).remove(&self.id);
        }
    }
}

struct TurnRoute {
    events: mpsc::UnboundedSender<TurnEvent>,
    turn_id: Mutex<Option<String>>,
    turn_ready: Notify,
}

impl TurnRoute {
    fn set_turn_id(&self, turn_id: &str) {
        *lock(&self.turn_id) = Some(turn_id.to_string());
        self.turn_ready.notify_one();
    }

    fn turn_id(&self) -> Option<String> {
        lock(&self.turn_id).clone()
    }
}

enum TurnEvent {
    Item(Value),
    Completed(Value),
    Disconnected(String),
}

struct TurnGuard {
    session: Arc<AppServerSession>,
    thread_id: String,
    route: Arc<TurnRoute>,
    completed: bool,
}

impl TurnGuard {
    fn finish(&mut self) {
        self.completed = true;
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        let thread_id = self.thread_id.clone();
        let route = Arc::clone(&self.route);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.session.remove_route(&thread_id);
            return;
        };
        let session = Arc::downgrade(&self.session);
        let completed = self.completed;
        runtime.spawn(async move {
            let turn_id = if completed {
                None
            } else {
                match route.turn_id() {
                    Some(turn_id) => Some(turn_id),
                    None => {
                        tokio::select! {
                            _ = route.turn_ready.notified() => route.turn_id(),
                            _ = tokio::time::sleep(Duration::from_secs(30)) => None,
                        }
                    }
                }
            };
            if let Some(session) = session.upgrade() {
                if let Some(turn_id) = turn_id {
                    let _ = session
                        .request(
                            "turn/interrupt",
                            json!({ "threadId": thread_id, "turnId": turn_id }),
                        )
                        .await;
                }
                session.remove_route(&thread_id);
                session.delete_thread(&thread_id).await;
            }
        });
    }
}

async fn read_stdout<R>(session: Weak<AppServerSession>, output: R)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(output).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                debug!(target: "codex_stdout", "{line}");
                let Ok(message) = serde_json::from_str(&line) else {
                    warn!("Ignoring malformed Codex app-server JSONL");
                    continue;
                };
                let Some(session) = session.upgrade() else {
                    break;
                };
                session.handle_message(message);
            }
            Ok(None) => {
                if let Some(session) = session.upgrade() {
                    session.fail("Codex app-server stdout closed".to_string());
                }
                break;
            }
            Err(error) => {
                if let Some(session) = session.upgrade() {
                    session.fail(format!("Codex app-server stdout failed: {error}"));
                }
                break;
            }
        }
    }
}

fn thread_start_params(config: &CodexConfig, cwd: &std::path::Path) -> Value {
    json!({
        "model": config.model,
        "cwd": cwd.to_string_lossy(),
        "approvalPolicy": "never",
        "sandbox": "read-only",
        // Ephemeral threads can't be deleted and the app-server keeps ~12 MB each
        // (OOM every 4h); persisted ones are deleted after the turn instead.
        "ephemeral": false,
        "serviceName": "openmmo-agent-client"
    })
}

fn turn_start_params(config: &CodexConfig, thread_id: &str, prompt: String) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
        "model": config.model,
        "effort": config.reasoning_effort,
        "approvalPolicy": "never"
    })
}

fn required_string(value: &Value, pointer: &str, method: &str) -> anyhow::Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("Codex app-server {method} returned no {pointer}"))
}

fn capture_turn_messages(value: &Value, last_text: &mut String) {
    let Some(items) = value.pointer("/turn/items").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        capture_agent_message(item, last_text);
    }
}

fn capture_agent_message(item: &Value, last_text: &mut String) {
    if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            *last_text = text.to_string();
        }
    }
}

fn completed_turn(
    status: &str,
    last_text: String,
    error: Option<String>,
) -> anyhow::Result<String> {
    if status != "completed" {
        return Err(anyhow::anyhow!(
            "Codex turn {status}: {}",
            error.unwrap_or_else(|| "no error detail".to_string())
        ));
    }
    let response = last_text.trim().to_string();
    if response.is_empty() {
        return Err(anyhow::anyhow!("Codex produced no agentMessage"));
    }
    Ok(response)
}

fn rpc_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct CodexInvoker {
    config: CodexConfig,
    system_prompt: String,
    run_dir: RunDir,
    app_server: CodexAppServer,
}

impl CodexInvoker {
    pub fn new(
        config: &CodexConfig,
        system_prompt: String,
        app_server: CodexAppServer,
    ) -> anyhow::Result<Self> {
        info!("Codex invoker ready (model={})", config.model);
        Ok(Self {
            config: config.clone(),
            system_prompt,
            run_dir: RunDir::create()?,
            app_server,
        })
    }
}

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
        debug!(">>> TO CODEX ({} bytes):\n{}", content.len(), content);
        let prompt = format!("{}\n\n{}", self.system_prompt, content);
        let response = self
            .app_server
            .send_turn(&self.config, self.run_dir.path(), prompt)
            .await?;
        debug!("<<< FROM CODEX ({} bytes):\n{}", response.len(), response);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, split};

    fn test_session(stream: tokio::io::DuplexStream) -> Arc<AppServerSession> {
        let (output, input) = split(stream);
        let session = Arc::new(AppServerSession {
            input: AsyncMutex::new(Box::new(input)),
            pending: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            dead: AtomicBool::new(false),
            process: Mutex::new(None),
            _run_dir: None,
        });
        tokio::spawn(read_stdout(Arc::downgrade(&session), output));
        session
    }

    type Requests = tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>;
    type Output = tokio::io::WriteHalf<tokio::io::DuplexStream>;

    async fn read_request(lines: &mut Requests) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .unwrap();
        serde_json::from_str(&line.unwrap().unwrap()).unwrap()
    }

    async fn expect_delete(requests: &mut Requests, output: &mut Output, thread_id: &str) {
        let delete = read_request(requests).await;
        assert_eq!(delete["method"], "thread/delete");
        assert_eq!(delete["params"]["threadId"], thread_id);
        write_message(output, json!({ "id": delete["id"].clone(), "result": {} })).await;
    }

    async fn write_message(output: &mut Output, message: Value) {
        output
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn runs_each_turn_in_a_fresh_thread_then_deletes_it() {
        let (client, server) = duplex(64 * 1024);
        let session = test_session(client);
        let (server_input, mut server_output) = split(server);
        let mut requests = BufReader::new(server_input).lines();

        let fake_server = tokio::spawn(async move {
            let thread = read_request(&mut requests).await;
            assert_eq!(thread["method"], "thread/start");
            assert_eq!(thread["params"]["ephemeral"], false);
            assert_eq!(thread["params"]["sandbox"], "read-only");
            assert_eq!(thread["params"]["approvalPolicy"], "never");
            write_message(
                &mut server_output,
                json!({
                    "id": thread["id"].clone(),
                    "result": { "thread": { "id": "thread-1" } }
                }),
            )
            .await;

            let turn = read_request(&mut requests).await;
            assert_eq!(turn["method"], "turn/start");
            assert_eq!(turn["params"]["threadId"], "thread-1");
            assert_eq!(turn["params"]["effort"], "low");
            assert_eq!(turn["params"]["input"][0]["text"], "system\n\nevent");
            write_message(
                &mut server_output,
                json!({
                    "id": turn["id"].clone(),
                    "result": {
                        "turn": { "id": "turn-1", "status": "inProgress", "items": [] }
                    }
                }),
            )
            .await;
            write_message(
                &mut server_output,
                json!({
                    "method": "item/completed",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "item": { "id": "item-1", "type": "agentMessage", "text": " answer " }
                    }
                }),
            )
            .await;
            write_message(
                &mut server_output,
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thread-1",
                        "turn": { "id": "turn-1", "status": "completed", "items": [] }
                    }
                }),
            )
            .await;

            expect_delete(&mut requests, &mut server_output, "thread-1").await;
        });

        let config = CodexConfig::default();
        let response = session
            .run_turn(
                &config,
                std::path::Path::new("/tmp"),
                "system\n\nevent".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(response, "answer");
        fake_server.await.unwrap();
    }

    #[tokio::test]
    async fn failed_turn_start_releases_its_thread() {
        let (client, server) = duplex(16 * 1024);
        let session = test_session(client);
        let (server_input, mut server_output) = split(server);
        let mut requests = BufReader::new(server_input).lines();

        let fake_server = tokio::spawn(async move {
            let thread = read_request(&mut requests).await;
            write_message(
                &mut server_output,
                json!({
                    "id": thread["id"].clone(),
                    "result": { "thread": { "id": "thread-failed" } }
                }),
            )
            .await;

            let turn = read_request(&mut requests).await;
            write_message(
                &mut server_output,
                json!({
                    "id": turn["id"].clone(),
                    "error": { "message": "turn rejected" }
                }),
            )
            .await;

            expect_delete(&mut requests, &mut server_output, "thread-failed").await;
        });

        let error = session
            .run_turn(
                &CodexConfig::default(),
                std::path::Path::new("/tmp"),
                "event".to_string(),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Codex app-server turn/start failed: turn rejected"
        );
        fake_server.await.unwrap();
        assert!(lock(&session.turns).is_empty());
    }

    #[tokio::test]
    async fn dropping_a_turn_interrupts_only_that_turn() {
        let (client, server) = duplex(16 * 1024);
        let session = test_session(client);
        let (server_input, mut server_output) = split(server);
        let mut requests = BufReader::new(server_input).lines();
        let (events, _) = mpsc::unbounded_channel();
        let route = Arc::new(TurnRoute {
            events,
            turn_id: Mutex::new(Some("turn-9".to_string())),
            turn_ready: Notify::new(),
        });
        lock(&session.turns).insert("thread-9".to_string(), Arc::clone(&route));

        drop(TurnGuard {
            session: Arc::clone(&session),
            thread_id: "thread-9".to_string(),
            route,
            completed: false,
        });

        let interrupt = read_request(&mut requests).await;
        assert_eq!(interrupt["method"], "turn/interrupt");
        assert_eq!(interrupt["params"]["threadId"], "thread-9");
        assert_eq!(interrupt["params"]["turnId"], "turn-9");
        write_message(
            &mut server_output,
            json!({ "id": interrupt["id"].clone(), "result": {} }),
        )
        .await;

        expect_delete(&mut requests, &mut server_output, "thread-9").await;
    }

    #[test]
    fn reports_failed_turn_details() {
        let error =
            completed_turn("failed", String::new(), Some("rate limited".to_string())).unwrap_err();
        assert_eq!(error.to_string(), "Codex turn failed: rate limited");
    }
}
