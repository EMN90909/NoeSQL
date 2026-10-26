//! In-process multiversion snapshot isolation.
//!
//! Readers pin an immutable Arc<Database> generation and never block writers.
//! Writers build an isolated copy and use optimistic generation validation
//! before publishing. This is a process-local component, NOT disk MVCC; it
//! must not be used by multiple independent processes without a durable
//! generation/CAS backend and WAL-integrated commit protocol.
use crate::reference::{parse, Database, ExecutionReport, Params, ReferenceError, Statement};
use std::fmt;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub enum MvccError {
    Sql(ReferenceError),
    Conflict { expected: u64, current: u64 },
    VersionExhausted,
    Poisoned,
}
impl fmt::Display for MvccError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(error) => write!(f, "{error}"),
            Self::Conflict { expected, current } =>
                write!(f, "write conflict: transaction began at {expected}, current generation is {current}"),
            Self::VersionExhausted => f.write_str("MVCC generation exhausted"),
            Self::Poisoned => f.write_str("MVCC lock poisoned"),
        }
    }
}
impl std::error::Error for MvccError {}
impl From<ReferenceError> for MvccError {
    fn from(value: ReferenceError) -> Self { Self::Sql(value) }
}

struct Head {
    generation: u64,
    database: Arc<Database>,
}

#[derive(Clone)]
pub struct MvccStore { head: Arc<RwLock<Head>> }

#[derive(Clone)]
pub struct ReadSnapshot {
    generation: u64,
    database: Arc<Database>,
}

impl ReadSnapshot {
    pub fn generation(&self) -> u64 { self.generation }
    pub fn query(&self, source: &str, params: &Params) -> Result<ExecutionReport, MvccError> {
        let statements = parse(source)?;
        Ok(self.database.query_parsed(&statements, params)?)
    }
}

pub struct Transaction {
    head: Arc<RwLock<Head>>,
    base: ReadSnapshot,
    pending: Option<Database>,
    modified: bool,
}

impl MvccStore {
    pub fn new(database: Database) -> Self {
        Self {
            head: Arc::new(RwLock::new(Head {
                generation: 0, database: Arc::new(database),
            })),
        }
    }

    pub fn snapshot(&self) -> Result<ReadSnapshot, MvccError> {
        let guard = self.head.read().map_err(|_| MvccError::Poisoned)?;
        Ok(ReadSnapshot { generation: guard.generation, database: guard.database.clone() })
    }

    pub fn begin(&self) -> Result<Transaction, MvccError> {
        Ok(Transaction {
            head: self.head.clone(),
            base: self.snapshot()?,
            pending: None,
            modified: false,
        })
    }
}

impl Transaction {
    pub fn base_generation(&self) -> u64 { self.base.generation }

    pub fn execute(&mut self, source: &str, params: &Params) -> Result<ExecutionReport, MvccError> {
        let statements = parse(source)?;
        if Database::statements_read_only(&statements) {
            // An unmodified transaction can query its pinned immutable view.
            return match &self.pending {
                Some(database) => Ok(database.query_parsed(&statements, params)?),
                None => Ok(self.base.database.query_parsed(&statements, params)?),
            };
        }
        // Statement-level errors leave this transaction's previous successful
        // statements intact, since execute_parsed is atomic.
        let database = self.pending.get_or_insert_with(|| (*self.base.database).clone());
        let result = database.execute_parsed(&statements, params)?;
        self.modified = true;
        Ok(result)
    }

    pub fn rollback(self) {}

    pub fn commit(self) -> Result<u64, MvccError> {
        if !self.modified {
            return Ok(self.base.generation);
        }
        let mut head = self.head.write().map_err(|_| MvccError::Poisoned)?;
        if head.generation != self.base.generation {
            return Err(MvccError::Conflict {
                expected: self.base.generation, current: head.generation,
            });
        }
        let next = head.generation.checked_add(1).ok_or(MvccError::VersionExhausted)?;
        head.database = Arc::new(self.pending.expect("modified transaction owns pending data"));
        head.generation = next;
        Ok(next)
    }
}


/* A principal-facing façade deliberately never exposes the unrestricted
   Transaction or Database. Snapshot permissions are evaluated on a frozen
   generation; writes are published only after both query and output budgets
   succeed. Like MvccStore, this is process-local, NOT durable RLS/MVCC. */
