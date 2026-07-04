use clap::Parser;
use jukebox::cli::{self, Cmd};
use jukebox::config;

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
        Cmd::Play => { eprintln!("(TUI not implemented yet)"); }
        Cmd::Sync => { eprintln!("(sync not implemented yet)"); }
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
                        hit.score * 100.0,
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
