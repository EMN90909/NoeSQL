use crate::value::{Value, ValueType};
use crate::bplus_index::BPlusIndex;
use std::sync::{Arc, Mutex, OnceLock};
use std::cmp::Ordering;
use crate::planner::{choose_relational_plan, JoinPlan};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Row = BTreeMap<String, Value>;
pub type Params = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceError {
    pub code: &'static str,
    pub line: usize,
    pub column: usize,
    pub message: String,
}
impl std::fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}:{}: {}", self.code, self.line, self.column, self.message)
    }
}
impl std::error::Error for ReferenceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TokenKind { Ident, Int, Decimal, Str, Op, Sep, Eof }
#[derive(Clone, Debug)]
struct Token { kind: TokenKind, text: String, line: usize, column: usize }

fn failure(code: &'static str, token: &Token, message: impl Into<String>) -> ReferenceError {
    ReferenceError { code, line: token.line, column: token.column, message: message.into() }
}

fn tokenize(source: &str) -> Result<Vec<Token>, ReferenceError> {
    if source.len() > crate::MAX_SOURCE_BYTES {
        return Err(ReferenceError { code: "NOE005", line: 1, column: 1, message: "source exceeds 1 MiB limit".into() });
    }
    let bytes = source.as_bytes();
    // Reserve a typical query-sized token buffer, not an unbounded buffer
    // based on attacker-controlled source length.
    let mut out = Vec::with_capacity(64);
    let (mut i, mut line, mut column, mut depth) = (0usize, 1usize, 1usize, 0usize);
    while i < bytes.len() {
        // Check incrementally: never accumulate a million tiny tokens before
        // discovering that the program exceeded the public compiler limit.
        if out.len() > 100_000 {
            return Err(ReferenceError { code: "NOE006", line, column, message: "too many tokens".into() });
        }
        let (at_line, at_column) = (line, column);
        let ch = bytes[i];
        macro_rules! adv {
            ($count:expr) => {{
                for _ in 0..$count {
                    if bytes[i] == b'\n' { line += 1; column = 1; } else { column += 1; }
                    i += 1;
                }
            }};
        }
        if matches!(ch, b' ' | b'\t' | b'\r') { adv!(1); continue; }
        if ch == b'\n' || ch == b';' {
            adv!(1);
            out.push(Token { kind: TokenKind::Sep, text: ";".into(), line: at_line, column: at_column });
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' { adv!(1); }
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            adv!(2);
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' { adv!(2); closed = true; break; }
                adv!(1);
            }
            if !closed { return Err(ReferenceError { code: "NOE001", line: at_line, column: at_column, message: "unterminated comment".into() }); }
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { adv!(1); }
            out.push(Token { kind: TokenKind::Ident, text: source[start..i].into(), line: at_line, column: at_column });
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() { adv!(1); }
            let decimal = i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit();
            if decimal {
                adv!(1);
                while i < bytes.len() && bytes[i].is_ascii_digit() { adv!(1); }
            }
            out.push(Token { kind: if decimal { TokenKind::Decimal } else { TokenKind::Int },
                text: source[start..i].into(), line: at_line, column: at_column });
            continue;
        }
        if ch == b'"' {
            adv!(1);
            let mut text = Vec::new();
            let mut closed = false;
            while i < bytes.len() {
                let current = bytes[i];
                adv!(1);
                if current == b'"' { closed = true; break; }
                if current == b'\\' {
                    if i >= bytes.len() { break; }
                    let escaped = bytes[i];
                    adv!(1);
                    text.push(match escaped {
                        b'n' => b'\n', b'r' => b'\r', b't' => b'\t', b'"' => b'"', b'\\' => b'\\',
                        _ => return Err(ReferenceError { code: "NOE003", line: at_line, column: at_column, message: "invalid string escape".into() }),
                    });
                } else {
                    if current == b'\n' { return Err(ReferenceError { code: "NOE002", line: at_line, column: at_column, message: "unterminated string".into() }); }
                    text.push(current);
                }
            }
            if !closed { return Err(ReferenceError { code: "NOE002", line: at_line, column: at_column, message: "unterminated string".into() }); }
            let text = String::from_utf8(text).map_err(|_| ReferenceError { code: "NOE003", line: at_line, column: at_column, message: "string is not valid UTF-8".into() })?;
            out.push(Token { kind: TokenKind::Str, text, line: at_line, column: at_column });
            continue;
        }
        let duo = if i + 1 < bytes.len() {
            match (bytes[i], bytes[i + 1]) {
                (b'=', b'=') => Some("=="),
                (b'!', b'=') => Some("!="),
                (b'<', b'=') => Some("<="),
                (b'>', b'=') => Some(">="),
                (b'&', b'&') => Some("&&"),
                (b'|', b'|') => Some("||"),
                _ => None,
            }
        } else { None };
        if let Some(duo) = duo {
            adv!(2);
            out.push(Token { kind: TokenKind::Op, text: duo.into(), line: at_line, column: at_column });
            continue;
        }
        if b"{}():,.?=$+-*/!<>".contains(&ch) {
            let text = (ch as char).to_string();
            if matches!(ch, b'{' | b'(') {
                depth += 1;
                if depth > 64 { return Err(ReferenceError { code: "NOE007", line: at_line, column: at_column, message: "nesting exceeds maximum".into() }); }
            } else if matches!(ch, b'}' | b')') { depth = depth.saturating_sub(1); }
            adv!(1);
            out.push(Token { kind: TokenKind::Op, text, line: at_line, column: at_column });
            continue;
        }
        return Err(ReferenceError { code: "NOE004", line: at_line, column: at_column, message: format!("unexpected byte 0x{ch:02x}") });
    }
    if out.len() > 100_000 { return Err(ReferenceError { code: "NOE006", line, column, message: "too many tokens".into() }); }
    out.push(Token { kind: TokenKind::Eof, text: "<eof>".into(), line, column });
    Ok(out)
}

