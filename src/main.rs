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
            let cat_path = cfg.filtered_dir.join("catalog.json");
            // A missing/empty catalog means the user hasn't run `jukebox sync`
            // yet (or the last sync indexed 0 tracks). Surface a clear recovery
            // hint and exit gracefully instead of launching the TUI into a
            // mid-playback "file not found" state.
            let cat = match catalog::Catalog::load_for_playback(&cat_path)? {
                Some(c) => c,
                None => {
                    eprintln!(
                        "No playable catalog found at {}.\n\
                         Run `jukebox sync` first to index your library, then `jukebox`.",
                        cat_path.display()
                    );
                    return Ok(());
                }
            };
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

            // Restore the session's track_cache from disk IMMEDIATELY after
            // creating the app, BEFORE the layout/resume-hint block below.
            // The track_cache holds video_id → title/artist mappings from the
            // previous session. Without this, the resume hint shows the raw
            // video_id instead of the real title, and the Queue view shows
            // "Resuming…" for all tracks. The cache must also be re-restored
            // after the browser re-spawn below (which replaces the session
            // with a fresh one — empty cache).
            let cached_tracks = jukebox::yt::cache::load_track_cache();
            if !cached_tracks.is_empty() {
                if let Some(session) = app.yt_session.as_mut() {
                    session.restore_track_cache(cached_tracks.clone());
                }
            }

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
                // RC11-DEF-014: restore the last-focused browse cursors so the
                // user returns to the last-played track instead of track 1.
                // clamp_cursors (called per-frame + on play) keeps them valid
                // if the catalog changed since the save.
                app.cursors.artist = layout.last_cursor_artist;
                app.cursors.album = layout.last_cursor_album;
                app.cursors.track = layout.last_cursor_track;
                app.cursors.playlist = layout.last_cursor_playlist;
                app.player_bar_state.big_pref = layout.player_bar_mode == "big";
                app.player_bar_state.track_layout =
                    jukebox::tui::view::player_bar_big::TrackLayoutMode::parse(
                        &layout.track_layout_mode,
                    );
                app.sidebar_visible = layout.sidebar_visible;
                app.playlist_col = jukebox::tui::app::PlaylistColumnState {
                    width: layout.playlist_col.width,
                    group_by_type: layout.playlist_col.group_by_type,
                    show_counts: layout.playlist_col.show_counts,
                };
                let (tw, _) = crossterm::terminal::size().unwrap_or((0, 0));
                app.playlist_col.clamp_width(tw.max(1));
                // RC11-DEF-014: restore the last-played track + position so a
                // "resume" hint can show and `resume_last()` (R / Enter on the
                // restored cursor) can seek to the saved position. The hint
                // is built from the catalog title + a M:SS position; when the
                // track id is gone (catalog changed) or no track was played
                // yet, no hint is shown.
                app.last_played_track_id = layout.last_played_track_id.clone();
                app.last_played_position = layout.last_played_position;
                app.last_played_context_ids = layout.last_played_context_ids.clone();
                app.last_played_context_tracks = layout.last_played_context_tracks.clone();
                app.last_played_context_key = layout.last_played_context_key.clone();
                // Populate the session's track_cache from the persisted
                // context tracks so the Queue view + now-playing bar show
                // real titles immediately — no need to wait for
                // get_playlist/search to fire. This is the key fix: the
                // metadata travels with the context, not just the video_ids.
                if !layout.last_played_context_tracks.is_empty() {
                    if let Some(session) = app.yt_session.as_mut() {
                        for t in &layout.last_played_context_tracks {
                            if !t.video_id.is_empty() && session.track_for(&t.video_id).is_none() {
                                session.cache_track_pub(&jukebox::yt::proto::RemoteTrackSummary {
                                    video_id: t.video_id.clone(),
                                    title: t.title.clone(),
                                    artist: t.artist.clone(),
                                    album: t.album.clone(),
                                    dur: None,
                                    isrc: None,
                                });
                            }
                        }
                    }
                }
                app.player_bar_state.big_pref =
                    crate::tui::view::player_bar_big::PlayerBarMode::parse(&layout.player_bar_mode)
                        == crate::tui::view::player_bar_big::PlayerBarMode::Big;
                app.player_bar_state.track_layout =
                    crate::tui::view::player_bar_big::TrackLayoutMode::parse(
                        &layout.track_layout_mode,
                    );
                if let Some(id) = &layout.last_played_track_id {
                    // Resolve the title from the local catalog, the session's
                    // restored track_cache, the persisted context tracks.
                    let title = app
                        .track_by_id_fast(id)
                        .map(|t| t.title.clone())
                        .or_else(|| {
                            app.yt_session
                                .as_ref()
                                .and_then(|s| s.track_for(id))
                                .map(|t| t.title.clone())
                        })
                        .or_else(|| {
                            layout
                                .last_played_context_tracks
                                .iter()
                                .find(|t| &t.video_id == id)
                                .map(|t| t.title.clone())
                        })
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| "previous track".to_string());
                    let pos = layout.last_played_position.max(0.0);
                    let m = (pos as u64) / 60;
                    let s = (pos as u64) % 60;
                    app.resume_hint = Some(format!("resume: {title} at {m}:{s:02} · R to resume"));
                    app.pending_resume = Some((id.clone(), pos));
                    // Remember the track id so on_tick can fire a
                    // get_watch_playlist to fetch the radio queue (which
                    // caches track metadata) after the browser re-spawn
                    // completes. The hint will update when the metadata lands.
                    app.pending_metadata_fetch = Some(id.clone());
                    // Re-fetch the source YouTube playlist so ALL context
                    // tracks get real titles (not "Loading…"). The previous
                    // session may have saved context tracks with empty titles
                    // (metadata hadn't landed before exit). get_playlist(key)
                    // re-caches every track's metadata.
                    if let Some(key) = &app.last_played_context_key {
                        app.pending_context_playlist_fetch = Some(key.clone());
                    }
                }
                // Restore the saved browser-auth preference and re-spawn with
                // browser auth. We ALWAYS use spawn_browser when yt_browser is
                // set — the persisted cookies file (yt-cookies.txt) does NOT
                // authenticate for all YouTube endpoints (get_liked_songs and
                // get_library_playlists fail with pasted cookies because
                // YouTube requires the full browser session cookies for those
                // endpoints, not just SAPISID). browser_cookie3 reads the
                // full browser cookie jar (including SID, HSID, SSID that
                // those endpoints need), so browser auth works everywhere.
                // The macOS Keychain prompt is a one-time cost per launch;
                // browser_cookie3 caches the jar for the process lifetime.
                app.yt_browser = layout.yt_browser.clone();
                if !app.yt_browser.is_empty() {
                    // Re-spawn with browser auth (reads the browser cookie jar).
                    if let Ok(s) = jukebox::yt::session::Session::spawn_browser(
                        &app.yt_python,
                        &app.yt_script,
                        app.yt_browser.clone(),
                    ) {
                        app.yt_session = Some(s);
                        // Re-restore the track cache + context tracks into the
                        // new session (spawn_browser replaced the session with
                        // a fresh one — empty track_cache).
                        if !cached_tracks.is_empty() {
                            if let Some(session) = app.yt_session.as_mut() {
                                session.restore_track_cache(cached_tracks.clone());
                            }
                        }
                        if !layout.last_played_context_tracks.is_empty() {
                            if let Some(session) = app.yt_session.as_mut() {
                                for t in &layout.last_played_context_tracks {
                                    if !t.video_id.is_empty()
                                        && session.track_for(&t.video_id).is_none()
                                    {
                                        session.cache_track_pub(
                                            &jukebox::yt::proto::RemoteTrackSummary {
                                                video_id: t.video_id.clone(),
                                                title: t.title.clone(),
                                                artist: t.artist.clone(),
                                                album: t.album.clone(),
                                                dur: None,
                                                isrc: None,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        // Authenticated but NOT synced — the credential hasn't
                        // been verified by a data fetch yet. The probe below (or
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
                let evs: Vec<_> = app
                    .reco_events
                    .recent(app.reco_events.len())
                    .into_iter()
                    .cloned()
                    .collect();
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

            // Track cache was already restored above (before the resume hint).
            // No need to re-load here.

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
                // RC11-DEF-014: persist the last-played track + position +
                // browse cursors so the next launch can offer a resume.
                last_played_track_id: app.last_played_track_id.as_deref(),
                last_played_position: app.last_played_position,
                last_played_context_ids: &app.last_played_context_ids,
                last_played_context_tracks: &app.last_played_context_tracks,
                last_played_context_key: app.last_played_context_key.as_deref(),
                last_cursor_artist: app.cursors.artist,
                last_cursor_album: app.cursors.album,
                last_cursor_track: app.cursors.track,
                last_cursor_playlist: app.cursors.playlist,
                player_bar_mode: if app.player_bar_state.big_pref {
                    "big"
                } else {
                    "mini"
                },
                track_layout_mode: app.player_bar_state.track_layout.as_str(),
                sidebar_visible: app.sidebar_visible,
                playlist_col: &app.playlist_col,
            });
            let _ = state::save_playlists(&app.playlists);
            let _ = state::save_command_history(&app.command_history);
            // Persist the session's track_cache so the next launch can restore
            // track titles/artists for resume (the in-memory cache doesn't
            // survive a restart).
            if let Some(session) = &app.yt_session {
                let tracks = session.all_cached_tracks();
                let _ = jukebox::yt::cache::save_track_cache(&tracks);
            }
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
                anyhow::bail!(
                    "standardize.sh failed — see output above; run `jukebox sync` \
                     again once the source dir has playable .flac files"
                );
            }
            let cat = catalog::Catalog::load(&cfg.filtered_dir.join("catalog.json"))?;
            // Defense in depth: standardize.sh exits non-zero on 0 tracks, but a
            // stale empty catalog (e.g. from a pre-fix sync) must not be reported
            // as "synced: 0 tracks" success.
            cat.require_tracks()?;
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
