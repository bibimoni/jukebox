use std::path::PathBuf;
use std::sync::Mutex;

use jukebox::catalog::Catalog;
use jukebox::mode::SourceMode;
use jukebox::player::StubPlayer;
use jukebox::state::CachedTrackMeta;
use jukebox::tui::app::App;
use jukebox::tui::pane::model::{ModuleId, PaneId, PaneNode, SplitAxis, UiMode};
use jukebox::tui::pane::registry::registry;
use jukebox::tui::pane::render::render_pane_workspace;
use jukebox::tui::queue::{RepeatMode, ShuffleMode};
use jukebox::yt::state::YtState;
use ratatui::{backend::TestBackend, Terminal};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn resumable_youtube_app() -> App {
    let catalog = Catalog {
        version: 1,
        built_at: "test".into(),
        source_root: PathBuf::new(),
        tracks: Vec::new(),
    };
    let mut app = App::new(catalog, Box::new(StubPlayer::default()), None, None);
    app.resume_hint = Some(
        "resume: ヨルシカ - だから僕は音楽を辞めた (Music Video) - That's Why I Gave Up on Music at 2:54 · R to resume"
            .into(),
    );
    app.last_played_track_id = Some("video-1".into());
    app.last_played_context_tracks = vec![CachedTrackMeta {
        video_id: "video-1".into(),
        title: "ヨルシカ - だから僕は音楽を辞めた (Music Video) - That's Why I Gave Up on Music"
            .into(),
        artist: "ヨルシカ".into(),
        album: None,
    }];
    app.last_played_position = 174.0;
    app.source_mode = SourceMode::Youtube;
    app.yt_state = YtState::Ready;
    app
}

fn playing_context_app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("first.flac"), b"x").unwrap();
    std::fs::write(root.join("second.flac"), b"x").unwrap();
    let catalog = Catalog {
        version: 1,
        built_at: "test".into(),
        source_root: root,
        tracks: vec![
            jukebox::catalog::Track {
                id: "first".into(),
                artists: vec!["Ado".into()],
                primary_artist: "Ado".into(),
                title: "First".into(),
                album: Some("Album".into()),
                track_number: Some(1),
                disc_number: None,
                bit_depth: 24,
                sample_rate_hz: 96_000,
                isrc: None,
                source_path: PathBuf::from("music/first.flac"),
                symlinked_into_artists: vec!["Ado".into()],
            },
            jukebox::catalog::Track {
                id: "second".into(),
                artists: vec!["Ado".into()],
                primary_artist: "Ado".into(),
                title: "Second".into(),
                album: Some("Album".into()),
                track_number: Some(2),
                disc_number: None,
                bit_depth: 24,
                sample_rate_hz: 96_000,
                isrc: None,
                source_path: PathBuf::from("music/second.flac"),
                symlinked_into_artists: vec!["Ado".into()],
            },
        ],
    };
    std::mem::forget(dir);
    let mut app = App::new(catalog, Box::new(StubPlayer::default()), None, None);
    app.play_in_context_ids(vec!["first".into(), "second".into()], "first");
    app
}

fn render_now_playing_module(app: &mut App, width: u16, height: u16) -> String {
    let _guard = ENV_LOCK.lock().unwrap();
    render_now_playing_module_unlocked(app, width, height)
}

fn render_now_playing_module_unlocked(app: &mut App, width: u16, height: u16) -> String {
    render_now_playing_module_with_focus_unlocked(app, width, height, false)
}

fn render_now_playing_module_with_focus_unlocked(
    app: &mut App,
    width: u16,
    height: u16,
    focused: bool,
) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let module = registry().get(ModuleId::NowPlaying).unwrap();
    terminal
        .draw(|frame| module.render_with_focus(frame, frame.area(), app, focused))
        .unwrap();

    let mut output = String::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(terminal.backend().buffer()[(x, y)].symbol());
        }
        output.push_str(line.trim_end());
        output.push('\n');
    }
    output
}

