use std::path::Path;

use crate::error::{Error, Result};
use crate::model::Roadmap;

pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<Roadmap> {
    let display = path.as_ref().to_path_buf();
    let content = std::fs::read_to_string(&display).map_err(|source| Error::Io {
        path: display.clone(),
        source,
    })?;
    parse_str_with_path(&content, &display)
}

pub fn parse_str(s: &str) -> Result<Roadmap> {
    parse_str_with_path(s, &PathBuf::from("<string>"))
}

fn parse_str_with_path(s: &str, path: &Path) -> Result<Roadmap> {
    toml::from_str(s).map_err(|source| Error::Toml {
        path: path.to_path_buf(),
        source,
    })
}
