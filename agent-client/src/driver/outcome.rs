use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::warn;

use crate::state::SharedState;

use super::action::AgentAction;

#[derive(Debug, PartialEq)]
pub(super) enum ActionOutcome {
    Ran,
    Failed,
}

pub(super) async fn settle_action(
    state: &Arc<Mutex<SharedState>>,
    action: &AgentAction,
    (events_before, commands_before): (usize, u64),
) -> ActionOutcome {
    let mut state = state.lock().await;
    let failed = state
        .agent_events_from(events_before)
        .iter()
        .any(|event| reports_failure(event));
    if failed {
        return ActionOutcome::Failed;
    }

    let (events_now, commands_now) = state.action_progress();
    if action.outcome_speaks_for_itself()
        || events_now > events_before
        || commands_now > commands_before
    {
        return ActionOutcome::Ran;
    }

    warn!("Action {} produced no result at all", action.label());
    state.push_agent_event(format!(
        "[NoResult] Your {} did nothing and reached no one — it was not possible here. \
         Try something else rather than repeating it.",
        action.label()
    ));
    ActionOutcome::Failed
}

pub(super) fn reports_failure(event: &str) -> bool {
    let Some(tag) = event
        .strip_prefix('[')
        .and_then(|event| event.split(']').next())
    else {
        return false;
    };
    tag.ends_with("Failed") || matches!(tag, "Unreachable" | "NoResult")
}