fn render_workspace(app: &mut App, width: u16, height: u16) -> String {
    let _guard = ENV_LOCK.lock().unwrap();
    render_workspace_unlocked(app, width, height)
}

fn render_workspace_unlocked(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_pane_workspace(frame, frame.area(), app))
        .unwrap();

    let mut output = String::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(terminal.backend().buffer()[(x, y)].symbol());
        }
        output.push_str(line.trim_end());
        output.push('\n');
    }
    output
}

#[test]
fn now_playing_pane_uses_responsive_deck_for_resumable_youtube_track() {
    let mut app = resumable_youtube_app();
    let output = render_now_playing_module(&mut app, 160, 30);

    assert!(output.contains("NOW PLAYING"), "{output}");
    assert!(output.contains("STOPPED"), "{output}");
    assert!(output.contains("[Space] Resume from 2:54"), "{output}");
    assert!(output.contains("Up Next"), "{output}");
    assert!(output.contains("YOUTUBE"), "{output}");
    let secondary_line = output
        .lines()
        .find(|line| line.contains("That's Why I Gave Up on Music"))
        .unwrap_or_default();
    assert!(
        !secondary_line.contains("Music Video"),
        "translated title must render separately from the source title:\n{output}"
    );
    assert!(
        output.contains("YOUTUBE · VIDEO"),
        "known media type must be secondary metadata:\n{output}"
    );

    for legacy in [
        "resume:",
        "--bit / -- kHz",
        "SHUF ",
        "RPT ",
        "CONT ",
        "PREF ",
        "Next: -",
        "[ok] YT connected",
    ] {
        assert!(
            !output.contains(legacy),
            "legacy pane text {legacy:?} must not render:\n{output}"
        );
    }
}

#[test]
fn focused_now_playing_pane_owns_one_notched_border() {
    let mut app = resumable_youtube_app();
    app.pane_workspace.root = PaneNode::Split {
        axis: SplitAxis::Vertical,
        ratio: 0.8,
        first: Box::new(PaneNode::Leaf {
            id: PaneId(0),
            module: ModuleId::NowPlaying,
        }),
        second: Box::new(PaneNode::Leaf {
            id: PaneId(1),
            module: ModuleId::Placeholder,
        }),
    };
    app.pane_workspace.focused_pane = PaneId(0);
    app.pane_workspace.mode = UiMode::Normal;

    let output = render_workspace(&mut app, 200, 40);
    let uppercase = output.to_uppercase();
    assert_eq!(
        uppercase.matches("NOW PLAYING").count(),
        1,
        "Now Playing must not have nested titles:\n{output}"
    );
    assert!(
        output.contains("▶ NOW PLAYING · FOCUSED"),
        "focused deck title must carry textual focus cues:\n{output}"
    );
    assert!(
        !output.contains('┏'),
        "Now Playing must use a single-line rounded border:\n{output}"
    );
    assert!(
        output.contains("┬─ Up Next") && !output.contains("╭─ Up Next"),
        "Up Next must share the outer deck border instead of nesting a panel:\n{output}"
    );
}

#[test]
fn compact_and_minimal_panes_do_not_restore_the_raw_source_title() {
    for (width, height) in [(70, 20), (45, 15)] {
        let mut app = resumable_youtube_app();
        let output = render_now_playing_module(&mut app, width, height);
        assert!(
            !output.contains("Music Video"),
            "{width}x{height} must use normalized structured metadata:\n{output}"
        );
        assert!(output.contains("Resume"), "{output}");
        assert!(!output.contains("resume:"), "{output}");
    }
}

#[test]
fn compact_pane_uses_compact_mode_words() {
    let mut app = resumable_youtube_app();
    app.transport.shuffle = ShuffleMode::Random;
    app.transport.repeat = RepeatMode::One;
    let output = render_now_playing_module(&mut app, 70, 20);
    assert!(
        output.contains("Random · Repeat One · Continue Off"),
        "{output}"
    );
    assert!(!output.contains("Shuffle:"), "{output}");
}

