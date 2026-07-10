//! Per-connection authentication state.

use hyge_editor_protocol::Envelope;

/// Authentication state for one TCP connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConnectionAuth {
    authenticated: bool,
}

impl ConnectionAuth {
    pub(crate) fn is_authenticated(self) -> bool {
        self.authenticated
    }

    pub(crate) fn authenticate(&mut self, request: &Envelope, expected_token: &str) -> bool {
        let token = request
            .payload
            .get("session_token")
            .and_then(serde_json::Value::as_str);
        if token == Some(expected_token) {
            self.authenticated = true;
            true
        } else {
            false
        }
    }
}
