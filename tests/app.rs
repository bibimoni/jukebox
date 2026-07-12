use jukebox::catalog::Catalog;
use jukebox::player::StubPlayer;
use jukebox::tui::app::{App, View, YtList, YtListKind};
use jukebox::tui::queue::{ContinueMode, RepeatMode, ShuffleMode};

fn cat_album() -> (tempfile::TempDir, Catalog, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let tracks: Vec<_> = (1..=3).map(|n| serde_json::json!({
        "id":format!("t{n}"),"artists":["40mP"],"primary_artist":"40mP","title":format!("Song{n}"),
        "album":"Cosmic","track_number":n,"bit_depth":24,"sample_rate_hz":96000,
        "source_path":format!("lossless/40mP/{n:02}.flac"),"symlinked_into_artists":["40mP"]
    })).collect();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":tracks}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap(), lossless)
}

#[test]
fn play_selected_sets_context_and_starts_playback() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Browse to the album's track column; cursor on track 2.
    app.view = View::Artists;
    app.cursors.artist = 0; // 40mP
    app.cursors.album = 0; // Cosmic
    app.cursors.track = 1; // Song2
    app.play_selected();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));
    // context is the album; next → Song3 (t3)
    app.next();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t3"));
    // prev → back to Song2 (consume off, history works)
    app.prev();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t2"));
}

#[test]
fn dead_track_skipped_and_marked() {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("X")).unwrap();
    std::fs::write(lossless.join("X").join("02.flac"), b"x").unwrap(); // only t2 exists
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
          {"id":"dead1","artists":["X"],"primary_artist":"X","title":"Gone","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/01.flac","symlinked_into_artists":["X"]},
          {"id":"alive2","artists":["X"],"primary_artist":"X","title":"Here","bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/X/02.flac","symlinked_into_artists":["X"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Set context to both tracks, start at dead1.
    app.play_in_context_ids(vec!["dead1".into(), "alive2".into()], "dead1");
    assert!(app.dead.contains("dead1"));
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("alive2"));
}

#[test]
fn cycle_shuffle_advances_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["t1".into(), "t2".into(), "t3".into()], "t1");
    app.cycle_shuffle(); // Off -> Smart
    assert_eq!(app.transport.shuffle, ShuffleMode::Smart);
    app.cycle_shuffle(); // Smart -> Random
    assert_eq!(app.transport.shuffle, ShuffleMode::Random);
    app.cycle_shuffle(); // Random -> Off
    assert_eq!(app.transport.shuffle, ShuffleMode::Off);
}

#[test]
fn cycle_repeat_advances_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.cycle_repeat();
    assert_eq!(app.transport.repeat, RepeatMode::All);
    app.cycle_repeat();
    assert_eq!(app.transport.repeat, RepeatMode::One);
    app.cycle_repeat();
    assert_eq!(app.transport.repeat, RepeatMode::Off);
}

#[test]
fn volume_clamps_and_mutes() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.volume = 5;
    app.volume_down();
    assert_eq!(app.volume, 0);
    app.volume_down();
    assert_eq!(app.volume, 0); // clamped
    app.volume = 98;
    app.volume_up();
    assert_eq!(app.volume, 100);
    app.volume_up();
    assert_eq!(app.volume, 100);
    let was = app.volume;
    app.toggle_mute();
    assert!(app.muted);
    app.toggle_mute();
    assert!(!app.muted);
    assert_eq!(app.volume, was);
}

