//! Deny-by-default authorizer for embedded NoeSQL sessions.
//!
//! Production API users must expose only an AuthorizedSession, not unrestricted
//! Database::execute. No authentication, process isolation, RLS or cryptographic
//! audit chain is claimed. The unguarded Database API is administrative.
use crate::reference::{parse, Database, ExecutionReport, Params, ReferenceError, Expr, Select, Statement};
use std::collections::BTreeMap;
use crate::value::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, Default)]
pub struct TableGrant { pub read: bool, pub write: bool }

#[derive(Clone, Debug)]
struct RowPolicy { field: String, equals: Value }

#[derive(Clone, Debug)]
pub struct AccessControl {
    pub schema_admin: bool,
    grants: BTreeMap<String, TableGrant>,
    row_policies: BTreeMap<String, RowPolicy>,
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

impl Default for AccessControl {
    fn default() -> Self { Self::new() }
}

impl AccessControl {
    pub fn new() -> Self {
        Self {
            schema_admin: false,
            grants: BTreeMap::new(),
            row_policies: BTreeMap::new(),
            max_source_bytes: 64 * 1024,
            max_returned_rows: 10_000,
        }
    }
    pub fn grant(&mut self, table: impl Into<String>, read: bool, write: bool) {
        self.grants.insert(table.into(), TableGrant { read, write });
    }

    /// Require every visible/mutated row to match an immutable tenant value.
    /// This scoped mode refuses JOINs involving policy-protected tables.
    pub fn restrict_rows(&mut self, table: impl Into<String>, field: impl Into<String>, equals: Value) {
        self.row_policies.insert(table.into(), RowPolicy { field: field.into(), equals });
    }

    fn constrained(&self, table: &str, filter: Expr) -> Expr {
        if let Some(policy) = self.row_policies.get(table) {
            let condition = Expr::Binary(
                "==".into(),
                Box::new(Expr::Column(policy.field.clone())),
                Box::new(Expr::Literal(policy.equals.clone())),
            );
            Expr::Binary("&&".into(), Box::new(filter), Box::new(condition))
        } else { filter }
    }

