use crate::value::{Value, ValueType};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

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
enum TokenKind { Ident, Int, Str, Op, Sep, Eof }
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
    let mut out = Vec::new();
    let (mut i, mut line, mut column, mut depth) = (0usize, 1usize, 1usize, 0usize);
    while i < bytes.len() {
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
            out.push(Token { kind: TokenKind::Int, text: source[start..i].into(), line: at_line, column: at_column });
            continue;
        }
        if ch == b'"' {
            adv!(1);
            let mut text = String::new();
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
                        b'n' => '\n', b'r' => '\r', b't' => '\t', b'"' => '"', b'\\' => '\\',
                        _ => return Err(ReferenceError { code: "NOE003", line: at_line, column: at_column, message: "invalid string escape".into() }),
                    });
                } else {
                    if current == b'\n' { return Err(ReferenceError { code: "NOE002", line: at_line, column: at_column, message: "unterminated string".into() }); }
                    text.push(current as char);
                }
            }
            if !closed { return Err(ReferenceError { code: "NOE002", line: at_line, column: at_column, message: "unterminated string".into() }); }
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
    fn take(&mut self) -> Token { let t = self.tokens[self.pos].clone(); self.pos += 1; t }
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

#[derive(Clone, Debug)]
pub struct Table { pub fields: Vec<Field>, pub rows: Vec<Row>, pub(crate) next_auto: i64 }
#[derive(Clone, Debug, Default)]
pub struct Database { pub name: Option<String>, pub tables: BTreeMap<String, Table> }
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
            ("!", Value::Bool(value)) => Ok(Value::Bool(!value)), ("-", Value::Int(value)) => Ok(Value::Int(-value)),
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
        let statements = parse(source)?; self.execute_many(&statements, params)
    }
    fn execute_many(&mut self, statements: &[Statement], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let mut report = ExecutionReport::default();
        for statement in statements { let current = self.execute_one(statement, params)?; report.affected += current.affected; if !current.rows.is_empty() { report.rows = current.rows; } }
        Ok(report)
    }
    fn execute_one(&mut self, statement: &Statement, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        match statement {
            Statement::Database(name) => { self.name = Some(name.clone()); Ok(ExecutionReport::default()) }
            Statement::Table(name, fields) => {
                if let Some(existing) = self.tables.get(name) {
                    if existing.fields.len() == fields.len() && existing.fields.iter().zip(fields).all(|(a,b)| a.name == b.name && a.kind == b.kind) { return Ok(ExecutionReport::default()); }
                    return db_error("NOE200", format!("table {name} already has a different schema"));
                }
                let mut names = BTreeSet::new(); for field in fields { if !names.insert(field.name.clone()) { return db_error("NOE201", format!("duplicate field {}", field.name)); } }
                self.tables.insert(name.clone(), Table { fields: fields.clone(), rows: Vec::new(), next_auto: 1 }); Ok(ExecutionReport::default())
            }
            Statement::AlterAdd(name, fields) => {
                let table = self.tables.get_mut(name).ok_or_else(|| db_failure("NOE202", format!("unknown table {name}")))?;
                for field in fields {
                    if table.fields.iter().any(|f| f.name == field.name) { return db_error("NOE201", format!("duplicate field {}", field.name)); }
                    let value = if let Some(default) = &field.default { eval(default, &Row::new(), params)? } else { Value::Null };
                    if field.required && value == Value::Null { return db_error("NOE203", format!("required field {} needs default", field.name)); }
                    for row in &mut table.rows { row.insert(field.name.clone(), value.clone()); }
                    table.fields.push(field.clone());
                }
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
    fn validate(name: &str, table: &Table, row: &Row, skip: Option<usize>) -> Result<(), ReferenceError> {
        for field in &table.fields {
            let value = row.get(&field.name).unwrap_or(&Value::Null);
            if field.required && *value == Value::Null { return db_error("NOE210", format!("{name}.{} is required", field.name)); }
            if !field.kind.accepts(value) { return db_error("NOE211", format!("{name}.{} expects {:?}, got {}", field.name, field.kind, value.type_name())); }
            if (field.unique || field.key) && *value != Value::Null {
                for (index, existing) in table.rows.iter().enumerate() { if skip != Some(index) && existing.get(&field.name) == Some(value) { return db_error("NOE212", format!("unique constraint failed on {name}.{}", field.name)); } }
            }
        }
        for key in row.keys() { if !table.fields.iter().any(|field| &field.name == key) { return db_error("NOE213", format!("unknown field {name}.{key}")); } }
        Ok(())
    }
    fn insert(&mut self, name: &str, values: &[(String, Expr)], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get_mut(name).ok_or_else(|| db_failure("NOE202", format!("unknown table {name}")))?;
        let mut row = Row::new(); for (field, expr) in values { row.insert(field.clone(), eval(expr, &Row::new(), params)?); }
        for field in &table.fields {
            if !row.contains_key(&field.name) {
                let value = if field.auto { let value = Value::Int(table.next_auto); table.next_auto = table.next_auto.checked_add(1).ok_or_else(|| db_failure("NOE214", "auto integer exhausted"))?; value }
                    else if let Some(default) = &field.default { eval(default, &row, params)? } else { Value::Null };
                row.insert(field.name.clone(), value);
            }
        }
        Self::validate(name, table, &row, None)?; table.rows.push(row);
        Ok(ExecutionReport { affected: 1, rows: Vec::new() })
    }
    fn source_rows(&self, select: &Select, params: &Params) -> Result<Vec<Row>, ReferenceError> {
        let left = self.tables.get(&select.table).ok_or_else(|| db_failure("NOE202", format!("unknown table {}", select.table)))?;
        let mut rows = Vec::new();
        if let Some(join) = &select.join {
            let right = self.tables.get(&join.table).ok_or_else(|| db_failure("NOE202", format!("unknown table {}", join.table)))?;
            for left_row in &left.rows {
                let mut matched = false;
                for right_row in &right.rows {
                    let row = merge(&select.table, left_row, &join.table, Some(right_row));
                    if predicate(Some(&join.on), &row, params)? { matched = true; rows.push(row); }
                }
                if join.left && !matched { rows.push(merge(&select.table, left_row, &join.table, None)); }
            }
        } else { rows.extend(left.rows.iter().map(|row| qualified(&select.table, row))); }
        rows.into_iter().filter_map(|row| match predicate(select.filter.as_ref(), &row, params) { Ok(true) => Some(Ok(row)), Ok(false) => None, Err(e) => Some(Err(e)) }).collect()
    }
    fn aggregate(projection: &Projection, rows: &[Row]) -> Result<(String, Value), ReferenceError> {
        let Projection::Aggregate { function, argument, alias } = projection else { return db_error("NOE230", "expected aggregate"); };
        if function == "count" { return Ok((alias.clone(), Value::Int(rows.len() as i64))); }
        let field = argument.as_ref().ok_or_else(|| db_failure("NOE230", format!("{function} requires column")))?;
        let values: Vec<Value> = rows.iter().filter_map(|row| lookup(row, field).cloned()).filter(|v| *v != Value::Null).collect();
        let value = match function.as_str() {
            "sum" => {
                let mut total = 0i64;
                for value in values { total = total.checked_add(value.as_i64().ok_or_else(|| db_failure("NOE231", "sum requires int"))?).ok_or_else(|| db_failure("NOE232", "aggregate overflow"))?; }
                Value::Int(total)
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
        let source = self.source_rows(select, params)?;
        let aggregated = !select.group.is_empty() || select.projections.iter().any(|p| matches!(p, Projection::Aggregate { .. }));
        let mut rows = Vec::new();
        if aggregated {
            let mut groups: BTreeMap<String, Vec<Row>> = BTreeMap::new();
            if select.group.is_empty() { groups.insert(String::new(), source); }
            else {
                for row in source { let key = select.group.iter().map(|name| format!("{:?}", lookup(&row, name).unwrap_or(&Value::Null))).collect::<Vec<_>>().join("|"); groups.entry(key).or_default().push(row); }
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
        Ok(ExecutionReport { affected: 0, rows: rows[start..end].to_vec() })
    }
    fn update(&mut self, name: &str, filter: &Expr, values: &[(String, Expr)], params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get_mut(name).ok_or_else(|| db_failure("NOE202", format!("unknown table {name}")))?;
        let original = table.rows.clone(); let mut affected = 0usize;
        for index in 0..original.len() {
            if !predicate(Some(filter), &qualified(name, &original[index]), params)? { continue; }
            let mut candidate = original[index].clone();
            for (field, expr) in values { let context = qualified(name, &candidate); candidate.insert(field.clone(), eval(expr, &context, params)?); }
            Self::validate(name, table, &candidate, Some(index))?; table.rows[index] = candidate; affected += 1;
        }
        Ok(ExecutionReport { affected, rows: Vec::new() })
    }
    fn delete(&mut self, name: &str, filter: &Expr, params: &Params) -> Result<ExecutionReport, ReferenceError> {
        let table = self.tables.get_mut(name).ok_or_else(|| db_failure("NOE202", format!("unknown table {name}")))?;
        let mut kept = Vec::new(); let mut affected = 0usize;
        for row in table.rows.drain(..) { if predicate(Some(filter), &qualified(name, &row), params)? { affected += 1; } else { kept.push(row); } }
        table.rows = kept; Ok(ExecutionReport { affected, rows: Vec::new() })
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
}
