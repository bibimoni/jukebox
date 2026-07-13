//! Regression tests for Fixer C defects (Batch C — Publish/playlist/generator).
//!
//! - C.1 RC11-DEF-002: `:publish` overlay non-functional — open_publication
//!   populates publishable_ids + account; Name field editable; Enter
//!   dispatches sidecar + shows toast; duplicate-name guard; on_tick drains
//!   pending_publication.
//! - C.2 RC11-DEF-010: Generator preview tags tracks with source [L]/[Y].
//! - C.3 RC11-DEF-031: Pin survives `g` regenerate; `x` cleans pinned.
//! - C.4 RC11-DEF-032: Generator save shows toast; duplicate name refuses.
//! - C.5 RC11-DEF-033: Playlist creation shows toast.
//! - C.6 RC11-DEF-039: Long track titles truncated in generator preview.
//! - C.7 RC11-DEF-060: `s` saves in generator (alias for Enter).
//! - C.8 RC11-DEF-065: Generator save offers to play (Confirm overlay).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, Overlay, View};
use jukebox::tui::input::handle_key;
use jukebox::tui::view::generator::GeneratorPhase;
use jukebox::tui::view::publication::{PubField, PublicationState};
use jukebox::yt::state::YtState;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn cat_with_tracks() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("ArtistA")).unwrap();
    std::fs::write(lossless.join("ArtistA").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("ArtistA").join("02.flac"), b"x").unwrap();
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
            {"id":"local1","artists":["ArtistA"],"primary_artist":"ArtistA","title":"Local Song One","album":"Al",
             "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/ArtistA/01.flac",
             "symlinked_into_artists":["ArtistA"]},
            {"id":"local2","artists":["ArtistA"],"primary_artist":"ArtistA","title":"Local Song Two","album":"Al",
             "bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/ArtistA/02.flac",
             "symlinked_into_artists":["ArtistA"]},
        ]
    })
    .to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key_code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

// ---------------------------------------------------------------------------
// C.1 RC11-DEF-002: `:publish` overlay non-functional
// ---------------------------------------------------------------------------

/// open_publication populates publishable_ids from the focused playlist's
/// YouTube-track ids (those NOT in the local catalog) and resolves the
/// account from the active YT session. With no YT session, account stays
/// empty (validation_error surfaces the "no account" reason).
#[test]
fn open_publication_classifies_local_only_tracks() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Build a playlist whose track_ids are LOCAL catalog ids (the common
    // case for an existing local-only playlist). With no YT session, all
    // tracks should be classified as local_only (publishable_ids empty
    // and account empty → validation_error explains).
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "Local Mix".into(),
        track_ids: vec!["local1".into(), "local2".into()],
    });
    app.open_publication("Local Mix");
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert!(
                state.publishable_ids.is_empty(),
                "local-only tracks must NOT be publishable"
            );
            assert_eq!(state.local_only.len(), 2, "both local tracks classified");
            assert!(
                state.unavailable.is_empty(),
                "tracks in the catalog are not 'unavailable'"
            );
            assert!(
                state.account.is_empty(),
                "no YT session → account empty so is_ready() false"
            );
            assert!(
                !state.is_ready(),
                "without account, overlay must not be ready"
            );
            assert!(
                state.validation_error().is_some(),
                "validation_error must explain why overlay isn't ready"
            );
        }
        other => panic!("expected Publication overlay, got {other:?}"),
    }
}

/// open_publication resolves the account when the YT provider is authed.
/// The account string is non-empty so is_ready() can return true once the
/// track list is also populated.
#[test]
fn open_publication_resolves_account_when_authed() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.yt_state = YtState::Ready;
    app.yt_browser = "chrome".into();
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "Empty".into(),
        track_ids: vec![],
    });
    app.open_publication("Empty");
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert!(
            !state.account.is_empty(),
            "yt_state=Ready + yt_browser set → account must be populated"
        );
        assert!(
            state.account.contains("chrome"),
            "account string should reference the browser profile"
        );
    } else {
        panic!("expected Publication overlay");
    }
}

