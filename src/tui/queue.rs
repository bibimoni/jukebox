/// A play queue with deterministic (seeded) Fisher–Yates shuffle.
#[derive(Default, Clone)]
pub struct Queue {
    items: Vec<String>,
    order: Vec<usize>,   // permutation; identity until shuffled
    order_cursor: usize,
}

impl Queue {
    pub fn new() -> Self { Self::default() }

    pub fn enqueue(&mut self, id: String) {
        let idx = self.items.len();
        self.items.push(id);
        self.order.push(idx);
    }

    pub fn items(&self) -> &Vec<String> { &self.items }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }

    /// Fisher–Yates with a linear congruential RNG seeded by `seed`.
    pub fn shuffle(&mut self, seed: u64) {
        let n = self.items.len();
        self.order = (0..n).collect();
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        for i in (1..n).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (state >> 33) as usize % (i + 1);
            self.order.swap(i, j);
        }
        self.order_cursor = 0;
    }

    /// Index into items for the current position.
    pub fn current(&self) -> Option<&String> {
        let &oidx = self.order.get(self.order_cursor)?;
        self.items.get(oidx)
    }
    pub fn current_index(&self) -> Option<usize> { self.order.get(self.order_cursor).copied() }

    pub fn next(&mut self) {
        if self.order.is_empty() { return; }
        self.order_cursor = (self.order_cursor + 1) % self.order.len();
    }
    /// Move the cursor so that `id` becomes the current track. Used by mouse
    /// click-to-play in the Queue pane. No-op if the id isn't queued.
    pub fn jump_to(&mut self, id: &str) {
        if let Some(pos) = self.items.iter().position(|x| x == id) {
            if let Some(cursor) = self.order.iter().position(|&o| o == pos) {
                self.order_cursor = cursor;
            }
        }
    }
    pub fn prev(&mut self) {
        if self.order.is_empty() { return; }
        self.order_cursor = (self.order_cursor + self.order.len() - 1) % self.order.len();
    }

    /// The track that will play after the current one (wraps). `None` if the
    /// queue has fewer than 2 items.
    pub fn peek_next(&self) -> Option<&String> {
        if self.order.len() < 2 { return None; }
        let next_cursor = (self.order_cursor + 1) % self.order.len();
        let &oidx = self.order.get(next_cursor)?;
        self.items.get(oidx)
    }

    /// Remove the currently-playing track from the queue and leave the cursor
    /// pointing at what was the next track (now shifted into the current
    /// slot). Preserves shuffle order. Used by consume mode: a track is
    /// dropped from the queue once it has finished playing.
    pub fn consume_current(&mut self) {
        if self.order.is_empty() { return; }
        let cur_oidx = self.order[self.order_cursor];
        self.items.remove(cur_oidx);
        self.order.remove(self.order_cursor);
        for o in self.order.iter_mut() {
            if *o > cur_oidx { *o -= 1; }
        }
        // If the cursor landed past the (now shorter) order, wrap to 0 — also
        // covers the empty case (len 0, cursor >= 0 always true).
        if self.order_cursor >= self.order.len() {
            self.order_cursor = 0;
        }
    }
    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.items.iter().position(|x| x == id) {
            self.items.remove(pos);
            self.order = (0..self.items.len()).collect();
            if self.order_cursor >= self.order.len() && !self.order.is_empty() {
                self.order_cursor = 0;
            }
        }
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.order_cursor = 0;
    }
}
