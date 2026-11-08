//! Content-addressed, immutable v3 *physical* relational snapshot pages.
//!
//! This avoids rewriting 4 KiB blocks that have identical bytes in previous
//! generations, but the caller still serializes the entire logical database.
//! It is NOT a row/record pager, WAL allocator, or online compactor.
//!
//! All blobs are immutable SHA-256-addressed, fully fsynced before HEAD may
//! refer to them. The parent data-pages directory is fsynced once per write.
//! Interrupted writes may leave validated orphan blobs/temporary files for
//! the explicit v3 vacuum operation.
use crate::hash256::sha256;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const PAGE_BYTES: usize = 4096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
type Result<T> = std::result::Result<T, String>;
fn err(value: impl std::fmt::Display) -> String { value.to_string() }

pub(crate) fn data_dir(versions: &Path) -> PathBuf { versions.join("data-pages") }

pub(crate) fn digest_hex(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest { output.push_str(&format!("{byte:02x}")); }
    output
}

pub(crate) fn safe_blob_name(name: &str) -> bool {
    name.len() == 68 && name.ends_with(".bpd") &&
        name.as_bytes()[..64].iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub(crate) fn safe_temp_name(name: &str) -> bool {
    let Some(stem) = name.strip_prefix(".bpd-").and_then(|s| s.strip_suffix(".tmp")) else {
        return false;
    };
    let mut parts = stem.split('-');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(pid), Some(nanos), Some(seq), None) =>
            pid.parse::<u32>().is_ok() &&
            nanos.parse::<u128>().is_ok() &&
            seq.parse::<u64>().is_ok() &&
            [pid,nanos,seq].iter().all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())),
        _ => false,
    }
}

fn sync_directory(dir: &Path) -> Result<()> {
    File::open(dir).and_then(|file| file.sync_all()).map_err(err)
}
fn checked_dir(versions: &Path) -> Result<PathBuf> {
    let directory = data_dir(versions);
    if !directory.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => sync_directory(versions)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(e) => return Err(err(e)),
        }
    }
    if !fs::symlink_metadata(&directory).map_err(err)?.file_type().is_dir() {
        return Err("v3 data-pages path must be a regular directory".into());
    }
    Ok(directory)
}
fn open_blob(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // O_NOFOLLOW
    }
    let file = options.open(path).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("v3 data blob must be a regular file".into());
    }
    Ok(file)
}

pub(crate) fn read_page(versions: &Path, digest: &[u8; 32]) -> Result<[u8; PAGE_BYTES]> {
    let directory = data_dir(versions);
    let path = directory.join(format!("{}.bpd", digest_hex(digest)));
    let mut file = open_blob(&path)?;
    if file.metadata().map_err(err)?.len() != PAGE_BYTES as u64 {
        return Err("v3 data page has invalid length".into());
    }
    let mut block = [0u8; PAGE_BYTES];
    file.read_exact(&mut block).map_err(err)?;
    if sha256(&block) != *digest {
        return Err("v3 content-addressed data page SHA256 mismatch".into());
    }
    Ok(block)
}

fn publish_page(directory: &Path, block: &[u8; PAGE_BYTES]) -> Result<[u8; 32]> {
    let digest = sha256(block);
    let path = directory.join(format!("{}.bpd", digest_hex(&digest)));
    let verify_existing = || -> Result<()> {
        let mut file = open_blob(&path)?;
        if file.metadata().map_err(err)?.len() != PAGE_BYTES as u64 {
            return Err("existing v3 data page has invalid length".into());
        }
        let mut existing = [0u8; PAGE_BYTES];
        file.read_exact(&mut existing).map_err(err)?;
        if existing != *block {
            return Err("existing v3 data page hash collision or corruption".into());
        }
        Ok(())
    };
    if path.exists() {
        verify_existing()?;
        return Ok(digest);
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_err(err)?.as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".bpd-{}-{nanos}-{sequence}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(err)?;
        file.write_all(block).map_err(err)?;
        file.sync_all().map_err(err)?;
        drop(file);
        match fs::hard_link(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => verify_existing(),
            Err(e) => Err(err(e)),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(digest)
}

/// Write only not-yet-present immutable physical blocks. All references
/// become durable BEFORE the generation file and HEAD are published.
pub(crate) fn publish_pages(versions: &Path, payload: &[u8]) -> Result<Vec<[u8; 32]>> {
    let directory = checked_dir(versions)?;
    let mut digests = Vec::with_capacity(payload.len().div_ceil(PAGE_BYTES));
    for chunk in payload.chunks(PAGE_BYTES) {
        let mut block = [0u8; PAGE_BYTES];
        block[..chunk.len()].copy_from_slice(chunk);
        digests.push(publish_page(&directory, &block)?);
    }
    sync_directory(&directory)?;
    Ok(digests)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filename_validation_rejects_unsafe_and_noncanonical_entries() {
        assert!(safe_blob_name(&format!("{}.bpd", "0".repeat(64))));
        assert!(!safe_blob_name(&format!("{}.bpd", "A".repeat(64))));
        assert!(!safe_blob_name("../unsafe.bpd"));
        assert!(!safe_blob_name(&format!("{}.bpd/evil", "1".repeat(64))));
        assert!(safe_temp_name(".bpd-123-456-789.tmp"));
        for name in [".bpd-1-2.tmp", ".bpd-1-2-3.tmp/x", ".bpd-1-2--3.tmp",
                     ".bpd-1-2-3.bpd", ".bpd-1-2-3-4.tmp"] {
            assert!(!safe_temp_name(name), "{name}");
        }
    }
}
