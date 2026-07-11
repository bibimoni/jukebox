use jukebox::tui::view::theme::{disp_width, no_color, pad_between, quality_color};

#[test]
fn disp_width_counts_cjk_as_two() {
    assert_eq!(disp_width("abc"), 3);
    assert_eq!(disp_width("あいう"), 6); // hiragana, 2 each
    assert_eq!(disp_width("Ado"), 3);
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
