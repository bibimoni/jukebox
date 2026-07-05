use anyhow::{anyhow, Result};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::config::validate_source_dir;

/// Prompt on stdin for a source directory, with `default` prefilled.
/// Reads one line, expands `~`, validates, repeats on bad input up to 3 times.
pub fn prompt_source_dir(default: &Path) -> Result<PathBuf> {
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    prompt_source_dir_with(&mut lock, default)
}

/// Testable variant: reads from `r`, writes the prompt to stderr.
pub fn prompt_source_dir_with<R: BufRead>(r: &mut R, default: &Path) -> Result<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    for _ in 0..3 {
        eprint!("Lossless source dir [{}]: ", default.display());
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 { return Err(anyhow!("no input on stdin")); }
        let raw = line.trim();
        let expanded = if let Some(rest) = raw.strip_prefix('~') {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            home.join(rest)
        } else if raw.is_empty() {
            default.to_path_buf()
        } else {
            PathBuf::from(raw)
        };
        match validate_source_dir(&expanded) {
            Ok(()) => return Ok(expanded.canonicalize().unwrap_or(expanded)),
            Err(e) => eprintln!("  invalid: {e}"),
        }
    }
    Err(anyhow!("gave up after 3 invalid attempts"))
}
