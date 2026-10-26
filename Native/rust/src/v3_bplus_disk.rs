//! Immutable 4096-byte B+ index pages for NQDB0003 generations.
//! The manifest is included in the fsynced generation file and therefore
//! protected by its committed HEAD checksum. Node pages are content addressed:
//! a changed leaf rewrites its parent path, not every unrelated node.
//! Note: building a changed index remains O(n log n) in memory; only
//! physical writes are incremental. No garbage collection is attempted.
use crate::bplus_index::BPlusIndex;
use crate::v3_bplus_cow::{self as cow, Pair};
use crate::hash256::sha256;
use crate::reference::{Database, Statement};
use crate::value::{Value, ValueType};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const PAGE_BYTES: usize = 4096;
const LEAF_FANOUT: usize = 240;
const BRANCH_FANOUT: usize = 80;
const MAX_DEPTH: usize = 16;
const NODE_MAGIC: &[u8; 8] = b"NQBI0001";
pub(crate) const NODE_MAGIC_V2: &[u8; 8] = b"NQBI0002";
pub(crate) const MAP_MAGIC_V2: &[u8; 8] = b"NQIM0002";
const MAP_MAGIC: &[u8; 8] = b"NQIM0001";
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn index_dir(versions: &Path) -> PathBuf { versions.join("index-pages") }
fn digest_hex(hash: &[u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in hash { result.push_str(&format!("{byte:02x}")); }
    result
}
fn blob_path(directory: &Path, digest: &[u8; 32]) -> PathBuf {
    directory.join(format!("{}.bpi", digest_hex(digest)))
}
fn u16_at(page: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(page[at..at+2].try_into().expect("checked page"))
}
fn u32_at(page: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(page[at..at+4].try_into().expect("checked page"))
}
fn i64_at(page: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(page[at..at+8].try_into().expect("checked page"))
}
fn u64_at(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at+8].try_into().expect("checked page"))
}
fn sync_dir(path: &Path) -> Result<()> { File::open(path).and_then(|dir| dir.sync_all()).map_err(err) }
pub(crate) fn prepare_dir(versions: &Path) -> Result<PathBuf> {
    let dir = index_dir(versions);
    if !dir.exists() {
        match fs::create_dir(&dir) {
            Ok(()) => sync_dir(versions)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(e) => return Err(err(e)),
        }
    }
    if !fs::symlink_metadata(&dir).map_err(err)?.file_type().is_dir() {
        return Err("index-pages is not a regular directory".into());
    }
    Ok(dir)
}
pub(crate) fn publish_blob(directory: &Path, image: &[u8; PAGE_BYTES]) -> Result<[u8; 32]> {
    let hash = sha256(image);
    let path = blob_path(directory, &hash);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            f.write_all(image).map_err(err)?;
            f.sync_all().map_err(err)?;
            sync_dir(directory)?;
        },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // A prior generation owns this blob. Never overwrite shared pages.
            let mut f = File::open(&path).map_err(err)?;
            if f.metadata().map_err(err)?.len() != PAGE_BYTES as u64 {
                return Err("existing index page has wrong length".into());
            }
            let mut old = [0; PAGE_BYTES];
            f.read_exact(&mut old).map_err(err)?;
            if old != *image { return Err("existing index page hash collision or corruption".into()); }
        },
        Err(e) => return Err(err(e)),
    }
    Ok(hash)
}
pub(crate) fn read_blob(directory: &Path, hash: &[u8; 32]) -> Result<[u8; PAGE_BYTES]> {
    let path = blob_path(directory, hash);
    let mut f = File::open(path).map_err(err)?;
    if f.metadata().map_err(err)?.len() != PAGE_BYTES as u64 {
        return Err("index page size is not 4096".into());
    }
    let mut bytes = [0; PAGE_BYTES];
    f.read_exact(&mut bytes).map_err(err)?;
    if sha256(&bytes) != *hash ||
       (&bytes[..8] != NODE_MAGIC && &bytes[..8] != NODE_MAGIC_V2) {
        return Err("index page checksum or magic mismatch".into());
    }
    Ok(bytes)
}
#[derive(Clone)]
struct NodeRef { min: i64, max: i64, sha: [u8; 32] }
fn store_leaf(directory: &Path, slice: &[(i64, u64)]) -> Result<NodeRef> {
    if slice.is_empty() || slice.len() > LEAF_FANOUT { return Err("bad index leaf fanout".into()); }
    let mut image = [0; PAGE_BYTES];
    image[..8].copy_from_slice(NODE_MAGIC);
    image[8] = 1;
    image[12..16].copy_from_slice(&(slice.len() as u32).to_le_bytes());
    for (n, (key, position)) in slice.iter().enumerate() {
        let at = 64 + n*16;
        image[at..at+8].copy_from_slice(&key.to_le_bytes());
        image[at+8..at+16].copy_from_slice(&position.to_le_bytes());
    }
    Ok(NodeRef { min: slice[0].0, max: slice[slice.len()-1].0,
        sha: publish_blob(directory, &image)? })
}
fn store_branch(directory: &Path, children: &[NodeRef]) -> Result<NodeRef> {
    if children.is_empty() || children.len() > BRANCH_FANOUT {
        return Err("bad index branch fanout".into());
    }
    let mut image = [0; PAGE_BYTES];
    image[..8].copy_from_slice(NODE_MAGIC);
    image[8] = 2;
    image[12..16].copy_from_slice(&(children.len() as u32).to_le_bytes());
    for (n, child) in children.iter().enumerate() {
        let at = 64 + n*48;
        image[at..at+8].copy_from_slice(&child.min.to_le_bytes());
        image[at+8..at+16].copy_from_slice(&child.max.to_le_bytes());
        image[at+16..at+48].copy_from_slice(&child.sha);
    }
    Ok(NodeRef { min: children[0].min, max: children[children.len()-1].max,
        sha: publish_blob(directory, &image)? })
}
fn persist_tree(directory: &Path, entries: &[(i64, u64)]) -> Result<[u8; 32]> {
    if entries.is_empty() { return Ok([0; 32]); }
    let mut level = Vec::new();
    for chunk in entries.chunks(LEAF_FANOUT) { level.push(store_leaf(directory, chunk)?); }
    let mut depth = 1;
    while level.len() > 1 {
        if depth >= MAX_DEPTH { return Err("index tree too deep".into()); }
        let mut parent = Vec::new();
        for group in level.chunks(BRANCH_FANOUT) {
            parent.push(store_branch(directory, group)?);
        }
        level = parent;
        depth += 1;
    }
    Ok(level[0].sha)
}
fn expected_indexes(db: &Database) -> BTreeSet<(String, String)> {
    let mut set = BTreeSet::new();
    for (table_name, table) in &db.tables {
        for field in &table.fields {
            if matches!(field.kind, ValueType::Int | ValueType::Ref) && (field.key || field.unique) {
                set.insert((table_name.clone(), field.name.clone()));
            }
        }
    }
    for index in db.indexes.values() {
        if index.columns.len() != 1 { continue; }
        let field = &index.columns[0];
        if let Some(table) = db.tables.get(&index.table) {
            if table.fields.iter().any(|candidate|
                candidate.name == *field && matches!(candidate.kind, ValueType::Int | ValueType::Ref)) {
                set.insert((index.table.clone(), field.clone()));
            }
        }
    }
    set
}
fn row_entries(db: &Database, table: &str, field: &str) -> Result<Vec<(i64, u64)>> {
    let rows = &db.tables.get(table).ok_or("index table missing")?.rows;
    let mut result = Vec::new();
    for (position, row) in rows.iter().enumerate() {
        if let Some(Value::Int(key)) = row.get(field) {
            result.push((*key, position as u64));
        }
    }
    result.sort_unstable();
    Ok(result)
}
/// Keyed by (indexed value, stable PRIMARY KEY), never row-vector ordinal.
/// Row deletion therefore changes only its own keys rather than renumbering
/// every entry after the deleted position.
fn stable_entries(db: &Database, table: &str, field: &str) -> Result<Vec<Pair>> {
    let table = db.tables.get(table).ok_or("index table missing")?;
    let primary = table.fields.iter().find(|field| field.key)
        .ok_or("indexed table has no primary key")?;
    let mut result = Vec::new();
    for row in &table.rows {
        let pk = match row.get(&primary.name) {
            Some(Value::Int(id)) => *id,
            _ => return Err("indexed row lacks integer primary key".into()),
        };
        if let Some(Value::Int(indexed)) = row.get(field) {
            result.push((*indexed, pk));
        }
    }
    result.sort_unstable();
    if result.windows(2).any(|pairs| pairs[0] >= pairs[1]) {
        return Err("duplicate stable B+ index entry".into());
    }
    Ok(result)
}
fn index_is_dirty(table: &str, column: &str, primary: &str, statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Add(name, _) | Statement::Delete(name, _) |
        Statement::Table(name, _) if name == table => true,
        Statement::Set(name, _, values) if name == table =>
            values.iter().any(|(name, _)| name == column || name == primary),
        Statement::Atomic(nested) => index_is_dirty(table, column, primary, nested),
        _ => false,
    })
}
pub type IndexCapture = BTreeMap<(String, String), Vec<Pair>>;
/// Capture only the potentially changed index entries BEFORE a private
/// relational mutation. This bounded migration journal avoids cloning the
/// entire database. Delta discovery still visits affected table rows (O(n));
/// individual disk B+ entry mutations are O(log n).
pub fn capture_dirty_indexes(db: &Database, statements: &[Statement]) -> Result<IndexCapture> {
    let mut out = BTreeMap::new();
    for (table, field) in expected_indexes(db) {
        let primary = db.tables[&table].fields.iter().find(|f| f.key)
            .ok_or("indexed table has no primary key")?;
        if index_is_dirty(&table, &field, &primary.name, statements) {
            out.insert((table.clone(), field.clone()), stable_entries(db, &table, &field)?);
        }
    }
    Ok(out)
}
/// Parse roots from a prior HEAD-authenticated, fully verified index manifest.
/// Original ordinal index manifests are recognized but upgraded to stable ID
/// trees once on their first subsequent mutation.
fn parse_roots(padded: &[u8]) -> Result<(bool, BTreeMap<(String, String), ([u8;32], u64)>)> {
    if padded.len() < 44 ||
       (&padded[..8] != MAP_MAGIC && &padded[..8] != MAP_MAGIC_V2) {
        return Err("invalid previous B+ manifest header".into());
    }
    let stable = &padded[..8] == MAP_MAGIC_V2;
    let count = u32_at(padded, 8) as usize;
    let mut cursor = 12usize;
    let mut result = BTreeMap::new();
    for _ in 0..count {
        if cursor + 44 > padded.len() { return Err("truncated previous B+ manifest".into()); }
        let tn = u16_at(padded, cursor) as usize;
        let cn = u16_at(padded, cursor+2) as usize;
        let rows = u64_at(padded, cursor+4);
        let mut root = [0; 32]; root.copy_from_slice(&padded[cursor+12..cursor+44]);
        cursor += 44;
        let end = cursor.checked_add(tn).and_then(|n| n.checked_add(cn))
            .ok_or("previous B+ manifest name overflow")?;
        if end.checked_add(32).is_none_or(|n| n > padded.len()) {
            return Err("previous B+ manifest name truncated".into());
        }
        let table = std::str::from_utf8(&padded[cursor..cursor+tn]).map_err(err)?.to_string();
        cursor += tn;
        let column = std::str::from_utf8(&padded[cursor..cursor+cn]).map_err(err)?.to_string();
        cursor += cn;
        if result.insert((table, column), (root, rows)).is_some() {
            return Err("duplicate previous B+ root".into());
        }
    }
    if cursor + 32 > padded.len() ||
        sha256(&padded[..cursor]).as_slice() != &padded[cursor..cursor+32] ||
        padded[cursor+32..].iter().any(|b| *b != 0) {
        return Err("previous B+ manifest digest mismatch".into());
    }
    Ok((stable, result))
}
fn diff_entries(before: &[Pair], after: &[Pair]) -> (Vec<Pair>, Vec<Pair>) {
    let mut remove = Vec::new();
    let mut insert = Vec::new();
    let mut old = 0;
    let mut new = 0;
    while old < before.len() || new < after.len() {
        match (before.get(old), after.get(new)) {
            (Some(a), Some(b)) if a == b => { old+=1; new+=1; },
            (Some(a), Some(b)) if a < b => { remove.push(*a); old+=1; },
            (Some(_), Some(b)) => { insert.push(*b); new+=1; },
            (Some(a), None) => { remove.push(*a); old+=1; },
            (None, Some(b)) => { insert.push(*b); new+=1; },
            (None, None) => break,
        }
    }
    (remove, insert)
}

