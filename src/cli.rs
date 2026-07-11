use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::{config_path, Config};

#[derive(Parser, Debug)]
#[command(name = "jukebox", version, about = "Filtered-lossless jukebox")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Launch the TUI (default).
    Play,
    /// Run standardize.sh then rebuild the search index.
    Sync,
    /// Build/rebuild the Tantivy search index from catalog.json.
    Index,
    /// Re-run the directory prompt, or set a field.
    Config {
        /// e.g. `set source_dir <path>`
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// One-shot CLI search.
    Search {
        /// query string
        query: Vec<String>,
    },
}

/// Load config, or run the first-run prompt then load.
pub fn ensure_config() -> Result<Config> {
    if let Some(cfg) = Config::load()? {
        if cfg.source_dir.as_os_str().is_empty() {
            first_run()?;
        } else {
            return Ok(cfg);
        }
    } else {
        first_run()?;
    }
    Config::load()?.ok_or_else(|| anyhow::anyhow!("config still missing after first-run"))
}

fn first_run() -> Result<()> {
    eprintln!("Welcome to jukebox. Let's configure your library.");
    let default = dirs::home_dir()
        .map(|h| h.join("Music/lossless"))
        .unwrap_or_default();
    let source = crate::prompt::prompt_source_dir(&default)?;
    let cfg = Config::default_for(source);
    cfg.save()?;
    eprintln!("Saved config to {}", config_path().display());
    Ok(())
}
