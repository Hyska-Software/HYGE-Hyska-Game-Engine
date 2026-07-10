//! Per-connection authentication state.

use hyge_editor_protocol::Envelope;

use crate::state::SessionBinding;

/// Authentication state for one TCP connection.
#[derive(Debug, Default)]
pub(crate) struct ConnectionAuth {
    pub(crate) binding: Option<SessionBinding>,
    seen_message_ids: std::collections::HashSet<String>,
}

impl ConnectionAuth {
    pub(crate) fn authenticate(&mut self, request: &Envelope, expected_token: &str) -> bool {
        let token = request
            .payload
            .get("session_token")
            .and_then(serde_json::Value::as_str);
        token == Some(expected_token)
    }

    pub(crate) fn mark_message_id(&mut self, message_id: &str) -> bool {
        self.seen_message_ids.insert(message_id.to_owned())
    }
}
