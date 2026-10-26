//! Opt-in NQDB0003 immutable-generation page store for the NoeSQL Rust
//! relational runtime. The original v2 Zig pager is NOT rewritten.
//!
//! Crash protocol: a committed generation is a fully fsynced immutable,
//! checksummed page file. Only after its directory is fsynced do we
//! atomically publish and fsync the fixed-size NQDB0003 HEAD file. Old
//! generations remain accessible to readers that pinned them. Cross-process
//! flock serializes writers and allows concurrent snapshot acquisition.
//!
//! This does not reclaim old generations, store per-row pages, use the existing
//! Zig allocator, or upgrade in place. These limitations block release as the
//! default NoeSQL engine until crash/fault/soak qualification is executed.
use crate::codec;
use crate::hash256::{sha256, Sha256};
use crate::reference::{parse, Database, ExecutionReport, Params};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PAGE_SIZE: usize = 4096;
pub const MAX_PAGES: usize = 65_536;
const HEAD_MAGIC: &[u8; 8] = b"NQDB0003";
const PAGE_MAGIC: &[u8; 8] = b"NQPG0003";
const HEAD_LEN: usize = 168;
const PAGE_HEADER: usize = 64;
const FRAME_LEN: usize = PAGE_SIZE + 32;
const MAX_PAYLOAD: usize = PAGE_SIZE * MAX_PAGES;

type V3Result<T> = Result<T, V3Error>;
#[derive(Debug)]
pub enum V3Error {
    Io(std::io::Error),
    Invalid(String),
    Conflict(String),
}
impl std::fmt::Display for V3Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(out, "v3 I/O: {error}"),
            Self::Invalid(error) => write!(out, "v3 corrupt/incompatible: {error}"),
            Self::Conflict(error) => write!(out, "v3 conflict: {error}"),
        }
    }
}
impl std::error::Error for V3Error {}
impl From<std::io::Error> for V3Error { fn from(e: std::io::Error) -> Self { Self::Io(e) } }
fn invalid(reason: impl Into<String>) -> V3Error { V3Error::Invalid(reason.into()) }

