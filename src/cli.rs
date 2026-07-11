use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::{config_path, Config};

#[derive(Parser, Debug)]
#[command(name = "jukebox", version, about = "Filtered-lossless jukebox")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
    /// Increase feedback verbosity: `-v` = verbose (show the YT provider
    /// label even when Ready), `-vv` = debug (also show a per-tick counter).
    /// Mutually exclusive with `--quiet` (quiet wins).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Suppress non-error footer output (the key-hint bar + transient
    /// success messages). Errors + provider-state labels still show.
    /// Overrides `--verbose` when both are given.
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,
}

/// Verbosity level resolved from the `--verbose`/`--quiet` CLI flags. Stored
/// on [`crate::tui::app::App`] and consulted by the footer / view layer to
/// decide how much chrome to render. `Quiet` shows only errors; `Normal` is
/// the default hint bar; `Verbose` shows the YT provider label even when
/// Ready; `Debug` adds a per-tick diagnostic counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
    Debug,
}

impl Verbosity {
    /// Resolve the level from the parsed CLI flags. `quiet` (a bool)
    /// wins over `verbose` (a count): `jukebox -qv` is Quiet. `verbose`
    /// counts as: 0 → Normal, 1 → Verbose, 2+ → Debug.
    pub fn from_flags(quiet: bool, verbose: u8) -> Self {
        if quiet {
            return Verbosity::Quiet;
        }
        match verbose {
            0 => Verbosity::Normal,
            1 => Verbosity::Verbose,
            _ => Verbosity::Debug,
        }
    }
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
