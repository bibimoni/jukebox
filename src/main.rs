use clap::Parser;
use jukebox::cli::{self, Cmd};
use jukebox::config;
use jukebox::tui::app::{ColumnWidths, View};
use jukebox::tui::queue::{RepeatMode, ShuffleMode};
use jukebox::{catalog, player, search, state, tui};

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
            // The search index may not exist yet on a fresh install (the user
            // hasn't run `jukebox sync`); treat that as "no search" rather than
            // blocking playback.
            let searcher = search::Searcher::open(&cfg.filtered_dir.join("search-index")).ok();
            let player = player::launch(cfg.player, &cfg.mpv_socket);

            let mut app = tui::App::new(cat, player, searcher);
            app.switch_sample_rate = cfg.switch_sample_rate;

            // Restore persisted layout + transport modes. Best-effort: a missing
            // or corrupt DB just falls back to defaults. At startup the transport
            // context is empty (no track_ids), so the shuffle mode's order-rebuild
            // is a no-op — direct field assignment is equivalent to `set_shuffle`
            // here, and avoids the `&self`/`&mut self.transport` split-borrow.
            if let Ok(layout) = state::load_layout() {
                app.column_widths = ColumnWidths {
                    rail: layout.widths.rail,
                    col1: layout.widths.col1,
                    col2: layout.widths.col2,
                    col3: layout.widths.col3,
                };
                app.volume = layout.volume;
                app.transport.shuffle = match layout.shuffle.as_str() {
                    "smart" => ShuffleMode::Smart,
                    "random" => ShuffleMode::Random,
                    _ => ShuffleMode::Off,
                };
                app.transport.repeat = match layout.repeat.as_str() {
                    "all" => RepeatMode::All,
                    "one" => RepeatMode::One,
                    _ => RepeatMode::Off,
                };
                app.transport.continue_mode = match layout.continue_mode.as_str() {
                    "next" => tui::queue::ContinueMode::NextAlbum,
                    "radio" => tui::queue::ContinueMode::Radio,
                    _ => tui::queue::ContinueMode::Off,
                };
                app.view = match layout.focus.as_str() {
                    "playlists" => View::Playlists,
                    "queue" => View::Queue,
                    _ => View::Artists,
                };
            }
            if let Ok(pls) = state::load_playlists() {
                app.playlists = pls;
            }

            // Capture the default audio device's current format BEFORE entering
            // the loop, so the panic hook installed inside `event::run` can
            // restore it on exit/crash. `None` on non-macOS.
            let captured = jukebox::audio::capture_default_format();

            tui::event::run(&mut app, captured)?;

            // Persist final state on a clean exit. Best-effort: a failed save
            // (e.g. read-only config dir) must not turn a successful session
            // into an error.
            let _ = state::save_layout(
                app.focus_key(),
                &app.column_widths,
                app.volume,
                app.transport.shuffle,
                app.transport.repeat,
                app.transport.continue_mode,
            );
            let _ = state::save_playlists(&app.playlists);
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
