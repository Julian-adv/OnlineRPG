//! LLM Scheduler: centralized priority queue + concurrency limiter for LLM calls.
//!
//! All NPC drivers submit LLM requests through the scheduler instead of calling
//! backends directly. The scheduler dispatches requests respecting `max_concurrent`
//! and prioritizing urgent events over routine/idle polls.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

use crate::driver::LlmBackend;
use crate::state::EventUrgency;

tokio::task_local! {
    /// How long the request waited for a slot before this task was dispatched.
    /// Published into the dispatched task so an observer running inside it (the
    /// spectator panel's backend wrapper) can attribute the wait without the
    /// scheduler having to know that observer exists.
    static QUEUE_WAIT: Duration;
}

/// Queue wait for the LLM turn running in the current task, if dispatched by
/// the scheduler. `None` when called outside a dispatched turn.
pub fn queue_wait() -> Option<Duration> {
    QUEUE_WAIT.try_with(|d| *d).ok()
}

/// Priority levels for LLM requests (lower number = higher priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LlmPriority {
    /// Urgent: combat damage, direct chat, death (process ASAP)
    Urgent = 0,
    /// Routine: periodic active-mode poll with events
    Routine = 1,
    /// Idle: periodic idle poll (lowest priority)
    Idle = 2,
}

impl From<EventUrgency> for LlmPriority {
    fn from(u: EventUrgency) -> Self {
        match u {
            EventUrgency::Urgent => LlmPriority::Urgent,
            EventUrgency::Routine => LlmPriority::Routine,
            EventUrgency::Noise => LlmPriority::Idle,
        }
    }
}

/// A request submitted to the scheduler.
struct LlmRequest {
    priority: LlmPriority,
    submitted_at: Instant,
    prompt: String,
    invoker: Arc<dyn LlmBackend>,
    response_tx: oneshot::Sender<anyhow::Result<String>>,
    /// Label for logging (e.g. NPC account name)
    label: String,
}

// BinaryHeap is a max-heap: we want urgent (priority=0) to be "greatest".
impl Eq for LlmRequest {}
impl PartialEq for LlmRequest {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.submitted_at == other.submitted_at
    }
}

impl Ord for LlmRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower priority value = more urgent = should be popped first (= "greater")
        (other.priority as u8)
            .cmp(&(self.priority as u8))
            // Break ties by submission time: older requests first
            .then_with(|| other.submitted_at.cmp(&self.submitted_at))
    }
}

impl PartialOrd for LlmRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Caps how long one backend call may run. Without it a hung endpoint or a
/// wedged CLI subprocess holds one of the scheduler's few slots forever, and
/// two of those stall every NPC.
///
/// Cancelling the inner future is what actually stops the work: reqwest aborts
/// the request, and the stdio backends spawn their CLI with `kill_on_drop`, so
/// the child is killed instead of left running.
pub struct TimeoutBackend {
    inner: Arc<dyn LlmBackend>,
    timeout: Duration,
}

impl TimeoutBackend {
    /// `Duration::ZERO` leaves the backend unwrapped — an escape hatch for
    /// stepping through a backend under a debugger.
    pub fn wrap(inner: Arc<dyn LlmBackend>, timeout: Duration) -> Arc<dyn LlmBackend> {
        if timeout.is_zero() {
            return inner;
        }
        Arc::new(Self { inner, timeout })
    }
}

#[async_trait]
impl LlmBackend for TimeoutBackend {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        tokio::time::timeout(self.timeout, self.inner.send_message(content))
            .await
            .map_err(|_| anyhow::anyhow!("LLM call timed out after {}s", self.timeout.as_secs()))?
    }
}

/// Handle for submitting LLM requests to the scheduler.
#[derive(Clone)]
pub struct LlmScheduler {
    request_tx: mpsc::UnboundedSender<LlmRequest>,
    request_timeout: Duration,
}

impl LlmScheduler {
    /// Create a new scheduler and spawn its background task.
    ///
    /// `max_concurrent`: maximum number of simultaneous LLM calls across all NPCs.
    /// `request_timeout`: what every backend is wrapped in before it can be
    /// submitted — it guards the slot, not any one provider.
    pub fn new(max_concurrent: usize, request_timeout: Duration) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        tokio::spawn(scheduler_loop(request_rx, max_concurrent));
        info!(
            "LLM scheduler started (max_concurrent={}, request_timeout={}s)",
            max_concurrent,
            request_timeout.as_secs()
        );
        Self {
            request_tx,
            request_timeout,
        }
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Submit an LLM request and wait for the response.
    ///
    /// The request is queued by priority. When a slot is available, the scheduler
    /// dispatches the call and returns the result.
    pub async fn submit(
        &self,
        label: &str,
        priority: LlmPriority,
        prompt: String,
        invoker: Arc<dyn LlmBackend>,
    ) -> anyhow::Result<String> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = LlmRequest {
            priority,
            submitted_at: Instant::now(),
            prompt,
            invoker,
            response_tx,
            label: label.to_string(),
        };
        self.request_tx
            .send(request)
            .map_err(|_| anyhow::anyhow!("LLM scheduler shut down"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("LLM scheduler dropped request"))?
    }
}

