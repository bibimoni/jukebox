//! Color palette + display-width helpers (rewritten in Task 2).

use ratatui::style::Color;

/// True when NO_COLOR is set (no-color.org). Colors must not be the only signal.
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

/// Semantic color tokens. [`Theme::default`] honors `NO_COLOR`: when set, every
/// field collapses to [`Color::Reset`] so the TUI stays usable in monochrome.
pub struct Theme {
    pub accent: Color, // focus + selection
    pub dim: Color,    // chrome / borders unfocused
    pub text: Color,
    pub muted: Color,
    pub hi_fg: Color, // text on accent background
    pub hires: Color, // Hi-Res quality accent
    pub cd: Color,    // CD-quality accent
}

impl Default for Theme {
    fn default() -> Self {
        if no_color() {
            Theme {
                accent: Color::Reset,
                dim: Color::Reset,
                text: Color::Reset,
                muted: Color::Reset,
                hi_fg: Color::Reset,
                hires: Color::Reset,
                cd: Color::Reset,
            }
        } else {
            Theme {
                accent: Color::Cyan,
                dim: Color::DarkGray,
                text: Color::Reset,
                muted: Color::DarkGray,
                hi_fg: Color::Black,
                hires: Color::Magenta,
                cd: Color::Green,
            }
        }
    }
}

/// Accent color for the quality tag: Magenta for Hi-Res (24-bit or ≥48kHz),
/// Green for CD. Returns [`Color::Reset`] when `NO_COLOR` is set.
pub fn quality_color(bit_depth: u32, sample_rate_hz: u32) -> Color {
    if no_color() {
        return Color::Reset;
    }
    if bit_depth >= 24 || sample_rate_hz >= 48000 {
        Color::Magenta
    } else {
        Color::Green
    }
}

/// Approximate display width: ASCII = 1, CJK + kana + fullwidth = 2. Good
/// enough for terminal alignment of mixed JP/EN titles without pulling in the
/// unicode-width crate.
pub fn disp_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if (0x1100..=0x115F).contains(&cp)                    // Hangul Jamo
            || (0x2E80..=0xA4CF).contains(&cp) && cp != 0x303F // CJK radicals / Yi
            || (0xAC00..=0xD7A3).contains(&cp)                // Hangul syllables
            || (0xF900..=0xFAFF).contains(&cp)                // CJK compat ideographs
            || (0xFE30..=0xFE4F).contains(&cp)                // CJK compat forms
            || (0xFF00..=0xFF60).contains(&cp)                // fullwidth forms
            || (0xFFE0..=0xFFE6).contains(&cp)                // fullwidth signs
            || (0x3000..=0x303F).contains(&cp)                // CJK symbols (incl. ・)
            || (0x3040..=0x30FF).contains(&cp)                // Hiragana + Katakana
            || (0x4E00..=0x9FFF).contains(&cp)
            // CJK Unified Ideographs
            {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Right-pad `left` so that `right` sits flush against the pane's right edge.
/// `width` is the inner pane width (borders already subtracted). CJK/wide
/// characters are counted as 2 columns via [`disp_width`] so alignment holds for
/// Japanese titles (Ado / Aimer / etc.), not just ASCII.
pub fn pad_between(left: &str, right: &str, width: usize) -> String {
    let lw = disp_width(left);
    let rw = disp_width(right);
    let pad = width.saturating_sub(lw + rw);
    format!("{}{}{}", left, " ".repeat(pad), right)
}
