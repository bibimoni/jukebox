//! Bounded ring buffer of diagnostic messages (provider errors, respawn
//! notices, sidecar failures) surfaced to the user via the diagnostics
//! overlay. The buffer is bounded so a long-running session with repeated
//! errors doesn't grow unbounded.
//!
//! Slice 7 (feedback / logging / diagnostics). The App owns a single
//! [`Diagnostics`] and pushes provider errors into it from `on_tick` when
//! `yt_error` changes; [`crate::tui::view::diagnostics`] renders the buffer
//! as a scrollable overlay.

use std::collections::VecDeque;

/// Hard cap on the number of retained messages. The oldest entry is evicted
/// once the cap is reached, so the buffer stays O(1) space overhead per
/// message after steady state.
const MAX_MESSAGES: usize = 100;

/// A bounded ring buffer of recent diagnostic messages. Stored as a
/// [`VecDeque`] so push + evict-oldest are both O(1); the buffer is kept
/// contiguous inside [`Diagnostics::push`] so [`Diagnostics::messages`] can
/// hand out a single `&[String]` view without mutation or allocation.
#[derive(Debug, Default)]
pub struct Diagnostics {
    messages: VecDeque<String>,
}

impl Diagnostics {
    /// Construct an empty buffer.
    pub fn new() -> Self {
        Self {
            messages: VecDeque::with_capacity(MAX_MESSAGES),
        }
    }

    /// Append `msg`, evicting the oldest entry when the buffer is full. Keeps
    /// the underlying ring buffer contiguous (see [`Self::messages`]).
    pub fn push(&mut self, msg: String) {
        if self.messages.len() >= MAX_MESSAGES {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
        // Re-pack the ring so `messages()` (which only has `&self`) can return
        // a single contiguous slice. Without this the deque could be split
        // across the ring boundary after a pop_front + push_back wrap.
        self.messages.make_contiguous();
    }

    /// A contiguous slice view of the buffered messages, oldest first. The
    /// buffer is always contiguous after a `push` (and after `Default::default`
    /// / `new`, which are empty), so this is a single-slice view with no
    /// allocation.
    pub fn messages(&self) -> &[String] {
        // `as_slices()` returns `(front, back)`; the deque is contiguous, so
        // `back` is empty and `front` is the full view.
        self.messages.as_slices().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_read_back() {
        let mut d = Diagnostics::new();
        assert!(d.messages().is_empty());
        d.push("first".into());
        d.push("second".into());
        assert_eq!(d.messages(), &["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn evicts_oldest_when_full() {
        let mut d = Diagnostics::new();
        for i in 0..(MAX_MESSAGES + 5) {
            d.push(format!("m{i}"));
        }
        assert_eq!(d.messages().len(), MAX_MESSAGES);
        // Oldest 5 evicted; first remaining = m5.
        assert_eq!(d.messages().first().map(|s| s.as_str()), Some("m5"));
        assert_eq!(d.messages().last().map(|s| s.as_str()), Some("m104"));
    }

    #[test]
    fn default_is_empty() {
        let d = Diagnostics::default();
        assert!(d.messages().is_empty());
    }
}
