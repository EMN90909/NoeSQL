use crate::zig_pager::{Pager, PAGE_BYTES};
use crate::{validate_database_path, Error};
use std::path::Path;

const MAGIC: [u8; 8] = *b"NQNR0001";
const STORE_MAGIC: [u8; 8] = *b"NQRO0001";

unsafe extern "C" {
    fn noesqlNativeBridgeVersion() -> u32;
    fn noesqlStage2BoolRow(
        operation: i64,
        present: i64,
        current_id: i64,
        current_enabled: i64,
        requested_id: i64,
        requested_enabled: i64,
    ) -> i64;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    id: i64,
    enabled: bool,
}

pub fn api_version() -> u32 {
    unsafe { noesqlNativeBridgeVersion() }
}

fn decode_row(page: &[u8; PAGE_BYTES]) -> Result<Option<Row>, Error> {
    if page.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if page[..MAGIC.len()] != MAGIC {
        return Err(Error::Provider(-2));
    }
    if page[16] > 1 {
        return Err(Error::Provider(-2));
    }

    let mut id = [0u8; 8];
    id.copy_from_slice(&page[8..16]);
    Ok(Some(Row {
        id: i64::from_le_bytes(id),
        enabled: page[16] != 0,
    }))
}

fn encode_row(row: Option<Row>) -> [u8; PAGE_BYTES] {
    let mut page = [0u8; PAGE_BYTES];
    if let Some(row) = row {
        page[..MAGIC.len()].copy_from_slice(&MAGIC);
        page[8..16].copy_from_slice(&row.id.to_le_bytes());
        page[16] = u8::from(row.enabled);
    }
    page
}

fn stage_and_commit(pager: &Pager, page_id: u32, row: Option<Row>) -> Result<(), Error> {
    let page = encode_row(row);
    if pager.read_page(0)?.is_none() {
        let mut marker = [0u8; PAGE_BYTES];
        marker[..STORE_MAGIC.len()].copy_from_slice(&STORE_MAGIC);
        if let Err(error) = pager.stage_page(0, &marker) {
            let _ = pager.rollback();
            return Err(error);
        }
    }
    if let Err(error) = pager.stage_page(page_id, &page) {
        let _ = pager.rollback();
        return Err(error);
    }
    if let Err(error) = pager.commit() {
        let _ = pager.rollback();
        return Err(error);
    }
    Ok(())
}

fn execute(
    path: &Path,
    page_id: u32,
    row_id: i64,
    enabled: bool,
    operation: i64,
) -> Result<Option<bool>, Error> {
    validate_database_path(path)?;
    if page_id == 0 || page_id >= 256 {
        return Err(Error::Provider(-211));
    }
    if !(1..=4).contains(&operation) {
        return Err(Error::Provider(-1));
    }

    let pager = Pager::open(path)?;
    if let Some(header) = pager.read_page(0)? {
        if header[..8] != STORE_MAGIC || header[8..].iter().any(|b| *b != 0) {
            return Err(Error::Codec("database is not a Noqeri raw-row store".into()));
        }
    }
    let current = match pager.read_page(page_id)? {
        Some(page) => decode_row(&page)?,
        None => None,
    };

    let (present, current_id, current_enabled) = match current {
        Some(row) => (1, row.id, if row.enabled { 1 } else { 0 }),
        None => (0, 0, 0),
    };

    let result = unsafe {
        noesqlStage2BoolRow(
            operation,
            present,
            current_id,
            current_enabled,
            row_id,
            if enabled { 1 } else { 0 },
        )
    };

    match operation {
        1 => match result {
            100 => Ok(Some(false)),
            101 => Ok(Some(true)),
            110 => Ok(None),
            other => Err(Error::Provider(other as i32)),
        },
        2 => match result {
            200 | 201 => {
                stage_and_commit(
                    &pager,
                    page_id,
                    Some(Row {
                        id: row_id,
                        enabled: result == 201,
                    }),
                )?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        3 => match result {
            300 | 301 => {
                stage_and_commit(
                    &pager,
                    page_id,
                    Some(Row {
                        id: row_id,
                        enabled: result == 301,
                    }),
                )?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        4 => match result {
            400 => {
                stage_and_commit(&pager, page_id, None)?;
                Ok(None)
            }
            other => Err(Error::Provider(other as i32)),
        },
        _ => unreachable!(),
    }
}

pub fn insert_bool_row(path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(path, page_id, row_id, enabled, 2).map(|_| ())
}

pub fn get_bool_row(path: &Path, page_id: u32, row_id: i64) -> Result<Option<bool>, Error> {
    execute(path, page_id, row_id, false, 1)
}

pub fn set_bool_row(path: &Path, page_id: u32, row_id: i64, enabled: bool) -> Result<(), Error> {
    execute(path, page_id, row_id, enabled, 3).map(|_| ())
}

pub fn delete_bool_row(path: &Path, page_id: u32, row_id: i64) -> Result<(), Error> {
    execute(path, page_id, row_id, false, 4).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_codec_is_stable_and_rejects_bad_magic() {
        let row = Row { id: 44, enabled: true };
        let page = encode_row(Some(row));
        assert_eq!(decode_row(&page).unwrap(), Some(row));
        assert_eq!(decode_row(&encode_row(None)).unwrap(), None);

        let mut bad = page;
        bad[0] = b'X';
        assert!(decode_row(&bad).is_err());
    }
}