use crate::authorization::{AccessControl, AccessError};

#[derive(Debug)]
pub enum AuthorizedMvccError {
    Access(AccessError),
    Mvcc(MvccError),
}
impl std::fmt::Display for AuthorizedMvccError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Access(e) => write!(f, "{e}"), Self::Mvcc(e) => write!(f, "{e}") }
    }
}
impl std::error::Error for AuthorizedMvccError {}
impl From<AccessError> for AuthorizedMvccError {
    fn from(e: AccessError) -> Self { Self::Access(e) }
}
impl From<MvccError> for AuthorizedMvccError {
    fn from(e: MvccError) -> Self { Self::Mvcc(e) }
}
impl From<ReferenceError> for AuthorizedMvccError {
    fn from(e: ReferenceError) -> Self { Self::Mvcc(MvccError::Sql(e)) }
}

#[derive(Clone)]
pub struct AuthorizedMvccStore {
    store: MvccStore,
    access: AccessControl,
}
pub struct AuthorizedReadSnapshot {
    snapshot: ReadSnapshot,
    access: AccessControl,
}
pub struct AuthorizedTransaction {
    inner: Transaction,
    access: AccessControl,
}

impl AuthorizedMvccStore {
    pub fn new(store: MvccStore, access: AccessControl) -> Self { Self { store, access } }
    pub fn snapshot(&self) -> Result<AuthorizedReadSnapshot, AuthorizedMvccError> {
        Ok(AuthorizedReadSnapshot { snapshot: self.store.snapshot()?, access: self.access.clone() })
    }
    pub fn begin(&self) -> Result<AuthorizedTransaction, AuthorizedMvccError> {
        Ok(AuthorizedTransaction { inner: self.store.begin()?, access: self.access.clone() })
    }
}
impl AuthorizedReadSnapshot {
    pub fn generation(&self) -> u64 { self.snapshot.generation() }
    pub fn query(&self, source: &str, params: &Params) -> Result<ExecutionReport, AuthorizedMvccError> {
        let statements = self.access.prepare(source, params)?;
        if !Database::statements_read_only(&statements) {
            return Err(AccessError::Denied("snapshot is read-only".into()).into());
        }
        let result = self.snapshot.database.query_parsed(&statements, params)?;
        self.access.validate_result(&result)?;
        Ok(result)
    }
}
impl AuthorizedTransaction {
    pub fn base_generation(&self) -> u64 { self.inner.base_generation() }
    pub fn execute(&mut self, source: &str, params: &Params) -> Result<ExecutionReport, AuthorizedMvccError> {
        let statements = self.access.prepare(source, params)?;
        if Database::statements_read_only(&statements) {
            let database = match &self.inner.pending {
                Some(pending) => pending,
                None => self.inner.base.database.as_ref(),
            };
            let result = database.query_parsed(&statements, params)?;
            self.access.validate_result(&result)?;
            return Ok(result);
        }
        // Stage the complete program separately. In particular, a result
        // budget failure cannot leave partial writes pending for a later commit.
        let mut working = match &self.inner.pending {
            Some(pending) => pending.clone(),
            None => (*self.inner.base.database).clone(),
        };
        let result = working.execute_parsed(&statements, params)?;
        self.access.validate_result(&result)?;
        self.inner.pending = Some(working);
        self.inner.modified = true;
        Ok(result)
    }
    pub fn rollback(self) { self.inner.rollback(); }
    pub fn commit(self) -> Result<u64, AuthorizedMvccError> { Ok(self.inner.commit()?) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use std::sync::Barrier;
    use std::thread;

    fn seed() -> Database {
        let mut db = Database::default();
        db.execute(
            "table records { id:int key auto\n owner:int required }\nrecords add {owner:1}",
            &Params::new(),
        ).unwrap();
        db
    }

    #[test]
    fn snapshot_is_stable_across_writer_commits() {
        let store = MvccStore::new(seed());
        let old = store.snapshot().unwrap();
        let mut tx = store.begin().unwrap();
        tx.execute("records add {owner:2}", &Params::new()).unwrap();
        assert_eq!(tx.commit().unwrap(), 1);

        let prior = old.query("records get id,owner sort id", &Params::new()).unwrap();
        let latest = store.snapshot().unwrap();
        assert_eq!(prior.rows.len(), 1);
        assert_eq!(latest.query("records get id,owner", &Params::new()).unwrap().rows.len(), 2);
        assert_eq!(old.generation(), 0);
        assert_eq!(latest.generation(), 1);
    }

    #[test]
    fn stale_writer_cannot_overwrite_newer_committed_generation() {
        let store = MvccStore::new(seed());
        let mut first = store.begin().unwrap();
        let mut second = store.begin().unwrap();
        first.execute("records add {owner:3}", &Params::new()).unwrap();
        second.execute("records add {owner:4}", &Params::new()).unwrap();
        assert_eq!(first.commit().unwrap(), 1);
        assert!(matches!(second.commit(), Err(MvccError::Conflict { expected: 0, current: 1 })));
        let rows = store.snapshot().unwrap()
            .query("records get owner sort owner", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].get("owner"), Some(&Value::Int(3)));
    }