/// Regression for the "Previous does not go back across a context switch" bug.
///
/// `play_selected`/`play_in_context_ids` previously did not push the current
/// playback to `transport.history` before `switch_context`, so after switching
/// context (e.g. playing a search result, then a track from a different
/// context) `prev()` had nothing to pop and just replayed the current track.
/// After the fix, `prev()` pops back to the prior track+context.
#[test]
fn prev_across_context_switch_returns_prior_track() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Play t1 in context X = Search{[t1,t2]}.
    app.play_in_context_ids(vec!["t1".into(), "t2".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    // Switch to t3 in a fresh context Y = Search{[t3]}.
    app.play_in_context_ids(vec!["t3".into()], "t3");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t3"));
    // prev must return to the prior track (t1), not replay t3.
    app.prev();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
}

/// Regression for the "Collaboration albums fragment" bug.
///
/// Albums are grouped by `primary_artist`, so a collaboration album (tracks
/// with different primary_artists but the same album title) appeared under
/// each artist with only that artist's tracks. Browsing it showed/played only
/// a subset, so `>` ran out of tracks and stopped playback. After the fix,
/// `tracks_for_album` returns the FULL album across all primary_artists, and
/// both the rendered track list and the playback context use it.
#[test]
fn collaboration_album_shows_and_plays_all_tracks() {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::create_dir_all(lossless.join("B")).unwrap();
    std::fs::create_dir_all(lossless.join("C")).unwrap();
    for n in 1..=3 {
        let artist = ["A", "B", "C"][n - 1];
        std::fs::write(lossless.join(artist).join(format!("{n:02}.flac")), b"x").unwrap();
    }
    let json = serde_json::json!({
        "version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),
        "tracks":[
          {"id":"t1","artists":["A"],"primary_artist":"A","title":"One","album":"Collab","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac","symlinked_into_artists":["A"]},
          {"id":"t2","artists":["B"],"primary_artist":"B","title":"Two","album":"Collab","track_number":2,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/B/02.flac","symlinked_into_artists":["B"]},
          {"id":"t3","artists":["C"],"primary_artist":"C","title":"Three","album":"Collab","track_number":3,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/C/03.flac","symlinked_into_artists":["C"]},
        ]
    }).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);

    // tracks_for_album returns all 3 ids across all primary_artists, in
    // (disc, track_number) order.
    let ids = app.tracks_for_album("Collab");
    assert_eq!(
        ids,
        vec!["t1".to_string(), "t2".to_string(), "t3".to_string()]
    );

    // Browse A → "Collab" album (under A, the album only has t1 in its
    // track_indices, but the focused-album track list and playback context
    // must be the FULL album).
    app.view = View::Artists;
    app.cursors.artist = 0; // A
    app.cursors.album = 0; // Collab
    app.cursors.track = 0; // t1
    app.play_selected();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    // `>` advances to a 2nd distinct track (previously stopped: only 1 track
    // in the context).
    app.next();
    let after = app
        .now_playing
        .clone()
        .expect("next must not stop playback");
    assert_ne!(after.id(), "t1", "next must advance to a distinct track");
    assert!(["t2", "t3"].contains(&after.id()));
}

#[test]
fn clamp_cursors_rescues_stale_album_cursor() {
    // Simulate a stale album cursor: artist cursor on 40mP (1 album) but
    // cursors.album = 5 (out of bounds). Without clamping, the Tracks column
    // renders empty and play_selected plays nothing ("this artist has no
    // songs"). clamp_cursors must pull it back into a valid range.
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.cursors.artist = 0; // 40mP, 1 album
    app.cursors.album = 5; // stale / out of bounds
    app.cursors.track = 9; // stale / out of bounds
    app.clamp_cursors();
    let n_albums = app.albums_by_artist.get("40mP").map(|v| v.len()).unwrap();
    assert!(
        app.cursors.album < n_albums,
        "album cursor must be in range"
    );
    let n_tracks = app.current_context_ids().len();
    assert!(n_tracks > 0);
    assert!(
        app.cursors.track < n_tracks,
        "track cursor must be in range"
    );
}

#[test]
fn changing_artist_resets_album_and_track_cursors() {
    // Moving the artist cursor (col 0) must reset album+track to 0 so the
    // new artist's first album/track is shown — not a stale index that leaves
    // Tracks empty. Uses a 2-artist catalog so `Down` can actually advance.
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("A")).unwrap();
    std::fs::create_dir_all(lossless.join("B")).unwrap();
    std::fs::write(lossless.join("A").join("01.flac"), b"x").unwrap();
    std::fs::write(lossless.join("B").join("01.flac"), b"x").unwrap();
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":[
      {"id":"a1","artists":["Aaa"],"primary_artist":"Aaa","title":"A1","album":"AlbA","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/A/01.flac","symlinked_into_artists":["Aaa"]},
      {"id":"b1","artists":["Bbb"],"primary_artist":"Bbb","title":"B1","album":"AlbB","track_number":1,"bit_depth":16,"sample_rate_hz":44100,"source_path":"lossless/B/01.flac","symlinked_into_artists":["Bbb"]},
    ]}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    let cat = Catalog::load(&p).unwrap();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    // Park stale album/track cursors.
    app.cursors.album = 4;
    app.cursors.track = 7;
    app.view = View::Artists;
    app.focus_col = 0;
    // `Down` moves artist Aaa → Bbb; set_focused_cursor must reset album+track.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.cursors.artist, 1, "artist cursor should have advanced");
    assert_eq!(
        app.cursors.album, 0,
        "album cursor must reset on artist change"
    );
    assert_eq!(
        app.cursors.track, 0,
        "track cursor must reset on artist change"
    );
}