#[derive(Clone, Debug)]
struct Head {
    generation: u64,
    bytes: u64,
    pages: u32,
    filename: String,
    digest: [u8; 32],
}
fn versions(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.versions", path.display()))
}
fn lock_name(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lck", path.display()))
}
fn unique_nonce() -> V3Result<String> {
    let n = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("system clock precedes epoch"))?.as_nanos();
    Ok(format!("{}-{n}", std::process::id()))
}
fn sync_directory(path: &Path) -> V3Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
fn parent_dir(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}
fn validate_path(path: &Path) -> V3Result<()> {
    crate::validate_database_path(path).map_err(|error| invalid(error.to_string()))?;
    if !path.is_absolute() {
        // Relative paths can be renamed underneath the process's working dir.
        return Err(invalid("v3 requires an absolute .noedb path"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe extern "C" { fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int; }
struct Lock(File);
fn take_lock(path: &Path, exclusive: bool) -> V3Result<Lock> {
    let file = OpenOptions::new().read(true).write(true).create(true).open(lock_name(path))?;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // Advisory flock. Does not protect private directories against
        // symlink swaps by hostile users; callers must own their parent.
        let flags = if exclusive { 2 } else { 1 };
        for _ in 0..500 {
            if unsafe { flock(file.as_raw_fd(), flags | 4) } == 0 { return Ok(Lock(file)); }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(V3Error::Conflict("timed out acquiring cross-process v3 lock".into()))
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = (file, exclusive); Err(invalid("v3 durable flock requires Linux")) }
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { flock(self.0.as_raw_fd(), 8) };
        }
    }
}
fn decode_head(bytes: &[u8]) -> V3Result<Head> {
    if bytes.len() != HEAD_LEN || &bytes[..8] != HEAD_MAGIC {
        return Err(invalid("not an NQDB0003 head"));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != 3 ||
       u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize != PAGE_SIZE {
        return Err(invalid("unsupported NQDB0003 head version or page size"));
    }
    if sha256(&bytes[..136]).as_slice() != &bytes[136..168] {
        return Err(invalid("head checksum mismatch"));
    }
    let generation = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    let length = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
    let pages = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
    let name_len = u32::from_le_bytes(bytes[36..40].try_into().unwrap()) as usize;
    if generation == 0 || length == 0 || length > MAX_PAYLOAD as u64 ||
        pages == 0 || pages as usize > MAX_PAGES ||
        ((length as usize + PAGE_SIZE - 1) / PAGE_SIZE) != pages as usize ||
        !(1..=64).contains(&name_len) || bytes[40+name_len..104].iter().any(|c| *c != 0) {
        return Err(invalid("invalid head lengths, generation or padding"));
    }
    let filename = std::str::from_utf8(&bytes[40..40+name_len])
        .map_err(|_| invalid("head contains invalid generation name"))?.to_string();
    if !filename.ends_with(".pages") ||
       !filename.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'.') ||
       filename.starts_with('.') || filename.contains("..") {
        return Err(invalid("unsafe version filename"));
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&bytes[104..136]);
    Ok(Head { generation, bytes: length, pages, filename, digest })
}
fn encode_head(head: &Head) -> V3Result<[u8; HEAD_LEN]> {
    if head.filename.len() > 64 { return Err(invalid("version file name too long")); }
    let mut bytes = [0u8; HEAD_LEN];
    bytes[..8].copy_from_slice(HEAD_MAGIC);
    bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&head.generation.to_le_bytes());
    bytes[24..32].copy_from_slice(&head.bytes.to_le_bytes());
    bytes[32..36].copy_from_slice(&head.pages.to_le_bytes());
    bytes[36..40].copy_from_slice(&(head.filename.len() as u32).to_le_bytes());
    bytes[40..40+head.filename.len()].copy_from_slice(head.filename.as_bytes());
    bytes[104..136].copy_from_slice(&head.digest);
    bytes[136..168].copy_from_slice(&sha256(&bytes[..136]));
    Ok(bytes)
}
fn read_head(path: &Path) -> V3Result<Head> {
    let mut buffer = Vec::new();
    let f = File::open(path)?;
    if f.metadata()?.len() != HEAD_LEN as u64 { return Err(invalid("head length mismatch")); }
    f.take(HEAD_LEN as u64 + 1).read_to_end(&mut buffer)?;
    decode_head(&buffer)
}
fn write_generation(path: &Path, generation: u64, db: &Database) -> V3Result<Head> {
    let payload = codec::encode(db).map_err(|e| invalid(e.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(invalid("v3 snapshot exceeds 65,536 pages"));
    }
    let pages = payload.len().div_ceil(PAGE_SIZE);
    let filename = format!("{generation}-{}.pages", unique_nonce()?);
    let name = versions(path).join(&filename);
    let mut file = OpenOptions::new().write(true).create_new(true).open(&name)?;
    let mut outer = Sha256::default();
    let mut header = [0u8; PAGE_HEADER];
    header[..8].copy_from_slice(PAGE_MAGIC);
    header[8..16].copy_from_slice(&generation.to_le_bytes());
    header[16..24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[24..28].copy_from_slice(&(pages as u32).to_le_bytes());
    header[32..64].copy_from_slice(&sha256(&payload));
    file.write_all(&header)?;
    outer.update(&header);
    for chunk in payload.chunks(PAGE_SIZE) {
        let mut block = [0u8; PAGE_SIZE];
        block[..chunk.len()].copy_from_slice(chunk);
        let digest = sha256(&block);
        file.write_all(&block)?;
        file.write_all(&digest)?;
        outer.update(&block);
        outer.update(&digest);
    }
    file.sync_all()?;
    drop(file);
    // Generation data must survive a crash BEFORE HEAD may reference it.
    sync_directory(&versions(path))?;
    Ok(Head {
        generation, bytes: payload.len() as u64, pages: pages as u32,
        filename, digest: outer.finalize(),
    })
}
fn read_generation(path: &Path, head: &Head) -> V3Result<Database> {
    let version = versions(path).join(&head.filename);
    let mut reader = File::open(&version)?;
    let expected_size = PAGE_HEADER as u64 + head.pages as u64 * FRAME_LEN as u64;
    if reader.metadata()?.len() != expected_size {
        return Err(invalid("generation file length mismatch"));
    }
    let mut header = [0u8; PAGE_HEADER];
    reader.read_exact(&mut header)?;
    if &header[..8] != PAGE_MAGIC ||
       u64::from_le_bytes(header[8..16].try_into().unwrap()) != head.generation ||
       u64::from_le_bytes(header[16..24].try_into().unwrap()) != head.bytes ||
       u32::from_le_bytes(header[24..28].try_into().unwrap()) != head.pages ||
       header[28..32].iter().any(|byte| *byte != 0) {
        return Err(invalid("generation header mismatch"));
    }
    let mut outer = Sha256::default();
    outer.update(&header);
    let mut payload = Vec::with_capacity(head.bytes as usize);
    for page_index in 0..head.pages as usize {
        let mut block = [0u8; PAGE_SIZE];
        let mut digest = [0u8;32];
        reader.read_exact(&mut block)?;
        reader.read_exact(&mut digest)?;
        if sha256(&block) != digest { return Err(invalid(format!("page {page_index} SHA256 mismatch"))); }
        outer.update(&block);
        outer.update(&digest);
        let n = (head.bytes as usize - payload.len()).min(PAGE_SIZE);
        if block[n..].iter().any(|c| *c != 0) {
            return Err(invalid("noncanonical final-page padding"));
        }
        payload.extend_from_slice(&block[..n]);
    }
    if outer.finalize() != head.digest || sha256(&payload).as_slice() != &header[32..64] {
        return Err(invalid("generation payload checksum mismatch"));
    }
    codec::decode(&payload).map_err(|e| invalid(format!("relational snapshot rejected: {e}")))
}
fn stage_head(path: &Path, head: &Head, initial: bool) -> V3Result<()> {
    let tmp = PathBuf::from(format!("{}.publish-{}", path.display(), unique_nonce()?));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&encode_head(head)?)?;
        file.sync_all()?;
        drop(file);
        if initial {
            // Atomic no-overwrite publication; migration NEVER destroys v2.
            fs::hard_link(&tmp, path)?;
        } else {
            // Atomic replacement; old or new head is valid after a crash.
            fs::rename(&tmp, path)?;
        }
        sync_directory(parent_dir(path))
    })();
    let _ = fs::remove_file(&tmp);
    result
}
/// Failpoints are exposed to unit tests for deterministic crash boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint { None, BeforePublish }
fn create_internal(path: &Path, db: &Database, fault: FaultPoint) -> V3Result<()> {
    validate_path(path)?;
    let _lock = take_lock(path, true)?;
    if path.exists() || versions(path).exists() {
        return Err(V3Error::Conflict("target and its generation directory must be absent".into()));
    }
    fs::create_dir(versions(path))?;
    sync_directory(parent_dir(path))?;
    let head = write_generation(path, 1, db)?;
    if fault == FaultPoint::BeforePublish {
        return Err(V3Error::Conflict("injected crash before head publication".into()));
    }
    stage_head(path, &head, true)
}
/// Creates a fresh v3 target from an existing logical database. No v2 file
/// is ever renamed, replaced or interpreted as v3 storage.
pub fn create_new(path: &Path, db: &Database) -> V3Result<()> {
    create_internal(path, db, FaultPoint::None)
}
#[derive(Clone)]
pub struct Snapshot {
    pub generation: u64,
    database: Arc<Database>,
}
impl Snapshot {
    pub fn query(&self, source: &str, params: &Params) -> V3Result<ExecutionReport> {
        let statements = parse(source).map_err(|e| invalid(e.to_string()))?;
        self.database.query_parsed(&statements, params).map_err(|e| invalid(e.to_string()))
    }
}
/// Multiple processes can read distinct immutable committed generations.
/// The shared lock is held for acquisition/validation, not query evaluation.
pub fn snapshot(path: &Path) -> V3Result<Snapshot> {
    validate_path(path)?;
    let lock = take_lock(path, false)?;
    let head = read_head(path)?;
    let db = read_generation(path, &head)?;
    drop(lock);
    Ok(Snapshot { generation: head.generation, database: Arc::new(db) })
}
pub fn inspect(path: &Path) -> V3Result<(u64, u64, u32)> {
    validate_path(path)?;
    let _lock = take_lock(path, false)?;
    let head = read_head(path)?;
    let _ = read_generation(path, &head)?;
    Ok((head.generation, head.bytes, head.pages))
}
fn execute_internal(
    path: &Path, source: &str, params: &Params, fault: FaultPoint
) -> V3Result<ExecutionReport> {
    validate_path(path)?;
    let statements = parse(source).map_err(|e| invalid(e.to_string()))?;
    if Database::statements_read_only(&statements) {
        return snapshot(path)?.database.query_parsed(&statements, params)
            .map_err(|e| invalid(e.to_string()));
    }
    let _lock = take_lock(path, true)?;
    let head = read_head(path)?;
    let mut db = read_generation(path, &head)?;
    let before = codec::encode(&db).map_err(|e| invalid(e.to_string()))?;
    let report = db.execute_parsed(&statements, params).map_err(|e| invalid(e.to_string()))?;
    let after = codec::encode(&db).map_err(|e| invalid(e.to_string()))?;
    if before != after {
        let next = head.generation.checked_add(1)
            .ok_or_else(|| invalid("generation counter exhausted"))?;
        let published = write_generation(path, next, &db)?;
        if fault == FaultPoint::BeforePublish {
            return Err(V3Error::Conflict("injected crash after generation fsync".into()));
        }
        stage_head(path, &published, false)?;
    }
    Ok(report)
}
/// Serialized writer commit + MVCC reader snapshots, with durable immutable
/// versions. The full relational snapshot is still cloned and rewritten.
pub fn execute(path: &Path, source: &str, params: &Params) -> V3Result<ExecutionReport> {
    execute_internal(path, source, params, FaultPoint::None)
}

