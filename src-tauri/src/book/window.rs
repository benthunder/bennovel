use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

/// Keeps only the sections around the reading position in memory.
///
/// With `behind = 1, ahead = 2` and the reader on section 5, sections 4..=7 may be
/// cached; everything else is dropped as soon as the position moves.
pub struct WindowCache<T> {
    behind: usize,
    ahead: usize,
    items: BTreeMap<usize, Arc<T>>,
}

impl<T> WindowCache<T> {
    pub fn new(behind: usize, ahead: usize) -> Self {
        Self {
            behind,
            ahead,
            items: BTreeMap::new(),
        }
    }

    /// Indexes that belong in memory while reading `center` of a book with `len` sections.
    pub fn window(&self, center: usize, len: usize) -> RangeInclusive<usize> {
        let last = len.saturating_sub(1);
        let center = center.min(last);
        center.saturating_sub(self.behind)..=(center + self.ahead).min(last)
    }

    /// Order to load neighbours in: forward first, since that is where readers go.
    pub fn prefetch_order(&self, center: usize, len: usize) -> Vec<usize> {
        let w = self.window(center, len);
        let forward = (center + 1..=*w.end()).filter(|i| *i < len);
        let backward = (*w.start()..center).rev();
        forward.chain(backward).collect()
    }

    pub fn get(&self, index: usize) -> Option<Arc<T>> {
        self.items.get(&index).cloned()
    }

    pub fn insert(&mut self, index: usize, item: Arc<T>) {
        self.items.insert(index, item);
    }

    /// Drops every section outside the window around `center`.
    pub fn retain_around(&mut self, center: usize, len: usize) {
        let w = self.window(center, len);
        self.items.retain(|i, _| w.contains(i));
    }

    #[cfg(test)]
    pub fn cached(&self) -> Vec<usize> {
        self.items.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_clamped_to_book() {
        let c = WindowCache::<()>::new(1, 2);
        assert_eq!(c.window(0, 10), 0..=2);
        assert_eq!(c.window(1, 10), 0..=3);
        assert_eq!(c.window(9, 10), 8..=9);
        assert_eq!(c.window(0, 1), 0..=0);
    }

    #[test]
    fn prefetch_goes_forward_first() {
        let c = WindowCache::<()>::new(1, 2);
        assert_eq!(c.prefetch_order(1, 10), vec![2, 3, 0]);
        assert_eq!(c.prefetch_order(9, 10), vec![8]);
    }

    #[test]
    fn moving_evicts_far_sections() {
        let mut c = WindowCache::new(1, 2);
        for i in 0..=3 {
            c.insert(i, Arc::new(i));
        }
        c.retain_around(4, 10);
        assert_eq!(c.cached(), vec![3]);
    }
}