#[test]
fn play_selected_plays_highlighted_track_not_a_stale_one() {
    // After switching artist (which resets track to 0), play_selected must
    // play the track under the cursor — not a stale track index from before.
    let (_d, cat) = cat_two_albums();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.view = View::Artists;
    app.cursors.artist = 0; // 40mP
    app.cursors.album = 0; // Cosmic
    app.cursors.track = 2; // t3 (the 3rd track)
    app.play_selected();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("t3"),
        "play_selected must play the highlighted track, not a stale one"
    );
}

/// Catalog where artist "40mP" has TWO albums so we can verify NextAlbum
/// auto-continuation: "Cosmic" (t1..t3) and "Solo" (s1).
fn cat_two_albums() -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let lossless = d.path().join("lossless");
    std::fs::create_dir_all(lossless.join("40mP")).unwrap();
    for n in 1..=3 {
        std::fs::write(lossless.join("40mP").join(format!("c{n:02}.flac")), b"x").unwrap();
    }
    std::fs::write(lossless.join("40mP").join("s01.flac"), b"x").unwrap();
    let tracks: Vec<_> = (1..=3).map(|n| serde_json::json!({
        "id":format!("t{n}"),"artists":["40mP"],"primary_artist":"40mP","title":format!("Cosmic{n}"),
        "album":"Cosmic","track_number":n,"bit_depth":24,"sample_rate_hz":96000,
        "source_path":format!("lossless/40mP/c{n:02}.flac"),"symlinked_into_artists":["40mP"]
    })).collect();
    let mut tracks = tracks;
    tracks.push(serde_json::json!({
        "id":"s1","artists":["40mP"],"primary_artist":"40mP","title":"SoloTrack",
        "album":"Solo","track_number":1,"bit_depth":16,"sample_rate_hz":44100,
        "source_path":"lossless/40mP/s01.flac","symlinked_into_artists":["40mP"]
    }));
    let json = serde_json::json!({"version":1,"built_at":"x","source_root":lossless.to_str().unwrap(),"tracks":tracks}).to_string();
    let p = d.path().join("catalog.json");
    std::fs::write(&p, json).unwrap();
    (d, Catalog::load(&p).unwrap())
}

#[test]
fn cycle_continue_advances_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Radio);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);
}

#[test]
fn continue_next_album_auto_advances_to_next_album() {
    let (_d, cat) = cat_two_albums();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.continue_mode = ContinueMode::NextAlbum;
    // Browse 40mP (artist 0) → Cosmic (album 0) → last track (t3, index 2),
    // then play. This builds a real Album context so NextAlbum can continue.
    app.view = View::Artists;
    app.cursors.artist = 0;
    app.cursors.album = 0; // Cosmic (sorted before Solo)
    app.cursors.track = 2; // t3 — the last track of Cosmic
    app.play_selected();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t3"));
    // `>` at the end of Cosmic with continue=NextAlbum must switch to "Solo"
    // and play its first track (s1) — not stop.
    app.next();
    assert_eq!(
        app.now_playing.as_ref().map(|s| s.id()),
        Some("s1"),
        "NextAlbum continue should advance to the next album's first track"
    );
    // prev returns to the prior track (t3) — history pushed across the switch.
    app.prev();
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t3"));
}

#[test]
fn continue_radio_keeps_playing_at_context_end() {
    let (_d, cat) = cat_two_albums();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.transport.continue_mode = ContinueMode::Radio;
    // Play a single-track context (t1) so `>` exhausts it immediately.
    app.play_in_context_ids(vec!["t1".into()], "t1");
    assert_eq!(app.now_playing.as_ref().map(|s| s.id()), Some("t1"));
    // `>` with continue=Radio must NOT stop — it switches to the whole library
    // (a Radio/Search context) and plays some track from it.
    app.next();
    assert!(
        app.now_playing.is_some(),
        "Radio continue must keep playing, not stop"
    );
    let after = app.now_playing.clone().unwrap();
    // The radio context is the whole library (t1,t2,t3,s1); the next track
    // must be one of those and (smart shuffle, no back-to-back same artist)
    // distinct from t1 when possible.
    assert!(["t1", "t2", "t3", "s1"].contains(&after.id()));
}

#[test]
fn cycle_continue_is_mode_aware() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Radio);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);

    app.source_mode = jukebox::mode::SourceMode::Youtube;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::YouTube);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);

    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::NextAlbum);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Radio);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::YouTube);
    app.cycle_continue();
    assert_eq!(app.transport.continue_mode, ContinueMode::Off);
}

#[test]
fn mixed_mode_plays_local_when_track_in_catalog() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Mixed;
    app.play_in_context_ids(vec!["t1".into()], "t1");
    // now_playing is Local (the catalog track), not Remote.
    assert!(matches!(
        app.now_playing,
        Some(jukebox::source::TrackSource::Local { ref track_id }) if track_id == "t1"
    ));
}

