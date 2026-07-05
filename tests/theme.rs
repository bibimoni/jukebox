use jukebox::tui::view::theme::{disp_width, pad_between, quality_color, no_color};

#[test]
fn disp_width_counts_cjk_as_two() {
    assert_eq!(disp_width("abc"), 3);
    assert_eq!(disp_width("あいう"), 6);          // hiragana, 2 each
    assert_eq!(disp_width("Ado"), 3);
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