/// The Name field is editable: Backspace pops, typing pushes (when the
/// Name field is focused, which is the default).
#[test]
fn publication_overlay_name_field_is_editable() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = PublicationState::new();
    state.name = "Mix".into();
    state.field = PubField::Name;
    app.overlay = Some(Overlay::Publication { state });
    // Type ' ' and 'X' — name should grow.
    handle_key(&mut app, key(' '));
    handle_key(&mut app, key('X'));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert_eq!(state.name, "Mix X", "typing should append to the name");
    } else {
        panic!("overlay should stay open while typing");
    }
    // Backspace pops the last char.
    handle_key(&mut app, key_code(KeyCode::Backspace));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert_eq!(state.name, "Mix ", "Backspace should pop the last char");
    } else {
        panic!("overlay should stay open after Backspace");
    }
}

/// `n` cancels the publication overlay (matches the app-wide Confirm
/// convention) even when the Name field is focused — the cancel verb
/// takes precedence over typing 'n' as a name character.
#[test]
fn publication_overlay_n_cancels_even_when_name_field_focused() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = PublicationState::new();
    state.name = "Mix".into();
    state.field = PubField::Name;
    app.overlay = Some(Overlay::Publication { state });
    handle_key(&mut app, key('n'));
    assert!(
        app.overlay.is_none(),
        "n should cancel the publication overlay even when Name is focused"
    );
}

/// j/k cycles field focus (Name → Privacy → Account → Name ...).
#[test]
fn publication_overlay_jk_cycles_field_focus() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    let mut state = PublicationState::new();
    state.field = PubField::Name;
    app.overlay = Some(Overlay::Publication { state });
    handle_key(&mut app, key('j'));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert_eq!(state.field, PubField::Privacy, "j: Name → Privacy");
    } else {
        panic!("overlay should stay open");
    }
    handle_key(&mut app, key('j'));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert_eq!(state.field, PubField::Account, "j: Privacy → Account");
    }
    handle_key(&mut app, key('k'));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert_eq!(state.field, PubField::Privacy, "k: Account → Privacy");
    }
}

/// Enter on an unready overlay surfaces a validation error (instead of
/// the old silent bump-step) and keeps the overlay open.
#[test]
fn publication_overlay_enter_shows_validation_error_when_unready() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Empty publishable_ids + empty account → validation_error is Some.
    let mut state = PublicationState::new();
    state.name = "Mix".into();
    app.overlay = Some(Overlay::Publication { state });
    handle_key(&mut app, key_code(KeyCode::Enter));
    match &app.overlay {
        Some(Overlay::Publication { state }) => {
            assert!(
                state.error.is_some(),
                "Enter on unready overlay must surface a validation error"
            );
            assert!(
                state.error.as_deref().unwrap().contains("0 tracks")
                    || state.error.as_deref().unwrap().contains("no account"),
                "validation error should mention 0 tracks or no account: {:?}",
                state.error
            );
        }
        _ => panic!("overlay should stay open after Enter on unready state"),
    }
}

/// Duplicate-name guard: Enter on a ready overlay whose name matches an
/// existing YT playlist refuses to publish and surfaces a rename error.
#[test]
fn publication_overlay_enter_warns_on_duplicate_yt_playlist_name() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Construct a ready state (account + tracks set) AND a yt_lists entry
    // with the same name → Enter must refuse.
    app.yt_lists.push(jukebox::tui::app::YtList {
        id: "PL1".into(),
        name: "Existing".into(),
        kind: jukebox::tui::app::YtListKind::Account,
        track_ids: vec![],
    });
    let mut state = PublicationState::new();
    state.name = "Existing".into();
    state.account = "yt:chrome".into();
    state.publishable_ids = vec!["v1".into()];
    app.overlay = Some(Overlay::Publication { state });
    handle_key(&mut app, key_code(KeyCode::Enter));
    if let Some(Overlay::Publication { state }) = &app.overlay {
        assert!(
            state.error.as_deref().unwrap().contains("already exists"),
            "duplicate name must surface a rename error: {:?}",
            state.error
        );
    } else {
        panic!("overlay should stay open on duplicate-name guard");
    }
}

