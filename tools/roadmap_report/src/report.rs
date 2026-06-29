use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::Roadmap;

#[derive(Debug, Clone, Serialize)]
pub struct PhaseReport {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub total: usize,
    pub complete: usize,
    pub in_progress: usize,
    pub blocked: usize,
    pub not_started: usize,
    pub percent_complete: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Totals {
    pub items: usize,
    pub complete: usize,
    pub in_progress: usize,
    pub blocked: usize,
    pub not_started: usize,
    pub percent_complete: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    pub updated_at: String,
    pub phases: Vec<PhaseReport>,
    pub totals: Totals,
}

impl Report {
    pub fn from_roadmap(roadmap: &Roadmap) -> Self {
        let mut by_phase: BTreeMap<&str, Vec<&crate::model::Item>> = BTreeMap::new();
        for item in &roadmap.items {
            by_phase
                .entry(item.phase.as_str())
                .or_default()
                .push(item);
        }

        let phase_reports: Vec<PhaseReport> = roadmap
            .phases
            .iter()
            .map(|phase| {
                let items = by_phase
                    .get(phase.id.as_str())
                    .cloned()
                    .unwrap_or_default();
                let total = items.len();
                let complete = items.iter().filter(|i| i.status == "complete").count();
                let in_progress = items.iter().filter(|i| i.status == "in_progress").count();
                let blocked = items.iter().filter(|i| i.status == "blocked").count();
                let not_started = items.iter().filter(|i| i.status == "not_started").count();
                let percent = percent(complete, total);
                PhaseReport {
                    id: phase.id.clone(),
                    name: phase.name.clone(),
                    goal: phase.goal.clone(),
                    total,
                    complete,
                    in_progress,
                    blocked,
                    not_started,
                    percent_complete: percent,
                }
            })
            .collect();

        let total_items = roadmap.items.len();
        let total_complete = roadmap.items.iter().filter(|i| i.status == "complete").count();
        let total_in_progress = roadmap
            .items
            .iter()
            .filter(|i| i.status == "in_progress")
            .count();
        let total_blocked = roadmap.items.iter().filter(|i| i.status == "blocked").count();
        let total_not_started = roadmap
            .items
            .iter()
            .filter(|i| i.status == "not_started")
            .count();

        let totals = Totals {
            items: total_items,
            complete: total_complete,
            in_progress: total_in_progress,
            blocked: total_blocked,
            not_started: total_not_started,
            percent_complete: percent(total_complete, total_items),
        };

        Report {
            title: roadmap.title.clone(),
            updated_at: roadmap.updated_at.clone(),
            phases: phase_reports,
            totals,
        }
    }

    pub fn format_human(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{}\n", self.title));
        out.push_str(&format!("Updated: {}\n", self.updated_at));
        out.push('\n');

        let id_width = self
            .phases
            .iter()
            .map(|p| p.id.len())
            .max()
            .unwrap_or(8)
            .max(8);
        let name_width = self
            .phases
            .iter()
            .map(|p| p.name.len())
            .max()
            .unwrap_or(20)
            .max(20);

        for phase in &self.phases {
            let name = truncate(&phase.name, name_width);
            out.push_str(&format!(
                "{:<id_w$}  {:<name_w$}  {:>3}/{:<3}  {:<11}  {:>5.1}%\n",
                phase.id,
                name,
                phase.complete,
                phase.total,
                primary_status_label(phase),
                phase.percent_complete,
                id_w = id_width,
                name_w = name_width,
            ));
        }

        out.push('\n');
        out.push_str(&format!(
            "Total: {}/{} complete ({:.1}%)\n",
            self.totals.complete, self.totals.items, self.totals.percent_complete
        ));

        out.push_str("\nBy status:\n");
        out.push_str(&format!("  complete:    {}\n", self.totals.complete));
        out.push_str(&format!("  in_progress: {}\n", self.totals.in_progress));
        out.push_str(&format!("  blocked:     {}\n", self.totals.blocked));
        out.push_str(&format!(
            "  not_started: {}\n",
            self.totals.not_started
        ));

        out
    }
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        round2((part as f64) * 100.0 / (total as f64))
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut t = s[..max.saturating_sub(1)].to_string();
        t.push('…');
        t
    }
}

