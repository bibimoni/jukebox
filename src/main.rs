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

            // Resolve the YouTube sidecar script (mirrors standardize.sh
            // resolution): manifest dir in dev, sibling-of-binary when installed.
            let yt_script = {
                let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/yt/yt.py");
                if p.exists() {
                    p
                } else {
                    std::env::current_exe()?.parent().unwrap().join("scripts/yt/yt.py")
                }
            };
            // Resolve a python that has the YT deps. Prefer, in order:
            //   $JUKEBOX_YT_PYTHON  →  the jukebox venv (`:yt setup` creates it)
            //   →  system `python3`.
            // The venv lives next to the cookies file in the config dir.
            let yt_python = std::env::var_os("JUKEBOX_YT_PYTHON")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    let venv_py = jukebox::yt::session::venv_python();
                    if venv_py.exists() { Some(venv_py) } else { None }
                })
                .unwrap_or_else(|| std::path::PathBuf::from("python3"));
            // Spawn the sidecar best-effort. Auth preference is restored from
            // state.db below (after `app` exists so we can set the field), so
            // the initial spawn here is guest/pasted-cookies only; if a browser
            // was saved we re-spawn with it once the layout is loaded. A missing
            // python3/script/cookies just means YT features degrade to clean
            // stops — local playback is unaffected.
            let cookies = jukebox::yt::session::load_cookies();
            let yt_session = jukebox::yt::session::Session::spawn(&yt_python, &yt_script, cookies).ok();

            let mut app = tui::App::new(cat, player, searcher, yt_session);
            app.switch_sample_rate = cfg.switch_sample_rate;
            app.yt_python = yt_python;
            app.yt_script = yt_script;

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
                    "youtube" => tui::queue::ContinueMode::YouTube,
                    _ => tui::queue::ContinueMode::Off,
                };
                app.view = match layout.focus.as_str() {
                    "playlists" => View::Playlists,
                    "queue" => View::Queue,
                    "youtube" => View::Youtube,
                    _ => View::Artists,
                };
                app.source_mode = jukebox::mode::SourceMode::from_str(&layout.source_mode);
                // Restore the saved browser-auth preference. We do NOT re-spawn
                // `spawn_browser` here (that re-reads Chrome's Keychain-encrypted
                // cookie store → a password prompt every launch). Instead the
                // launch spawn above used `load_cookies()`, which reads the
                // decrypted cookies `:yt auth browser` persisted to our config
                // `cookies_file()` (0600) — no Keychain, no prompt. Only re-read
                // the browser when that cache is missing/empty (first launch
                // after choosing the browser, or cookies expired): then the one
                // Keychain prompt is expected, and it re-persists the cache.
                app.yt_browser = layout.yt_browser.clone();
                let cached_cookies = jukebox::yt::session::load_cookies();
                if !app.yt_browser.is_empty() && cached_cookies.is_none() {
                    // No cached cookies yet — read from the browser now (prompt)
                    // and persist, so future launches are prompt-free.
                    if let Ok(s) = jukebox::yt::session::Session::spawn_browser(
                        &app.yt_python, &app.yt_script, app.yt_browser.clone(),
                    ) {
                        app.yt_session = Some(s);
                        app.yt_status = Some(format!("YT auth: connected via {}", app.yt_browser));
                    } else {
                        // Browser spawn failed (e.g. profile locked, deps
                        // missing) — surface it; user can `:yt setup` / re-auth.
                        app.yt_error = Some(format!(
                            "could not read {} cookies — run :yt setup, or :yt auth browser {}",
                            app.yt_browser, app.yt_browser,
                        ));
                        app.yt_browser.clear();
                    }
                } else if !app.yt_browser.is_empty() && app.yt_session.is_some() {
                    // Cached cookies loaded — already authed, no prompt.
                    app.yt_status = Some(format!("YT auth: connected via {}", app.yt_browser));
                }
            }
            if let Ok(pls) = state::load_playlists() {
                app.playlists = pls;
            }

            // Probe the YT sidecar before entering the loop: fire a real
            // library_playlists fetch (it needs network — unlike ping, which
            // the sidecar answers even when ytmusicapi init failed). If it
            // can't reach YouTube (network block, VPN, rate-limit), don't strand
            // the user in the persisted Y view staring at an empty "loading…".
            // Fall back to Artists and surface why in the footer.
            if app.yt_session.is_some() {
                let mut reachable = false;
                if let Some(s) = app.yt_session.as_mut() {
                    // Short deadline: the sidecar's ytmusicapi init prints a
                    // network-flavored error and sets have=False, so this fetch
                    // returns an error fast (not a hang).
                    match s.library_playlists() {
                        Ok(_) => reachable = true,
                        Err(e) => {
                            app.yt_session = None;
                            app.yt_error = Some(format!(
                                "YouTube unreachable: {e}. Likely a network block, \
                                 VPN, or IP rate-limit — check your connection. \
                                 (Not fixed by :yt setup.)",
                            ));
                        }
                    }
                }
                if !reachable && app.view == View::Youtube {
                    app.view = View::Artists;
                }
            } else if app.view == View::Youtube {
                // No session was ever created (no auth) — don't open on the Y view.
                app.view = View::Artists;
            }

            // Apply the restored volume to the player backend so the persisted
            // level actually takes effect on launch (mpv defaults to 100%).
            let _ = app.player.set_volume(app.volume);
            let _ = app.player.set_muted(app.muted);

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
                app.source_mode,
                &app.yt_browser,
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