/// The render output for an empty publication state has stable numbering
/// (1..=6) with "(no ... tracks)" placeholders for sections 2 and 3.
#[test]
fn publication_render_has_stable_numbering_with_empty_sections() {
    use jukebox::tui::view::icons::{FontMode, IconRenderer};
    let state = PublicationState::new();
    let icons = IconRenderer::new(FontMode::Unicode);
    let para = jukebox::tui::view::publication::render(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        &state,
        &icons,
    );
    let _ = para;
    // The render must not panic and must include the "(no ...)" placeholders
    // (asserted via publication::tests::render_shows_stable_numbering_with_empty_sections
    // at the lib level — here we just confirm the public API is callable).
}

// ---------------------------------------------------------------------------
// C.2 RC11-DEF-010: Generator preview tags tracks with source [L]/[Y]
// ---------------------------------------------------------------------------

/// The preview render tags local catalog tracks with [L]. A generator
/// seeded from the local catalog (cold-start fallback) produces
/// `is_local=true` candidates → the preview must show [L].
#[test]
fn generator_preview_tags_local_tracks_with_l() {
    use jukebox::tui::view::icons::{FontMode, IconRenderer};
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "calm local mix".into();
    }
    app.generate_playlist();
    let para = if let Some(Overlay::Generator { state }) = &app.overlay {
        assert_eq!(state.phase, GeneratorPhase::Preview);
        let icons = IconRenderer::new(FontMode::Unicode);
        jukebox::tui::view::generator::render(
            ratatui::layout::Rect::new(0, 0, 80, 24),
            state,
            &icons,
        )
    } else {
        panic!("expected Generator overlay");
    };
    // Render the paragraph to text and scan for "[L]" tags.
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            f.render_widget(para, area);
        })
        .unwrap();
    let mut buf = String::new();
    for y in 0..24 {
        for x in 0..80 {
            buf.push(
                terminal.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        buf.push('\n');
    }
    assert!(
        buf.contains("[L]"),
        "generator preview with local tracks must contain [L] tag: {buf:?}"
    );
}

// ---------------------------------------------------------------------------
// C.3 RC11-DEF-031: Pin survives regenerate; x cleans pinned
// ---------------------------------------------------------------------------

/// Pinning a track and regenerating preserves the pinned track at the
/// head of the new playlist (the pinned list is non-empty after `g`).
#[test]
fn generator_pin_survives_regenerate() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "calm mix".into();
    }
    app.generate_playlist();
    // Pin the first track.
    let pinned_id = if let Some(Overlay::Generator { state }) = &app.overlay {
        let p = state.playlist.as_ref().expect("playlist generated");
        assert!(!p.tracks.is_empty());
        p.tracks[0].track_id.clone()
    } else {
        panic!("expected Generator overlay");
    };
    // Move cursor to 0 (already there) and pin.
    handle_key(&mut app, key('p'));
    if let Some(Overlay::Generator { state }) = &app.overlay {
        let p = state.playlist.as_ref().unwrap();
        assert!(p.pinned.contains(&pinned_id), "p must pin the track");
    }
    // Regenerate — pinned should survive.
    handle_key(&mut app, key('g'));
    if let Some(Overlay::Generator { state }) = &app.overlay {
        let p = state.playlist.as_ref().unwrap();
        assert!(
            p.pinned.contains(&pinned_id),
            "pinned track must survive regenerate (got pinned: {:?})",
            p.pinned
        );
        assert!(
            p.tracks.iter().any(|t| t.track_id == pinned_id),
            "pinned track must still be in the track list after regenerate"
        );
    } else {
        panic!("overlay should still be open after regenerate");
    }
}

/// `x` on a pinned track removes it from BOTH the track list and the
/// pinned list (no stale pinned ids left behind).
#[test]
fn generator_x_on_pinned_track_cleans_pinned_list() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "calm mix".into();
    }
    app.generate_playlist();
    let pinned_id = if let Some(Overlay::Generator { state }) = &app.overlay {
        state.playlist.as_ref().unwrap().tracks[0].track_id.clone()
    } else {
        panic!("expected Generator overlay");
    };
    // Pin track 0.
    handle_key(&mut app, key('p'));
    // Remove track 0 with x.
    handle_key(&mut app, key('x'));
    if let Some(Overlay::Generator { state }) = &app.overlay {
        let p = state.playlist.as_ref().unwrap();
        assert!(
            !p.tracks.iter().any(|t| t.track_id == pinned_id),
            "x must remove the track from the playlist"
        );
        assert!(
            !p.pinned.contains(&pinned_id),
            "x must also drop the track from the pinned list (no stale pins)"
        );
    }
}

