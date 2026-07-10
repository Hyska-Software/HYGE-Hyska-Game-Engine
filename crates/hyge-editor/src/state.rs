//! Service-owned editor session metadata.

/// Mutable metadata owned by the service between requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorState {
    /// Last opened project path.
    pub project: Option<String>,
    /// Last opened scene path.
    pub scene: Option<String>,
}