/// Persist new indexes by balanced bulk creation; update existing stable-key
/// indexes by logarithmic copy-on-write insertion/removal, not by rebuilding.
/// SHA-verified unchanged subtrees are reused by hash across generations.
pub fn write_index_manifest(versions: &Path, db: &Database) -> Result<Vec<u8>> {
    write_index_manifest_delta(versions, db, None, &[], None)
}
pub fn write_index_manifest_delta(
    versions: &Path,
    db: &Database,
    previous: Option<&[u8]>,
    statements: &[Statement],
    before: Option<&IndexCapture>,
) -> Result<Vec<u8>> {
    let (stable, previous) = previous.map(parse_roots).transpose()?
        .unwrap_or((false, BTreeMap::new()));
    let directory = prepare_dir(versions)?;
    let mut output = Vec::new();
    output.extend_from_slice(MAP_MAGIC_V2);
    let fields = expected_indexes(db);
    if fields.len() > u32::MAX as usize { return Err("too many indexes".into()); }
    output.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for (table, field) in fields {
        if table.len() > u16::MAX as usize || field.len() > u16::MAX as usize {
            return Err("index name too long".into());
        }
        let primary = db.tables[&table].fields.iter().find(|f| f.key)
            .ok_or("indexed table has no primary key")?;
        let dirty = index_is_dirty(&table, &field, &primary.name, statements);
        let previous_root = previous.get(&(table.clone(), field.clone()));
        let (root, count) = if stable && !dirty {
            if let Some(&(root, count)) = previous_root {
                (root, count)
            } else {
                let entries = stable_entries(db, &table, &field)?;
                let root = cow::build(&directory, entries.clone())?
                    .map_or([0;32], |node| node.hash);
                (root, entries.len() as u64)
            }
        } else if stable && previous_root.is_some() {
            let (prior_hash, prior_count) = *previous_root.unwrap();
            let prior = before.and_then(|capture| capture.get(&(table.clone(), field.clone())))
                .ok_or("dirty stable index is missing its pre-mutation journal")?;
            if prior.len() as u64 != prior_count {
                return Err("pre-mutation index count differs from committed manifest".into());
            }
            let after = stable_entries(db, &table, &field)?;
            let (removals, insertions) = diff_entries(prior, &after);
            let mut root = cow::root(&directory, &prior_hash)?;
            for pair in removals { root = cow::remove(&directory, root, pair)?; }
            for pair in insertions { root = cow::insert(&directory, root, pair)?; }
            (root.map_or([0;32], |node| node.hash), after.len() as u64)
        } else {
            // Creation, new index, or the single mandatory migration from
            // NQBI0001 row offsets to NQBI0002 stable primary-key identity.
            let entries = stable_entries(db, &table, &field)?;
            let root = cow::build(&directory, entries.clone())?
                .map_or([0;32], |node| node.hash);
            (root, entries.len() as u64)
        };
        output.extend_from_slice(&(table.len() as u16).to_le_bytes());
        output.extend_from_slice(&(field.len() as u16).to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        output.extend_from_slice(&root);
        output.extend_from_slice(table.as_bytes());
        output.extend_from_slice(field.as_bytes());
    }
    let digest = sha256(&output);
    output.extend_from_slice(&digest);
    Ok(output)
}
fn walk_tree(directory: &Path, hash: &[u8; 32], depth: usize,
    visited: &mut BTreeSet<[u8; 32]>, output: &mut Vec<(i64, u64)>)
    -> Result<(i64, i64)> {
    if depth > MAX_DEPTH || !visited.insert(*hash) { return Err("index cycle/depth exceeded".into()); }
    let image = read_blob(directory, hash)?;
    if image[9..12].iter().any(|b| *b != 0) || image[16..64].iter().any(|b| *b != 0) {
        return Err("invalid B+ page header padding".into());
    }
    let count = u32_at(&image, 12) as usize;
    match image[8] {
        1 => {
            if count == 0 || count > LEAF_FANOUT { return Err("invalid leaf size".into()); }
            let mut lo = i64::MAX; let mut hi = i64::MIN;
            for i in 0..count {
                let at = 64+i*16;
                let key = i64_at(&image, at);
                let position = u64_at(&image, at+8);
                // Also compare across adjacent leaves, not just inside one
                // page: a malformed parent must never reorder query results.
                if output.last().is_some_and(|prior| *prior > (key, position)) {
                    return Err("B+ leaf entries out of order".into());
                }
                lo = lo.min(key); hi = hi.max(key);
                output.push((key, position));
            }
            if image[64+count*16..].iter().any(|b| *b != 0) {
                return Err("noncanonical leaf padding".into());
            }
            Ok((lo, hi))
        },
        2 => {
            if count == 0 || count > BRANCH_FANOUT { return Err("invalid branch size".into()); }
            let mut lo = i64::MAX; let mut hi = i64::MIN;
            let mut previous_max: Option<i64> = None;
            for i in 0..count {
                let at = 64+i*48;
                let lower = i64_at(&image, at);
                let upper = i64_at(&image, at+8);
                if lower > upper || previous_max.is_some_and(|prev| lower < prev) {
                    return Err("non-monotone branch separators".into());
                }
                let mut child = [0; 32]; child.copy_from_slice(&image[at+16..at+48]);
                let (actual_min, actual_max) = walk_tree(directory, &child, depth+1, visited, output)?;
                if actual_min != lower || actual_max != upper {
                    return Err("B+ branch separator does not match child".into());
                }
                lo = lo.min(lower); hi = upper;
                previous_max = Some(upper);
            }
            if image[64+count*48..].iter().any(|b| *b != 0) {
                return Err("noncanonical branch padding".into());
            }
            Ok((lo, hi))
        },
        _ => Err("unrecognized B+ node type".into()),
    }
}
/// Fail closed on corrupt committed indexes; do not silently rebuild them
/// from the relational rows. Install a validated warm query cache on cold load.
pub fn load_index_manifest(versions: &Path, db: &Database, padded: &[u8]) -> Result<()> {
    if padded.len() < 44 || (&padded[..8] != MAP_MAGIC && &padded[..8] != MAP_MAGIC_V2) {
        return Err("invalid B+ manifest".into());
    }
    let stable = &padded[..8] == MAP_MAGIC_V2;
    let count = u32_at(padded, 8) as usize;
    let expected = expected_indexes(db);
    if count != expected.len() { return Err("B+ manifest schema mismatch".into()); }
    let mut cursor = 12usize;
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        if cursor+44 > padded.len() { return Err("truncated B+ manifest".into()); }
        let table_length = u16_at(padded, cursor) as usize;
        let field_length = u16_at(padded, cursor+2) as usize;
        let entry_count = u64_at(padded, cursor+4);
        let mut root = [0; 32]; root.copy_from_slice(&padded[cursor+12..cursor+44]);
        cursor += 44;
        if cursor+table_length+field_length+32 > padded.len() {
            return Err("truncated index name".into());
        }
        let table = std::str::from_utf8(&padded[cursor..cursor+table_length])
            .map_err(err)?.to_string();
        cursor += table_length;
        let field = std::str::from_utf8(&padded[cursor..cursor+field_length])
            .map_err(err)?.to_string();
        cursor += field_length;
        if !expected.contains(&(table.clone(), field.clone())) ||
            !seen.insert((table.clone(), field.clone())) {
            return Err("unexpected or duplicated B+ index".into());
        }
        let mut found = Vec::new();
        if stable {
            found = cow::scan(&index_dir(versions), cow::root(&index_dir(versions), &root)?)?
                .into_iter().map(|(key, id)| (key, id as u64)).collect();
        } else if root != [0; 32] {
            let mut visited = BTreeSet::new();
            walk_tree(&index_dir(versions), &root, 0, &mut visited, &mut found)?;
        }
        let positions = if stable {
            let expected = stable_entries(db, &table, &field)?;
            if found.len() as u64 != entry_count ||
                found.iter().map(|(k, id)| (*k, *id as i64)).collect::<Vec<_>>() != expected {
                return Err("stable B+ index disagrees with committed primary-key rows".into());
            }
            let target = db.tables.get(&table).ok_or("indexed table missing")?;
            let key = target.fields.iter().find(|f| f.key)
                .ok_or("missing indexed table primary key")?;
            let mut row_lookup = BTreeMap::new();
            for (position, row) in target.rows.iter().enumerate() {
                let Some(Value::Int(pk)) = row.get(&key.name) else {
                    return Err("indexed row lacks integer primary key".into());
                };
                if row_lookup.insert(*pk, position).is_some() {
                    return Err("duplicate committed table primary key".into());
                }
            }
            found.into_iter().map(|(value, id)| {
                let position = row_lookup.get(&(id as i64))
                    .ok_or("stable B+ index refers to nonexistent primary key")?;
                Ok((value, *position))
            }).collect::<Result<Vec<_>>>()?
        } else {
            if found.len() as u64 != entry_count || found != row_entries(db, &table, &field)? {
                return Err("legacy B+ index disagrees with committed relational snapshot".into());
            }
            found.into_iter().map(|(value, ordinal)| {
                usize::try_from(ordinal).map(|index| (value, index)).map_err(err)
            }).collect::<Result<Vec<_>>>()?
        };
        db.tables.get(&table).ok_or("missing index table")?
            .install_index(&field, BPlusIndex::build(positions))?;
    }
    if cursor+32 > padded.len() { return Err("index manifest lacks checksum".into()); }
    if sha256(&padded[..cursor]).as_slice() != &padded[cursor..cursor+32] ||
        padded[cursor+32..].iter().any(|b| *b != 0) {
        return Err("index manifest checksum or padding mismatch".into());
    }
    Ok(())
}
pub fn safe_index_blob_name(name: &str) -> bool {
    name.len() == 68 && name.ends_with(".bpi") &&
        name.get(..64).is_some_and(|hash|
            hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Params;
    #[test]
    fn unchanged_indexed_columns_reuse_entire_previous_root_without_page_writes() {
        use crate::reference::parse;
        let base = std::env::temp_dir().join(format!(
            "noesql-bplus-delta-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir(&base).unwrap();
        let mut db = Database::default();
        db.execute("table logs { id:int key auto\n owner:int required\n score:int required }\nindex owner_idx on logs(owner)",
            &Params::new()).unwrap();
        for i in 0..600 {
            db.execute(&format!("logs add {{owner:{},score:{i}}}", i%60), &Params::new()).unwrap();
        }
        let previous = write_index_manifest(&base, &db).unwrap();
        let before = fs::read_dir(index_dir(&base)).unwrap().count();
        let statements = parse("logs where id == 10 set score = 999").unwrap();
        db.execute("logs where id == 10 set score = 999", &Params::new()).unwrap();
        let reused = write_index_manifest_delta(&base, &db, Some(&previous), &statements).unwrap();
        assert_eq!(previous, reused, "non-indexed update changed index roots");
        assert_eq!(fs::read_dir(index_dir(&base)).unwrap().count(), before);
        load_index_manifest(&base, &db, &reused).unwrap();
        let changed = parse("logs add {owner:3,score:999}").unwrap();
        db.execute("logs add {owner:3,score:999}", &Params::new()).unwrap();
        let next = write_index_manifest_delta(&base, &db, Some(&reused), &changed).unwrap();
        assert_ne!(reused, next);
        assert!(fs::read_dir(index_dir(&base)).unwrap().count() > before);
        load_index_manifest(&base, &db, &next).unwrap();
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn unchanged_disk_index_pages_reused_and_cold_reader_installs_bplus() {
        let base = std::env::temp_dir().join(format!("noesql-index-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir(&base).unwrap();
        let mut db = Database::default();
        db.execute("table records { id:int key auto\n owner:int required }\nindex by_owner on records(owner)",
            &Params::new()).unwrap();
        for i in 0..900 {
            db.execute(&format!("records add {{owner:{}}}", i % 45), &Params::new()).unwrap();
        }
        let m1 = write_index_manifest(&base, &db).unwrap();
        let before = fs::read_dir(index_dir(&base)).unwrap().count();
        load_index_manifest(&base, &db, &m1).unwrap();
        assert!(db.tables["records"].seek_cache.get().is_some());
        db.execute("records add {owner:2}", &Params::new()).unwrap();
        let m2 = write_index_manifest(&base, &db).unwrap();
        let after = fs::read_dir(index_dir(&base)).unwrap().count();
        assert!(after > before);
        assert!(after - before < before, "all index pages rewritten, no COW reuse");
        load_index_manifest(&base, &db, &m2).unwrap();
        let _ = fs::remove_dir_all(base);
    }
}
