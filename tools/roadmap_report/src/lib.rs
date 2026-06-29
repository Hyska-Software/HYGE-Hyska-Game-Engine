//! Roadmap report library: parse, validate, and emit phase-grouped progress
//! from `docs/roadmap.toml` files following the Hyge v1 schema.
//!
//! Implements roadmap item R-003.

pub mod error;
pub mod model;
pub mod parse;
pub mod report;
pub mod validate;

pub use error::{Error, Result};
pub use model::{Item, Metadata, Phase, Roadmap};
pub use parse::{parse_file, parse_str};
pub use report::{PhaseReport, Report, Totals};
pub use validate::{validate, Severity, ValidationIssue};
