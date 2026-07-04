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
            let cfg = cli::ensure_config()?;
            let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            let index_path = cfg.filtered_dir.join("search-index");
            let searcher = match search::Searcher::open(&index_path) {
                Ok(s) => Some(s),
                Err(_) => {
                    eprintln!(
                        "search index not found at {}; run `jukebox sync` to build it",
                        index_path.display()
                    );
                    None
                }
            };
            let player = player::launch(cfg.player, &cfg.mpv_socket);
            let mut app = tui::App::new(cat, player, searcher);
            app.switch_sample_rate = cfg.switch_sample_rate;
            app.run()?;
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