fn primary_status_label(phase: &PhaseReport) -> &'static str {
    if phase.total == 0 {
        "no_items"
    } else if phase.complete == phase.total {
        "complete"
    } else if phase.blocked > 0 {
        "blocked"
    } else if phase.in_progress > 0 {
        "in_progress"
    } else {
        "not_started"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Metadata, Phase, Roadmap};

    fn empty_metadata() -> Metadata {
        Metadata {
            source_architecture: String::new(),
            source_agents: String::new(),
            target_state: String::new(),
            msrv: String::new(),
            channel: String::new(),
        }
    }

    fn empty_roadmap() -> Roadmap {
        Roadmap {
            version: 1,
            title: "Test".to_string(),
            updated_at: "2026-01-01".to_string(),
            status_scale: vec![
                "not_started".into(),
                "in_progress".into(),
                "blocked".into(),
                "complete".into(),
            ],
            priority_scale: vec!["P0".into(), "P1".into(), "P2".into(), "P3".into()],
            risk_scale: vec!["low".into(), "medium".into(), "high".into()],
            metadata: empty_metadata(),
            phases: vec![],
            items: vec![],
        }
    }

    fn item(id: &str, phase: &str, status: &str) -> Item {
        Item {
            id: id.to_string(),
            title: format!("Item {}", id),
            phase: phase.to_string(),
            owner: "test".to_string(),
            priority: "P0".to_string(),
            status: status.to_string(),
            risk: "low".to_string(),
            dependencies: vec![],
            summary: String::new(),
            completed_at: None,
            completion_notes: None,
            acceptance: vec![],
        }
    }

    #[test]
    fn empty_roadmap_reports_zero() {
        let report = Report::from_roadmap(&empty_roadmap());
        assert_eq!(report.totals.items, 0);
        assert_eq!(report.totals.complete, 0);
        assert_eq!(report.totals.percent_complete, 0.0);
    }

    #[test]
    fn counts_match_across_phases() {
        let mut r = empty_roadmap();
        r.phases = vec![
            Phase {
                id: "phase_0".into(),
                name: "First".into(),
                goal: "".into(),
            },
            Phase {
                id: "phase_1".into(),
                name: "Second".into(),
                goal: "".into(),
            },
        ];
        r.items = vec![
            item("R-001", "phase_0", "complete"),
            item("R-002", "phase_0", "in_progress"),
            item("R-003", "phase_1", "not_started"),
            item("R-004", "phase_1", "complete"),
        ];
        let report = Report::from_roadmap(&r);

        assert_eq!(report.totals.items, 4);
        assert_eq!(report.totals.complete, 2);
        assert_eq!(report.totals.in_progress, 1);
        assert_eq!(report.totals.not_started, 1);

        let phase_0 = &report.phases[0];
        assert_eq!(phase_0.total, 2);
        assert_eq!(phase_0.complete, 1);

        let phase_1 = &report.phases[1];
        assert_eq!(phase_1.total, 2);
        assert_eq!(phase_1.complete, 1);

        // Sum across phases matches totals
        let sum_complete: usize = report.phases.iter().map(|p| p.complete).sum();
        assert_eq!(sum_complete, report.totals.complete);
    }

    #[test]
    fn human_output_contains_phases_and_total() {
        let mut r = empty_roadmap();
        r.phases = vec![Phase {
            id: "phase_0".into(),
            name: "Foundation".into(),
            goal: "Setup".into(),
        }];
        r.items = vec![item("R-001", "phase_0", "complete")];
        let report = Report::from_roadmap(&r);
        let human = report.format_human();
        assert!(human.contains("phase_0"));
        assert!(human.contains("Foundation"));
        assert!(human.contains("Total: 1/1 complete"));
    }
}
