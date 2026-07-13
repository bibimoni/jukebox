use clap::Parser;
use jukebox::cli::{self, Cmd};
use jukebox::config;
use jukebox::tui::app::{ColumnWidths, View};
use jukebox::tui::queue::{RepeatMode, ShuffleMode};
use jukebox::{catalog, player, search, state, tui};

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    // Resolve verbosity once from the parsed flags; `quiet` wins over
    // `verbose`. `Cmd::Play` applies it to the App after construction.
    let verbosity = cli::Verbosity::from_flags(args.quiet, args.verbose);
    match args.cmd.unwrap_or(Cmd::Play) {
        Cmd::Config { args } => {
            let _cfg = cli::ensure_config()?;
            if !args.is_empty() {
                eprintln!(
                    "config edits are not yet supported; edit {}",
                    config::config_path().display()
                );
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
                let p =
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/yt/yt.py");
                if p.exists() {
                    p
                } else {
                    std::env::current_exe()?
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("scripts/yt/yt.py")
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
                    if venv_py.exists() {
                        Some(venv_py)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| std::path::PathBuf::from("python3"));
            // Spawn the sidecar best-effort. Auth preference is restored from
            // state.db below (after `app` exists so we can set the field), so
            // the initial spawn here is guest/pasted-cookies only; if a browser
            // was saved we re-spawn with it once the layout is loaded. A missing
            // python3/script/cookies just means YT features degrade to clean
            // stops — local playback is unaffected.
            let cookies = jukebox::yt::session::load_cookies();
            let yt_session =
                jukebox::yt::session::Session::spawn(&yt_python, &yt_script, cookies).ok();

            let mut app = tui::App::new(cat, player, searcher, yt_session);
            app.switch_sample_rate = cfg.switch_sample_rate;
            app.yt_python = yt_python;
            app.yt_script = yt_script;
            // Apply the resolved verbosity (from --verbose / --quiet) so the
            // footer / view layer can render the right amount of chrome.
            app.verbosity = verbosity;

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
                app.source_mode = jukebox::mode::SourceMode::parse_mode(&layout.source_mode);
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
                        &app.yt_python,
                        &app.yt_script,
                        app.yt_browser.clone(),
                    ) {
                        app.yt_session = Some(s);
                        // Authenticated but NOT synced — the credential hasn't
                        // been verified by a data fetch yet. The old code set
                        // yt_status = "connected via {browser}" here (yt-recon §8
                        // location 1), which was false-ready. The probe below (or
                        // refresh_yt_lists) must succeed to promote to Ready.
                        app.yt_state = jukebox::yt::state::YtState::AuthenticatedNotSynced;
                    } else {
                        // Browser spawn failed (e.g. profile locked, deps
                        // missing) — a hard failure, not retryable. The user
                        // needs :yt setup or a re-auth, not R.
                        app.yt_error = Some(format!(
                            "could not read {} cookies — run :yt setup, or :yt auth browser {}",
                            app.yt_browser, app.yt_browser,
                        ));
                        app.yt_browser.clear();
                        app.yt_state = jukebox::yt::state::YtState::Failed;
                    }
                } else if !app.yt_browser.is_empty() && app.yt_session.is_some() {
                    // Cached cookies loaded — the sidecar spawned with them, but
                    // we have NOT verified the credential works yet. Set
                    // AuthenticatedNotSynced (not "connected") until the probe
                    // below confirms. This fixes yt-recon §8 location 2.
                    app.yt_state = jukebox::yt::state::YtState::AuthenticatedNotSynced;
                }
            }
            if let Ok(pls) = state::load_playlists() {
                app.playlists = pls;
            }
            // Restore persisted `:` command history (D4 — bounded, dedup-adjacent,
            // persisted to state.db under 'command_history'). Best-effort: a
            // missing/corrupt DB just falls back to empty.
            if let Ok(hist) = state::load_command_history() {
                app.command_history = hist;
            }

            // DEF-034: load the listening-event history from state.db so the
            // recommendation profile survives restarts. Best-effort: a missing
            // or corrupt DB falls back to an empty profile (cold start). After
            // loading, enable per-event persistence so new events are saved.
            if let Ok(events) = state::load_events(10_000) {
                app.reco_events.extend_from(events);
                let evs: Vec<_> = app.reco_events.recent(app.reco_events.len()).into_iter().cloned().collect();
                app.reco_profile = jukebox::reco::profile::UserProfile::build_from_events(&evs);
            }
            app.persist_events = true;

            // Load cached YT lists from state.db so a launch while offline
            // shows cached playlists immediately. load_yt_lists_from_cache
            // also marks the state ReadyStale when the sidecar couldn't start
            // (offline — showing cached, press R to retry). The fire-and-forget
            // refresh below overwrites the cached lists with fresh data and
            // promotes to Ready when the network is up.
            app.load_yt_lists_from_cache();

            // Fire-and-forget YT refresh at launch. Replaces the old blocking
            // library_playlists() probe, which (a) hung launch on a network
            // timeout and (b) before the keep-session fix, discarded the session
            // on any error — forcing a re-login every launch (yt-recon §3 root
            // cause, the "repeatedly must log in" symptom). Now: if the sidecar
            // is up, fire a refresh and let on_tick promote to Ready (or demote
            // to ProviderError/AuthExpired) when the response lands. The Y view
            // shows "loading…" meanwhile (yt_lists_loading), which is the correct
            // UX — no need to fall back to Artists. If the sidecar never started
            // (no python3 / missing script) and there are NO cached lists, that's
            // a hard failure; fall back to Artists. When cached lists exist, the
            // helper above already set ReadyStale so the user can browse them.
            if app.yt_session.is_some() {
                // refresh_yt_lists sets Synchronizing + fires send_refresh; on_tick
                // promotes to Ready on success or demotes on error. The session
                // is NEVER discarded on a fetch failure — only the state label
                // changes, so the user presses R (retry_yt_probe) instead of
                // re-authenticating.
                app.refresh_yt_lists();
            } else if app.yt_lists.is_empty() {
                // Spawn failed (no python3 / missing script) and no cached
                // lists — a hard failure, not retryable. The user needs :yt
                // setup or an environment fix, not :yt auth or R.
                app.yt_state = jukebox::yt::state::YtState::Failed;
                app.yt_error = Some("YT sidecar could not start — run :yt setup".into());
                if app.view == View::Youtube {
                    app.view = View::Artists;
                }
            }
            // else: session is None but cached lists exist — load_yt_lists_from_cache
            // already set ReadyStale; leave the cached lists visible.

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
            let _ = state::save_layout(&state::LayoutSave {
                focus: app.focus_key(),
                widths: &app.column_widths,
                volume: app.volume,
                shuffle: app.transport.shuffle,
                repeat: app.transport.repeat,
                continue_mode: app.transport.continue_mode,
                source_mode: app.source_mode,
                yt_browser: &app.yt_browser,
            });
            let _ = state::save_playlists(&app.playlists);
            let _ = state::save_command_history(&app.command_history);
        }
        Cmd::Sync => {
            let cfg = cli::ensure_config()?;
            let script =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/standardize.sh");
            // Fall back to a sibling-of-binary location if running installed.
            let script = if script.exists() {
                script
            } else {
                std::env::current_exe()?
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("scripts/standardize.sh")
            };
            let status = std::process::Command::new(&script)
                .args([
                    "--source",
                    &cfg.source_dir.display().to_string(),
                    "--out",
                    &cfg.filtered_dir.display().to_string(),
                ])
                .status()?;
            if !status.success() {
                anyhow::bail!("standardize.sh failed");
            }
            let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
            println!("synced: {} tracks", cat.tracks.len());
        }
        Cmd::Index => {
            let cfg = cli::ensure_config()?;
            let cat = jukebox::catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            jukebox::search::build_index(&cat, &cfg.filtered_dir.join("search-index"))?;
            println!("indexed {} tracks", cat.tracks.len());
        }
        Cmd::Search { query } => {
            let cfg = cli::ensure_config()?;
            let s = jukebox::search::Searcher::open(&cfg.filtered_dir.join("search-index"))?;
            let q = query.join(" ");
            let cat = jukebox::catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            for hit in s.search(&q, 25)? {
                if let Some(t) = cat.tracks.iter().find(|t| t.id == hit.track_id) {
                    println!(
                        "{:>3.0}%  {} — {} [{}]",
                        // BM25 is unbounded; cap the displayed relevance at 100%
                        // so a strong match reads as 100% rather than e.g. 200%.
                        (hit.score * 100.0).clamp(0.0, 100.0),
                        jukebox::sanitize_for_terminal(&t.title),
                        jukebox::sanitize_for_terminal(&t.primary_artist),
                        jukebox::sanitize_for_terminal(&t.quality_label())
                    );
                }
            }
        }
    }
    Ok(())
}
