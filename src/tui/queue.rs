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
    pub fn prev(&mut self) {
        if self.order.is_empty() { return; }
        self.order_cursor = (self.order_cursor + self.order.len() - 1) % self.order.len();
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