pub fn lex(source: &str) -> Result<Vec<(String, usize, usize)>, ReferenceError> {
    Ok(tokenize(source)?.into_iter().filter(|t| t.kind != TokenKind::Eof).map(|t| (t.text, t.line, t.column)).collect())
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub kind: ValueType,
    pub reference: Option<String>,
    pub required: bool,
    pub key: bool,
    pub auto: bool,
    pub unique: bool,
    pub default: Option<Expr>,
}
#[derive(Clone, Debug)]
pub enum Expr {
    Literal(Value),
    Column(String),
    Param(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
}
#[derive(Clone, Debug)]
pub enum Projection {
    All,
    Column(String),
    Aggregate { function: String, argument: Option<String>, alias: String },
}
#[derive(Clone, Debug)]
pub struct Join { pub table: String, pub left: bool, pub on: Expr }
#[derive(Clone, Debug)]
pub struct QueryParam {
    pub name: String,
    pub kind: ValueType,
    pub default: Option<Expr>,
}
#[derive(Clone, Debug)]
pub struct NamedQuery {
    pub params: Vec<QueryParam>,
    pub select: Select,
}
#[derive(Clone, Debug)]
pub struct IndexDefinition {
    pub table: String,
    pub columns: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Select {
    pub table: String,
    pub projections: Vec<Projection>,
    pub join: Option<Join>,
    pub filter: Option<Expr>,
    pub group: Vec<String>,
    pub having: Option<Expr>,
    pub sort: Option<String>,
    pub skip: usize,
    pub take: Option<usize>,
}
#[derive(Clone, Debug)]
pub enum Statement {
    Database(String),
    Table(String, Vec<Field>),
    AlterAdd(String, Vec<Field>),
    Index { name: String, table: String, columns: Vec<String> },
    Query { name: String, params: Vec<QueryParam>, select: Select },
    Add(String, Vec<(String, Expr)>),
    Get(Select),
    Set(String, Expr, Vec<(String, Expr)>),
    Delete(String, Expr),
    Atomic(Vec<Statement>),
}

struct Parser { tokens: Vec<Token>, pos: usize }
impl Parser {
    fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }
    fn current(&self) -> &Token { &self.tokens[self.pos] }
    fn is(&self, value: &str) -> bool { self.current().text == value }
    fn take(&mut self) -> Token {
        let index = self.pos;
        self.pos += 1;
        // Parsing owns the token stream. Move its text into the AST instead of
        // allocating another String for every identifier, literal and symbol.
        std::mem::replace(
            &mut self.tokens[index],
            Token { kind: TokenKind::Eof, text: String::new(), line: 0, column: 0 },
        )
    }
    fn eat(&mut self, value: &str) -> bool { if self.is(value) { self.take(); true } else { false } }
    fn need(&mut self, value: &str) -> Result<(), ReferenceError> {
        if self.eat(value) { Ok(()) } else { Err(failure("NOE010", self.current(), format!("expected {value}, found {}", self.current().text))) }
    }
    fn id(&mut self) -> Result<String, ReferenceError> {
        if self.current().kind != TokenKind::Ident { return Err(failure("NOE011", self.current(), "expected identifier")); }
        if matches!(self.current().text.as_str(), "__proto__" | "constructor" | "prototype") { return Err(failure("NOE019", self.current(), "reserved identifier")); }
        Ok(self.take().text)
    }
    fn separators(&mut self) { while self.eat(";") {} }
    fn boundary(&mut self) -> Result<(), ReferenceError> {
        if !self.is(";") && !self.is("}") && self.current().kind != TokenKind::Eof { return Err(failure("NOE012", self.current(), "expected newline or semicolon")); }
        self.separators(); Ok(())
    }
    fn expr(&mut self, minimum: u8) -> Result<Expr, ReferenceError> {
        let first = self.take();
        let mut left = match first.text.as_str() {
            "(" => { let value = self.expr(0)?; self.need(")")?; value }
            "!" | "-" => Expr::Unary(first.text, Box::new(self.expr(7)?)),
            "$" => Expr::Param(self.id()?),
            "true" => Expr::Literal(Value::Bool(true)),
            "false" => Expr::Literal(Value::Bool(false)),
            "null" => Expr::Literal(Value::Null),
            _ if first.kind == TokenKind::Str => Expr::Literal(Value::Text(first.text)),
            _ if first.kind == TokenKind::Int => Expr::Literal(Value::Int(first.text.parse::<i64>().map_err(|_| failure("NOE014", &first, "integer out of range"))?)),
            _ if first.kind == TokenKind::Decimal => Expr::Literal(Value::Decimal(
                crate::decimal::Decimal::parse(&first.text)
                    .ok_or_else(|| failure("NOE014", &first, "decimal precision exceeds 38 digits / six fractional places"))?)),
            _ if first.kind == TokenKind::Ident => {
                let mut name = first.text;
                while self.eat(".") { name.push('.'); name.push_str(&self.id()?); }
                Expr::Column(name)
            }
            _ => return Err(failure("NOE015", &first, "expected expression")),
        };
        loop {
            let precedence = match self.current().text.as_str() {
                "||" => 1, "&&" => 2, "==" | "!=" => 3, "<" | "<=" | ">" | ">=" => 4,
                "+" | "-" => 5, "*" | "/" => 6, _ => 0,
            };
            if precedence == 0 || precedence < minimum { break; }
            let op = self.take().text;
            let right = self.expr(precedence + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn fields(&mut self) -> Result<Vec<Field>, ReferenceError> {
        self.need("{")?; self.separators();
        let mut fields = Vec::new();
        while !self.is("}") {
            if self.current().kind == TokenKind::Eof { return Err(failure("NOE017", self.current(), "unterminated table declaration")); }
            let name = self.id()?; self.need(":")?; let type_name = self.id()?;
            let kind = ValueType::parse(&type_name).ok_or_else(|| failure("NOE020", self.current(), format!("unknown type {type_name}")))?;
            let reference = if kind == ValueType::Ref { Some(self.id()?) } else { None };
            let _nullable = self.eat("?");
            let (mut required, mut key, mut auto, mut unique) = (false, false, false, false);
            loop {
                match self.current().text.as_str() {
                    "required" => { required = true; self.take(); }
                    "key" => { key = true; unique = true; required = true; self.take(); }
                    "auto" => { auto = true; self.take(); }
                    "unique" => { unique = true; self.take(); }
                    _ => break,
                }
            }
            let default = if self.eat("=") { Some(self.expr(0)?) } else { None };
            fields.push(Field { name, kind, reference, required, key, auto, unique, default });
            if self.eat(",") { self.separators(); } else { self.boundary()?; }
        }
        self.need("}")?; Ok(fields)
    }
    fn object(&mut self) -> Result<Vec<(String, Expr)>, ReferenceError> {
        self.need("{")?; self.separators(); let mut out = Vec::new();
        while !self.is("}") {
            let name = self.id()?; self.need(":")?; out.push((name, self.expr(0)?));
            if self.eat(",") { self.separators(); } else if !self.is("}") { self.boundary()?; }
        }
        self.need("}")?; Ok(out)
    }
    fn qualified(&mut self) -> Result<String, ReferenceError> {
        let mut name = self.id()?; while self.eat(".") { name.push('.'); name.push_str(&self.id()?); } Ok(name)
    }
    fn projection(&mut self) -> Result<Projection, ReferenceError> {
        if self.eat("*") { return Ok(Projection::All); }
        let name = self.qualified()?;
        if !self.eat("(") { return Ok(Projection::Column(name)); }
        if !matches!(name.as_str(), "count" | "sum" | "min" | "max") { return Err(failure("NOE030", self.current(), format!("unsupported aggregate {name}"))); }
        let argument = if self.eat("*") { if name != "count" { return Err(failure("NOE030", self.current(), "only count accepts *")); } None }
            else if self.is(")") { None } else { Some(self.qualified()?) };
        self.need(")")?;
        if name != "count" && argument.is_none() { return Err(failure("NOE030", self.current(), format!("{name} requires a column"))); }
        let alias = if self.eat("as") { self.id()? } else { argument.as_ref().map(|a| format!("{}_{}", name, a.replace('.', "_"))).unwrap_or_else(|| name.clone()) };
        Ok(Projection::Aggregate { function: name, argument, alias })
    }
    fn select(&mut self, table: String) -> Result<Select, ReferenceError> {
        self.need("get")?; let mut projections = vec![self.projection()?]; while self.eat(",") { projections.push(self.projection()?); }
        let join = if self.eat("left") { self.need("join")?; let table = self.id()?; self.need("on")?; Some(Join { table, left: true, on: self.expr(0)? }) }
            else if self.eat("join") { let table = self.id()?; self.need("on")?; Some(Join { table, left: false, on: self.expr(0)? }) } else { None };
        let filter = if self.eat("where") { Some(self.expr(0)?) } else { None };
        let mut group = Vec::new(); if self.eat("group") { group.push(self.qualified()?); while self.eat(",") { group.push(self.qualified()?); } }
        let having = if self.eat("having") { Some(self.expr(0)?) } else { None };
        let sort = if self.eat("sort") { Some(self.qualified()?) } else { None };
        let skip = if self.eat("skip") { match self.expr(0)? { Expr::Literal(Value::Int(v)) if v >= 0 => v as usize, _ => return Err(failure("NOE031", self.current(), "skip requires non-negative integer")) } } else { 0 };
        let take = if self.eat("take") { match self.expr(0)? { Expr::Literal(Value::Int(v)) if v >= 0 => Some(v as usize), _ => return Err(failure("NOE031", self.current(), "take requires non-negative integer")) } } else { None };
        Ok(Select { table, projections, join, filter, group, having, sort, skip, take })
    }
    fn statement(&mut self) -> Result<Statement, ReferenceError> {
        self.separators();
        if self.eat("db") { return Ok(Statement::Database(self.id()?)); }
        if self.eat("table") { let name = self.id()?; return Ok(Statement::Table(name, self.fields()?)); }
        if self.eat("alter") { let name = self.id()?; self.need("add")?; return Ok(Statement::AlterAdd(name, self.fields()?)); }
        if self.eat("index") {
            let name = self.id()?;
            self.need("on")?;
            let table = self.id()?;
            self.need("(")?;
            let mut columns = vec![self.id()?];
            while self.eat(",") { columns.push(self.id()?); }
            self.need(")")?;
            return Ok(Statement::Index { name, table, columns });
        }
        if self.eat("query") {
            let name = self.id()?;
            self.need("(")?;
            let mut params = Vec::new();
            while !self.is(")") {
                let param_name = self.id()?;
                self.need(":")?;
                let type_name = self.id()?;
                let kind = ValueType::parse(&type_name).ok_or_else(|| failure("NOE115", self.current(), "unsupported parameter type"))?;
                if kind == ValueType::Ref { return Err(failure("NOE115", self.current(), "named-query parameters cannot be ref")); }
                let default = if self.eat("=") { Some(self.expr(0)?) } else { None };
                params.push(QueryParam { name: param_name, kind, default });
                if !self.eat(",") { break; }
            }
            self.need(")")?;
            self.need("{")?;
            self.separators();
            let statement = self.statement()?;
            self.separators();
            self.need("}")?;
            let Statement::Get(select) = statement else {
                return Err(failure("NOE016", self.current(), "named query must be a get statement"));
            };
            return Ok(Statement::Query { name, params, select });
        }
        if self.eat("atomic") {
            self.need("{")?; self.separators(); let mut body = Vec::new();
            while !self.is("}") { if self.current().kind == TokenKind::Eof { return Err(failure("NOE017", self.current(), "unterminated atomic scope")); } body.push(self.statement()?); self.boundary()?; }
            self.need("}")?; return Ok(Statement::Atomic(body));
        }
        let table = self.id()?;
        if self.eat("add") { return Ok(Statement::Add(table, self.object()?)); }
        if self.is("get") { return Ok(Statement::Get(self.select(table)?)); }
        if self.eat("where") {
            let filter = self.expr(0)?;
            if self.eat("delete") { return Ok(Statement::Delete(table, filter)); }
            self.need("set")?; let mut values = Vec::new();
            loop { let name = self.id()?; self.need("=")?; values.push((name, self.expr(0)?)); if !self.eat(",") { break; } }
            return Ok(Statement::Set(table, filter, values));
        }
        Err(failure("NOE018", self.current(), "expected add, get, or where after table name"))
    }
    fn parse(mut self) -> Result<Vec<Statement>, ReferenceError> {
        let mut out = Vec::new(); self.separators();
        while self.current().kind != TokenKind::Eof { out.push(self.statement()?); self.boundary()?; }
        Ok(out)
    }
}
pub fn parse(source: &str) -> Result<Vec<Statement>, ReferenceError> { Parser::new(tokenize(source)?).parse() }

#[derive(Debug)]
pub struct Table {
    pub fields: Vec<Field>,
    pub rows: Vec<Row>,
    pub(crate) next_auto: i64,
    // Cache is deliberately not cloned into a write transaction. The row
    // vector is public for trusted runtime migrations, so callers modifying
    // it directly must rebuild their Database before querying.
    pub(crate) seek_cache: OnceLock<Mutex<BTreeMap<String, Arc<BPlusIndex>>>>,
}
impl Clone for Table {
    fn clone(&self) -> Self {
        Self { fields: self.fields.clone(), rows: self.rows.clone(),
            next_auto: self.next_auto, seek_cache: OnceLock::new() }
    }
}
impl Table {
    /// Installed from a generation-bound, SHA-verified 4 KiB B+ page tree.
    /// Direct modification of public Table.rows outside database APIs
    /// invalidates this invariant; trusted callers must rebuild the table.
    pub(crate) fn install_index(&self, field: &str, index: BPlusIndex) -> Result<(), String> {
        let cache = self.seek_cache.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut guard = cache.lock().map_err(|_| "index cache poisoned")?;
        guard.insert(field.to_string(), Arc::new(index));
        Ok(())
    }
    fn indexed_row_positions(&self, field: &str, key: i64) -> Option<Vec<usize>> {
        let cache = self.seek_cache.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut guard = cache.lock().ok()?; // poisoned caches fail back to full scan
        let index = guard.entry(field.to_string()).or_insert_with(|| {
            Arc::new(BPlusIndex::build(self.rows.iter().enumerate().filter_map(
                |(position, row)| match row.get(field) {
                    Some(Value::Int(value)) => Some((*value, position)),
                    _ => None,
                }
            )))
        });
        Some(index.seek_equal(key))
    }
}

fn indexed_condition(expr: &Expr, table: &str, params: &Params,
    indexed: &impl Fn(&str) -> bool) -> Option<(String, i64)> {
    if let Expr::Binary(op, left, right) = expr {
        if op == "&&" {
            return indexed_condition(left, table, params, indexed)
                .or_else(|| indexed_condition(right, table, params, indexed));
        }
        if op == "==" {
            for (column, value) in [(left.as_ref(), right.as_ref()), (right.as_ref(), left.as_ref())] {
                if let Expr::Column(name) = column {
                    let field = name.strip_prefix(&format!("{table}."))
                        .or_else(|| if name.contains('.') { None } else { Some(name.as_str()) })?;
                    if !indexed(field) { continue; }
                    let number = match value {
                        Expr::Literal(Value::Int(n)) => Some(*n),
                        Expr::Param(param) => params.get(param).and_then(Value::as_i64),
                        _ => None,
                    };
                    if let Some(number) = number { return Some((field.to_string(), number)); }
                }
            }
        }
    }
    None
}
#[derive(Clone, Debug)]
pub struct Database {
    pub name: Option<String>,
    pub version: u32,
    pub tables: BTreeMap<String, Table>,
    pub indexes: BTreeMap<String, IndexDefinition>,
    pub queries: BTreeMap<String, NamedQuery>,
}
impl Default for Database {
    fn default() -> Self {
        Self {
            name: None,
            version: 1,
            tables: BTreeMap::new(),
            indexes: BTreeMap::new(),
            queries: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct ExecutionReport { pub affected: usize, pub rows: Vec<Row> }

fn lookup<'a>(row: &'a Row, name: &str) -> Option<&'a Value> {
    row.get(name).or_else(|| {
        if name.contains('.') { return None; }
        let suffix = format!(".{name}");
        let mut found = row.iter().filter(|(key, _)| key.ends_with(&suffix));
        let first = found.next().map(|(_, value)| value);
        if found.next().is_none() { first } else { None }
    })
}
fn eval(expr: &Expr, row: &Row, params: &Params) -> Result<Value, ReferenceError> {
    match expr {
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Column(name) => lookup(row, name).cloned().ok_or_else(|| ReferenceError { code: "NOE120", line: 1, column: 1, message: format!("unknown field {name}") }),
        Expr::Param(name) => params.get(name).cloned().ok_or_else(|| ReferenceError { code: "NOE121", line: 1, column: 1, message: format!("unknown parameter ${name}") }),
        Expr::Unary(op, arg) => match (op.as_str(), eval(arg, row, params)?) {
            ("!", Value::Bool(value)) => Ok(Value::Bool(!value)),
            ("-", Value::Decimal(value)) => value.checked_neg()
                .map(Value::Decimal)
                .ok_or_else(|| ReferenceError { code: "NOE124", line: 1, column: 1,
                    message: "decimal arithmetic overflow".into() }),
            ("-", Value::Int(value)) => value
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| ReferenceError {
                    code: "NOE124", line: 1, column: 1,
                    message: "integer arithmetic failed".into(),
                }),
            _ => Err(ReferenceError { code: "NOE122", line: 1, column: 1, message: "invalid unary operand".into() }),
        },
        Expr::Binary(op, left, right) => {
            let left = eval(left, row, params)?;
            if op == "&&" && left == Value::Bool(false) { return Ok(Value::Bool(false)); }
            if op == "||" && left == Value::Bool(true) { return Ok(Value::Bool(true)); }
            let right = eval(right, row, params)?;
            match op.as_str() {
                "&&" | "||" => match (left, right) { (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(if op == "&&" { a && b } else { a || b })), _ => type_error("boolean operator requires bool") },
                "+" | "-" | "*" | "/" => match (left, right) {
                    (Value::Decimal(a), Value::Decimal(b)) => {
                        let value = match op.as_str() {
                            "+" => a.checked_add(b), "-" => a.checked_sub(b),
                            "*" => a.checked_mul(b), "/" => a.checked_div(b), _ => None,
                        }.ok_or_else(|| db_failure("NOE124", "decimal overflow, divide by zero or intermediate precision limit"))?;
                        Ok(Value::Decimal(value))
                    },
                    (Value::Int(a), Value::Int(b)) => {
                        let value = match op.as_str() { "+" => a.checked_add(b), "-" => a.checked_sub(b), "*" => a.checked_mul(b), "/" if b != 0 => a.checked_div(b), _ => None }
                            .ok_or_else(|| ReferenceError { code: "NOE124", line: 1, column: 1, message: "integer arithmetic failed".into() })?;
                        Ok(Value::Int(value))
                    }
                    _ => type_error("arithmetic requires int"),
                },
                "==" => Ok(Value::Bool(left == right)), "!=" => Ok(Value::Bool(left != right)),
                "<" | "<=" | ">" | ">=" => {
                    let order = left.compare(&right).ok_or_else(|| ReferenceError { code: "NOE123", line: 1, column: 1, message: "incompatible comparison".into() })?;
                    Ok(Value::Bool(match op.as_str() { "<" => order == Ordering::Less, "<=" => order != Ordering::Greater, ">" => order == Ordering::Greater, ">=" => order != Ordering::Less, _ => false }))
                }
                _ => type_error("unsupported operator"),
            }
        }
    }
}
fn type_error<T>(message: &str) -> Result<T, ReferenceError> { Err(ReferenceError { code: "NOE123", line: 1, column: 1, message: message.into() }) }
fn predicate(filter: Option<&Expr>, row: &Row, params: &Params) -> Result<bool, ReferenceError> {
    match filter { None => Ok(true), Some(expr) => eval(expr, row, params)?.truthy().ok_or_else(|| ReferenceError { code: "NOE125", line: 1, column: 1, message: "predicate must be bool".into() }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExprType { Null, Int, Decimal, Bool, Text }

fn value_expr_type(value: &Value) -> ExprType {
    match value {
        Value::Null => ExprType::Null,
        Value::Int(_) => ExprType::Int,
        Value::Decimal(_) => ExprType::Decimal,
        Value::Bool(_) => ExprType::Bool,
        Value::Text(_) => ExprType::Text,
    }
}

fn field_expr_type(kind: ValueType) -> ExprType {
    match kind {
        ValueType::Int | ValueType::Ref => ExprType::Int,
        ValueType::Decimal => ExprType::Decimal,
        ValueType::Bool => ExprType::Bool,
        ValueType::Text | ValueType::Date | ValueType::DateTime | ValueType::Uuid |
        ValueType::Json | ValueType::Binary => ExprType::Text,
    }
}

fn infer_expr_type(
    expr: &Expr,
    columns: &BTreeMap<String, ExprType>,
    params: &BTreeMap<String, ExprType>,
) -> Result<ExprType, ReferenceError> {
    match expr {
        Expr::Literal(value) => Ok(value_expr_type(value)),
        Expr::Column(name) => columns.get(name).copied()
            .ok_or_else(|| db_failure("NOE120", format!("unknown field {name}"))),
        Expr::Param(name) => params.get(name).copied()
            .ok_or_else(|| db_failure("NOE121", format!("unknown parameter ${name}"))),
        Expr::Unary(op, arg) => {
            let kind = infer_expr_type(arg, columns, params)?;
            match (op.as_str(), kind) {
                ("!", ExprType::Bool) => Ok(ExprType::Bool),
                ("-", ExprType::Int) => Ok(ExprType::Int),
                ("-", ExprType::Decimal) => Ok(ExprType::Decimal),
                _ => db_error("NOE122", "invalid unary operand"),
            }
        }
        Expr::Binary(op, left, right) => {
            let left = infer_expr_type(left, columns, params)?;
            let right = infer_expr_type(right, columns, params)?;
            match op.as_str() {
                "+" | "-" | "*" | "/" if left == ExprType::Int && right == ExprType::Int => Ok(ExprType::Int),
                "+" | "-" | "*" | "/" if left == ExprType::Decimal && right == ExprType::Decimal => Ok(ExprType::Decimal),
                "&&" | "||" if left == ExprType::Bool && right == ExprType::Bool => Ok(ExprType::Bool),
                "==" | "!=" | "<" | "<=" | ">" | ">=" if left == right && left != ExprType::Null => Ok(ExprType::Bool),
                _ => db_error("NOE123", format!("incompatible {left:?} {op} {right:?}")),
            }
        }
    }
}
fn qualified(table: &str, row: &Row) -> Row {
    let mut out = Row::new();
    for (name, value) in row { out.insert(name.clone(), value.clone()); out.insert(format!("{table}.{name}"), value.clone()); }
    out
}
fn merge(left_name: &str, left: &Row, right_name: &str, right: Option<&Row>) -> Row {
    let mut out = qualified(left_name, left);
    if let Some(right) = right {
        for (name, value) in right { out.insert(format!("{right_name}.{name}"), value.clone()); if !out.contains_key(name) { out.insert(name.clone(), value.clone()); } }
    }
    out
}

impl Database {
    pub fn execute(&mut self, source: &str, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let statements = parse(source)?;
        self.execute_parsed(&statements, params)
    }

    /// All-select programs cannot mutate state: avoid copying the complete
    /// database and preserve the identity of its existing row allocations.
    pub fn statements_read_only(statements: &[Statement]) -> bool {
        statements.iter().all(|statement| matches!(statement, Statement::Get(_)))
    }

    /// Execute against a caller-owned private database that is discarded on
    /// any error. Avoids a second full row clone inside a durable write.
    /// NEVER call on public/shared state: partial mutation on failure is
    /// intentional and the caller must withhold publication.
    pub(crate) fn execute_isolated_parsed(
        &mut self, statements: &[Statement], params: &Params
    ) -> Result<ExecutionReport, ReferenceError> {
        if Self::statements_read_only(statements) {
            self.query_parsed(statements, params)
        } else {
            self.execute_many(statements, params)
        }
    }

    /// Execute an already parsed NoeSQL program. Used by the durable host
    /// to parse outside the storage lock and avoid parsing a second time.
    pub fn execute_parsed(&mut self, statements: &[Statement], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        if Self::statements_read_only(statements) {
            return self.query_parsed(statements, params);
        }
        // A write program is still atomic: work on a copy until all
        // statements have succeeded, including the late-failure case.
        let mut working = self.clone();
        let report = working.execute_many(statements, params)?;
        *self = working;
        Ok(report)
    }
    /// Execute SELECT statements against an immutable database snapshot.
    /// This is used by the in-process MVCC reader path and never clones rows.
    pub fn query_parsed(&self, statements: &[Statement], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        if !Self::statements_read_only(statements) {
            return db_error("NOE241", "read-only snapshot cannot execute writes");
        }
        let mut report = ExecutionReport::default();
        for statement in statements {
            if let Statement::Get(select) = statement {
                let current = self.select(select, params)?;
                report.affected += current.affected;
                // Each read replaces the previous read result, including an
                // empty result set. Otherwise a later empty GET leaks stale rows.
                report.rows = current.rows;
            }
        }
        Ok(report)
    }

    /// Describe the plan the current engine will actually use. The index
    /// catalog is currently metadata, not an executable B+tree access path.
    /// EXPLAIN must never imply that a secondary index was used.
    pub fn explain(&self, source: &str, params: &Params) -> Result<String, ReferenceError> {
        let statements = parse(source)?;
        let [Statement::Get(select)] = statements.as_slice() else {
            return db_error("NOE240", "EXPLAIN requires exactly one read-only get query");
        };
        self.validate_select(select, params)?;
        let left = self.tables.get(&select.table)
            .ok_or_else(|| db_failure("NOE100", format!("unknown table {}", select.table)))?;
        let right = select.join.as_ref().and_then(|join| self.tables.get(&join.table));
        let plan = choose_relational_plan(select, left, right);
        let join = match &plan.join {
            JoinPlan::None => "none".to_string(),
            JoinPlan::Nested => "nested-loop".to_string(),
            JoinPlan::Hash { left_key, right_key } => format!("hash({left_key}={right_key})"),
        };
        let sort = select.sort.as_deref().unwrap_or("none");
        let indexed = |field: &str| {
            left.fields.iter().any(|f| f.name == field
                && matches!(f.kind, ValueType::Int | ValueType::Ref)
                && (f.key || f.unique))
            || self.indexes.values().any(|def| def.table == select.table
                && def.columns.len() == 1 && def.columns[0] == field
                && left.fields.iter().any(|f| f.name == field
                    && matches!(f.kind, ValueType::Int | ValueType::Ref)))
        };
        let access = if select.join.is_none() {
            select.filter.as_ref().and_then(|filter|
                indexed_condition(filter, &select.table, params, &indexed))
                .map(|(field, _)| format!("memory_bplus({field})"))
                .unwrap_or_else(|| "full_scan".into())
        } else { "full_scan".into() };
        Ok(format!(
            "access={} table={} rows={} join={} right_rows={} push_where={} grouped={} sort={} skip={} take={}",
            access,
            select.table,
            plan.left_rows,
            join,
            plan.right_rows,
            plan.push_where,
            !select.group.is_empty(),
            sort,
            select.skip,
            select.take.map(|n| n.to_string()).unwrap_or_else(|| "all".into()),
        ))
    }

    // The report contains the *last executed GET*, including zero rows.
    // Mutations do not produce query rows, and nested atomic blocks can
    // themselves contain GET statements.
    fn contains_get(statement: &Statement) -> bool {
        match statement {
            Statement::Get(_) => true,
            Statement::Atomic(body) => body.iter().any(Self::contains_get),
            _ => false,
        }
    }

    fn execute_many(&mut self, statements: &[Statement], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let mut report = ExecutionReport::default();
        for statement in statements {
            let current = self.execute_one(statement, params)?;
            report.affected += current.affected;
            if Self::contains_get(statement) {
                report.rows = current.rows;
            }
        }
        Ok(report)
    }
    fn execute_one(&mut self, statement: &Statement, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        match statement {
            Statement::Database(name) => {
                if let Some(existing) = &self.name {
                    if existing != name { return db_error("NOE101", "cannot change database identity"); }
                }
                self.name = Some(name.clone());
                Ok(ExecutionReport::default())
            }
            Statement::Table(name, fields) => {
                if self.tables.contains_key(name) { return db_error("NOE102", format!("duplicate table {name}")); }
                if fields.is_empty() || fields.len() > 256 { return db_error("NOE109", "table requires 1..256 fields"); }
                let mut names = BTreeSet::new();
                let mut keys = 0usize;
                let mut autos = 0usize;
                for field in fields {
                    if !names.insert(field.name.clone()) { return db_error("NOE103", format!("duplicate field {}", field.name)); }
                    if field.key {
                        keys += 1;
                        if field.kind != ValueType::Int { return db_error("NOE105", "primary key must be int"); }
                    }
                    if field.auto {
                        autos += 1;
                        if !field.key || field.kind != ValueType::Int { return db_error("NOE106", "auto requires int key"); }
                    }
                    if field.reference.is_some() && field.default.is_some() { return db_error("NOE107", "references cannot have defaults"); }
                    if let Some(default) = &field.default {
                        let value = eval(default, &Row::new(), &Params::new())?;
                        if !field.kind.accepts(&value) || value == Value::Null {
                            return db_error("NOE124", format!("invalid default for {}", field.name));
                        }
                    }
                }
                if keys != 1 || autos > 1 { return db_error("NOE108", "exactly one integer primary key required"); }
                self.tables.insert(name.clone(), Table { fields: fields.clone(), rows: Vec::new(), next_auto: 1, seek_cache: OnceLock::new() });
                Ok(ExecutionReport::default())
            }
            Statement::AlterAdd(name, fields) => {
                if fields.is_empty() { return db_error("NOE240", "alter add requires at least one new field"); }
                let table = self.tables.get_mut(name).ok_or_else(|| db_failure("NOE100", format!("unknown table {name}")))?;
                let mut added = BTreeSet::new();
                for field in fields {
                    if table.fields.iter().any(|f| f.name == field.name) || !added.insert(field.name.clone()) {
                        return db_error("NOE241", format!("duplicate migration field {}", field.name));
                    }
                    if !matches!(field.kind, ValueType::Int | ValueType::Text | ValueType::Bool |
                        ValueType::Date | ValueType::DateTime | ValueType::Uuid |
                        ValueType::Json | ValueType::Binary | ValueType::Decimal)
                        || field.key || field.auto || field.unique || field.reference.is_some() {
                        return db_error("NOE242", "alter add supports only scalar non-key, non-unique fields");
                    }
                    let value = if let Some(default) = &field.default {
                        match default {
                            Expr::Literal(value) if field.kind.accepts(value) => value.clone(),
                            _ => return db_error("NOE243", "migration default must be a typed literal"),
                        }
                    } else { Value::Null };
                    if !table.rows.is_empty() && field.required && value == Value::Null {
                        return db_error("NOE244", "required new column needs a nonnull default on nonempty table");
                    }
                    for row in &mut table.rows { row.insert(field.name.clone(), value.clone()); }
                    table.seek_cache = OnceLock::new();
                    table.fields.push(field.clone());
                }
                self.version = self.version.checked_add(1).ok_or_else(|| db_failure("NOE245", "schema version exhausted"))?;
                Ok(ExecutionReport::default())
            }
            Statement::Index { name, table, columns } => {
                if self.indexes.contains_key(name) { return db_error("NOE110", format!("duplicate index {name}")); }
                let target = self.tables.get(table).ok_or_else(|| db_failure("NOE100", format!("unknown table {table}")))?;
                if columns.is_empty() || columns.len() > 16 {
                    return db_error("NOE111", "index requires 1..16 columns");
                }
                let unique: BTreeSet<_> = columns.iter().collect();
                if unique.len() != columns.len() { return db_error("NOE111", "index columns must be unique"); }
                for column in columns {
                    if !target.fields.iter().any(|field| &field.name == column) {
                        return db_error("NOE112", format!("unknown index field {column}"));
                    }
                }
                self.indexes.insert(name.clone(), IndexDefinition { table: table.clone(), columns: columns.clone() });
                Ok(ExecutionReport::default())
            }
            Statement::Query { name, params: declared, select } => {
                if self.queries.contains_key(name) { return db_error("NOE113", format!("duplicate query {name}")); }
                if !self.tables.contains_key(&select.table) { return db_error("NOE100", format!("unknown table {}", select.table)); }
                let mut names = BTreeSet::new();
                for param in declared {
                    if !names.insert(param.name.clone()) { return db_error("NOE114", format!("duplicate parameter {}", param.name)); }
                    if let Some(default) = &param.default {
                        let value = eval(default, &Row::new(), &Params::new())?;
                        if !param.kind.accepts(&value) || value == Value::Null {
                            return db_error("NOE115", format!("invalid default for parameter {}", param.name));
                        }
                    }
                }
                let parameter_types = declared.iter()
                    .map(|param| (param.name.clone(), field_expr_type(param.kind)))
                    .collect();
                self.validate_select_types(select, &parameter_types)?;
                self.queries.insert(name.clone(), NamedQuery { params: declared.clone(), select: select.clone() });
                Ok(ExecutionReport::default())
            }
            Statement::Add(name, values) => self.insert(name, values, params),
            Statement::Get(select) => self.select(select, params),
            Statement::Set(name, filter, values) => self.update(name, filter, values, params),
            Statement::Delete(name, filter) => self.delete(name, filter, params),
            Statement::Atomic(body) => {
                let snapshot = self.clone();
                match self.execute_many(body, params) { Ok(report) => Ok(report), Err(error) => { *self = snapshot; Err(error) } }
            }
        }
    }
    fn validate(&self, name: &str, row: &Row, skip: Option<usize>) -> Result<(), ReferenceError> {
        let table = self.tables.get(name).ok_or_else(|| db_failure("NOE100", format!("unknown table {name}")))?;
        for field in &table.fields {
            let value = row.get(&field.name).unwrap_or(&Value::Null);
            if (field.required || field.key) && *value == Value::Null {
                return db_error("NOE201", format!("{} is required", field.name));
            }
            if !field.kind.accepts(value) {
                return db_error("NOE202", format!("invalid {}: expected {:?}", field.name, field.kind));
            }
            if (field.unique || field.key) && *value != Value::Null {
                for (index, existing) in table.rows.iter().enumerate() {
                    if skip != Some(index) && existing.get(&field.name) == Some(value) {
                        return db_error("NOE203", format!("duplicate unique value for {}", field.name));
                    }
                }
            }
            if let Some(reference) = &field.reference {
                if *value != Value::Null {
                    let target = self.tables.get(reference).ok_or_else(|| db_failure("NOE204", format!("unknown reference {reference}")))?;
                    let key = target.fields.iter().find(|candidate| candidate.key)
                        .ok_or_else(|| db_failure("NOE205", format!("reference target {reference} has no key")))?;
                    if !target.rows.iter().any(|candidate| candidate.get(&key.name) == Some(value)) {
                        return db_error("NOE205", format!("foreign key {} refers to missing {reference} row", field.name));
                    }
                }
            }
        }
        for key in row.keys() {
            if !table.fields.iter().any(|field| &field.name == key) {
                return db_error("NOE117", format!("unknown field {name}.{key}"));
            }
        }
        Ok(())
    }

    fn insert(&mut self, name: &str, values: &[(String, Expr)], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get(name).ok_or_else(|| db_failure("NOE100", format!("unknown table {name}")))?;
        let fields = table.fields.clone();
        let next_auto = table.next_auto;

        let mut seen = BTreeSet::new();
        let mut row = Row::new();
        for (field, expr) in values {
            if !seen.insert(field.clone()) { return db_error("NOE116", format!("duplicate assignment {field}")); }
            if !fields.iter().any(|candidate| &candidate.name == field) {
                return db_error("NOE117", format!("unknown field {field}"));
            }
            row.insert(field.clone(), eval(expr, &Row::new(), params)?);
        }

        let mut consumed_auto = false;
        for field in &fields {
            if !row.contains_key(&field.name) {
                let value = if field.auto {
                    consumed_auto = true;
                    Value::Int(next_auto)
                } else if let Some(default) = &field.default {
                    eval(default, &row, params)?
                } else {
                    Value::Null
                };
                row.insert(field.name.clone(), value);
            }
        }

        self.validate(name, &row, None)?;

        let key_value = fields.iter().find(|field| field.key)
            .and_then(|field| row.get(&field.name))
            .and_then(Value::as_i64);
        let table = self.tables.get_mut(name).expect("table validated above");
        table.seek_cache = OnceLock::new();
        table.rows.push(row);
        if consumed_auto {
            table.next_auto = next_auto.checked_add(1).ok_or_else(|| db_failure("NOE214", "auto integer exhausted"))?;
        }
        if let Some(key) = key_value {
            if key >= table.next_auto {
                table.next_auto = key.checked_add(1).ok_or_else(|| db_failure("NOE214", "auto integer exhausted"))?;
            }
        }
        Ok(ExecutionReport { affected: 1, rows: Vec::new() })
    }

    fn select_columns(&self, select: &Select) -> Result<BTreeMap<String, ExprType>, ReferenceError> {
        let base = self.tables.get(&select.table)
            .ok_or_else(|| db_failure("NOE100", format!("unknown table {}", select.table)))?;
        let mut columns = BTreeMap::new();
        for field in &base.fields {
            let kind = field_expr_type(field.kind);
            columns.insert(field.name.clone(), kind);
            columns.insert(format!("{}.{}", select.table, field.name), kind);
        }
        if let Some(join) = &select.join {
            let right = self.tables.get(&join.table)
                .ok_or_else(|| db_failure("NOE100", format!("unknown table {}", join.table)))?;
            for field in &right.fields {
                columns.insert(format!("{}.{}", join.table, field.name), field_expr_type(field.kind));
            }
        }
        Ok(columns)
    }

    fn validate_select_types(
        &self,
        select: &Select,
        params: &BTreeMap<String, ExprType>,
    ) -> Result<(), ReferenceError> {
        let columns = self.select_columns(select)?;

        if let Some(join) = &select.join {
            if infer_expr_type(&join.on, &columns, params)? != ExprType::Bool {
                return db_error("NOE230", "join condition must be bool");
            }
        }
        if let Some(filter) = &select.filter {
            if infer_expr_type(filter, &columns, params)? != ExprType::Bool {
                return db_error("NOE125", "where expression must be bool");
            }
        }

        let grouped = !select.group.is_empty()
            || select.projections.iter().any(|projection| matches!(projection, Projection::Aggregate { .. }));
        let mut outputs = BTreeMap::new();

        for projection in &select.projections {
            match projection {
                Projection::All => {
                    if grouped { return db_error("NOE233", "cannot group *"); }
                }
                Projection::Column(name) => {
                    let kind = columns.get(name).copied()
                        .ok_or_else(|| db_failure("NOE120", format!("unknown projection {name}")))?;
                    if grouped && !select.group.iter().any(|group| group == name) {
                        return db_error("NOE235", format!("projection must be grouped: {name}"));
                    }
                    outputs.insert(name.clone(), kind);
                }
                Projection::Aggregate { function, argument, alias } => {
                    let kind = match function.as_str() {
                        "count" => ExprType::Int,
                        "sum" => {
                            let field = argument.as_ref().ok_or_else(|| db_failure("NOE030", "sum requires a column"))?;
                            let kind = columns.get(field).copied()
                                .ok_or_else(|| db_failure("NOE120", format!("unknown aggregate column {field}")))?;
                            if !matches!(kind, ExprType::Int | ExprType::Decimal) {
                                return db_error("NOE236", "sum requires int or decimal");
                            }
                            kind
                        }
                        "min" | "max" => {
                            let field = argument.as_ref().ok_or_else(|| db_failure("NOE030", format!("{function} requires a column")))?;
                            columns.get(field).copied()
                                .ok_or_else(|| db_failure("NOE120", format!("unknown aggregate column {field}")))?
                        }
                        _ => return db_error("NOE030", format!("unsupported aggregate {function}")),
                    };
                    if outputs.insert(alias.clone(), kind).is_some() {
                        return db_error("NOE234", format!("duplicate output {alias}"));
                    }
                }
            }
        }

        let mut seen_groups = BTreeSet::new();
        for group in &select.group {
            if !seen_groups.insert(group.clone()) { return db_error("NOE232", format!("duplicate group {group}")); }
            if !columns.contains_key(group) { return db_error("NOE120", format!("unknown group {group}")); }
        }

        if let Some(having) = &select.having {
            let mut having_columns = columns.clone();
            having_columns.extend(outputs.clone());
            if infer_expr_type(having, &having_columns, params)? != ExprType::Bool {
                return db_error("NOE125", "having expression must be bool");
            }
        }

        if let Some(sort) = &select.sort {
            if grouped {
                if !outputs.contains_key(sort) && !select.group.iter().any(|group| group == sort) {
                    return db_error("NOE237", format!("group sort must use output column or alias: {sort}"));
                }
            } else if !columns.contains_key(sort) {
                return db_error("NOE120", format!("unknown sort column {sort}"));
            }
        }
        Ok(())
    }

    fn validate_select(&self, select: &Select, params: &Params) -> Result<(), ReferenceError> {
        let types = params.iter().map(|(name, value)| (name.clone(), value_expr_type(value))).collect();
        self.validate_select_types(select, &types)
    }

    fn source_rows(&self, select: &Select, params: &Params) -> Result<Vec<Row>, ReferenceError> {
        let left = self.tables.get(&select.table)
            .ok_or_else(|| db_failure("NOE100", format!("unknown table {}", select.table)))?;
        let mut rows = Vec::new();

        if let Some(join) = &select.join {
            let right = self.tables.get(&join.table)
                .ok_or_else(|| db_failure("NOE100", format!("unknown table {}", join.table)))?;
            let plan = choose_relational_plan(select, left, Some(right));

            match plan.join {
                JoinPlan::Hash { left_key, right_key } => {
                    let mut candidates: HashMap<Value, Vec<&Row>> = HashMap::new();
                    for right_row in &right.rows {
                        if let Some(value) = right_row.get(&right_key) {
                            candidates.entry(value.clone()).or_default().push(right_row);
                        }
                    }

                    for left_row in &left.rows {
                        let mut matched = false;
                        if let Some(key) = left_row.get(&left_key) {
                            if let Some(right_rows) = candidates.get(key) {
                                for right_row in right_rows {
                                    let row = merge(&select.table, left_row, &join.table, Some(right_row));
                                    if predicate(Some(&join.on), &row, params)? {
                                        matched = true;
                                        rows.push(row);
                                    }
                                }
                            }
                        }
                        if join.left && !matched {
                            rows.push(merge(&select.table, left_row, &join.table, None));
                        }
                    }
                }
                JoinPlan::Nested | JoinPlan::None => {
                    for left_row in &left.rows {
                        let mut matched = false;
                        for right_row in &right.rows {
                            let row = merge(&select.table, left_row, &join.table, Some(right_row));
                            if predicate(Some(&join.on), &row, params)? {
                                matched = true;
                                rows.push(row);
                            }
                        }
                        if join.left && !matched {
                            rows.push(merge(&select.table, left_row, &join.table, None));
                        }
                    }
                }
            }
        } else {
            // Only AND-conjoined equality on an explicit integer index is
            // eligible. The original WHERE filter is always evaluated below.
            // This is an immutable in-memory B+ access path, built only once
            // per long-lived snapshot and invalidated before every mutation.
            let indexed = |field: &str| {
                left.fields.iter().any(|f| f.name == field
                    && matches!(f.kind, ValueType::Int | ValueType::Ref)
                    && (f.key || f.unique))
                || self.indexes.values().any(|definition| definition.table == select.table
                    && definition.columns.len() == 1 && definition.columns[0] == field
                    && left.fields.iter().any(|f| f.name == field
                        && matches!(f.kind, ValueType::Int | ValueType::Ref)))
            };
            if let Some((field, value)) = select.filter.as_ref()
                .and_then(|filter| indexed_condition(filter, &select.table, params, &indexed)) {
                if let Some(positions) = left.indexed_row_positions(&field, value) {
                    rows.extend(positions.iter().map(|&i| qualified(&select.table, &left.rows[i])));
                } else {
                    rows.extend(left.rows.iter().map(|row| qualified(&select.table, row)));
                }
            } else {
                rows.extend(left.rows.iter().map(|row| qualified(&select.table, row)));
            }
        }

        rows.into_iter()
            .filter_map(|row| match predicate(select.filter.as_ref(), &row, params) {
                Ok(true) => Some(Ok(row)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
    fn aggregate(projection: &Projection, rows: &[Row]) -> Result<(String, Value), ReferenceError> {
        let Projection::Aggregate { function, argument, alias } = projection else { return db_error("NOE230", "expected aggregate"); };
        if function == "count" { return Ok((alias.clone(), Value::Int(rows.len() as i64))); }
        let field = argument.as_ref().ok_or_else(|| db_failure("NOE230", format!("{function} requires column")))?;
        let values: Vec<Value> = rows.iter().filter_map(|row| lookup(row, field).cloned()).filter(|v| *v != Value::Null).collect();
        let value = match function.as_str() {
            "sum" => {
                let mut total_int = 0i64;
                let mut total_decimal = crate::decimal::Decimal::from_scaled(0).expect("zero is valid");
                let mut decimal = false;
                for value in values {
                    match value {
                        Value::Int(n) if !decimal => {
                            total_int = total_int.checked_add(n).ok_or_else(|| db_failure("NOE232", "aggregate overflow"))?;
                        },
                        Value::Decimal(n) => {
                            if total_int != 0 { return db_error("NOE231", "mixed numeric aggregate"); }
                            decimal = true;
                            total_decimal = total_decimal.checked_add(n)
                                .ok_or_else(|| db_failure("NOE232", "decimal aggregate overflow"))?;
                        },
                        _ => return db_error("NOE231", "sum requires int or decimal"),
                    }
                }
                if decimal { Value::Decimal(total_decimal) } else { Value::Int(total_int) }
            }
            "min" | "max" => {
                let mut iter = values.into_iter(); let mut best = iter.next().unwrap_or(Value::Null);
                for value in iter { if let Some(order) = value.compare(&best) { if (function == "min" && order == Ordering::Less) || (function == "max" && order == Ordering::Greater) { best = value; } } }
                best
            }
            _ => return db_error("NOE230", format!("unsupported aggregate {function}")),
        };
        Ok((alias.clone(), value))
    }
    fn project_group(&self, select: &Select, rows: &[Row], params: &Params) -> Result<Option<Row>, ReferenceError> {
        let first = rows.first().cloned().unwrap_or_default(); let mut out = Row::new();
        for projection in &select.projections {
            match projection {
                Projection::All => for (name, value) in &first { if !name.contains('.') { out.insert(name.clone(), value.clone()); } },
                Projection::Column(name) => { out.insert(name.clone(), lookup(&first, name).cloned().unwrap_or(Value::Null)); }
                Projection::Aggregate { .. } => { let (name, value) = Self::aggregate(projection, rows)?; out.insert(name, value); }
            }
        }
        if predicate(select.having.as_ref(), &out, params)? { Ok(Some(out)) } else { Ok(None) }
    }
    fn select(&self, select: &Select, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        self.validate_select(select, params)?;
        let source = self.source_rows(select, params)?;
        let aggregated = !select.group.is_empty() || select.projections.iter().any(|p| matches!(p, Projection::Aggregate { .. }));
        let mut rows = Vec::new();
        if aggregated {
            // Typed composite keys avoid lossy display/serialization delimiters,
            // and preserve NULL, bool, integer and text identity exactly.
            let mut groups: BTreeMap<Vec<Value>, Vec<Row>> = BTreeMap::new();
            if select.group.is_empty() { groups.insert(Vec::new(), source); }
            else {
                for row in source {
                    let key: Vec<Value> = select.group.iter()
                        .map(|name| lookup(&row, name).cloned().unwrap_or(Value::Null))
                        .collect();
                    groups.entry(key).or_default().push(row);
                }
            }
            for group in groups.values() { if let Some(row) = self.project_group(select, group, params)? { rows.push(row); } }
        } else {
            for source in source {
                let mut out = Row::new();
                for projection in &select.projections {
                    match projection {
                        Projection::All => for (name, value) in &source { if !name.contains('.') { out.insert(name.clone(), value.clone()); } },
                        Projection::Column(name) => { out.insert(name.clone(), lookup(&source, name).cloned().unwrap_or(Value::Null)); }
                        Projection::Aggregate { .. } => unreachable!(),
                    }
                }
                rows.push(out);
            }
        }
        if let Some(sort) = &select.sort { rows.sort_by(|a,b| lookup(a, sort).unwrap_or(&Value::Null).compare(lookup(b, sort).unwrap_or(&Value::Null)).unwrap_or(Ordering::Equal)); }
        let start = select.skip.min(rows.len()); let end = select.take.map(|n| start.saturating_add(n).min(rows.len())).unwrap_or(rows.len());
        // Consume the query buffer instead of cloning every projected row a
        // second time when applying SKIP/TAKE to the result.
        Ok(ExecutionReport { affected: 0, rows: rows.into_iter().skip(start).take(end - start).collect() })
    }
    fn update(&mut self, name: &str, filter: &Expr, values: &[(String, Expr)], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get(name).ok_or_else(|| db_failure("NOE100", format!("unknown table {name}")))?;
        let original = table.rows.clone();
        let key_name = table.fields.iter().find(|field| field.key).map(|field| field.name.clone());
        let field_names: BTreeSet<String> = table.fields.iter().map(|field| field.name.clone()).collect();

        let mut seen = BTreeSet::new();
        for (field, _) in values {
            if !seen.insert(field.clone()) { return db_error("NOE116", format!("duplicate assignment {field}")); }
            if !field_names.contains(field) { return db_error("NOE117", format!("unknown field {field}")); }
            if key_name.as_ref() == Some(field) { return db_error("NOE118", "cannot update primary key"); }
        }

        let mut affected = 0usize;
        for index in 0..original.len() {
            if !predicate(Some(filter), &qualified(name, &original[index]), params)? { continue; }
            let mut candidate = self.tables.get(name).expect("table exists").rows[index].clone();
            for (field, expr) in values {
                let context = qualified(name, &candidate);
                candidate.insert(field.clone(), eval(expr, &context, params)?);
            }
            self.validate(name, &candidate, Some(index))?;
            let table = self.tables.get_mut(name).expect("table exists");
            table.seek_cache = OnceLock::new();
            table.rows[index] = candidate;
            affected += 1;
        }
        Ok(ExecutionReport { affected, rows: Vec::new() })
    }

    fn delete(&mut self, name: &str, filter: &Expr, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get(name).ok_or_else(|| db_failure("NOE100", format!("unknown table {name}")))?;
        let key = table.fields.iter().find(|field| field.key)
            .ok_or_else(|| db_failure("NOE108", "table has no primary key"))?
            .name.clone();

        let mut hit_keys = Vec::new();
        for row in &table.rows {
            if predicate(Some(filter), &qualified(name, row), params)? {
                if let Some(value) = row.get(&key) { hit_keys.push(value.clone()); }
            }
        }

        if !hit_keys.is_empty() {
            for (other_name, other) in &self.tables {
                if other_name == name { continue; }
                for field in &other.fields {
                    if field.reference.as_deref() == Some(name) {
                        for row in &other.rows {
                            if let Some(value) = row.get(&field.name) {
                                if hit_keys.iter().any(|key_value| key_value == value) {
                                    return db_error("NOE208", format!("delete blocked by reference {}.{}", other_name, field.name));
                                }
                            }
                        }
                    }
                }
            }
        }

        let table = self.tables.get_mut(name).expect("table exists");
        table.seek_cache = OnceLock::new();
        let before = table.rows.len();
        table.rows.retain(|row| !hit_keys.iter().any(|key_value| row.get(&key) == Some(key_value)));
        Ok(ExecutionReport { affected: before - table.rows.len(), rows: Vec::new() })
    }

    pub fn run_named(&self, name: &str, args: &Params) -> Result<ExecutionReport, ReferenceError> {
        let definition = self.queries.get(name).ok_or_else(|| db_failure("NOE131", format!("unknown query {name}")))?;
        let mut bound = Params::new();

        for param in &definition.params {
            let value = if let Some(value) = args.get(&param.name) {
                value.clone()
            } else if let Some(default) = &param.default {
                eval(default, &Row::new(), &Params::new())?
            } else {
                return db_error("NOE132", format!("missing parameter {}", param.name));
            };
            if value == Value::Null || !param.kind.accepts(&value) {
                return db_error("NOE132", format!("invalid parameter {}", param.name));
            }
            bound.insert(param.name.clone(), value);
        }
        for name in args.keys() {
            if !definition.params.iter().any(|param| &param.name == name) {
                return db_error("NOE133", format!("unknown parameter {name}"));
            }
        }
        self.select(&definition.select, &bound)
    }
}
fn db_failure(code: &'static str, message: impl Into<String>) -> ReferenceError { ReferenceError { code, line: 1, column: 1, message: message.into() } }
fn db_error<T>(code: &'static str, message: impl Into<String>) -> Result<T, ReferenceError> { Err(db_failure(code, message)) }

pub fn rows_json(rows: &[Row]) -> String {
    let mut out = String::from("[");
    for (i, row) in rows.iter().enumerate() {
        if i != 0 { out.push(','); } out.push('{');
        for (j, (name, value)) in row.iter().enumerate() {
            if j != 0 { out.push(','); } Value::Text(name.clone()).encode_json(&mut out); out.push(':'); value.encode_json(&mut out);
        }
        out.push('}');
    }
    out.push(']'); out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> Params { Params::new() }

    #[test]
    fn last_read_only_get_wins_even_when_it_matches_no_rows() {
        let mut db = Database::default();
        db.execute("table items { id:int key auto\\n value:int required }\\nitems add {value:10}", &params()).unwrap();
        let parsed = parse("items get id where id == 1; items get id where id == 999").unwrap();
        let snapshot = db.query_parsed(&parsed, &params()).unwrap();
        assert!(snapshot.rows.is_empty(), "empty final GET must discard earlier results");
        assert_eq!(snapshot.affected, 0);
        let via_execute = db.execute_parsed(&parsed, &params()).unwrap();
        assert!(via_execute.rows.is_empty());
        let reversed = db.execute("items get id where id == 999; items get id where id == 1", &params()).unwrap();
        assert_eq!(reversed.rows.len(), 1);
    }

    #[test]
    fn nested_atomic_get_does_not_leak_rows_from_earlier_get() {
        let mut db = Database::default();
        db.execute("table items { id:int key auto\\n value:int required }\\nitems add {value:10}", &params()).unwrap();
        let report = db.execute(
            "items get id where id == 1; atomic { items get id where id == 999; }",
            &params(),
        ).unwrap();
        assert!(report.rows.is_empty());

        let mixed = db.execute(
            "atomic { items add {value:20}; atomic { items get id where id == 2; items get id where id == 999; }; }",
            &params(),
        ).unwrap();
        assert_eq!(mixed.affected, 1, "writes must still accumulate affected rows");
        assert!(mixed.rows.is_empty(), "nested empty GET must replace earlier rows");
        assert_eq!(db.execute("items get id", &params()).unwrap().rows.len(), 2);

        // A trailing write has no result set and must not erase the last GET.
        let trailing_write = db.execute(
            "items get id where id == 1; items where id == 999 delete",
            &params(),
        ).unwrap();
        assert_eq!(trailing_write.rows.len(), 1);
    }

    #[test]
    fn crud_constraints_and_atomic_rollback() {
        let mut db = Database::default();
        db.execute(r#"db school
table users {
 id:int key auto
 name:text required unique
 active:bool required = true
}
users add {name:"Amina"}
users add {name:"Brian",active:false}"#, &params()).unwrap();
        assert_eq!(db.execute("users get id,name,active sort id", &params()).unwrap().rows.len(), 2);
        assert!(db.execute(r#"atomic {
users add {name:"Cara"}
users add {name:"Amina"}
}"#, &params()).is_err());
        assert_eq!(db.execute("users get *", &params()).unwrap().rows.len(), 2);
        assert_eq!(db.execute("users where name == \"Brian\" set active = true", &params()).unwrap().affected, 1);
        assert_eq!(db.execute("users where name == \"Amina\" delete", &params()).unwrap().affected, 1);
    }

    #[test]
    fn joins_groups_having_and_parameters() {
        let mut db = Database::default();
        db.execute(r#"table teams { id:int key auto
 name:text required }
table tickets { id:int key auto
 team_id:ref teams required
 effort:int required }
teams add {name:"Build"}
teams add {name:"Ops"}
tickets add {team_id:1,effort:5}
tickets add {team_id:1,effort:3}
tickets add {team_id:2,effort:2}"#, &params()).unwrap();
        let rows = db.execute("tickets get teams.name,count(*) as tickets,sum(effort) as effort join teams on team_id == teams.id group teams.name having effort >= 5 sort teams.name", &params()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("tickets"), Some(&Value::Int(2)));
        assert_eq!(rows[0].get("effort"), Some(&Value::Int(8)));

        let mut bound = Params::new(); bound.insert("min".into(), Value::Int(4));
        assert_eq!(db.execute("tickets get id,effort where effort >= $min", &bound).unwrap().rows.len(), 1);
    }
    #[test]
    fn group_uses_exact_typed_composite_keys_with_embedded_delimiters() {
        let mut db = Database::default();
        db.execute(r#"table metrics { id:int key auto
 label:text required
 region:text required
 amount:int required }
metrics add {label:"west|east",region:"alpha",amount:3}
metrics add {label:"west",region:"east|alpha",amount:5}
metrics add {label:"west|east",region:"alpha",amount:7}
metrics add {label:"west",region:"east|alpha",amount:11}"#, &params()).unwrap();
        let result = db.execute("metrics get label,region,count(*) as items,sum(amount) as total group label,region sort label",
            &params()).unwrap();
        assert_eq!(result.rows.len(), 2);
        let mut totals = result.rows.iter().map(|row| (
            row.get("label").cloned().unwrap(),
            row.get("region").cloned().unwrap(),
            row.get("total").cloned().unwrap(),
        )).collect::<Vec<_>>();
        totals.sort();
        assert_eq!(totals, vec![
            (Value::Text("west".into()), Value::Text("east|alpha".into()), Value::Int(16)),
            (Value::Text("west|east".into()), Value::Text("alpha".into()), Value::Int(10)),
        ]);
    }

    #[test]
    fn decimal_type_exact_arithmetic_grouping_and_codec() {
        use crate::decimal::Decimal;
        let mut db = Database::default();
        db.execute(r#"table ledger { id:int key auto
 category:text required
 amount:decimal required }
ledger add {category:"a",amount:0.1}
ledger add {category:"a",amount:0.2}
ledger add {category:"b",amount:-0.000003}"#, &params()).unwrap();
        let mut sums = db.execute("ledger get category,sum(amount) as total group category sort category",
            &params()).unwrap().rows;
        assert_eq!(sums.len(), 2);
        assert_eq!(sums.remove(0).get("total"), Some(&Value::Decimal(Decimal::parse("0.3").unwrap())));
        let check = db.execute("ledger get amount where amount >= 0.2", &params()).unwrap().rows;
        assert_eq!(check.len(), 1);
        let mut loaded = crate::codec::decode(&crate::codec::encode(&db).unwrap()).unwrap();
        assert_eq!(loaded.execute("ledger get amount where amount == 0.1", &params()).unwrap().rows.len(), 1);
        assert!(db.execute("ledger add {category:\"wrong\",amount:1.0000001}", &params()).is_err());
        assert!(db.execute("ledger where id == 1 set amount = 1.0 / 0.0", &params()).is_err());
        let out = crate::reference::rows_json(&db.execute("ledger get amount take 1", &params()).unwrap().rows);
        assert!(out.contains("\"0.100000\""), "{out}");
    }

    #[test]
    fn explicit_single_column_integer_index_matches_scan_after_updates() {
        let mut db = Database::default();
        db.execute("table readings { id:int key auto\n owner:int required\n level:int required }\nindex owner_idx on readings(owner)", &params()).unwrap();
        for i in 0..550 {
            db.execute(&format!("readings add {{owner:{},level:{i}}}", i % 6), &params()).unwrap();
        }
        let mut p = params(); p.insert("who".into(), Value::Int(3));
        let query = "readings get id,owner,level where owner == $who sort id";
        let indexed = db.execute(query, &p).unwrap().rows;
        assert_eq!(indexed.len(), 92);
        assert!(db.tables["readings"].seek_cache.get().is_some());
        db.execute("readings where id == 4 set owner = 2", &params()).unwrap();
        assert!(db.tables["readings"].seek_cache.get().is_none());
        assert_eq!(db.execute(query, &p).unwrap().rows.len(), 91);
        db.execute("readings where id == 10 delete", &params()).unwrap();
        assert_eq!(db.execute(query, &p).unwrap().rows.len(), 90);
        assert!(db.execute("readings get id where id == 10", &params()).unwrap().rows.is_empty());
        // Unsupported OR never performs unsafe one-sided index pushdown.
        assert_eq!(db.execute("readings get id where owner == 3 || owner == 2", &params())
            .unwrap().rows.len(), 183);
    }

    #[test]
    fn explain_reports_index_only_when_same_access_path_is_eligible() {
        let mut db = Database::default();
        db.execute("table log { id:int key auto\n category:int required }\nindex by_category on log(category)", &params()).unwrap();
        let pk = db.explain("log get id where id == 7", &params()).unwrap();
        assert!(pk.contains("access=memory_bplus(id)"), "{pk}");
        let secondary = db.explain("log get id where category == 4", &params()).unwrap();
        assert!(secondary.contains("access=memory_bplus(category)"), "{secondary}");
        let full = db.explain("log get id where category >= 4", &params()).unwrap();
        assert!(full.contains("access=full_scan"), "{full}");
    }

}