#[test]
fn pane_breakpoints_use_renderable_deck_heights_and_cap_empty_space() {
    let cases = [
        (120, 11, "Up Next"),
        (80, 12, "Up next:"),
        (60, 7, "Continue Off"),
        (45, 3, "Resume"),
    ];
    for (width, height, expected) in cases {
        let mut app = resumable_youtube_app();
        let output = render_now_playing_module(&mut app, width, height);
        assert!(
            output.contains(expected),
            "{width}x{height} must render its pane breakpoint ({expected}):\n{output}"
        );
    }

    let mut app = resumable_youtube_app();
    let output = render_now_playing_module(&mut app, 160, 30);
    assert!(
        output.lines().skip(11).all(|line| line.trim().is_empty()),
        "wide pane chrome must stop after its content instead of enclosing empty rows:\n{output}"
    );
}

#[test]
fn resolving_pane_does_not_advertise_resume() {
    for (width, height) in [(100, 12), (70, 20), (45, 3)] {
        let mut app = resumable_youtube_app();
        app.pending_play = Some("video-2".into());
        let output = render_now_playing_module(&mut app, width, height);
        if width >= 60 {
            assert!(output.contains("RESOLVING"), "{output}");
        }
        assert!(output.contains("Finding the stream"), "{output}");
        assert!(!output.contains("[Space] Resume"), "{output}");
    }
}

#[test]
fn pending_remote_switch_takes_state_priority_over_old_playing_track() {
    let mut app = playing_context_app();
    app.pending_play = Some("remote-video".into());
    let output = render_now_playing_module(&mut app, 100, 12);
    assert!(output.contains("RESOLVING"), "{output}");
    assert!(!output.contains("▶ PLAYING"), "{output}");
}

#[test]
fn actual_pane_renderer_is_ascii_clean() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("JUKEBOX_FONT_MODE", "ascii");
    let catalog = Catalog {
        version: 1,
        built_at: "test".into(),
        source_root: PathBuf::new(),
        tracks: Vec::new(),
    };
    let mut app = App::new(catalog, Box::new(StubPlayer::default()), None, None);
    app.source_mode = SourceMode::Youtube;
    app.yt_state = YtState::Ready;
    let mut outputs = Vec::new();
    for (width, height) in [(120, 11), (100, 12), (70, 20), (45, 3)] {
        outputs.push(render_now_playing_module_with_focus_unlocked(
            &mut app, width, height, true,
        ));
    }
    app.pending_play = Some("video-2".into());
    outputs.push(render_now_playing_module_with_focus_unlocked(
        &mut app, 70, 20, true,
    ));
    std::env::remove_var("JUKEBOX_FONT_MODE");

    for output in outputs {
        assert!(output.is_ascii(), "ASCII pane leaked Unicode:\n{output}");
    }
}

#[test]
fn wide_up_next_uses_context_when_manual_queue_is_empty() {
    let mut app = playing_context_app();
    let output = render_now_playing_module(&mut app, 120, 11);
    assert!(output.contains("Second"), "{output}");
    assert!(!output.contains("Queue is empty"), "{output}");
}

#[test]
fn pane_workspace_routes_responsive_now_playing_matrix() {
    for (width, height, expected) in [
        (160, 30, "Up Next"),
        (120, 11, "Up Next"),
        (100, 12, "Up next:"),
        (80, 12, "Up next:"),
        (70, 20, "Continue Off"),
        (60, 20, "Continue Off"),
        (45, 15, "Resume"),
    ] {
        let mut app = resumable_youtube_app();
        app.pane_workspace.root = PaneNode::Leaf {
            id: PaneId(0),
            module: ModuleId::NowPlaying,
        };
        app.pane_workspace.focused_pane = PaneId(0);
        app.pane_workspace.mode = UiMode::Normal;
        let output = render_workspace(&mut app, width, height);
        assert!(
            output.contains(expected),
            "workspace {width}x{height} missing {expected}:\n{output}"
        );
        assert!(!output.contains("resume:"), "{output}");
        assert!(!output.contains("--bit"), "{output}");
    }
}
