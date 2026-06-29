use std::collections::HashSet;

use crate::model::Roadmap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub message: String,
    pub location: Option<String>,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.location {
            Some(loc) => write!(f, "[{:?}] {} (at {})", self.severity, self.message, loc),
            None => write!(f, "[{:?}] {}", self.severity, self.message),
        }
    }
}

const REQUIRED_STATUS: &[&str] = &["not_started", "in_progress", "blocked", "complete"];
const REQUIRED_PRIORITY: &[&str] = &["P0", "P1", "P2", "P3"];
const REQUIRED_RISK: &[&str] = &["low", "medium", "high"];

pub fn validate(roadmap: &Roadmap) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    validate_version(roadmap, &mut issues);
    validate_scales(roadmap, &mut issues);
    validate_phases(roadmap, &mut issues);
    validate_items(roadmap, &mut issues);
    validate_dependencies(roadmap, &mut issues);

    issues
}

fn validate_version(roadmap: &Roadmap, issues: &mut Vec<ValidationIssue>) {
    if roadmap.version != 1 {
        issues.push(ValidationIssue {
            severity: Severity::Error,
            message: format!(
                "unsupported version: {} (this tool targets v1)",
                roadmap.version
            ),
            location: Some("version".to_string()),
        });
    }
}

fn validate_scales(roadmap: &Roadmap, issues: &mut Vec<ValidationIssue>) {
    for required in REQUIRED_STATUS {
        if !roadmap.status_scale.iter().any(|s| s == required) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("status_scale missing required value '{}'", required),
                location: Some("status_scale".to_string()),
            });
        }
    }
    for required in REQUIRED_PRIORITY {
        if !roadmap.priority_scale.iter().any(|s| s == required) {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                message: format!("priority_scale missing recommended value '{}'", required),
                location: Some("priority_scale".to_string()),
            });
        }
    }
    for required in REQUIRED_RISK {
        if !roadmap.risk_scale.iter().any(|s| s == required) {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                message: format!("risk_scale missing recommended value '{}'", required),
                location: Some("risk_scale".to_string()),
            });
        }
    }
}

fn validate_phases(roadmap: &Roadmap, issues: &mut Vec<ValidationIssue>) {
    let mut seen: HashSet<&str> = HashSet::new();
    for phase in &roadmap.phases {
        if !seen.insert(phase.id.as_str()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("duplicate phase id: '{}'", phase.id),
                location: Some(format!("phases.{}", phase.id)),
            });
        }
    }
}

fn validate_items(roadmap: &Roadmap, issues: &mut Vec<ValidationIssue>) {
    let phase_ids: HashSet<&str> = roadmap.phases.iter().map(|p| p.id.as_str()).collect();
    let mut seen: HashSet<&str> = HashSet::new();

    for item in &roadmap.items {
        if !seen.insert(item.id.as_str()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!("duplicate item id: '{}'", item.id),
                location: Some(format!("items.{}", item.id)),
            });
        }

        if !phase_ids.contains(item.phase.as_str()) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!(
                    "item references unknown phase: '{}'",
                    item.phase
                ),
                location: Some(format!("items.{}.phase", item.id)),
            });
        }

        if !roadmap.status_scale.contains(&item.status) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!(
                    "item status '{}' not in status_scale {:?}",
                    item.status, roadmap.status_scale
                ),
                location: Some(format!("items.{}.status", item.id)),
            });
        }

        if !roadmap.priority_scale.contains(&item.priority) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!(
                    "item priority '{}' not in priority_scale {:?}",
                    item.priority, roadmap.priority_scale
                ),
                location: Some(format!("items.{}.priority", item.id)),
            });
        }

        if !roadmap.risk_scale.contains(&item.risk) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: format!(
                    "item risk '{}' not in risk_scale {:?}",
                    item.risk, roadmap.risk_scale
                ),
                location: Some(format!("items.{}.risk", item.id)),
            });
        }

        if item.owner.is_empty() {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: "item owner is empty".to_string(),
                location: Some(format!("items.{}.owner", item.id)),
            });
        }
    }
}

fn validate_dependencies(roadmap: &Roadmap, issues: &mut Vec<ValidationIssue>) {
    let item_ids: HashSet<&str> = roadmap.items.iter().map(|i| i.id.as_str()).collect();
    for item in &roadmap.items {
        if item.dependencies.contains(&item.id) {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                message: "item depends on itself".to_string(),
                location: Some(format!("items.{}.dependencies", item.id)),
            });
        }
        for dep in &item.dependencies {
            if !item_ids.contains(dep.as_str()) {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message: format!(
                        "item depends on unknown item: '{}'",
                        dep
                    ),
                    location: Some(format!("items.{}.dependencies", item.id)),
                });
            }
        }
    }
}
