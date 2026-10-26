//! Immutable fanout-bounded B+ pages for integer equality lookups.
//! This is a **memory** query index built lazily per snapshot, not a claim
//! that the NQDB0003 generation files contain persisted B+ pages.
//! The fanout is chosen to fit a future fixed 4096-byte index-page encoding.
#[derive(Clone, Debug)]
enum Page {
    Leaf { entries: Vec<(i64, usize)>, next: Option<usize>, max: i64 },
    Branch { bounds: Vec<i64>, children: Vec<usize>, max: i64 },
}
impl Page {
    fn max(&self) -> i64 {
        match self { Self::Leaf { max, .. } | Self::Branch { max, .. } => *max }
    }
}
/// 240 (8-byte key, 8-byte row position) pairs occupy 3,840 bytes.
pub const FANOUT: usize = 240;

#[derive(Clone, Debug, Default)]
pub struct BPlusIndex {
    pages: Vec<Page>,
    root: Option<usize>,
}
impl BPlusIndex {
    pub fn build<I: IntoIterator<Item=(i64, usize)>>(entries: I) -> Self {
        let mut sorted: Vec<_> = entries.into_iter().collect();
        sorted.sort_unstable();
        if sorted.is_empty() { return Self::default(); }
        let mut pages: Vec<Page> = Vec::new();
        let mut level: Vec<usize> = Vec::new();
        for chunk in sorted.chunks(FANOUT) {
            let id = pages.len();
            if let Some(&previous) = level.last() {
                if let Page::Leaf { next, .. } = &mut pages[previous] {
                    *next = Some(id);
                }
            }
            let max = chunk.last().expect("nonempty chunk").0;
            pages.push(Page::Leaf { entries: chunk.to_vec(), next: None, max });
            level.push(id);
        }
        while level.len() > 1 {
            let mut parents = Vec::new();
            for chunk in level.chunks(FANOUT) {
                let bounds: Vec<i64> = chunk.iter().map(|&id| pages[id].max()).collect();
                let max = *bounds.last().expect("nonempty level");
                let id = pages.len();
                pages.push(Page::Branch { bounds, children: chunk.to_vec(), max });
                parents.push(id);
            }
            level = parents;
        }
        Self { pages, root: level.first().copied() }
    }

    /// Return matching insertion-order row positions. Full query predicates
    /// must still be checked by the relational executor.
    pub fn seek_equal(&self, key: i64) -> Vec<usize> {
        let Some(mut id) = self.root else { return Vec::new(); };
        loop {
            match &self.pages[id] {
                Page::Branch { bounds, children, .. } => {
                    let at = bounds.partition_point(|&max| max < key);
                    let Some(&child) = children.get(at) else { return Vec::new(); };
                    id = child;
                }
                Page::Leaf { .. } => break,
            }
        }
        let mut rows = Vec::new();
        loop {
            let Page::Leaf { entries, next, .. } = &self.pages[id] else { unreachable!() };
            let at = entries.partition_point(|&(value, _)| value < key);
            for &(value, position) in &entries[at..] {
                if value > key { rows.sort_unstable(); return rows; }
                rows.push(position);
            }
            let Some(next_id) = *next else { break; };
            if self.pages[next_id].max() < key {
                id = next_id;
                continue;
            }
            // Duplicate keys can span leaf boundaries.
            let Page::Leaf { entries, .. } = &self.pages[next_id] else { unreachable!() };
            if entries.first().map(|(value, _)| *value > key).unwrap_or(true) { break; }
            id = next_id;
        }
        rows.sort_unstable();
        rows
    }
    pub fn page_count(&self) -> usize { self.pages.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fanout_crossings_and_duplicate_keys_have_exact_positions() {
        let mut pairs = Vec::new();
        for i in 0..1600usize { pairs.push(((i / 3) as i64 - 100, i)); }
        let index = BPlusIndex::build(pairs);
        assert!(index.page_count() > 1);
        assert_eq!(index.seek_equal(0), vec![300, 301, 302]);
        assert_eq!(index.seek_equal(-100), vec![0, 1, 2]);
        assert_eq!(index.seek_equal(9000), Vec::<usize>::new());
        assert_eq!(index.seek_equal(-101), Vec::<usize>::new());
        let crossing = BPlusIndex::build((0..600).map(|i| (7, i)));
        assert_eq!(crossing.seek_equal(7), (0..600).collect::<Vec<_>>());
    }
    #[test]
    fn empty_and_single_page_lookup() {
        assert!(BPlusIndex::default().seek_equal(1).is_empty());
        let tree = BPlusIndex::build([(4, 2), (1, 3), (4, 0)]);
        assert_eq!(tree.seek_equal(4), vec![0, 2]);
        assert_eq!(tree.page_count(), 1);
    }
}
