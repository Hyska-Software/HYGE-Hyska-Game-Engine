use serde::Deserialize;

/// Top-level roadmap document.
#[derive(Debug, Clone, Deserialize)]
pub struct Roadmap {
    pub version: u32,
    pub title: String,
    pub updated_at: String,
    pub status_scale: Vec<String>,
    pub priority_scale: Vec<String>,
    pub risk_scale: Vec<String>,
    pub metadata: Metadata,
    pub phases: Vec<Phase>,
    #[serde(default)]
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    pub source_architecture: String,
    pub source_agents: String,
    pub target_state: String,
    pub msrv: String,
    pub channel: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub goal: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub owner: String,
    pub priority: String,
    pub status: String,
    pub risk: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub completion_notes: Option<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
}
