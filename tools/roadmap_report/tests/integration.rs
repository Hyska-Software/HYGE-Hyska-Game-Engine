use std::path::PathBuf;

use hyge_roadmap_report::{parse_file, parse_str, report::Report, validate, Severity};

fn data_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("data");
    p.push(name);
    p
}

#[test]
fn parses_valid_roadmap_from_file() {
    let roadmap = parse_file(data_path("valid.toml")).expect("parse valid roadmap");
    assert_eq!(roadmap.version, 1);
    assert_eq!(roadmap.title, "Test Roadmap");
    assert_eq!(roadmap.phases.len(), 2);
    assert_eq!(roadmap.items.len(), 3);
}

#[test]
fn parses_valid_roadmap_from_str() {
    let content = include_str!("data/valid.toml");
    let roadmap = parse_str(content).expect("parse valid roadmap from str");
    assert_eq!(roadmap.version, 1);
    assert_eq!(roadmap.items.len(), 3);
}

#[test]
fn validates_duplicate_phase_id() {
    let content = include_str!("data/invalid_phase.toml");
    let roadmap = parse_str(content).expect("parse");
    let issues = validate(&roadmap);
    let dup = issues
        .iter()
        .find(|i| i.message.contains("duplicate phase id"))
        .expect("expected duplicate phase id error");
    assert_eq!(dup.severity, Severity::Error);
}

#[test]
fn validates_item_issues() {
    let content = include_str!("data/invalid_items.toml");
    let roadmap = parse_str(content).expect("parse");
    let issues = validate(&roadmap);

    let messages: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();

    assert!(
        messages
            .iter()
            .any(|m| m.contains("priority") && m.contains("P9")),
        "expected bad priority error, got: {:?}",
        messages
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("unknown phase")),
        "expected unknown phase error, got: {:?}",
        messages
    );
    assert!(
        messages.iter().any(|m| m.contains("depends on itself")),
        "expected self-dependency error, got: {:?}",
        messages
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("unknown item: 'R-999'")),
        "expected unknown dep error, got: {:?}",
        messages
    );

    // All item issues should be errors, not warnings
    let item_errors: Vec<_> = issues
        .iter()
        .filter(|i| i.location.as_deref().unwrap_or("").starts_with("items."))
        .collect();
    assert!(!item_errors.is_empty());
    assert!(item_errors.iter().all(|i| i.severity == Severity::Error));
}

#[test]
fn valid_roadmap_has_no_errors() {
    let content = include_str!("data/valid.toml");
    let roadmap = parse_str(content).expect("parse");
    let issues = validate(&roadmap);
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "valid roadmap should have no errors, got: {:?}",
        errors
    );
}

#[test]
fn report_phase_totals_match_items() {
    let roadmap = parse_file(data_path("valid.toml")).expect("parse");
    let report = Report::from_roadmap(&roadmap);

    let sum_total: usize = report.phases.iter().map(|p| p.total).sum();
    let sum_complete: usize = report.phases.iter().map(|p| p.complete).sum();
    assert_eq!(sum_total, report.totals.items);
    assert_eq!(sum_complete, report.totals.complete);
    assert_eq!(report.totals.items, 3);
    assert_eq!(report.totals.complete, 1);
    assert_eq!(report.totals.in_progress, 1);
    assert_eq!(report.totals.not_started, 1);
}

#[test]
fn report_human_contains_phase_ids_and_total() {
    let roadmap = parse_file(data_path("valid.toml")).expect("parse");
    let report = Report::from_roadmap(&roadmap);
    let human = report.format_human();

    for phase in &report.phases {
        assert!(
            human.contains(&phase.id),
            "human output should contain phase id {}",
            phase.id
        );
    }
    assert!(
        human.contains("Total: 1/3 complete"),
        "human output should contain total line, got:\n{}",
        human
    );
    assert!(human.contains("By status:"));
}

#[test]
fn report_json_is_valid_and_round_trips() {
    let roadmap = parse_file(data_path("valid.toml")).expect("parse");
    let report = Report::from_roadmap(&roadmap);
    let json = serde_json::to_string(&report).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse json");
    assert!(value.get("phases").is_some());
    assert!(value.get("totals").is_some());
    assert_eq!(value["totals"]["items"], 3);
    assert_eq!(value["totals"]["complete"], 1);
}

#[test]
fn unknown_file_fails() {
    let result = parse_file(data_path("does_not_exist.toml"));
    assert!(result.is_err(), "expected error for missing file");
}
