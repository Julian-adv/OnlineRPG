use crate::types::PlayerId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A consent request (party invite, friend request) awaiting an answer.
pub(crate) struct PendingConsent {
    pub expires_at: Instant,
    /// A declined request stays here (answered) until it expires: removing it
    /// would hand the spam brake back to the asker on every decline.
    pub answered: bool,
}

impl PendingConsent {
    pub fn new(ttl: Duration) -> Self {
        Self {
            expires_at: Instant::now() + ttl,
            answered: false,
        }
    }

    /// Still open to an answer: neither declined nor expired.
    pub fn awaiting(&self) -> bool {
        !self.answered && self.expires_at > Instant::now()
    }
}

/// Answer the request under `key`: accepting consumes it, declining keeps it
/// as the spam brake. False when there was nothing usable to answer.
pub(crate) fn answer_consent(
    requests: &mut HashMap<(PlayerId, PlayerId), PendingConsent>,
    key: (PlayerId, PlayerId),
    accept: bool,
) -> bool {
    let usable = requests.get(&key).is_some_and(PendingConsent::awaiting);
    if usable {
        if accept {
            requests.remove(&key);
        } else if let Some(request) = requests.get_mut(&key) {
            request.answered = true;
        }
    }
    usable
}
