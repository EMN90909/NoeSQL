//! Deny-by-default authorizer for embedded NoeSQL sessions.
//!
//! Production API users must expose only an AuthorizedSession, not unrestricted
//! Database::execute. No authentication, process isolation, RLS or cryptographic
//! audit chain is claimed. The unguarded Database API is administrative.
use crate::reference::{parse, Database, ExecutionReport, Params, ReferenceError, Select, Statement};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Default)]
pub struct TableGrant { pub read: bool, pub write: bool }

#[derive(Clone, Debug, Default)]
pub struct AccessControl {
    pub schema_admin: bool,
    grants: BTreeMap<String, TableGrant>,
    pub max_source_bytes: usize,
    pub max_returned_rows: usize,
}

#[derive(Debug)]
pub enum AccessError {
    Denied(String),
    Budget(String),
    Query(ReferenceError),
}
impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(reason) => write!(f, "NOE403: {reason}"),
            Self::Budget(reason) => write!(f, "NOE429: {reason}"),
            Self::Query(reason) => write!(f, "{reason}"),
        }
    }
}
impl std::error::Error for AccessError {}
impl From<ReferenceError> for AccessError {
    fn from(error: ReferenceError) -> Self { Self::Query(error) }
}

impl AccessControl {
    pub fn new() -> Self {
        Self {
            schema_admin: false,
            grants: BTreeMap::new(),
            max_source_bytes: 64 * 1024,
            max_returned_rows: 10_000,
        }
    }
    pub fn grant(&mut self, table: impl Into<String>, read: bool, write: bool) {
        self.grants.insert(table.into(), TableGrant { read, write });
    }
    fn table(&self, name: &str, write: bool) -> Result<(), AccessError> {
        let grant = self.grants.get(name).copied().unwrap_or_default();
        if if write { grant.write } else { grant.read } { Ok(()) } else {
            Err(AccessError::Denied(format!(
                "{} access denied to table {name}", if write { "write" } else { "read" }
            )))
        }
    }
    fn select(&self, select: &Select) -> Result<(), AccessError> {
        self.table(&select.table, false)?;
        if let Some(join) = &select.join { self.table(&join.table, false)?; }
        Ok(())
    }
    fn check(&self, statement: &Statement) -> Result<(), AccessError> {
        match statement {
            Statement::Get(select) => self.select(select),
            Statement::Add(table, _) | Statement::Set(table, _, _) |
            Statement::Delete(table, _) => self.table(table, true),
            Statement::Atomic(body) => {
                for statement in body { self.check(statement)?; }
                Ok(())
            },
            Statement::Database(_) | Statement::Table(_, _) | Statement::AlterAdd(_, _) |
            Statement::Index { .. } | Statement::Query { .. } => {
                if self.schema_admin { Ok(()) }
                else { Err(AccessError::Denied("schema administration requires explicit privilege".into())) }
            },
        }
    }

    pub fn execute(
        &self,
        database: &mut Database,
        source: &str,
        params: &Params,
    ) -> Result<ExecutionReport, AccessError> {
        if source.len() > self.max_source_bytes {
            return Err(AccessError::Budget("statement exceeds source-byte budget".into()));
        }
        let statements = parse(source)?;
        for statement in &statements { self.check(statement)?; }
        // Do not replace database on rejected budget or execution failure.
        let mut working = database.clone();
        let result = working.execute_parsed(&statements, params)?;
        if result.rows.len() > self.max_returned_rows {
            return Err(AccessError::Budget("result exceeds row budget".into()));
        }
        *database = working;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_by_default_and_nested_atomic_is_not_a_privilege_bypass() {
        let mut db = Database::default();
        db.execute(
            "table users { id:int key auto\n value:int required }\nusers add {value:5}",
            &Params::new(),
        ).unwrap();
        let mut acl = AccessControl::new();
        assert!(acl.execute(&mut db, "users get id,value", &Params::new()).is_err());
        acl.grant("users", true, false);
        assert_eq!(acl.execute(&mut db, "users get id,value", &Params::new()).unwrap().rows.len(), 1);
        assert!(acl.execute(&mut db, "atomic { users add {value:7} }", &Params::new()).is_err());
        assert!(acl.execute(&mut db, "table secrets { id:int key }", &Params::new()).is_err());
        let rows = db.execute("users get id,value", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn join_checks_both_tables_and_output_budget_does_not_commit_writes() {
        let mut db = Database::default();
        db.execute("table a { id:int key auto }\ntable b { id:int key auto }\na add {}\nb add {}",
            &Params::new()).unwrap();
        let mut acl = AccessControl::new();
        acl.grant("a", true, true);
        assert!(acl.execute(&mut db, "a get a.id,b.id join b on a.id == b.id", &Params::new()).is_err());
        acl.grant("b", true, false);
        let rows = acl.execute(&mut db, "a get a.id,b.id join b on a.id == b.id", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        acl.max_returned_rows = 0;
        assert!(acl.execute(&mut db, "a get id", &Params::new()).is_err());
        assert!(acl.execute(&mut db, "a add {}", &Params::new()).is_ok());
        assert_eq!(db.execute("a get id", &Params::new()).unwrap().rows.len(), 2);
    }
}
