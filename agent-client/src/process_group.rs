//! CLI backends spawn npm wrappers (`codex`, `claude`) whose real binary
//! is a grandchild; `kill_on_drop` alone leaves it running after a timeout.

use tokio::process::{Child, Command};

/// Kills the child's process group when dropped; disarmed once the child
/// has been waited on.
pub struct GroupKill(Option<u32>);

impl GroupKill {
    pub fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: signal to a group created by process_group(0).
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
}

/// Spawns `cmd` as its own process group with `kill_on_drop`.
pub fn spawn_in_group(cmd: &mut Command, what: &str) -> anyhow::Result<(Child, GroupKill)> {
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn {what} CLI: {e}"))?;
    let guard = GroupKill(child.id());
    Ok((child, guard))
}
