//! The transport engine: drives playback over a [`Context`].
//!
//! [`Transport`] owns the play order (a permutation over the context's track
//! ids), a cursor into that order, a history stack (for `prev`), a manual
//! "play next" queue, and the current shuffle/repeat modes. It knows nothing
//! about the audio backend — it just answers "what should play now / next /
//! previously" given a [`ContextResolver`] and a [`Catalog`] (the latter for
//! artist-aware smart shuffle).

use crate::catalog::Catalog;
use crate::tui::context::{Context, ContextResolver};

/// How the play order is derived from the context's track list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShuffleMode {
    Off,
    Smart,
    Random,
}

/// Whether/how playback loops when the context ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

/// What to do when the current context ends with repeat off. `Off` stops
/// playback (the context is exhausted); `NextAlbum` auto-continues to the next
/// album by the same artist; `Radio` auto-continues with the whole library
/// (shuffled) so music never stops. Set by the `c` key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContinueMode {
    Off,
    NextAlbum,
    Radio,
    YouTube,
}

pub struct Transport {
    pub context: Context,
    pub order: Vec<usize>, // permutation over context.track_ids
    pub cursor: usize,     // index into `order`
    pub history: Vec<(String, Context)>, // (track_id, context_at_play_time)
    pub manual_queue: Vec<String>,
    pub shuffle: ShuffleMode,
    pub repeat: RepeatMode,
    pub continue_mode: ContinueMode,
    // seeded RNG state so shuffle is deterministic across runs/tests
    rng_state: u64,
}

impl Transport {
    pub fn new(context: Context) -> Self {
        let n = context.track_ids_placeholder_len();
        Transport {
            context,
            order: (0..n).collect(),
            cursor: 0,
            history: Vec::new(),
            manual_queue: Vec::new(),
            shuffle: ShuffleMode::Off,
            repeat: RepeatMode::Off,
            continue_mode: ContinueMode::Off,
            rng_state: 0x9E3779B97F4A7C15,
        }
    }

    fn ids(&self, r: &dyn ContextResolver) -> Vec<String> {
        self.context.track_ids(r)
    }

    pub fn current(&self, r: &dyn ContextResolver, _cat: &Catalog) -> Option<String> {
        let ids = self.ids(r);
        let &oidx = self.order.get(self.cursor)?;
        ids.get(oidx).cloned()
    }

    pub fn peek_next(&self, r: &dyn ContextResolver, _cat: &Catalog) -> Option<String> {
        let ids = self.ids(r);
        if self.repeat == RepeatMode::One {
            return self.current(r, _cat);
        }
        let next_cursor = self.cursor + 1;
        if next_cursor < self.order.len() {
            ids.get(self.order[next_cursor]).cloned()
        } else if !self.manual_queue.is_empty() {
            self.manual_queue.first().cloned()
        } else if self.repeat == RepeatMode::All && !self.order.is_empty() {
            ids.get(self.order[0]).cloned()
        } else {
            None
        }
    }

    /// Jump to `track_id` within the current context. No-op if not present.
    pub fn play_at(&mut self, r: &dyn ContextResolver, _cat: &Catalog, track_id: &str) {
        let ids = self.ids(r);
        if let Some(pos) = ids.iter().position(|x| x == track_id) {
            if let Some(c) = self.order.iter().position(|&o| o == pos) {
                self.cursor = c;
            }
        }
    }

    pub fn next(&mut self, r: &dyn ContextResolver, cat: &Catalog) -> Option<String> {
        if self.repeat == RepeatMode::One {
            return self.current(r, cat);
        }
        // push current to history before advancing
        if let Some(cur) = self.current(r, cat) {
            self.history.push((cur, self.context.clone()));
        }
        let next_cursor = self.cursor + 1;
        if next_cursor < self.order.len() {
            self.cursor = next_cursor;
            self.current(r, cat)
        } else if !self.manual_queue.is_empty() {
            // Context exhausted and a manual "play next" track is queued: pop
            // and return it without mutating `self.context` (the controller's
            // approved simplification — keeps the original context intact).
            let id = self.manual_queue.remove(0);
            // The last context track played to completion (that's why we're
            // advancing to the manual queue), so its history entry is
            // legitimate and must be retained for `prev()`.
            Some(id)
        } else if self.repeat == RepeatMode::All && !self.order.is_empty() {
            self.cursor = 0;
            self.current(r, cat)
        } else {
            // Nothing to advance to: undo the history push (we never moved).
            self.history.pop();
            None
        }
    }

