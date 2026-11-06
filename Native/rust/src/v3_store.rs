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
use crate::v3_bplus_disk;
use crate::hash256::{sha256, Sha256};
use crate::reference::{parse, Database, ExecutionReport, Params, Statement};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
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
    Access(String),
}
impl std::fmt::Display for V3Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(out, "v3 I/O: {error}"),
            Self::Invalid(error) => write!(out, "v3 corrupt/incompatible: {error}"),
            Self::Conflict(error) => write!(out, "v3 conflict: {error}"),
            Self::Access(error) => write!(out, "v3 authorization/budget: {error}"),
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

// A private parent directory is still required: O_NOFOLLOW only protects the
// final pathname component, not hostile ancestor-directory replacements.
fn private_create_file(path: &Path) -> V3Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn read_regular_file(path: &Path) -> V3Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // Linux O_NOFOLLOW
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("expected a regular v3 data file"));
    }
    Ok(file)
}

fn private_create_dir(path: &Path) -> V3Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe extern "C" { fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int; }
struct Lock(File);
fn take_lock(path: &Path, exclusive: bool) -> V3Result<Lock> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // Never follow a preexisting lock symlink
    }
    let file = options.open(lock_name(path))?;
    if !file.metadata()?.is_file() {
        return Err(invalid("v3 lock is not a regular file"));
    }
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
    let checksum = sha256(&bytes[..136]);
    bytes[136..168].copy_from_slice(&checksum);
    Ok(bytes)
}
fn read_head(path: &Path) -> V3Result<Head> {
    let mut buffer = Vec::new();
    let f = read_regular_file(path)?;
    if f.metadata()?.len() != HEAD_LEN as u64 { return Err(invalid("head length mismatch")); }
    f.take(HEAD_LEN as u64 + 1).read_to_end(&mut buffer)?;
    decode_head(&buffer)
}
fn write_generation(path: &Path, generation: u64, db: &Database) -> V3Result<Head> {
    write_generation_delta(path, generation, db, None, &[], None)
}
fn write_generation_delta(
    path: &Path,
    generation: u64,
    db: &Database,
    previous: Option<&[u8]>,
    statements: &[Statement],
    before: Option<&v3_bplus_disk::IndexCapture>,
) -> V3Result<Head> {
    let payload = codec::encode(db).map_err(|e| invalid(e.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(invalid("v3 snapshot exceeds 65,536 pages"));
    }
    let pages = payload.len().div_ceil(PAGE_SIZE);
    // Write/sync dirty content-addressed B+ pages before publishing the
    // generation. The full index manifest is part of the generation digest.
    let index_manifest = v3_bplus_disk::write_index_manifest_delta(
        &versions(path), db, previous, statements, before
    ).map_err(invalid)?;
    let index_pages = index_manifest.len().div_ceil(PAGE_SIZE);
    if pages.checked_add(index_pages).is_none_or(|count| count > MAX_PAGES) {
        return Err(invalid("combined data and index manifest exceed 65,536 pages"));
    }
    let filename = format!("{generation}-{}.pages", unique_nonce()?);
    let name = versions(path).join(&filename);
    let mut file = private_create_file(&name)?;
    let mut outer = Sha256::default();
    let mut header = [0u8; PAGE_HEADER];
    header[..8].copy_from_slice(PAGE_MAGIC);
    header[8..16].copy_from_slice(&generation.to_le_bytes());
    header[16..24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[24..28].copy_from_slice(&(pages as u32).to_le_bytes());
    // Format extension: zero means legacy NQPG0003 without persistent
    // indexes; nonzero is the page count of the manifest after data frames.
    header[28..32].copy_from_slice(&(index_pages as u32).to_le_bytes());
    header[32..64].copy_from_slice(&sha256(&payload));
    file.write_all(&header)?;
    outer.update(&header);
    for chunk in payload.chunks(PAGE_SIZE).chain(index_manifest.chunks(PAGE_SIZE)) {
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
    let mut reader = read_regular_file(&version)?;
    let mut header = [0u8; PAGE_HEADER];
    reader.read_exact(&mut header)?;
    let index_pages = u32::from_le_bytes(header[28..32].try_into().unwrap()) as usize;
    if (head.pages as usize).checked_add(index_pages)
        .is_none_or(|count| count > MAX_PAGES) {
        return Err(invalid("index manifest page count exceeds v3 capacity"));
    }
    let expected_size = PAGE_HEADER as u64 +
        (head.pages as u64 + index_pages as u64) * FRAME_LEN as u64;
    if reader.metadata()?.len() != expected_size {
        return Err(invalid("generation file length mismatch"));
    }
    if &header[..8] != PAGE_MAGIC ||
       u64::from_le_bytes(header[8..16].try_into().unwrap()) != head.generation ||
       u64::from_le_bytes(header[16..24].try_into().unwrap()) != head.bytes ||
       u32::from_le_bytes(header[24..28].try_into().unwrap()) != head.pages {
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
    // The committed HEAD SHA also covers these manifest frames. Never
    // trust/rebuild a corrupt index silently after a cold reopen.
    let mut index_manifest = Vec::with_capacity(index_pages * PAGE_SIZE);
    for page_index in 0..index_pages {
        let mut block = [0u8; PAGE_SIZE];
        let mut digest = [0u8; 32];
        reader.read_exact(&mut block)?;
        reader.read_exact(&mut digest)?;
        if sha256(&block) != digest {
            return Err(invalid(format!("index manifest page {page_index} checksum mismatch")));
        }
        outer.update(&block);
        outer.update(&digest);
        index_manifest.extend_from_slice(&block);
    }
    if outer.finalize() != head.digest || sha256(&payload).as_slice() != &header[32..64] {
        return Err(invalid("generation payload checksum mismatch"));
    }
    let db = codec::decode(&payload)
        .map_err(|e| invalid(format!("relational snapshot rejected: {e}")))?;
    if index_pages != 0 {
        v3_bplus_disk::load_index_manifest(&versions(path), &db, &index_manifest)
            .map_err(invalid)?;
    }
    Ok(db)
}
/// Fetch the authenticated previous index roots. read_generation has
/// already verified the committed HEAD digest, all manifest pages, and
/// content-addressed nodes before a write reaches this function.
fn committed_index_manifest(path: &Path, head: &Head) -> V3Result<Option<Vec<u8>>> {
    let mut file = read_regular_file(&versions(path).join(&head.filename))?;
    let mut header = [0u8; PAGE_HEADER];
    file.read_exact(&mut header)?;
    let pages = u32::from_le_bytes(header[28..32].try_into().unwrap()) as usize;
    if pages == 0 { return Ok(None); } // pre-index NQDB0003 generation
    if pages > MAX_PAGES || (head.pages as usize).saturating_add(pages) > MAX_PAGES {
        return Err(invalid("committed index manifest exceeds v3 capacity"));
    }
    file.seek(SeekFrom::Start(PAGE_HEADER as u64 +
        head.pages as u64 * FRAME_LEN as u64))?;
    let mut manifest = Vec::with_capacity(pages * PAGE_SIZE);
    for _ in 0..pages {
        let mut page = [0u8; PAGE_SIZE];
        let mut checksum = [0u8; 32];
        file.read_exact(&mut page)?;
        file.read_exact(&mut checksum)?;
        if sha256(&page) != checksum {
            return Err(invalid("previous generation index manifest checksum mismatch"));
        }
        manifest.extend_from_slice(&page);
    }
    Ok(Some(manifest))
}

fn stage_head(path: &Path, head: &Head, initial: bool) -> V3Result<()> {
    let tmp = PathBuf::from(format!("{}.publish-{}", path.display(), unique_nonce()?));
    let result = (|| {
        let mut file = private_create_file(&tmp)?;
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
    private_create_dir(&versions(path))?;
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
/// Remove ONLY uncommitted generation files left by a crash during creation.
/// This is an explicit administrative recovery operation, never automatic.
/// Refuses to touch a published database or an unexpected sidecar entry.
pub fn discard_unpublished(path: &Path) -> V3Result<()> {
    validate_path(path)?;
    let _lock = take_lock(path, true)?;
    if path.exists() {
        return Err(V3Error::Conflict("cannot discard generations from a published v3 target".into()));
    }
    let directory = versions(path);
    if !directory.exists() { return Ok(()); }
    if !fs::symlink_metadata(&directory)?.file_type().is_dir() {
        return Err(invalid("v3 versions entry is not a directory"));
    }
    let mut files = Vec::new();
    let mut index_pages = Vec::new();
    let mut index_directory = None;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let type_info = entry.file_type()?;
        let name = entry.file_name();
        let file_name = name.to_str().ok_or_else(|| invalid("unrecognized v3 orphan filename"))?;
        if type_info.is_dir() && file_name == "index-pages" {
            if index_directory.is_some() {
                return Err(invalid("duplicate index-pages directory"));
            }
            for page in fs::read_dir(entry.path())? {
                let page = page?;
                if !page.file_type()?.is_file() ||
                    !page.file_name().to_str().is_some_and(|name|
                        v3_bplus_disk::safe_index_blob_name(name) ||
                        v3_bplus_disk::safe_unpublished_index_temp_name(name)) {
                    return Err(invalid("unexpected file in unpublished index-pages directory"));
                }
                index_pages.push(page.path());
            }
            index_directory = Some(entry.path());
        } else if type_info.is_file() &&
           file_name.ends_with(".pages") &&
           file_name.bytes().all(|c| c.is_ascii_digit() || c == b'-' || c == b'.') &&
           !file_name.starts_with('.') && !file_name.contains("..") {
            files.push(entry.path());
        } else {
            return Err(invalid("unexpected entry in unpublished versions directory"));
        }
    }
    // Validate every path before deleting anything. Published generations
    // can never reach this cleanup path because HEAD existence is checked.
    for file in files { fs::remove_file(file)?; }
    for page in index_pages { fs::remove_file(page)?; }
    if let Some(index_directory) = index_directory {
        sync_directory(&index_directory)?;
        fs::remove_dir(index_directory)?;
    }
    sync_directory(&directory)?;
    fs::remove_dir(&directory)?;
    sync_directory(parent_dir(path))?;
    Ok(())
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
fn material_change_possible(statements: &[Statement], affected: usize) -> bool {
    affected != 0 || statements.iter().any(|statement| !matches!(
        statement, Statement::Get(_) | Statement::Set(_, _, _) | Statement::Delete(_, _)
    ))
}

fn execute_internal(
    path: &Path, source: &str, params: &Params, fault: FaultPoint
) -> V3Result<ExecutionReport> {
    execute_with_access(path, source, params, fault, None)
}

fn execute_with_access(
    path: &Path,
    source: &str,
    params: &Params,
    fault: FaultPoint,
    access: Option<&crate::authorization::AccessControl>,
) -> V3Result<ExecutionReport> {
    validate_path(path)?;
    let statements = match access {
        Some(access) => access.prepare(source, params)
            .map_err(|e| V3Error::Access(e.to_string()))?,
        None => parse(source).map_err(|e| invalid(e.to_string()))?,
    };
    if Database::statements_read_only(&statements) {
        let report = snapshot(path)?.database.query_parsed(&statements, params)
            .map_err(|e| invalid(e.to_string()))?;
        if let Some(access) = access {
            access.validate_result(&report)
                .map_err(|e| V3Error::Access(e.to_string()))?;
        }
        return Ok(report);
    }
    let _lock = take_lock(path, true)?;
    let head = read_head(path)?;
    let mut db = read_generation(path, &head)?;
    // Private generation: no database-state clone and no redundant before/
    // after codec passes. Unsuccessful execution is never published.
    let before_indexes = v3_bplus_disk::capture_dirty_indexes(&db, &statements)
        .map_err(invalid)?;
    let report = db.execute_isolated_parsed(&statements, params)
        .map_err(|e| invalid(e.to_string()))?;
    // Validate the complete result BEFORE the new immutable generation is
    // published. A denied or over-budget statement must not reach durable HEAD.
    if let Some(access) = access {
        access.validate_result(&report)
            .map_err(|e| V3Error::Access(e.to_string()))?;
    }
    if material_change_possible(&statements, report.affected) {
        let next = head.generation.checked_add(1)
            .ok_or_else(|| invalid("generation counter exhausted"))?;
        let prior = committed_index_manifest(path, &head)?;
        let published = write_generation_delta(path, next, &db, prior.as_deref(), &statements, Some(&before_indexes))?;
        if fault == FaultPoint::BeforePublish {
            return Err(V3Error::Conflict("injected crash after generation fsync".into()));
        }
        stage_head(path, &published, false)?;
    }
    Ok(report)
}
/// Optimistic transaction commit against a pinned durable generation.
/// A stale writer fails rather than silently rebasing on newer data. Calling
/// this method does not expose a lock-free writer or per-row MVCC.
pub fn execute_if_generation(
    path: &Path,
    expected_generation: u64,
    source: &str,
    params: &Params,
) -> V3Result<(u64, ExecutionReport)> {
    validate_path(path)?;
    let statements = parse(source).map_err(|e| invalid(e.to_string()))?;
    let _lock = take_lock(path, true)?;
    let head = read_head(path)?;
    if head.generation != expected_generation {
        return Err(V3Error::Conflict(format!(
            "stale transaction generation: expected {expected_generation}, current {}",
            head.generation
        )));
    }
    let mut db = read_generation(path, &head)?;
    let before_indexes = v3_bplus_disk::capture_dirty_indexes(&db, &statements)
        .map_err(invalid)?;
    let report = db.execute_isolated_parsed(&statements, params)
        .map_err(|e| invalid(e.to_string()))?;
    if !material_change_possible(&statements, report.affected) {
        return Ok((head.generation, report));
    }
    let next = head.generation.checked_add(1)
        .ok_or_else(|| invalid("generation counter exhausted"))?;
    let prior = committed_index_manifest(path, &head)?;
    let published = write_generation_delta(
        path, next, &db, prior.as_deref(), &statements, Some(&before_indexes)
    )?;
    stage_head(path, &published, false)?;
    Ok((next, report))
}

/// Serialized writer commit + MVCC reader snapshots, with durable immutable
/// versions. The full relational snapshot is still cloned and rewritten.
pub fn execute(path: &Path, source: &str, params: &Params) -> V3Result<ExecutionReport> {
    execute_internal(path, source, params, FaultPoint::None)
}

/// Deny-by-default, explicitly granted in-process authorization on the live
/// immutable-generation path. The scoped AST and returned-row budget are
/// checked before durable publication. This does not persist principals or
/// policies, authenticate network users, or restrict the administrative API.
pub fn execute_authorized(
    path: &Path,
    source: &str,
    params: &Params,
    access: &crate::authorization::AccessControl,
) -> V3Result<ExecutionReport> {
    execute_with_access(path, source, params, FaultPoint::None, Some(access))
}


impl Snapshot {
    /// Trusted administrative inspection; do not give an unrestricted
    /// Snapshot to untrusted tenants.
    pub fn database(&self) -> &Database { &self.database }
}

/// Create a fresh NQDB0003 store on the first successful mutating program.
/// Never auto-migrate an existing NQDB0002 file; an explicit migration target
/// must be provided to the copy-and-verify CLI.
pub fn execute_or_create(path: &Path, source: &str, params: &Params) -> V3Result<ExecutionReport> {
    validate_path(path)?;
    if path.exists() {
        return execute(path, source, params);
    }
    if versions(path).exists() {
        return Err(V3Error::Conflict(
            "unpublished v3 generation directory exists; inspect or run v3-discard-unpublished".into()
        ));
    }
    let statements = parse(source).map_err(|e| invalid(e.to_string()))?;
    if Database::statements_read_only(&statements) {
        return Err(invalid("database does not exist; create with a schema/write program"));
    }
    let mut initial = Database::default();
    let report = initial.execute_parsed(&statements, params)
        .map_err(|e| invalid(e.to_string()))?;
    match create_new(path, &initial) {
        Ok(()) => Ok(report),
        // Concurrent creation may have won while this call validated the
        // source. Never retry if the target is not a published valid v3 DB.
        Err(V3Error::Conflict(_)) if path.exists() => execute(path, source, params),
        Err(error) => Err(error),
    }
}

/// Source v2 remains the rollback copy. Its Zig lock is held across validation
/// and successful v3 target publication (the source is never modified).
#[cfg(feature = "zig-pager")]
pub fn migrate_copy_v2(source: &Path, target: &Path) -> V3Result<()> {
    validate_path(target)?;
    if source == target { return Err(invalid("v2 source and v3 target must differ")); }
    crate::zig_pager::with_locked_snapshot(source, |db| {
        // If a crash happened after publishing HEAD but before the caller
        // received success, an identical, verified target is idempotently OK.
        // Different or corrupt targets always fail closed. The v2 source
        // remains locked throughout both the copy and verification.
        if !target.exists() {
            create_new(target, db).map_err(|e| e.to_string())?;
        }
        let imported = snapshot(target).map_err(|e| e.to_string())?;
        let before = codec::encode(db).map_err(|e| e.to_string())?;
        let after = codec::encode(&imported.database).map_err(|e| e.to_string())?;
        if before != after {
            return Err("existing v3 target does not match the locked v2 source; no overwrite".into());
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
        assert!(versions(&target).exists());
        discard_unpublished(&target).unwrap();
        assert!(!versions(&target).exists());
        create_new(&target, &db()).unwrap();
        assert_eq!(snapshot(&target).unwrap().generation, 1);
        assert!(discard_unpublished(&target).is_err(), "cannot delete published data");
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
    #[test]
    fn no_op_mutations_do_not_publish_new_generations() {
        let target = path("no-op");
        create_new(&target, &db()).unwrap();
        assert_eq!(snapshot(&target).unwrap().generation, 1);
        assert_eq!(execute(&target, "entries where id == 999 set owner = 2", &Params::new())
            .unwrap().affected, 0);
        assert_eq!(execute(&target, "entries where id == 999 delete", &Params::new())
            .unwrap().affected, 0);
        assert_eq!(snapshot(&target).unwrap().generation, 1);
        execute(&target, "entries add {owner:7}", &Params::new()).unwrap();
        assert_eq!(snapshot(&target).unwrap().generation, 2);
        cleanup(&target);
    }
    #[test]
    fn late_failure_on_private_mutation_does_not_publish() {
        let target = path("uncommitted");
        create_new(&target, &db()).unwrap();
        assert!(execute(&target, "atomic { entries add {owner:2}; entries add {owner:2} }",
            &Params::new()).is_ok());
        let base = snapshot(&target).unwrap().generation;
        assert!(execute(&target, "atomic { entries add {owner:3}; entries add {id:1,owner:4} }",
            &Params::new()).is_err());
        assert_eq!(snapshot(&target).unwrap().generation, base);
        assert_eq!(snapshot(&target).unwrap().query("entries get owner", &Params::new())
            .unwrap().rows.len(), 3);
        cleanup(&target);
    }

    #[test]
    fn nonindexed_update_reuses_persisted_bplus_pages_without_writing_nodes() {
        let target = path("unindexed-delta");
        let mut initial = Database::default();
        initial.execute("table events { id:int key auto\n owner:int required\n score:int required }\nindex by_owner on events(owner)\nevents add {owner:10,score:1}",
            &Params::new()).unwrap();
        create_new(&target, &initial).unwrap();
        let blobs = versions(&target).join("index-pages");
        let old_count = fs::read_dir(&blobs).unwrap().count();
        let generation = snapshot(&target).unwrap().generation;
        execute(&target, "events where id == 1 set score = 99", &Params::new()).unwrap();
        let cold = snapshot(&target).unwrap();
        assert_eq!(cold.generation, generation+1);
        assert_eq!(fs::read_dir(&blobs).unwrap().count(), old_count,
            "nonindexed field edit unexpectedly published dirty B+ pages");
        assert_eq!(cold.query("events get id,score where owner == 10", &Params::new())
            .unwrap().rows[0].get("score"), Some(&Value::Int(99)));
        cleanup(&target);
    }

    #[test]
    fn stable_primary_key_deletion_and_key_update_only_copy_logarithmic_index_paths() {
        use std::collections::BTreeMap;
        let target = path("cow-stable-row-id");
        let mut original = Database::default();
        original.execute("table events { id:int key auto\n owner:int required\n score:int required }\nindex by_owner on events(owner)",
            &Params::new()).unwrap();
        let table = original.tables.get_mut("events").unwrap();
        for id in 1..=720i64 {
            let mut row = BTreeMap::new();
            row.insert("id".into(), Value::Int(id));
            row.insert("owner".into(), Value::Int(id / 3));
            row.insert("score".into(), Value::Int(id));
            table.rows.push(row);
        }
        table.next_auto = 721;
        create_new(&target, &original).unwrap();
        let original_reader = snapshot(&target).unwrap();
        let index_dir = versions(&target).join("index-pages");
        let prior_pages = fs::read_dir(&index_dir).unwrap().count();
        execute(&target, "events where id == 2 delete", &Params::new()).unwrap();
        let deleted_pages = fs::read_dir(&index_dir).unwrap().count() - prior_pages;
        assert!(deleted_pages <= 6, "DELETE rewrote unrelated B+ pages: {deleted_pages}");
        let after_delete = snapshot(&target).unwrap();
        assert_eq!(after_delete.query("events get id where owner == 1", &Params::new())
            .unwrap().rows.len(), 3);
        assert_eq!(after_delete.query("events get id where owner == 0", &Params::new())
            .unwrap().rows.len(), 1);
        assert_eq!(after_delete.query("events get id where id == 3", &Params::new())
            .unwrap().rows.len(), 1);
        execute(&target, "events where id == 719 set owner = 0", &Params::new()).unwrap();
        let after_key_change = snapshot(&target).unwrap();
        assert_eq!(after_key_change.query("events get id where owner == 0", &Params::new())
            .unwrap().rows.len(), 2);
        assert_eq!(original_reader.query("events get id where owner == 0", &Params::new())
            .unwrap().rows.len(), 2);
        assert_eq!(after_key_change.query("events get id", &Params::new()).unwrap().rows.len(), 719);
        cleanup(&target);
    }

    #[test]
    fn persisted_bplus_pages_survive_cold_reopen_and_reuse_unchanged_pages() {
        use std::collections::BTreeSet;
        let target = path("disk-bplus");
        let mut initial = Database::default();
        initial.execute("table logs { id:int key auto\n owner:int required\n score:int required }\nindex owner_idx on logs(owner)",
            &Params::new()).unwrap();
        for i in 0..800usize {
            initial.execute(&format!("logs add {{owner:{},score:{i}}}", i % 80), &Params::new()).unwrap();
        }
        create_new(&target, &initial).unwrap();
        let index_dir = versions(&target).join("index-pages");
        let old_files: BTreeSet<_> = fs::read_dir(&index_dir).unwrap()
            .map(|e| e.unwrap().file_name()).collect();
        assert!(old_files.len() > 3, "expected multiple physical 4 KiB B+ pages");
        let head = read_head(&target).unwrap();
        let file = fs::read(versions(&target).join(head.filename)).unwrap();
        let manifest_pages = u32::from_le_bytes(file[28..32].try_into().unwrap());
        assert!(manifest_pages > 0, "v3 generation omitted persistent B+ manifest");
        let frozen = snapshot(&target).unwrap();
        assert!(frozen.database().tables["logs"].seek_cache.get().is_some());
        let matched = frozen.query("logs get id where owner == 7", &Params::new())
            .unwrap().rows.len();
        assert_eq!(matched, 10);
        execute(&target, "logs add {owner:7,score:1000}", &Params::new()).unwrap();
        let new_files: BTreeSet<_> = fs::read_dir(&index_dir).unwrap()
            .map(|e| e.unwrap().file_name()).collect();
        assert!(new_files.len() > old_files.len());
        let incremental = new_files.difference(&old_files).count();
        assert!(incremental < old_files.len(),
            "index write amplification rewrote more pages than initial index");
        let cold = snapshot(&target).unwrap();
        assert_eq!(cold.query("logs get id where owner == 7", &Params::new())
            .unwrap().rows.len(), 11);
        assert_eq!(frozen.query("logs get id where owner == 7", &Params::new())
            .unwrap().rows.len(), 10);
        execute(&target, "logs where id == 1 set owner = 7", &Params::new()).unwrap();
        assert_eq!(snapshot(&target).unwrap().query("logs get id where owner == 7", &Params::new())
            .unwrap().rows.len(), 12);
        execute(&target, "logs where id == 1 delete", &Params::new()).unwrap();
        assert_eq!(snapshot(&target).unwrap().query("logs get id where owner == 7", &Params::new())
            .unwrap().rows.len(), 11);
        cleanup(&target);
    }

    #[test]
    fn corrupted_persisted_index_page_fails_closed() {
        let target = path("bplus-corrupt");
        create_new(&target, &db()).unwrap();
        let saved = snapshot(&target).unwrap();
        let blobs = versions(&target).join("index-pages");
        let page = fs::read_dir(&blobs).unwrap().next().unwrap().unwrap().path();
        let mut file = OpenOptions::new().read(true).write(true).open(&page).unwrap();
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(71)).unwrap();
        file.write_all(&[255]).unwrap();
        file.sync_all().unwrap();
        assert!(snapshot(&target).is_err(), "corrupt B+ index page was silently ignored");
        assert_eq!(saved.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        cleanup(&target);
    }

    #[test]
    fn index_pages_written_before_failed_head_publish_do_not_change_visible_snapshot() {
        let target = path("bplus-fault");
        create_new(&target, &db()).unwrap();
        let generation = snapshot(&target).unwrap().generation;
        assert!(execute_internal(&target, "entries add {owner:2}", &Params::new(),
            FaultPoint::BeforePublish).is_err());
        let persisted = snapshot(&target).unwrap();
        assert_eq!(persisted.generation, generation);
        assert_eq!(persisted.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        cleanup(&target);
    }

    #[test]
    fn stale_generation_cannot_overwrite_durable_committed_rows() {
        let target = path("optimistic-cas");
        create_new(&target, &db()).unwrap();
        let pinned = snapshot(&target).unwrap();
        let (next, _) = execute_if_generation(
            &target, pinned.generation, "entries add {owner:2}", &Params::new()
        ).unwrap();
        assert_eq!(next, pinned.generation + 1);
        assert!(matches!(
            execute_if_generation(&target, pinned.generation,
                "entries add {owner:3}", &Params::new()),
            Err(V3Error::Conflict(_))
        ));
        let fresh = snapshot(&target).unwrap();
        assert_eq!(fresh.generation, next);
        assert_eq!(fresh.query("entries get owner", &Params::new()).unwrap().rows.len(), 2);
        assert_eq!(pinned.query("entries get owner", &Params::new()).unwrap().rows.len(), 1);
        cleanup(&target);
    }

    #[test]
    fn default_auto_create_is_v3_only_and_refuses_unknown_existing_format() {
        let target = path("default");
        let report = execute_or_create(&target, "table messages { id:int key auto\n title:text required }\nmessages add {title:\"first\"}", &Params::new()).unwrap();
        assert_eq!(report.affected, 1);
        assert_eq!(snapshot(&target).unwrap().query("messages get title", &Params::new()).unwrap().rows.len(), 1);
        let old = path("not-v3");
        fs::write(&old, b"NQDB0002").unwrap();
        assert!(execute_or_create(&old, "table messages { id:int key }", &Params::new()).is_err());
        assert_eq!(fs::read(&old).unwrap(), b"NQDB0002");
        cleanup(&old);
        cleanup(&target);
    }


    #[test]
    fn authorized_v3_transactions_do_not_publish_denied_or_over_budget_writes() {
        use crate::authorization::AccessControl;
        let target = path("authorized");
        create_new(&target, &db()).unwrap();
        let deny = AccessControl::new();
        assert!(matches!(
            execute_authorized(&target, "entries add {owner:1}", &Params::new(), &deny),
            Err(V3Error::Access(_))
        ));
        assert_eq!(inspect(&target).unwrap().0, 1);
        let mut scoped = AccessControl::new();
        scoped.grant("entries", true, true);
        scoped.restrict_rows("entries", "owner", Value::Int(1));
        assert!(matches!(
            execute_authorized(&target, "entries add {owner:2}", &Params::new(), &scoped),
            Err(V3Error::Access(_))
        ));
        assert!(matches!(
            execute_authorized(&target, "entries where id == 1 set owner = 2", &Params::new(), &scoped),
            Err(V3Error::Access(_))
        ));
        assert_eq!(inspect(&target).unwrap().0, 1);
        execute_authorized(&target, "entries add {owner:1}", &Params::new(), &scoped).unwrap();
        assert_eq!(inspect(&target).unwrap().0, 2);
        assert_eq!(
            execute_authorized(&target, "entries get owner", &Params::new(), &scoped)
                .unwrap().rows.len(), 2
        );
        let mut capped = scoped.clone();
        capped.max_returned_rows = 0;
        assert!(matches!(
            execute_authorized(
                &target,
                "atomic { entries add {owner:1}; entries get id }",
                &Params::new(),
                &capped,
            ),
            Err(V3Error::Access(_))
        ));
        assert_eq!(inspect(&target).unwrap().0, 2,
            "the over-budget transaction must not be published");
        assert_eq!(
            snapshot(&target).unwrap().query("entries get id", &Params::new())
                .unwrap().rows.len(), 2
        );
        cleanup(&target);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn v3_files_are_private_and_final_symlinks_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let target = path("private-files");
        create_new(&target, &db()).unwrap();
        let _ = snapshot(&target).unwrap();
        let head = read_head(&target).unwrap();
        let version_dir = versions(&target);
        let index_dir = version_dir.join("index-pages");
        let blob = fs::read_dir(&index_dir).unwrap().next().unwrap().unwrap().path();
        let files = [target.clone(), lock_name(&target),
            version_dir.join(&head.filename), blob.clone()];
        for file in &files {
            let mode = fs::metadata(file).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "v3 data exposed to group/other: {}", file.display());
        }
        for dir in [&version_dir, &index_dir] {
            let mode = fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "v3 directory is not private: {}", dir.display());
        }
        let alias = path("symlink-to-head");
        symlink(&target, &alias).unwrap();
        assert!(read_head(&alias).is_err(), "HEAD must not follow symlinks");
        fs::remove_file(alias).unwrap();

        let shadow = blob.with_extension("shadow");
        fs::rename(&blob, &shadow).unwrap();
        symlink(&shadow, &blob).unwrap();
        assert!(snapshot(&target).is_err(), "index blob symlink must not be followed");
        fs::remove_file(&blob).unwrap();
        fs::rename(&shadow, &blob).unwrap();
        assert!(snapshot(&target).is_ok());
        cleanup(&target);
    }

}
