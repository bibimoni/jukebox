//! Tests for the Nerd Font icon system.

use jukebox::tui::view::icons::{FontMode, Icon, IconRenderer};

#[test]
fn font_mode_default_is_unicode() {
    assert_eq!(FontMode::default(), FontMode::Unicode);
}

#[test]
fn font_mode_labels() {
    assert_eq!(FontMode::NerdFont.label(), "Nerd Font");
    assert_eq!(FontMode::Unicode.label(), "Unicode");
    assert_eq!(FontMode::Ascii.label(), "ASCII");
}

#[test]
fn icon_playing_glyph_in_all_modes() {
    let nf = IconRenderer::new(FontMode::NerdFont);
    let uni = IconRenderer::new(FontMode::Unicode);
    let asc = IconRenderer::new(FontMode::Ascii);
    assert!(!nf.glyph(Icon::Playing).is_empty());
    assert!(!uni.glyph(Icon::Playing).is_empty());
    assert!(!asc.glyph(Icon::Playing).is_empty());
}

#[test]
fn ascii_glyphs_are_ascii_safe() {
    let renderer = IconRenderer::new(FontMode::Ascii);
    for icon in [
        Icon::Local,
        Icon::Youtube,
        Icon::Hybrid,
        Icon::Playing,
        Icon::Paused,
        Icon::Buffering,
        Icon::Queue,
        Icon::Radio,
        Icon::Autoplay,
        Icon::Lyrics,
        Icon::Like,
        Icon::Hidden,
        Icon::Cached,
        Icon::Offline,
        Icon::Warning,
        Icon::Error,
        Icon::Refresh,
        Icon::Search,
        Icon::Generated,
    ] {
        let glyph = renderer.glyph(icon);
        for c in glyph.chars() {
            assert!(
                c.is_ascii(),
                "ASCII glyph for {:?} contains non-ASCII: {:?}",
                icon,
                glyph
            );
        }
    }
}

#[test]
fn no_essential_meaning_is_icon_only() {
    for icon in [
        Icon::Local,
        Icon::Youtube,
        Icon::Hybrid,
        Icon::Playing,
        Icon::Paused,
        Icon::Buffering,
        Icon::Queue,
        Icon::Radio,
        Icon::Autoplay,
        Icon::Lyrics,
        Icon::Like,
        Icon::Hidden,
        Icon::Cached,
        Icon::Offline,
        Icon::Warning,
        Icon::Error,
        Icon::Refresh,
        Icon::Search,
        Icon::Generated,
    ] {
        assert!(
            !icon.label().is_empty(),
            "icon {:?} has no text label",
            icon
        );
    }
}

#[test]
fn glyph_and_label_includes_both() {
    let renderer = IconRenderer::new(FontMode::Unicode);
    let combined = renderer.glyph_and_label(Icon::Playing);
    assert!(combined.contains("playing"));
}

#[test]
fn icon_renderer_mode_setter() {
    let mut renderer = IconRenderer::new(FontMode::Unicode);
    assert_eq!(renderer.mode(), FontMode::Unicode);
    renderer.set_mode(FontMode::Ascii);
    assert_eq!(renderer.mode(), FontMode::Ascii);
}

#[test]
fn font_mode_auto_detect_returns_valid_mode() {
    let mode = FontMode::auto_detect();
    assert!(matches!(
        mode,
        FontMode::NerdFont | FontMode::Unicode | FontMode::Ascii
    ));
}

#[test]
fn all_icons_have_distinct_labels() {
    let labels: Vec<&str> = [
        Icon::Local,
        Icon::Youtube,
        Icon::Hybrid,
        Icon::Playing,
        Icon::Paused,
        Icon::Buffering,
        Icon::Queue,
        Icon::Radio,
        Icon::Autoplay,
        Icon::Lyrics,
        Icon::Like,
        Icon::Hidden,
        Icon::Cached,
        Icon::Offline,
        Icon::Warning,
        Icon::Error,
        Icon::Refresh,
        Icon::Search,
        Icon::Generated,
    ]
    .iter()
    .map(|i| i.label())
    .collect();
    let unique: std::collections::HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), labels.len(), "duplicate icon labels found");
}

#[test]
fn generated_icon_distinguishable_from_search() {
    let renderer = IconRenderer::new(FontMode::Unicode);
    assert_ne!(
        renderer.glyph(Icon::Generated),
        renderer.glyph(Icon::Search)
    );
    assert_ne!(Icon::Generated.label(), Icon::Search.label());
}