    #[test]
    fn concurrent_readers_are_nonblocking_and_writes_validate_generations() {
        const READERS: usize = 24;
        let store = MvccStore::new(seed());
        let gate = Arc::new(Barrier::new(READERS + 1));
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let other = store.clone();
            let gate = gate.clone();
            readers.push(thread::spawn(move || {
                let snapshot = other.snapshot().unwrap();
                gate.wait();
                for _ in 0..100 {
                    let rows = snapshot.query("records get id,owner", &Params::new()).unwrap().rows;
                    assert_eq!(rows.len(), 1);
                }
            }));
        }
        gate.wait();
        let mut tx = store.begin().unwrap();
        tx.execute("records add {owner:5}", &Params::new()).unwrap();
        assert_eq!(tx.commit().unwrap(), 1);
        for join in readers { join.join().unwrap(); }
        assert_eq!(store.snapshot().unwrap()
            .query("records get id", &Params::new()).unwrap().rows.len(), 2);
    }

    #[test]
    fn read_only_snapshot_refuses_schema_mutation() {
        let store = MvccStore::new(seed());
        let snapshot = store.snapshot().unwrap();
        assert!(snapshot.query("records add {owner:7}", &Params::new()).is_err());
        let mut tx = store.begin().unwrap();
        assert!(tx.execute("records add {owner:99}\nrecords add {owner:1}", &Params::new()).is_ok());
        // Explicit rollback does not publish any rows.
        tx.rollback();
        assert_eq!(store.snapshot().unwrap()
            .query("records get id", &Params::new()).unwrap().rows.len(), 1);
    }
    #[test]
    fn protected_mvcc_snapshots_cannot_cross_tenants_or_commit_denied_results() {
        let mut acl = AccessControl::new();
        acl.grant("records", true, true);
        acl.restrict_rows("records", "owner", Value::Int(1));
        let secure = AuthorizedMvccStore::new(MvccStore::new(seed()), acl);
        let old = secure.snapshot().unwrap();
        let mut tx = secure.begin().unwrap();
        assert!(tx.execute("records add {owner:2}", &Params::new()).is_err());
        tx.execute("records add {owner:1}", &Params::new()).unwrap();
        assert_eq!(tx.commit().unwrap(), 1);
        assert_eq!(old.query("records get owner", &Params::new()).unwrap().rows.len(), 1);
        assert_eq!(secure.snapshot().unwrap()
            .query("records get owner", &Params::new()).unwrap().rows.len(), 2);
        assert!(old.query("records add {owner:1}", &Params::new()).is_err());
    }

    #[test]
    fn protected_mvcc_output_budget_failure_cannot_later_be_committed() {
        let mut acl = AccessControl::new();
        acl.grant("records", true, true);
        acl.max_returned_rows = 0;
        let secure = AuthorizedMvccStore::new(MvccStore::new(seed()), acl);
        let mut tx = secure.begin().unwrap();
        assert!(matches!(tx.execute("atomic { records add {owner:5}; records get owner }",
            &Params::new()), Err(AuthorizedMvccError::Access(AccessError::Budget(_)))));
        assert_eq!(tx.commit().unwrap(), 0);
        // Restore reads with a separately configured view of the same store.
        assert_eq!(secure.store.snapshot().unwrap()
            .query("records get owner", &Params::new()).unwrap().rows.len(), 1);
    }

}
