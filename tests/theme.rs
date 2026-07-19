use jukebox::tui::view::theme::{
    disp_width, header_style_for, no_color, pad_between, quality_color, Theme,
};
use ratatui::style::{Color, Modifier};

#[test]
fn disp_width_counts_cjk_as_two() {
    assert_eq!(disp_width("abc"), 3);
    assert_eq!(disp_width("あいう"), 6); // hiragana, 2 each
    assert_eq!(disp_width("Ado"), 3);
}

#[test]
fn disp_width_counts_terminal_emoji_width() {
    assert_eq!(disp_width("🎵"), 2);
    assert_eq!(disp_width("A🎵B"), 4);
}

#[test]
fn display_width_zero_width() {
    // Zero-width space (U+200B) and zero-width non-joiner (U+200C) count as 0.
    assert_eq!(
        disp_width("a\u{200B}b"),
        2,
        "zero-width space should not add width"
    );
    assert_eq!(
        disp_width("a\u{200C}b"),
        2,
        "zero-width non-joiner should not add width"
    );
    // BOM / zero-width no-break space (U+FEFF) counts as 0.
    assert_eq!(disp_width("\u{FEFF}abc"), 3, "BOM should not add width");
    // Combining diacritical marks (U+0300-036F) count as 0.
    // "A\u{0301}" is Á (A with combining acute) — display width 1, not 2.
    assert_eq!(
        disp_width("A\u{0301}"),
        1,
        "combining acute should not add width"
    );
    assert_eq!(
        disp_width("cafe\u{0301}"),
        4,
        "combining acute on e should not add width"
    );
    // Mixed: CJK + combining = 2 (not 3).
    assert_eq!(
        disp_width("\u{3042}\u{0301}"),
        2,
        "hiragana + combining mark = 2"
    );
    // Multiple combining marks on one base char.
    assert_eq!(
        disp_width("a\u{0301}\u{0308}"),
        1,
        "multiple combining marks on one base"
    );
}

#[test]
fn pad_between_right_aligns_right_field() {
    let s = pad_between("A Symphony", "24/96", 20);
    // "A Symphony" is 10 wide, "24/96" is 5 wide → 5 spaces between
    assert_eq!(s, "A Symphony     24/96");
}

#[test]
fn quality_color_codes_hires_differently_from_cd() {
    let cd = quality_color(16, 44100);
    let hires = quality_color(24, 96000);
    assert_ne!(cd, hires);
}

#[test]
fn no_color_reads_env() {
    // NO_COLOR not set in test env by default
    assert_eq!(no_color(), std::env::var_os("NO_COLOR").is_some());
}

#[test]
fn header_style_has_color_and_monochrome_branches() {
    let color = header_style_for(false);
    assert!(color.add_modifier.contains(Modifier::BOLD));
    assert_ne!(color.fg, Some(Color::Reset));
    let mono = header_style_for(true);
    assert!(mono.add_modifier.contains(Modifier::BOLD));
    assert_eq!(mono.fg, Some(Color::Reset));
}

// --- Zebra striping contrast regression tests ---
//
// Bug: color-mode `surface` was `Color::DarkGray` (0x808080). Zebra'd track
// rows render as `dim.bg(surface)` = Gray (0xC0C0C0) on DarkGray (0x808080)
// — only ~2.1:1 luminance contrast, below WCAG AA 4.5:1 for normal text. The
// track list is the primary browsable content, so every other row was hard
// to read. Fix: `surface` is now `Color::Indexed(236)` (0x303030), giving
// ~7.3:1 contrast vs Gray dim text.
//
// We do NOT toggle NO_COLOR/JUKEBOX_HIGH_CONTRAST env vars here (test races),
// so `Theme::default()` returns the color-mode palette when those vars are
// unset in the test environment. The sRGB values are known constants, so we
// assert against them directly.

/// sRGB channel → linear-light, then relative luminance per WCAG 2.1.
fn srgb_luminance(r: u8, g: u8, b: u8) -> f64 {
    let chan = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * chan(r) + 0.7152 * chan(g) + 0.0722 * chan(b)
}

/// WCAG contrast ratio between two relative luminances.
fn contrast(l1: f64, l2: f64) -> f64 {
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn zebra_contrast_color_mode_passes_wcag_aa() {
    let theme = Theme::default();
    assert_eq!(
        theme.dim,
        Color::Gray,
        "color-mode dim must be Gray (0xC0C0C0)"
    );
    assert_eq!(
        theme.surface,
        Color::Indexed(236),
        "color-mode surface must be Indexed(236) (0x303030), not DarkGray"
    );
    // Known sRGB values: Gray = 0xC0C0C0, Indexed(236) = 0x303030.
    let gray_l = srgb_luminance(0xC0, 0xC0, 0xC0);
    let surface_l = srgb_luminance(0x30, 0x30, 0x30);
    let ratio = contrast(gray_l, surface_l);
    // Gray (0xC0C0C0): each channel 192/255 = 0.7529 → linear 0.5276
    //   → L = 0.2126*0.5276 + 0.7152*0.5276 + 0.0722*0.5276 = 0.5276
    // Indexed(236) (0x303030): each channel 48/255 = 0.1882 → linear 0.0290
    //   → L = 0.0290
    // ratio = (0.5276 + 0.05) / (0.0290 + 0.05) = 0.5776 / 0.0790 ≈ 7.31
    // (The exact values are computed above; comment shows the ballpark.)
    assert!(
        ratio >= 4.5,
        "zebra contrast {ratio:.4} < 4.5 (WCAG AA); Gray on surface must be readable"
    );
}

#[test]
fn zebra_color_mode_surface_is_not_darkgray() {
    // Regression guard: the old bug value was Color::DarkGray (0x808080),
    // which gave only ~2.1:1 contrast vs Gray dim text. This test ensures a
    // future change doesn't reintroduce it. (We cannot test the NO_COLOR
    // branch's Black surface here because setting env vars in tests races.)
    let theme = Theme::default();
    assert_ne!(
        theme.surface,
        Color::DarkGray,
        "color-mode surface must not be DarkGray (2.1:1 contrast vs Gray dim)"
    );
}