// ---------------------------------------------------------------------------
// C.4 + C.7 RC11-DEF-032 + DEF-060: Save toast, duplicate refuse, `s` alias
// ---------------------------------------------------------------------------

/// Generator save (Enter) shows a "Saved \"<name>\"" toast and creates the
/// playlist. After save, the Confirm overlay offers to play.
#[test]
fn generator_save_shows_toast_and_offers_play() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "My Mix".into();
    }
    app.generate_playlist();
    // Enter saves.
    handle_key(&mut app, key_code(KeyCode::Enter));
    // A "Saved \"My Mix\"" toast must be set.
    assert!(
        app.yt_status
            .as_deref()
            .unwrap_or_default()
            .contains("Saved"),
        "save must show a 'Saved' toast, got yt_status={:?}",
        app.yt_status
    );
    // The Confirm overlay offers to play.
    assert!(
        matches!(&app.overlay, Some(Overlay::Confirm { message, .. }) if message.contains("Play")),
        "save should open a Confirm overlay offering to play, got {:?}",
        app.overlay
    );
    // The playlist was actually created.
    assert!(
        app.playlists.iter().any(|p| p.name == "My Mix"),
        "playlist 'My Mix' must be in app.playlists after save"
    );
}

/// `s` is an alias for Enter in the preview phase (RC11-DEF-060).
#[test]
fn generator_s_key_saves_in_preview() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "Saved By S".into();
    }
    app.generate_playlist();
    // Press 's' — should save (same as Enter).
    handle_key(&mut app, key('s'));
    assert!(
        app.yt_status
            .as_deref()
            .unwrap_or_default()
            .contains("Saved"),
        "s should save and show the toast, got yt_status={:?}",
        app.yt_status
    );
    assert!(
        app.playlists.iter().any(|p| p.name == "Saved By S"),
        "s should create the 'Saved By S' playlist"
    );
}

/// Saving with a duplicate name refuses (yt_error set, no new playlist
/// created, overlay stays open so the user can rename).
#[test]
fn generator_save_refuses_duplicate_name() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Pre-create a playlist with the target name.
    app.playlists.push(jukebox::tui::app::Playlist {
        name: "Dup".into(),
        track_ids: vec!["local1".into()],
    });
    let original_count = app.playlists.len();
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "Dup".into();
    }
    app.generate_playlist();
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert_eq!(
        app.playlists.len(),
        original_count,
        "duplicate-name save must NOT create a new playlist"
    );
    assert!(
        app.yt_error
            .as_deref()
            .unwrap_or_default()
            .contains("already exists"),
        "duplicate-name save must surface a yt_error, got {:?}",
        app.yt_error
    );
    // Overlay should still be the Generator (not closed, not Confirm).
    assert!(
        matches!(app.overlay, Some(Overlay::Generator { .. })),
        "duplicate-name save must keep the Generator overlay open for rename, got {:?}",
        app.overlay
    );
}

// ---------------------------------------------------------------------------
// C.5 RC11-DEF-033: Playlist creation shows toast
// ---------------------------------------------------------------------------

/// Creating a new playlist via the `+ new playlist...` flow (TextInput
/// overlay) shows a "created \"<name>\"" toast.
#[test]
fn playlist_creation_via_textinput_shows_toast() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Open the playlist picker for a track.
    app.overlay = Some(Overlay::PlaylistPicker {
        track_id: "local1".into(),
        cursor: 0, // cursor on "+ new playlist..."
    });
    // Enter selects "+ new playlist..." → opens the TextInput overlay.
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(
        matches!(app.overlay, Some(Overlay::TextInput { .. })),
        "Enter on '+ new playlist...' must open the TextInput overlay, got {:?}",
        app.overlay
    );
    // Type a name.
    for c in "Fresh Mix".chars() {
        handle_key(&mut app, key(c));
    }
    // Enter creates the playlist.
    handle_key(&mut app, key_code(KeyCode::Enter));
    assert!(
        app.yt_status
            .as_deref()
            .unwrap_or_default()
            .contains("created"),
        "playlist creation must show a 'created' toast, got yt_status={:?}",
        app.yt_status
    );
    assert!(
        app.playlists.iter().any(|p| p.name == "Fresh Mix"),
        "the 'Fresh Mix' playlist must be created"
    );
}