#[test]
fn s_instant_random_plays_in_context_local() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.instant_random();
    assert!(
        app.now_playing.is_some(),
        "instant random should play something"
    );
}

#[test] // `s` documents the shift+s Discover keybinding under test
fn s_discover_lists_local_albums_in_local_mode() {
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.source_mode = jukebox::mode::SourceMode::Local;
    app.open_discover();
    match &app.overlay {
        Some(jukebox::tui::app::Overlay::Discover { items, .. }) => {
            assert!(!items.is_empty(), "discover should list albums");
            assert!(items
                .iter()
                .any(|i| matches!(i, jukebox::tui::app::DiscoverItem::Album { .. })));
        }
        _ => panic!("expected Discover"),
    }
}

// --- YouTube view navigation: the shared `cursors.playlist` clamp bug -------
//
// `cursors.playlist` is shared between `View::Playlists` (local playlists)
// and `View::Youtube` (yt_lists). `clamp_cursors` used to clamp it against
// `playlists.len()` unconditionally — so in the YouTube view, with fewer
// local playlists than YouTube lists, every render yanked the cursor back
// down to `playlists.len()-1` and the user could not move between YouTube
// playlists. The clamp must be view-aware.

fn three_yt_lists() -> Vec<YtList> {
    (0..3)
        .map(|i| YtList {
            id: format!("PL{i}"),
            name: format!("List {i}"),
            kind: YtListKind::Account,
            track_ids: Vec::new(),
        })
        .collect()
}

#[test]
fn clamp_cursors_in_youtube_view_uses_yt_lists_len() {
    // 1 local playlist, 3 YouTube lists, YouTube view, cursor on list 2.
    // Before the fix: clamp_cursors clamped against playlists.len()=1, yanking
    // the cursor back to 0. After: it must clamp against yt_lists.len()=3 and
    // leave the cursor at 2.
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists = vec![jukebox::tui::app::Playlist {
        name: "local".into(),
        track_ids: vec![],
    }];
    app.yt_lists = three_yt_lists();
    app.view = View::Youtube;
    app.cursors.playlist = 2;
    app.clamp_cursors();
    assert_eq!(
        app.cursors.playlist, 2,
        "YouTube view must clamp against yt_lists.len(), not playlists.len()"
    );
}

#[test]
fn yt_view_navigation_not_blocked_by_local_playlists() {
    // End-to-end reproduction: 1 local playlist, 3 YouTube lists, YouTube view.
    // Simulate the render loop (handle_key then clamp_cursors, as layout.rs
    // does on every frame). Before the fix, pressing `j` twice left the
    // cursor stuck at 0 (clamped back from 1→0 each frame because
    // playlists.len()=1). After the fix, the cursor must reach 2.
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists = vec![jukebox::tui::app::Playlist {
        name: "local".into(),
        track_ids: vec![],
    }];
    app.yt_lists = three_yt_lists();
    app.view = View::Youtube;
    app.focus_col = 0;
    app.cursors.playlist = 0;

    // Press j (down) — simulating the real loop: handle_key then the render's
    // clamp_cursors. Two presses should reach list index 2.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use jukebox::tui::input::handle_key;
    let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
    for _ in 0..2 {
        handle_key(&mut app, j);
        app.clamp_cursors(); // what layout.rs:53 does every frame
    }
    assert_eq!(
        app.cursors.playlist, 2,
        "must be able to move down to the third YouTube playlist"
    );

    // Press k (up) once — should go back to 1, not get stuck.
    let k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
    handle_key(&mut app, k);
    app.clamp_cursors();
    assert_eq!(app.cursors.playlist, 1, "k must move up one");
}

#[test]
fn clamp_cursors_in_playlists_view_uses_playlists_len() {
    // Symmetric guard: in the Playlists view, the clamp must still use the
    // local playlists list (not yt_lists), so a stale yt_lists-heavy cursor
    // is pulled back into the local playlists' range.
    let (_d, cat, _l) = cat_album();
    let mut app = App::new(cat, Box::new(StubPlayer::default()), None, None);
    app.playlists = vec![jukebox::tui::app::Playlist {
        name: "local".into(),
        track_ids: vec![],
    }];
    app.yt_lists = three_yt_lists();
    app.view = View::Playlists;
    app.cursors.playlist = 2; // valid for yt_lists (3) but out of range for playlists (1)
    app.clamp_cursors();
    assert_eq!(
        app.cursors.playlist, 0,
        "Playlists view must clamp against playlists.len()"
    );
}
