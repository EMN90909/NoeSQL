use crate::reference::{Database, Expr, Field, Row, Table};
use crate::value::{Value, ValueType};
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"NQRP0001";
const MAX_STRING: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for CodecError {}

struct Writer { bytes: Vec<u8> }
impl Writer {
    fn new() -> Self { Self { bytes: Vec::new() } }
    fn u8(&mut self, value: u8) { self.bytes.push(value); }
    fn u32(&mut self, value: u32) { self.bytes.extend_from_slice(&value.to_le_bytes()); }
    fn i64(&mut self, value: i64) { self.bytes.extend_from_slice(&value.to_le_bytes()); }
    fn string(&mut self, value: &str) -> Result<(), CodecError> {
        if value.len() > MAX_STRING || value.len() > u32::MAX as usize { return Err(CodecError("string exceeds snapshot limit".into())); }
        self.u32(value.len() as u32);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn optional_string(&mut self, value: Option<&str>) -> Result<(), CodecError> {
        match value { Some(value) => { self.u8(1); self.string(value) }, None => { self.u8(0); Ok(()) } }
    }
}

struct Reader<'a> { bytes: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, at: 0 } }
    fn take(&mut self, count: usize) -> Result<&'a [u8], CodecError> {
        let end = self.at.checked_add(count).ok_or_else(|| CodecError("snapshot offset overflow".into()))?;
        if end > self.bytes.len() { return Err(CodecError("truncated snapshot".into())); }
        let out = &self.bytes[self.at..end]; self.at = end; Ok(out)
    }
    fn u8(&mut self) -> Result<u8, CodecError> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> Result<u32, CodecError> { Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| CodecError("bad u32".into()))?)) }
    fn i64(&mut self) -> Result<i64, CodecError> { Ok(i64::from_le_bytes(self.take(8)?.try_into().map_err(|_| CodecError("bad i64".into()))?)) }
    fn count(&mut self) -> Result<usize, CodecError> {
        let value = self.u32()? as usize;
        if value > MAX_ITEMS { return Err(CodecError("snapshot item count exceeds limit".into())); }
        Ok(value)
    }
    fn string(&mut self) -> Result<String, CodecError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING { return Err(CodecError("snapshot string exceeds limit".into())); }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| CodecError("snapshot contains invalid UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, CodecError> {
        match self.u8()? { 0 => Ok(None), 1 => Ok(Some(self.string()?)), _ => Err(CodecError("invalid optional-string tag".into())) }
    }
}

fn encode_value(writer: &mut Writer, value: &Value) -> Result<(), CodecError> {
    match value {
        Value::Null => writer.u8(0),
        Value::Bool(false) => writer.u8(1),
        Value::Bool(true) => writer.u8(2),
        Value::Int(value) => { writer.u8(3); writer.i64(*value); }
        Value::Text(value) => { writer.u8(4); writer.string(value)?; }
    }
    Ok(())
}
fn decode_value(reader: &mut Reader<'_>) -> Result<Value, CodecError> {
    match reader.u8()? {
        0 => Ok(Value::Null),
        1 => Ok(Value::Bool(false)),
        2 => Ok(Value::Bool(true)),
        3 => Ok(Value::Int(reader.i64()?)),
        4 => Ok(Value::Text(reader.string()?)),
        _ => Err(CodecError("invalid value tag".into())),
    }
}

fn encode_expr(writer: &mut Writer, expr: &Expr) -> Result<(), CodecError> {
    match expr {
        Expr::Literal(value) => { writer.u8(0); encode_value(writer, value)?; }
        Expr::Column(name) => { writer.u8(1); writer.string(name)?; }
        Expr::Param(name) => { writer.u8(2); writer.string(name)?; }
        Expr::Unary(op, arg) => { writer.u8(3); writer.string(op)?; encode_expr(writer, arg)?; }
        Expr::Binary(op, left, right) => { writer.u8(4); writer.string(op)?; encode_expr(writer, left)?; encode_expr(writer, right)?; }
    }
    Ok(())
}
fn decode_expr(reader: &mut Reader<'_>, depth: usize) -> Result<Expr, CodecError> {
    if depth > 64 { return Err(CodecError("snapshot expression nesting exceeds limit".into())); }
    match reader.u8()? {
        0 => Ok(Expr::Literal(decode_value(reader)?)),
        1 => Ok(Expr::Column(reader.string()?)),
        2 => Ok(Expr::Param(reader.string()?)),
        3 => { let op = reader.string()?; Ok(Expr::Unary(op, Box::new(decode_expr(reader, depth + 1)?))) }
        4 => {
            let op = reader.string()?;
            let left = decode_expr(reader, depth + 1)?;
            let right = decode_expr(reader, depth + 1)?;
            Ok(Expr::Binary(op, Box::new(left), Box::new(right)))
        }
        _ => Err(CodecError("invalid expression tag".into())),
    }
}

fn kind_tag(kind: ValueType) -> u8 {
    match kind { ValueType::Int => 1, ValueType::Bool => 2, ValueType::Text => 3, ValueType::Ref => 4 }
}
fn decode_kind(tag: u8) -> Result<ValueType, CodecError> {
    match tag { 1 => Ok(ValueType::Int), 2 => Ok(ValueType::Bool), 3 => Ok(ValueType::Text), 4 => Ok(ValueType::Ref), _ => Err(CodecError("invalid field type tag".into())) }
}