    pub fn prev(&mut self, r: &dyn ContextResolver, _cat: &Catalog) -> Option<String> {
        if let Some((id, ctx)) = self.history.pop() {
            self.context = ctx;
            // re-derive order/cursor for the restored context
            let ids = self.ids(r);
            if let Some(pos) = ids.iter().position(|x| x == &id) {
                self.order = (0..ids.len()).collect();
                self.cursor = pos;
            }
            Some(id)
        } else {
            // no history → replay current (same id) from the start
            self.current(r, _cat)
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn set_shuffle(&mut self, mode: ShuffleMode, r: &dyn ContextResolver, cat: &Catalog) {
        self.shuffle = mode;
        let n = self.ids(r).len();
        let current_id = self.current(r, cat);
        self.order = match mode {
            ShuffleMode::Off => (0..n).collect(),
            ShuffleMode::Random => self.fisher_yates(n),
            ShuffleMode::Smart => self.smart_shuffle(r, cat),
        };
        // Keep the currently-playing track at cursor position 0 so a shuffle
        // change doesn't yank the user away mid-playback.
        if let Some(id) = current_id {
            let ids = self.ids(r);
            if let Some(pos) = ids.iter().position(|x| x == &id) {
                if let Some(c) = self.order.iter().position(|&o| o == pos) {
                    self.order.swap(0, c);
                }
            }
            self.cursor = 0;
        }
    }

    pub fn reshuffle(&mut self, r: &dyn ContextResolver, cat: &Catalog) {
        // rotate the seed so a re-shuffle yields a different permutation
        self.rng_state = self.rng_state.wrapping_add(0x632BE59BD9B4C0A1);
        let m = self.shuffle;
        self.set_shuffle(m, r, cat);
    }

    pub fn enqueue(&mut self, track_id: String) {
        self.manual_queue.push(track_id);
    }

    pub fn remove_from_queue(&mut self, track_id: &str) {
        self.manual_queue.retain(|x| x != track_id);
    }

    pub fn clear_queue(&mut self) {
        self.manual_queue.clear();
    }

    pub fn switch_context(
        &mut self,
        context: Context,
        start_at: Option<&str>,
        r: &dyn ContextResolver,
        cat: &Catalog,
    ) {
        self.context = context;
        let n = self.ids(r).len();
        self.order = match self.shuffle {
            ShuffleMode::Off => (0..n).collect(),
            ShuffleMode::Random => self.fisher_yates(n),
            ShuffleMode::Smart => self.smart_shuffle(r, cat),
        };
        self.cursor = 0;
        if let Some(id) = start_at {
            self.play_at(r, cat, id);
        }
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn fisher_yates(&mut self, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (self.next_rand() as usize) % (i + 1);
            v.swap(i, j);
        }
        v
    }

    /// Artist-spaced shuffle: arrange track indices so no two adjacent share a
    /// `primary_artist`. Greedy: repeatedly pick a random remaining track whose
    /// artist differs from the last placed; if none qualify (one artist
    /// dominates the context), place any remaining. Falls back to a plain
    /// Fisher–Yates if it stalls repeatedly.
    fn smart_shuffle(&mut self, r: &dyn ContextResolver, cat: &Catalog) -> Vec<usize> {
        let ids = self.ids(r);
        let n = ids.len();
        if n <= 1 {
            return (0..n).collect();
        }
        let artist_of = |i: usize| -> String {
            ids.get(i)
                .and_then(|id| cat.tracks.iter().find(|t| &t.id == id))
                .map(|t| t.primary_artist.clone())
                .unwrap_or_default()
        };
        let mut remaining: Vec<usize> = (0..n).collect();
        let mut out: Vec<usize> = Vec::with_capacity(n);
        let mut last_artist = String::new();
        let mut stall = 0;
        while !remaining.is_empty() {
            // candidates with a different artist than the last placed
            let cands: Vec<usize> = remaining
                .iter()
                .enumerate()
                .filter(|(_, &idx)| artist_of(idx) != last_artist)
                .map(|(ri, _)| ri)
                .collect();
            let pick = if cands.is_empty() {
                stall += 1;
                if stall > n {
                    return self.fisher_yates(n); // give up, pure random
                }
                (self.next_rand() as usize) % remaining.len()
            } else {
                cands[(self.next_rand() as usize) % cands.len()]
            };
            let idx = remaining.remove(pick);
            last_artist = artist_of(idx);
            out.push(idx);
        }
        out
    }
}