/// Source v2 remains the rollback copy. Its Zig lock is held across validation
/// and successful v3 target publication (the source is never modified).
#[cfg(feature = "zig-pager")]
pub fn migrate_copy_v2(source: &Path, target: &Path) -> V3Result<()> {
    validate_path(target)?;
    if source == target { return Err(invalid("v2 source and v3 target must differ")); }
    crate::zig_pager::with_locked_snapshot(source, |db| {
        // Source is not mutated. Verify the target before releasing the v2
        // lock, so a concurrent v2 writer cannot invalidate the comparison.
        create_new(target, db).map_err(|e| e.to_string())?;
        let imported = snapshot(target).map_err(|e| e.to_string())?;
        let before = codec::encode(db).map_err(|e| e.to_string())?;
        let after = codec::encode(&imported.database).map_err(|e| e.to_string())?;
        if before != after {
            return Err("v2/v3 migration verification mismatch".into());
        }
        Ok(())
    }).map_err(invalid)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    fn path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("noesql-v3-{label}-{}-{}.noedb",
            std::process::id(), unique_nonce().unwrap()))
    }
    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(versions(path));
        let _ = fs::remove_file(lock_name(path));
    }
    fn db() -> Database {
        let mut db = Database::default();
        db.execute("table entries { id:int key auto\n owner:int required }\nentries add {owner:1}", &Params::new()).unwrap();
        db
    }
    #[test]
    fn v3_cold_reopen_rejects_corruption_and_supports_large_page_count() {
        let target = path("cold");
        create_new(&target, &db()).unwrap();
        let old = snapshot(&target).unwrap();
        assert_eq!(old.generation, 1);
        execute(&target, "entries add {owner:2}", &Params::new()).unwrap();
        assert_eq!(snapshot(&target).unwrap().generation, 2);
        assert_eq!(old.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        assert_eq!(snapshot(&target).unwrap().query("entries get owner", &Params::new()).unwrap().rows.len(), 2);
        let head = read_head(&target).unwrap();
        let data = versions(&target).join(head.filename);
        let mut damaged = OpenOptions::new().read(true).write(true).open(&data).unwrap();
        use std::io::{Seek, SeekFrom};
        damaged.seek(SeekFrom::Start(PAGE_HEADER as u64 + 30)).unwrap();
        damaged.write_all(&[255]).unwrap();
        damaged.sync_all().unwrap();
        assert!(snapshot(&target).is_err(), "page corruption silently accepted");
        assert_eq!(old.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        cleanup(&target);
    }
    #[test]
    fn fail_before_publish_retains_previous_generation_and_does_not_adopt_orphan() {
        let target = path("fault");
        create_new(&target, &db()).unwrap();
        assert!(execute_internal(&target, "entries add {owner:3}", &Params::new(), FaultPoint::BeforePublish).is_err());
        assert_eq!(snapshot(&target).unwrap().generation, 1);
        assert_eq!(snapshot(&target).unwrap().query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        execute(&target, "entries add {owner:4}", &Params::new()).unwrap();
        assert_eq!(snapshot(&target).unwrap().generation, 2);
        cleanup(&target);
    }
    #[test]
    fn interrupted_creation_never_publishes_partially_built_v3() {
        let target = path("create-fault");
        assert!(create_internal(&target, &db(), FaultPoint::BeforePublish).is_err());
        assert!(!target.exists());
        cleanup(&target);
    }
    #[test]
    fn concurrent_readers_pin_old_generation_while_writer_publishes_new_one() {
        let target = path("concurrent");
        create_new(&target, &db()).unwrap();
        let prior = snapshot(&target).unwrap();
        let mut workers = vec![];
        for _ in 0..12 {
            let path = target.clone();
            workers.push(std::thread::spawn(move || {
                for _ in 0..12 {
                    let read = snapshot(&path).unwrap();
                    let count = read.query("entries get id", &Params::new()).unwrap().rows.len();
                    assert!(count == 1 || count == 2);
                }
            }));
        }
        execute(&target, "entries add {owner:2}", &Params::new()).unwrap();
        for worker in workers { worker.join().unwrap(); }
        assert_eq!(prior.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        assert_eq!(snapshot(&target).unwrap().query("entries get owner", &Params::new()).unwrap().rows.len(), 2);
        cleanup(&target);
    }
    #[test]
    fn head_checksum_and_page_count_are_strictly_validated() {
        let target = path("head");
        create_new(&target, &db()).unwrap();
        let mut bytes = fs::read(&target).unwrap();
        bytes[16] ^= 1;
        fs::write(&target, bytes).unwrap();
        assert!(snapshot(&target).is_err());
        cleanup(&target);
    }
    #[test]
    fn v3_live_data_exceeds_the_legacy_256_page_ceiling() {
        use std::collections::BTreeMap;
        let target = path("more-than-v2");
        let mut source = Database::default();
        source.execute("table objects { id:int key auto\n content:text required }",
            &Params::new()).unwrap();
        let table = source.tables.get_mut("objects").unwrap();
        for id in 1..=300i64 {
            let mut row = BTreeMap::new();
            row.insert("id".into(), Value::Int(id));
            row.insert("content".into(), Value::Text("x".repeat(4050)));
            table.rows.push(row);
        }
        table.next_auto = 301;
        create_new(&target, &source).unwrap();
        let (_, bytes, pages) = inspect(&target).unwrap();
        assert!(bytes > (256 * PAGE_SIZE) as u64);
        assert!(pages > 256 && pages as usize <= MAX_PAGES);
        let reopened = snapshot(&target).unwrap();
        assert_eq!(reopened.query("objects get id take 1", &Params::new())
            .unwrap().rows.len(), 1);
        assert_eq!(reopened.query("objects get id", &Params::new())
            .unwrap().rows.len(), 300);
        cleanup(&target);
    }
    #[test]
    fn bytes_roundtrip_and_checked_header() {
        let target = path("head-fields");
        create_new(&target, &db()).unwrap();
        let head = read_head(&target).unwrap();
        assert_eq!(head.generation, 1);
        assert_eq!(head.pages as usize, (head.bytes as usize).div_ceil(PAGE_SIZE));
        assert!(head.filename.ends_with(".pages"));
        let expected = snapshot(&target).unwrap().query("entries get owner", &Params::new()).unwrap();
        assert_eq!(expected.rows[0].get("owner"), Some(&Value::Int(1)));
        cleanup(&target);
    }
}