/// The scheduler's main loop. Receives requests, queues by priority, dispatches
/// up to `max_concurrent` at a time.
async fn scheduler_loop(
    mut request_rx: mpsc::UnboundedReceiver<LlmRequest>,
    max_concurrent: usize,
) {
    let mut queue: BinaryHeap<LlmRequest> = BinaryHeap::new();
    let mut in_flight: usize = 0;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<()>();

    loop {
        // Dispatch as many queued requests as slots allow
        while in_flight < max_concurrent {
            if let Some(req) = queue.pop() {
                // Skip requests whose receiver has been dropped (NPC disconnected)
                if req.response_tx.is_closed() {
                    debug!("[Scheduler] Skipping orphaned request for '{}'", req.label);
                    continue;
                }
                in_flight += 1;
                debug!(
                    "[Scheduler] Dispatching {:?} request for '{}' ({} in flight, {} queued)",
                    req.priority,
                    req.label,
                    in_flight,
                    queue.len()
                );
                let done_tx = done_tx.clone();
                let waited = req.submitted_at.elapsed();
                tokio::spawn(QUEUE_WAIT.scope(waited, async move {
                    let result = req.invoker.send_message(&req.prompt).await;
                    let _ = req.response_tx.send(result);
                    let _ = done_tx.send(());
                }));
            } else {
                break;
            }
        }

        // Wait for new request or completion notification
        tokio::select! {
          recv = request_rx.recv() => {
            match recv {
              Some(req) => {
                debug!(
                  "[Scheduler] Queued {:?} request for '{}' ({} in flight, {} queued)",
                  req.priority, req.label, in_flight, queue.len() + 1
                );
                queue.push(req);
              }
              None => {
                // All senders dropped — scheduler shutting down
                debug!("[Scheduler] All senders dropped, shutting down");
                return;
              }
            }
          }
          Some(()) = done_rx.recv() => {
            in_flight = in_flight.saturating_sub(1);
            debug!("[Scheduler] Request completed ({} in flight, {} queued)", in_flight, queue.len());
          }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A backend that sleeps, then reports the queue wait the scheduler scoped
    /// into its task — proving the wait is readable exactly where the panel
    /// wrapper reads it.
    struct Probe {
        hold: Duration,
        seen: Arc<std::sync::Mutex<Vec<Option<Duration>>>>,
    }

    #[async_trait]
    impl LlmBackend for Probe {
        async fn send_message(&self, _prompt: &str) -> anyhow::Result<String> {
            self.seen.lock().unwrap().push(queue_wait());
            tokio::time::sleep(self.hold).await;
            Ok(String::new())
        }
    }

    /// An idle probe, for tests that need a backend but assert nothing on it.
    fn probe() -> Arc<dyn LlmBackend> {
        Arc::new(Probe {
            hold: Duration::ZERO,
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    /// Reports whether its call was cancelled: a hung stdio backend only stops
    /// spawning-and-leaking children if the future is actually dropped.
    #[derive(Default)]
    struct Hang {
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    }

    struct SetOnDrop(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl LlmBackend for Hang {
        async fn send_message(&self, _prompt: &str) -> anyhow::Result<String> {
            let _guard = SetOnDrop(Arc::clone(&self.cancelled));
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    #[tokio::test]
    async fn a_hung_backend_gives_up_and_is_cancelled() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let backend = TimeoutBackend::wrap(
            Arc::new(Hang {
                cancelled: Arc::clone(&cancelled),
            }),
            Duration::from_millis(50),
        );

        let err = backend.send_message("p").await.unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        // Dropping the inner future is what kills a `kill_on_drop` child.
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_hung_backend_only_holds_its_slot_until_the_timeout() {
        let sched = LlmScheduler::new(1, Duration::from_millis(50));
        let hung = TimeoutBackend::wrap(Arc::new(Hang::default()), sched.request_timeout());

        let stuck = {
            let s = sched.clone();
            tokio::spawn(async move {
                s.submit("hung", LlmPriority::Routine, "p".into(), hung)
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await; // let it claim the only slot

        // Would never return if the hung call kept the slot.
        let next = tokio::time::timeout(
            Duration::from_secs(5),
            sched.submit("next", LlmPriority::Routine, "p".into(), probe()),
        )
        .await;
        assert!(next.expect("slot never freed").is_ok());
        assert!(stuck.await.unwrap().is_err());
    }

    #[test]
    fn a_zero_timeout_leaves_the_backend_alone() {
        let inner = probe();
        let wrapped = TimeoutBackend::wrap(Arc::clone(&inner), Duration::ZERO);
        assert!(Arc::ptr_eq(&inner, &wrapped));
    }

    #[tokio::test]
    async fn queue_wait_reflects_time_spent_behind_a_full_scheduler() {
        let sched = LlmScheduler::new(1, Duration::from_secs(120)); // one slot, so the second turn waits
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let backend = |hold| {
            Arc::new(Probe {
                hold,
                seen: Arc::clone(&seen),
            }) as Arc<dyn LlmBackend>
        };

        let hold = Duration::from_millis(300);
        let a = {
            let s = sched.clone();
            let b = backend(hold);
            tokio::spawn(async move { s.submit("a", LlmPriority::Routine, "p".into(), b).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await; // let `a` claim the slot
        let b = {
            let s = sched.clone();
            let b = backend(Duration::ZERO);
            tokio::spawn(async move { s.submit("b", LlmPriority::Routine, "p".into(), b).await })
        };

        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let waits = seen.lock().unwrap().clone();
        assert_eq!(waits.len(), 2);
        // `a` ran first with no meaningful wait; `b` sat behind `a`'s hold.
        assert!(waits[0].unwrap() < Duration::from_millis(100));
        assert!(
            waits[1].unwrap() >= Duration::from_millis(250),
            "second turn should report the queue wait, got {:?}",
            waits[1]
        );
    }

    #[tokio::test]
    async fn queue_wait_is_none_outside_a_dispatched_turn() {
        assert!(queue_wait().is_none());
    }
}
