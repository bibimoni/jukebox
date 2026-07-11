pub mod audio;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod lyrics;
pub mod mode;
pub mod player;
pub mod prompt;
pub mod search;
pub mod source;
pub mod state;
pub mod translit;
pub mod tui;
pub mod yt;

/// Strip terminal control characters from a string before printing to
/// stdout. Prevents escape-sequence injection from malicious track metadata
/// (e.g. a track title containing `\x1b[2J` to clear the screen, or
/// `\x1b[?1049h` to switch the alternate screen). Replaces C0 control chars
/// (0x00–0x1F except tab/newline/carriage-return) and DEL (0x7f) with `?`.
///
/// Used by the CLI (`jukebox search`) for all catalog-derived output so a
/// crafted title/artist/quality label can't hijack the user's terminal.
pub fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if (cp < 0x20 && c != '\t' && c != '\n' && c != '\r') || cp == 0x7f {
                '?'
            } else {
                c
            }
        })
        .collect()
}
