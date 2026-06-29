use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use hyge_roadmap_report::{parse_file, validate, Report, Severity};

#[derive(Parser, Debug)]
#[command(name = "hyge-roadmap-report", version, about = "Read docs/roadmap.toml and emit phase-grouped progress reports")]
struct Cli {
    /// Path to the roadmap.toml file.
    #[arg(long, short, default_value = "docs/roadmap.toml")]
    roadmap: PathBuf,

    /// Output format.
    #[arg(long, short, default_value = "human")]
    format: Format,

    /// Fail with non-zero exit if any validation error is found.
    #[arg(long)]
    strict: bool,

    /// Print only validation issues; no progress report.
    #[arg(long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Human,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let roadmap = parse_file(&cli.roadmap)
        .with_context(|| format!("failed to parse {}", cli.roadmap.display()))?;

    let issues = validate(&roadmap);
    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);

    if !cli.quiet {
        let report = Report::from_roadmap(&roadmap);
        match cli.format {
            Format::Human => print!("{}", report.format_human()),
            Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        }
    }

    if !issues.is_empty() {
        for issue in &issues {
            eprintln!("{}", issue);
        }
    }

    if cli.strict && has_errors {
        let error_count = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
        anyhow::bail!("roadmap validation failed with {} error(s)", error_count);
    }

    Ok(())
}