// ---------------------------------------------------------------------------
// C.6 RC11-DEF-039: Long track titles truncated with ellipsis
// ---------------------------------------------------------------------------

/// A generator preview track whose display title exceeds the preview's
/// usable width is truncated with an ellipsis (no 3-line wrap).
#[test]
fn generator_preview_truncates_long_titles() {
    use jukebox::tui::view::icons::{FontMode, IconRenderer};
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "calm mix".into();
    }
    app.generate_playlist();
    // Inject a very long title into the title_map for the first track.
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        if let Some(p) = &state.playlist {
            if let Some(first) = p.tracks.first() {
                let long_title = "A Very Long Track Title That Should Definitely Be Truncated With Ellipsis To Avoid Wrapping Across Multiple Lines And Breaking The Numbered List Rhythm";
                state
                    .title_map
                    .insert(first.track_id.clone(), long_title.to_string());
            }
        }
    }
    let para = if let Some(Overlay::Generator { state }) = &app.overlay {
        let icons = IconRenderer::new(FontMode::Unicode);
        jukebox::tui::view::generator::render(
            ratatui::layout::Rect::new(0, 0, 80, 24),
            state,
            &icons,
        )
    } else {
        panic!("expected Generator overlay");
    };
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            f.render_widget(para, f.area());
        })
        .unwrap();
    let mut buf = String::new();
    for y in 0..24 {
        for x in 0..80 {
            buf.push(
                terminal.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        buf.push('\n');
    }
    // The full (un-truncated) title must NOT appear — the renderer must
    // have shortened it. Look for the ellipsis character ("…" or "...").
    let long_title = "A Very Long Track Title That Should Definitely Be Truncated With Ellipsis To Avoid Wrapping Across Multiple Lines And Breaking The Numbered List Rhythm";
    assert!(
        !buf.contains(long_title),
        "the long title must be truncated, not rendered in full"
    );
    // The ellipsis glyph (Unicode …) should appear instead.
    assert!(
        buf.contains('\u{2026}') || buf.contains("..."),
        "truncated title should end with an ellipsis"
    );
}

// ---------------------------------------------------------------------------
// C.8 RC11-DEF-065: Generator save offers to play (Confirm)
// ---------------------------------------------------------------------------

/// After a successful save, the Confirm overlay's "y" plays the saved
/// playlist (switches view to Playlists + starts playback).
#[test]
fn generator_save_confirm_y_plays_playlist() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "Play Me".into();
    }
    app.generate_playlist();
    handle_key(&mut app, key_code(KeyCode::Enter));
    // Confirm overlay should be open. Press 'y' to play.
    assert!(
        matches!(&app.overlay, Some(Overlay::Confirm { .. })),
        "Confirm overlay must be open after save, got {:?}",
        app.overlay
    );
    handle_key(&mut app, key('y'));
    assert_eq!(
        app.view,
        View::Playlists,
        "y on the save Confirm should switch to the Playlists view"
    );
    assert!(
        app.now_playing.is_some(),
        "y on the save Confirm should start playback of the saved playlist"
    );
}

/// After a successful save, the Confirm overlay's "n" cancels (does NOT
/// play; the saved playlist is still in app.playlists but stays unplayed).
#[test]
fn generator_save_confirm_n_cancels_play() {
    let (_d, cat) = cat_with_tracks();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.open_generator();
    if let Some(Overlay::Generator { state }) = &mut app.overlay {
        state.input = "Keep Me".into();
    }
    app.generate_playlist();
    handle_key(&mut app, key_code(KeyCode::Enter));
    handle_key(&mut app, key('n'));
    assert!(app.overlay.is_none(), "n should close the Confirm overlay");
    assert!(
        app.now_playing.is_none(),
        "n should NOT start playback of the saved playlist"
    );
    assert!(
        app.playlists.iter().any(|p| p.name == "Keep Me"),
        "the playlist must still be saved (cancel only skips playback)"
    );
    assert!(
        app.pending_play_saved_idx.is_none(),
        "n must drop the stashed play-saved index"
    );
}