fn encode_field(writer: &mut Writer, field: &Field) -> Result<(), CodecError> {
    writer.string(&field.name)?;
    writer.u8(kind_tag(field.kind));
    writer.optional_string(field.reference.as_deref())?;
    let mut flags = 0u8;
    if field.required { flags |= 1; }
    if field.key { flags |= 2; }
    if field.auto { flags |= 4; }
    if field.unique { flags |= 8; }
    writer.u8(flags);
    match &field.default {
        Some(expr) => { writer.u8(1); encode_expr(writer, expr)?; }
        None => writer.u8(0),
    }
    Ok(())
}
fn decode_field(reader: &mut Reader<'_>) -> Result<Field, CodecError> {
    let name = reader.string()?;
    let kind = decode_kind(reader.u8()?)?;
    let reference = reader.optional_string()?;
    let flags = reader.u8()?;
    if flags & !0x0f != 0 { return Err(CodecError("unknown field flag".into())); }
    let default = match reader.u8()? { 0 => None, 1 => Some(decode_expr(reader, 0)?), _ => return Err(CodecError("invalid default-expression tag".into())) };
    Ok(Field {
        name, kind, reference,
        required: flags & 1 != 0,
        key: flags & 2 != 0,
        auto: flags & 4 != 0,
        unique: flags & 8 != 0,
        default,
    })
}

fn encode_row(writer: &mut Writer, row: &Row) -> Result<(), CodecError> {
    if row.len() > u32::MAX as usize { return Err(CodecError("row has too many fields".into())); }
    writer.u32(row.len() as u32);
    for (name, value) in row { writer.string(name)?; encode_value(writer, value)?; }
    Ok(())
}
fn decode_row(reader: &mut Reader<'_>) -> Result<Row, CodecError> {
    let count = reader.count()?;
    let mut row = BTreeMap::new();
    for _ in 0..count {
        let name = reader.string()?;
        if row.insert(name, decode_value(reader)?).is_some() { return Err(CodecError("duplicate row field".into())); }
    }
    Ok(row)
}

pub fn encode(database: &Database) -> Result<Vec<u8>, CodecError> {
    let mut writer = Writer::new();
    writer.bytes.extend_from_slice(MAGIC);
    writer.u32(1);
    writer.optional_string(database.name.as_deref())?;
    if database.tables.len() > u32::MAX as usize { return Err(CodecError("too many tables".into())); }
    writer.u32(database.tables.len() as u32);
    for (name, table) in &database.tables {
        writer.string(name)?;
        writer.i64(table.next_auto);
        if table.fields.len() > u32::MAX as usize || table.rows.len() > u32::MAX as usize { return Err(CodecError("table exceeds snapshot count limit".into())); }
        writer.u32(table.fields.len() as u32);
        for field in &table.fields { encode_field(&mut writer, field)?; }
        writer.u32(table.rows.len() as u32);
        for row in &table.rows { encode_row(&mut writer, row)?; }
    }
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Database, CodecError> {
    let mut reader = Reader::new(bytes);
    if reader.take(8)? != MAGIC { return Err(CodecError("invalid Rust snapshot payload magic".into())); }
    if reader.u32()? != 1 { return Err(CodecError("unsupported Rust snapshot payload version".into())); }
    let name = reader.optional_string()?;
    let table_count = reader.count()?;
    let mut tables = BTreeMap::new();
    for _ in 0..table_count {
        let table_name = reader.string()?;
        let next_auto = reader.i64()?;
        if next_auto < 1 { return Err(CodecError("invalid auto sequence".into())); }
        let field_count = reader.count()?;
        let mut fields = Vec::with_capacity(field_count);
        let mut names = std::collections::BTreeSet::new();
        for _ in 0..field_count {
            let field = decode_field(&mut reader)?;
            if !names.insert(field.name.clone()) { return Err(CodecError("duplicate schema field".into())); }
            fields.push(field);
        }
        let row_count = reader.count()?;
        let mut rows = Vec::with_capacity(row_count);
        for _ in 0..row_count { rows.push(decode_row(&mut reader)?); }
        if tables.insert(table_name, Table { fields, rows, next_auto }).is_some() { return Err(CodecError("duplicate table".into())); }
    }
    if reader.at != bytes.len() { return Err(CodecError("trailing bytes in snapshot payload".into())); }
    Ok(Database { name, tables })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Params;

    #[test]
    fn snapshot_round_trip_preserves_schema_rows_and_defaults() {
        let mut database = Database::default();
        database.execute(r#"db school
table users {
 id:int key auto
 name:text required
 active:bool required = true
}
users add {name:"Amina"}"#, &Params::new()).unwrap();
        let encoded = encode(&database).unwrap();
        let mut decoded = decode(&encoded).unwrap();
        assert_eq!(decoded.name.as_deref(), Some("school"));
        assert_eq!(decoded.execute("users get id,name,active", &Params::new()).unwrap().rows.len(), 1);
        decoded.execute("users add {name:\"Brian\"}", &Params::new()).unwrap();
        assert_eq!(decoded.execute("users get id sort id", &Params::new()).unwrap().rows[1].get("id"), Some(&Value::Int(2)));
    }

    #[test]
    fn corrupted_snapshot_is_rejected() {
        let mut payload = b"NQRP0001".to_vec();
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.push(9);
        assert!(decode(&payload).is_err());
    }
}