    fn scope_statement(&self, statement: &mut Statement, params: &Params) -> Result<(), AccessError> {
        match statement {
            Statement::Get(select) => {
                if select.join.as_ref().is_some_and(|join|
                    self.row_policies.contains_key(&select.table) ||
                    self.row_policies.contains_key(&join.table))
                {
                    return Err(AccessError::Denied("row policies on joined tables require a verified join-aware executor".into()));
                }
                if self.row_policies.contains_key(&select.table) {
                    let old = select.filter.take().unwrap_or(Expr::Literal(Value::Bool(true)));
                    select.filter = Some(self.constrained(&select.table, old));
                }
            },
            Statement::Set(table, filter, columns) => {
                if let Some(policy) = self.row_policies.get(table) {
                    if columns.iter().any(|(field, _)| field == &policy.field) {
                        return Err(AccessError::Denied("cannot update a row-policy tenant column".into()));
                    }
                }
                *filter = self.constrained(table, filter.clone());
            },
            Statement::Delete(table, filter) => {
                *filter = self.constrained(table, filter.clone());
            },
            Statement::Add(table, columns) => {
                if let Some(policy) = self.row_policies.get(table) {
                    let tenant = columns.iter().find(|(field, _)| field == &policy.field);
                    let matching = match tenant {
                        Some((_, Expr::Literal(value))) => value == &policy.equals,
                        Some((_, Expr::Param(name))) => params.get(name) == Some(&policy.equals),
                        _ => false,
                    };
                    if !matching {
                        return Err(AccessError::Denied("insert must supply the exact authorized row-policy value".into()));
                    }
                }
            },
            Statement::Atomic(statements) => {
                for nested in statements { self.scope_statement(nested, params)?; }
            },
            _ => {},
        }
        Ok(())
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

    /// Authorize and scope a complete program before any statement can run.
    /// Also shared by the protected MVCC facade; it must never run the
    /// unspecialized AST returned by `parse` for a restricted principal.
    pub(crate) fn prepare(&self, source: &str, params: &Params) -> Result<Vec<Statement>, AccessError> {
        if source.len() > self.max_source_bytes {
            return Err(AccessError::Budget("statement exceeds source-byte budget".into()));
        }
        let mut statements = parse(source)?;
        for statement in &statements { self.check(statement)?; }
        for statement in &mut statements { self.scope_statement(statement, params)?; }
        Ok(statements)
    }

    pub(crate) fn validate_result(&self, result: &ExecutionReport) -> Result<(), AccessError> {
        if result.rows.len() > self.max_returned_rows {
            Err(AccessError::Budget("result exceeds row budget".into()))
        } else { Ok(()) }
    }

    pub fn execute(
        &self,
        database: &mut Database,
        source: &str,
        params: &Params,
    ) -> Result<ExecutionReport, AccessError> {
        let statements = self.prepare(source, params)?;
        if Database::statements_read_only(&statements) {
            // Bounded, authorized reads do not need to clone the database.
            let result = database.query_parsed(&statements, params)?;
            self.validate_result(&result)?;
            return Ok(result);
        }
        let mut working = database.clone();
        let result = working.execute_parsed(&statements, params)?;
        self.validate_result(&result)?;
        *database = working;
        Ok(result)
    }
}

/// Restricted embedded façade. Callers must not expose an independent
/// unrestricted Database handle to untrusted principals.
pub struct AuthorizedSession {
    database: Database,
    access: AccessControl,
}
impl AuthorizedSession {
    pub fn new(database: Database, access: AccessControl) -> Self { Self { database, access } }
    pub fn execute(&mut self, source: &str, params: &Params) -> Result<ExecutionReport, AccessError> {
        self.access.execute(&mut self.database, source, params)
    }
    pub fn version(&self) -> u32 { self.database.version }
    // Intentionally crate-private: exporting an unrestricted Database from
    // a principal-facing session would trivially bypass every table grant.
    pub(crate) fn into_database(self) -> Database { self.database }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_policies_restrict_reads_writes_and_nested_transactions() {
        let mut database = Database::default();
        database.execute(
            "table events { id:int key auto\n owner:int required\n value:int required }\nevents add {owner:1,value:10}\nevents add {owner:2,value:20}",
            &Params::new(),
        ).unwrap();
        let mut rights = AccessControl::new();
        rights.grant("events", true, true);
        rights.restrict_rows("events", "owner", Value::Int(1));

        assert_eq!(rights.execute(&mut database, "events get id,owner,value", &Params::new()).unwrap().rows.len(), 1);
        assert_eq!(rights.execute(&mut database, "events where owner == 2 set value = 99", &Params::new()).unwrap().affected, 0);
        assert_eq!(rights.execute(&mut database, "events where owner == 2 delete", &Params::new()).unwrap().affected, 0);
        assert!(rights.execute(&mut database, "events add {owner:2,value:30}", &Params::new()).is_err());
        assert!(rights.execute(&mut database, "events where owner == 1 set owner = 2", &Params::new()).is_err());
        rights.execute(&mut database, "atomic { events add {owner:1,value:30} }", &Params::new()).unwrap();

        // The unrestricted admin view retains tenant 2 unmodified.
        let rows = database.execute("events get id,owner,value", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|r| r.get("owner") == Some(&Value::Int(2)) &&
            r.get("value") == Some(&Value::Int(20))));
    }

    #[test]
    fn restricted_session_keeps_database_handle_private() {
        let mut db = Database::default();
        db.execute("table clients { id:int key auto }", &Params::new()).unwrap();
        let mut grants = AccessControl::new();
        grants.grant("clients", true, false);
        let mut session = AuthorizedSession::new(db, grants);
        assert!(session.execute("clients add {}", &Params::new()).is_err());
        assert_eq!(session.execute("clients get id", &Params::new()).unwrap().rows.len(), 0);
        assert_eq!(session.version(), 1);
    }

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
    #[test]
    fn bounded_restricted_read_is_side_effect_free_and_denial_is_atomic() {
        let mut db = Database::default();
        db.execute("table messages { id:int key auto\n owner:int required }\nmessages add {owner:1}",
            &Params::new()).unwrap();
        let mut acl = AccessControl::new();
        acl.grant("messages", true, true);
        acl.restrict_rows("messages", "owner", Value::Int(1));
        assert_eq!(acl.execute(&mut db, "messages get id,owner", &Params::new())
            .unwrap().rows.len(), 1);
        acl.max_returned_rows = 0;
        assert!(matches!(acl.execute(&mut db, "messages get id", &Params::new()),
            Err(AccessError::Budget(_))));
        assert!(matches!(acl.execute(&mut db,
            "atomic { messages add {owner:1}; messages get id }", &Params::new()),
            Err(AccessError::Budget(_))));
        assert_eq!(db.execute("messages get id", &Params::new()).unwrap().rows.len(), 1);
    }

}
