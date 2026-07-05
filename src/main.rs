use clap::Parser;
use jukebox::cli::{self, Cmd};
use jukebox::config;
use jukebox::{catalog, player, search, tui};

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    match args.cmd.unwrap_or(Cmd::Play) {
        Cmd::Config { args } => {
            let _cfg = cli::ensure_config()?;
            if !args.is_empty() {
                eprintln!("config edits are not yet supported; edit {}", config::config_path().display());
            } else {
                println!("config: {}", config::config_path().display());
            }
        }
        Cmd::Play => {
            // TODO(tui-revamp): the old `tui::App`/`tui::Pane` were removed when
            // `src/tui/mod.rs` was scaffolded into the new module tree. The new
            // `tui::app::App` is implemented in Task 5 and re-wired here then.
            // Until then, `jukebox play` cannot launch the TUI.
            let _cfg = cli::ensure_config()?;
            anyhow::bail!("TUI revamp in progress (Task 1): `jukebox play` is temporarily unavailable");
        }
        Cmd::Sync => {
            let cfg = cli::ensure_config()?;
            let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("scripts/standardize.sh");
            // Fall back to a sibling-of-binary location if running installed.
            let script = if script.exists() { script } else {
                std::env::current_exe()?.parent().unwrap().join("scripts/standardize.sh")
            };
            let status = std::process::Command::new(&script)
                .args(["--source", &cfg.source_dir.display().to_string(),
                       "--out", &cfg.filtered_dir.display().to_string()])
                .status()?;
            if !status.success() { anyhow::bail!("standardize.sh failed"); }
            let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
            println!("synced: {} tracks", cat.tracks.len());
        }
        Cmd::Index => {
            let cfg = cli::ensure_config()?;
            let cat = jukebox::catalog::Catalog::load(
                &cfg.filtered_dir.join("catalog.json"),
            )?;
            jukebox::search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
            println!("indexed {} tracks", cat.tracks.len());
        }
        Cmd::Search { query } => {
            let cfg = cli::ensure_config()?;
            let s = jukebox::search::Searcher::open(
                &cfg.filtered_dir.join("search-index"),
            )?;
            let q = query.join(" ");
            let cat = jukebox::catalog::Catalog::load(
                &cfg.filtered_dir.join("catalog.json"),
            )?;
            for hit in s.search(&q, 25)? {
                if let Some(t) = cat.tracks.iter().find(|t| t.id == hit.track_id) {
                    println!(
                        "{:>3.0}%  {} — {} [{}]",
                        // BM25 is unbounded; cap the displayed relevance at 100%
                        // so a strong match reads as 100% rather than e.g. 200%.
                        (hit.score * 100.0).clamp(0.0, 100.0),
                        t.title,
                        t.primary_artist,
                        t.quality_label()
                    );
                }
            }
        }
    }
    Ok(())
}
