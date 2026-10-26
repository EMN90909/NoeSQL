//! NQDB0003 copy-on-write, stable-identity disk B+ tree.
//!
//! Each individual insertion/removal descends one root-to-leaf path (O(log n)
//! page reads) and writes its replacement path plus at most one sibling per
//! level (O(log n) page writes). Leaf identity is (indexed value, PRIMARY KEY),
//! not mutable row-vector offset; deletes never renumber surviving entries.
//! Old NQBI0001/ordinal trees are upgraded once at the manifest boundary.
//!
//! Index mutations have logarithmic page complexity. The surrounding Rust
//! relational executor still uses fully materialized tables and the v3 pager
//! still rewrites a complete relational data snapshot per transaction.
use crate::v3_bplus_disk::{publish_blob, read_blob, NODE_MAGIC_V2};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;
pub type Pair = (i64, i64);
pub type Hash = [u8; 32];
const PAGE_BYTES: usize = 4096;
const LEAF_CAP: usize = 240;   // header 64 + 240 * 16 = 3904
const BRANCH_CAP: usize = 60;  // header 64 + 60 * 64 = 3904
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub min: Pair,
    pub max: Pair,
    pub hash: Hash,
}
#[derive(Clone, Debug)]
enum Node {
    Leaf(Vec<Pair>),
    Branch(Vec<Child>),
}
fn ordered(entries: &[Pair]) -> bool {
    entries.windows(2).all(|w| w[0] < w[1])
}
fn ordered_refs(entries: &[Child]) -> bool {
    entries.windows(2).all(|w| w[0].max < w[1].min) &&
    entries.iter().all(|c| c.min <= c.max)
}
fn read_node(dir: &Path, hash: &Hash) -> Result<(Node, Child)> {
    let page = read_blob(dir, hash)?;
    if &page[..8] != NODE_MAGIC_V2 ||
        page[9..12].iter().any(|x| *x != 0) ||
        page[16..64].iter().any(|x| *x != 0) {
        return Err("invalid stable-key B+ page header".into());
    }
    let n = u32::from_le_bytes(page[12..16].try_into().unwrap()) as usize;
    match page[8] {
        1 => {
            if n == 0 || n > LEAF_CAP { return Err("bad B+ leaf occupancy".into()); }
            let mut pairs = Vec::with_capacity(n);
            for i in 0..n {
                let at = 64 + i*16;
                pairs.push((
                    i64::from_le_bytes(page[at..at+8].try_into().unwrap()),
                    i64::from_le_bytes(page[at+8..at+16].try_into().unwrap()),
                ));
            }
            if !ordered(&pairs) || page[64+n*16..].iter().any(|v| *v != 0) {
                return Err("unsorted or noncanonical B+ leaf".into());
            }
            let child = Child { min: pairs[0], max: *pairs.last().unwrap(), hash: *hash };
            Ok((Node::Leaf(pairs), child))
        }
        2 => {
            if n == 0 || n > BRANCH_CAP { return Err("bad B+ internal occupancy".into()); }
            let mut children = Vec::with_capacity(n);
            for i in 0..n {
                let at = 64+i*64;
                let mut hash = [0;32];
                hash.copy_from_slice(&page[at+32..at+64]);
                children.push(Child {
                    min: (
                        i64::from_le_bytes(page[at..at+8].try_into().unwrap()),
                        i64::from_le_bytes(page[at+8..at+16].try_into().unwrap())),
                    max: (
                        i64::from_le_bytes(page[at+16..at+24].try_into().unwrap()),
                        i64::from_le_bytes(page[at+24..at+32].try_into().unwrap())),
                    hash,
                });
            }
            if !ordered_refs(&children) || page[64+n*64..].iter().any(|v| *v != 0) {
                return Err("unsorted or noncanonical B+ internal page".into());
            }
            Ok((Node::Branch(children.clone()), Child {
                min: children[0].min, max: children.last().unwrap().max, hash: *hash
            }))
        }
        _ => Err("unrecognized stable-key B+ node type".into()),
    }
}
fn persist(dir: &Path, node: Node) -> Result<Child> {
    let mut page = [0u8; PAGE_BYTES];
    page[..8].copy_from_slice(NODE_MAGIC_V2);
    match node {
        Node::Leaf(entries) => {
            if entries.is_empty() || entries.len() > LEAF_CAP || !ordered(&entries) {
                return Err("cannot persist invalid B+ leaf".into());
            }
            page[8] = 1;
            page[12..16].copy_from_slice(&(entries.len() as u32).to_le_bytes());
            for (i, (key, id)) in entries.iter().enumerate() {
                let at = 64 + i*16;
                page[at..at+8].copy_from_slice(&key.to_le_bytes());
                page[at+8..at+16].copy_from_slice(&id.to_le_bytes());
            }
            Ok(Child { min: entries[0], max: *entries.last().unwrap(),
                hash: publish_blob(dir, &page)? })
        },
        Node::Branch(children) => {
            if children.is_empty() || children.len() > BRANCH_CAP || !ordered_refs(&children) {
                return Err("cannot persist invalid B+ branch".into());
            }
            page[8] = 2;
            page[12..16].copy_from_slice(&(children.len() as u32).to_le_bytes());
            for (i, child) in children.iter().enumerate() {
                let at = 64 + i*64;
                page[at..at+8].copy_from_slice(&child.min.0.to_le_bytes());
                page[at+8..at+16].copy_from_slice(&child.min.1.to_le_bytes());
                page[at+16..at+24].copy_from_slice(&child.max.0.to_le_bytes());
                page[at+24..at+32].copy_from_slice(&child.max.1.to_le_bytes());
                page[at+32..at+64].copy_from_slice(&child.hash);
            }
            Ok(Child { min: children[0].min, max: children.last().unwrap().max,
                hash: publish_blob(dir, &page)? })
        },
    }
}
fn balanced_chunks(total: usize, capacity: usize) -> Vec<usize> {
    if total == 0 { return Vec::new(); }
    let groups = total.div_ceil(capacity);
    let base = total/groups;
    let extra = total%groups;
    (0..groups).map(|i| base + if i < extra { 1 } else { 0 }).collect()
}
/// Used only for brand-new indexes and one-time migration from the old
/// ordinal-index format. Modifying an existing v2 index uses insert/remove.
pub fn build(dir: &Path, mut pairs: Vec<Pair>) -> Result<Option<Child>> {
    pairs.sort_unstable();
    if !pairs.windows(2).all(|x| x[0] < x[1]) {
        return Err("duplicate stable B+ index entry".into());
    }
    if pairs.is_empty() { return Ok(None); }
    let mut nodes = Vec::new();
    let mut cursor = 0;
    for n in balanced_chunks(pairs.len(), LEAF_CAP) {
        nodes.push(persist(dir, Node::Leaf(pairs[cursor..cursor+n].to_vec()))?);
        cursor += n;
    }
    let mut depth = 1;
    while nodes.len() > 1 {
        if depth >= MAX_DEPTH { return Err("index exceeds maximum depth".into()); }
        let mut parents = Vec::new();
        let mut cursor = 0;
        for n in balanced_chunks(nodes.len(), BRANCH_CAP) {
            parents.push(persist(dir, Node::Branch(nodes[cursor..cursor+n].to_vec()))?);
            cursor += n;
        }
        nodes = parents;
        depth += 1;
    }
    Ok(nodes.pop())
}
pub fn root(dir: &Path, hash: &Hash) -> Result<Option<Child>> {
    if *hash == [0;32] { Ok(None) }
    else { read_node(dir, hash).map(|(_, r)| Some(r)) }
}
fn insert_rec(dir: &Path, at: &Child, pair: Pair, depth: usize) -> Result<Vec<Child>> {
    if depth > MAX_DEPTH { return Err("B+ insert depth exceeded".into()); }
    match read_node(dir, &at.hash)?.0 {
        Node::Leaf(mut entries) => {
            match entries.binary_search(&pair) {
                Ok(_) => return Err("duplicate stable B+ entry".into()),
                Err(i) => entries.insert(i, pair),
            }
            if entries.len() <= LEAF_CAP { return Ok(vec![persist(dir, Node::Leaf(entries))?]); }
            let rhs = entries.split_off(entries.len()/2);
            Ok(vec![persist(dir, Node::Leaf(entries))?, persist(dir, Node::Leaf(rhs))?])
        },
        Node::Branch(mut children) => {
            let idx = children.partition_point(|c| c.max < pair).min(children.len()-1);
            let replacement = insert_rec(dir, &children[idx], pair, depth+1)?;
            children.splice(idx..idx+1, replacement);
            if children.len() <= BRANCH_CAP {
                return Ok(vec![persist(dir, Node::Branch(children))?]);
            }
            let rhs = children.split_off(children.len()/2);
            Ok(vec![persist(dir, Node::Branch(children))?,
                persist(dir, Node::Branch(rhs))?])
        },
    }
}
pub fn insert(dir: &Path, current: Option<Child>, pair: Pair) -> Result<Option<Child>> {
    let Some(root) = current else {
        return persist(dir, Node::Leaf(vec![pair])).map(Some);
    };
    let mut nodes = insert_rec(dir, &root, pair, 0)?;
    if nodes.len() == 1 { Ok(nodes.pop()) }
    else { Ok(Some(persist(dir, Node::Branch(nodes))?)) }
}
fn is_underfull(dir: &Path, child: &Child) -> Result<bool> {
    let node = read_node(dir, &child.hash)?.0;
    Ok(match node {
        Node::Leaf(xs) => xs.len() < LEAF_CAP/2,
        Node::Branch(xs) => xs.len() < BRANCH_CAP/2,
    })
}
fn redistribute(dir: &Path, a: Child, b: Child) -> Result<Vec<Child>> {
    let left = read_node(dir, &a.hash)?.0;
    let right = read_node(dir, &b.hash)?.0;
    match (left, right) {
        (Node::Leaf(mut l), Node::Leaf(r)) => {
            l.extend(r);
            if !ordered(&l) { return Err("adjacent leaves overlap".into()); }
            if l.len() <= LEAF_CAP { return Ok(vec![persist(dir, Node::Leaf(l))?]); }
            let right = l.split_off(l.len()/2);
            Ok(vec![persist(dir, Node::Leaf(l))?, persist(dir, Node::Leaf(right))?])
        },
        (Node::Branch(mut l), Node::Branch(r)) => {
            l.extend(r);
            if !ordered_refs(&l) { return Err("adjacent branches overlap".into()); }
            if l.len() <= BRANCH_CAP { return Ok(vec![persist(dir, Node::Branch(l))?]); }
            let right = l.split_off(l.len()/2);
            Ok(vec![persist(dir, Node::Branch(l))?, persist(dir, Node::Branch(right))?])
        },
        _ => Err("B+ tree adjacent siblings have incompatible node types".into()),
    }
}
fn delete_rec(dir: &Path, at: &Child, pair: Pair, depth: usize)
    -> Result<(Vec<Child>, bool)> {
    if depth > MAX_DEPTH { return Err("B+ delete depth exceeded".into()); }
    match read_node(dir, &at.hash)?.0 {
        Node::Leaf(mut xs) => {
            let Ok(i) = xs.binary_search(&pair) else { return Ok((vec![at.clone()], false)); };
            xs.remove(i);
            if xs.is_empty() { return Ok((Vec::new(), true)); }
            Ok((vec![persist(dir, Node::Leaf(xs))?], true))
        },
        Node::Branch(mut children) => {
            let idx = children.partition_point(|c| c.max < pair);
            if idx == children.len() || children[idx].min > pair {
                return Ok((vec![at.clone()], false));
            }
            let (replacement, removed) = delete_rec(dir, &children[idx], pair, depth+1)?;
            if !removed { return Ok((vec![at.clone()], false)); }
            let exists = !replacement.is_empty();
            children.splice(idx..idx+1, replacement);
            if children.is_empty() { return Ok((Vec::new(), true)); }
            if exists && children.len() > 1 {
                let index = idx.min(children.len()-1);
                if is_underfull(dir, &children[index])? {
                    let sibling_at = if index > 0 { index-1 } else { index };
                    let two = redistribute(dir, children[sibling_at].clone(),
                        children[sibling_at+1].clone())?;
                    children.splice(sibling_at..sibling_at+2, two);
                }
            }
            // Parent may underflow and will be rebalanced at the next level.
            Ok((vec![persist(dir, Node::Branch(children))?], true))
        },
    }
}
pub fn remove(dir: &Path, current: Option<Child>, pair: Pair) -> Result<Option<Child>> {
    let Some(root) = current else { return Err("missing B+ entry".into()); };
    let (mut nodes, removed) = delete_rec(dir, &root, pair, 0)?;
    if !removed { return Err("missing B+ entry".into()); }
    let Some(mut root) = nodes.pop() else { return Ok(None); };
    // Root shrink: a one-child internal root does not add an extra level.
    loop {
        let Node::Branch(children) = read_node(dir, &root.hash)?.0 else { break; };
        if children.len() != 1 { break; }
        root = children[0].clone();
    }
    Ok(Some(root))
}
fn scan_rec(dir: &Path, hash: &Hash, depth: usize, previous: &mut Option<Pair>,
    out: &mut Vec<Pair>) -> Result<Child> {
    if depth > MAX_DEPTH { return Err("B+ validation depth exceeded".into()); }
    let (node, reference) = read_node(dir, hash)?;
    match node {
        Node::Leaf(xs) => {
            if depth > 0 && xs.len() < LEAF_CAP/2 {
                return Err("underfull nonroot B+ leaf".into());
            }
            for pair in xs {
                if previous.as_ref().is_some_and(|last| *last >= pair) {
                    return Err("B+ global leaf ordering violation".into());
                }
                *previous = Some(pair);
                out.push(pair);
            }
        },
        Node::Branch(children) => {
            if depth > 0 && children.len() < BRANCH_CAP/2 {
                return Err("underfull nonroot B+ branch".into());
            }
            for child in children {
                let actual = scan_rec(dir, &child.hash, depth+1, previous, out)?;
                if actual.min != child.min || actual.max != child.max {
                    return Err("B+ separator differs from child bounds".into());
                }
            }
        }
    }
    Ok(reference)
}
pub fn scan(dir: &Path, root: Option<Child>) -> Result<Vec<Pair>> {
    let mut out = Vec::new();
    if let Some(root) = root {
        let actual = scan_rec(dir, &root.hash, 0, &mut None, &mut out)?;
        if actual.min != root.min || actual.max != root.max {
            return Err("B+ root separator mismatch".into());
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn directory(label: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("noesql-cow-tree-{label}-{}",
            std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn inserts_split_and_deletions_merge_stable_primary_keys() {
        let dir = directory("split-merge");
        let entries: Vec<_> = (0..720).map(|id| (id/7, id)).collect();
        let mut root = build(&dir, entries.clone()).unwrap();
        assert_eq!(scan(&dir, root.clone()).unwrap(), entries);
        root = insert(&dir, root, (777, 100_000)).unwrap();
        let mut expected = entries.clone();
        expected.push((777, 100_000));
        assert_eq!(scan(&dir, root.clone()).unwrap(), expected);
        for pair in entries.iter().step_by(2).copied() {
            root = remove(&dir, root, pair).unwrap();
        }
        expected.retain(|p| p.1%2 == 1 || p.1 == 100_000);
        assert_eq!(scan(&dir, root.clone()).unwrap(), expected);
        assert!(remove(&dir, root.clone(), (99, 999_999)).is_err());
        assert!(insert(&dir, root.clone(), expected[0]).is_err());
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn mixed_inserts_removals_and_reinsertions_match_sorted_reference() {
        use std::collections::BTreeSet;
        let dir = directory("differential");
        let mut reference = BTreeSet::new();
        let mut current = None;
        // Deliberately interleave duplicates in the indexed-value dimension,
        // nonmonotonic stable PKs, leaf boundary crossings and late deletes.
        for id in 0..510i64 {
            let pair = ((id*37)%97, id*11);
            assert!(reference.insert(pair));
            current = insert(&dir, current, pair).unwrap();
        }
        assert_eq!(scan(&dir, current.clone()).unwrap(),
            reference.iter().copied().collect::<Vec<_>>());
        for id in (0..510i64).step_by(7) {
            let pair = ((id*37)%97, id*11);
            assert!(reference.remove(&pair));
            current = remove(&dir, current, pair).unwrap();
        }
        for id in 510..760i64 {
            let pair = ((id*37)%97, id*11);
            assert!(reference.insert(pair));
            current = insert(&dir, current, pair).unwrap();
        }
        assert_eq!(scan(&dir, current).unwrap(),
            reference.into_iter().collect::<Vec<_>>());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn three_level_tree_mutates_only_one_logarithmic_path() {
        let dir = directory("three-level");
        // Above LEAF_CAP * BRANCH_CAP entries requires >1 internal page.
        let pairs: Vec<Pair> = (0..15_000i64).map(|id| (id, id)).collect();
        let root = build(&dir, pairs.clone()).unwrap();
        let initial_files = fs::read_dir(&dir).unwrap().count();
        let after_insert = insert(&dir, root.clone(), (15_001, 15_001)).unwrap();
        let incremental_files = fs::read_dir(&dir).unwrap().count() - initial_files;
        assert!(incremental_files <= 3, "three-level insert touched unrelated pages: {incremental_files}");
        assert_eq!(scan(&dir, root.clone()).unwrap(), pairs);
        assert_eq!(scan(&dir, after_insert.clone()).unwrap().len(), 15_001);
        let after_delete = remove(&dir, after_insert, (15_001, 15_001)).unwrap();
        assert_eq!(scan(&dir, after_delete).unwrap(), pairs);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn deletions_at_leaf_boundaries_redistribute_and_collapse_root() {
        let dir = directory("leaf-boundaries");
        let mut current = build(&dir, (0..241i64).map(|n| (n/3, n)).collect()).unwrap();
        for key in 0..241i64 {
            current = remove(&dir, current, (key/3, key)).unwrap();
            if key%50 == 0 || key == 240 {
                let expected: Vec<Pair> = ((key+1)..241).map(|n| (n/3,n)).collect();
                assert_eq!(scan(&dir, current.clone()).unwrap(), expected);
            }
        }
        assert!(current.is_none());
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn individual_changes_do_not_rewrite_unrelated_sibling_pages() {
        let dir = directory("path-copy");
        let entries: Vec<_> = (0..1000).map(|id| (id, id)).collect();
        let root = build(&dir, entries.clone()).unwrap();
        let before = fs::read_dir(&dir).unwrap().count();
        let updated = insert(&dir, root.clone(), (1200, 1200)).unwrap();
        let delta = fs::read_dir(&dir).unwrap().count() - before;
        assert!(delta <= 3, "single insert should change only leaf and parent");
        assert_eq!(scan(&dir, root).unwrap(), entries);
        assert_eq!(scan(&dir, updated).unwrap().len(), 1001);
        let _ = fs::remove_dir_all(dir);
    }
}
