use crate::reference::{
    Database, Expr, Field, IndexDefinition, Join, NamedQuery, Projection, QueryParam, Row, Select, Table,
};
use crate::value::{Value, ValueType};
use std::collections::{BTreeMap, BTreeSet};

const MAGIC: &[u8; 8] = b"NQRP0001";
const PAYLOAD_VERSION: u32 = 2;
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
    fn count(&mut self, value: usize) -> Result<(), CodecError> {
        if value > u32::MAX as usize { return Err(CodecError("snapshot item count exceeds u32".into())); }
        self.u32(value as u32);
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), CodecError> {
        if value.len() > MAX_STRING || value.len() > u32::MAX as usize {
            return Err(CodecError("string exceeds snapshot limit".into()));
        }
        self.u32(value.len() as u32);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn optional_string(&mut self, value: Option<&str>) -> Result<(), CodecError> {
        match value {
            Some(value) => { self.u8(1); self.string(value) }
            None => { self.u8(0); Ok(()) }
        }
    }
    fn optional_expr(&mut self, value: Option<&Expr>) -> Result<(), CodecError> {
        match value {
            Some(value) => { self.u8(1); encode_expr(self, value) }
            None => { self.u8(0); Ok(()) }
        }
    }
}

struct Reader<'a> { bytes: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, at: 0 } }
    fn take(&mut self, count: usize) -> Result<&'a [u8], CodecError> {
        let end = self.at.checked_add(count).ok_or_else(|| CodecError("snapshot offset overflow".into()))?;
        if end > self.bytes.len() { return Err(CodecError("truncated snapshot".into())); }
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, CodecError> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| CodecError("bad u32".into()))?))
    }
    fn i64(&mut self) -> Result<i64, CodecError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().map_err(|_| CodecError("bad i64".into()))?))
    }
    fn count(&mut self) -> Result<usize, CodecError> {
        let value = self.u32()? as usize;
        if value > MAX_ITEMS { return Err(CodecError("snapshot item count exceeds limit".into())); }
        // Every encoded item consumes at least one additional byte. Reject
        // impossible counts before callers allocate vectors with attacker-
        // controlled capacity (a corrupted 1 MiB snapshot used to request
        // allocations for up to 16 million decoded rows/fields).
        if value > self.bytes.len() - self.at {
            return Err(CodecError("snapshot item count exceeds remaining payload".into()));
        }
        Ok(value)
    }
    fn string(&mut self) -> Result<String, CodecError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING { return Err(CodecError("snapshot string exceeds limit".into())); }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| CodecError("snapshot contains invalid UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, CodecError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.string()?)),
            _ => Err(CodecError("invalid optional-string tag".into())),
        }
    }
    fn optional_expr(&mut self, depth: usize) -> Result<Option<Expr>, CodecError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(decode_expr(self, depth)?)),
            _ => Err(CodecError("invalid optional-expression tag".into())),
        }
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
        Expr::Binary(op, left, right) => {
            writer.u8(4);
            writer.string(op)?;
            encode_expr(writer, left)?;
            encode_expr(writer, right)?;
        }
    }
    Ok(())
}

fn decode_expr(reader: &mut Reader<'_>, depth: usize) -> Result<Expr, CodecError> {
    if depth > 64 { return Err(CodecError("snapshot expression nesting exceeds limit".into())); }
    match reader.u8()? {
        0 => Ok(Expr::Literal(decode_value(reader)?)),
        1 => Ok(Expr::Column(reader.string()?)),
        2 => Ok(Expr::Param(reader.string()?)),
        3 => {
            let op = reader.string()?;
            Ok(Expr::Unary(op, Box::new(decode_expr(reader, depth + 1)?)))
        }
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
    match kind {
        ValueType::Int => 1,
        ValueType::Bool => 2,
        ValueType::Text => 3,
        ValueType::Ref => 4,
        ValueType::Date => 5,
        ValueType::DateTime => 6,
        ValueType::Uuid => 7,
        ValueType::Json => 8,
        ValueType::Binary => 9,
    }
}
fn decode_kind(tag: u8) -> Result<ValueType, CodecError> {
    match tag {
        1 => Ok(ValueType::Int),
        2 => Ok(ValueType::Bool),
        3 => Ok(ValueType::Text),
        4 => Ok(ValueType::Ref),
        5 => Ok(ValueType::Date),
        6 => Ok(ValueType::DateTime),
        7 => Ok(ValueType::Uuid),
        8 => Ok(ValueType::Json),
        9 => Ok(ValueType::Binary),
        _ => Err(CodecError("invalid field type tag".into())),
    }
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
    writer.optional_expr(field.default.as_ref())
}

fn decode_field(reader: &mut Reader<'_>) -> Result<Field, CodecError> {
    let name = reader.string()?;
    let kind = decode_kind(reader.u8()?)?;
    let reference = reader.optional_string()?;
    let flags = reader.u8()?;
    if flags & !0x0f != 0 { return Err(CodecError("unknown field flag".into())); }
    let default = reader.optional_expr(0)?;
    Ok(Field {
        name,
        kind,
        reference,
        required: flags & 1 != 0,
        key: flags & 2 != 0,
        auto: flags & 4 != 0,
        unique: flags & 8 != 0,
        default,
    })
}

fn encode_projection(writer: &mut Writer, projection: &Projection) -> Result<(), CodecError> {
    match projection {
        Projection::All => writer.u8(0),
        Projection::Column(name) => { writer.u8(1); writer.string(name)?; }
        Projection::Aggregate { function, argument, alias } => {
            writer.u8(2);
            writer.string(function)?;
            writer.optional_string(argument.as_deref())?;
            writer.string(alias)?;
        }
    }
    Ok(())
}

fn decode_projection(reader: &mut Reader<'_>) -> Result<Projection, CodecError> {
    match reader.u8()? {
        0 => Ok(Projection::All),
        1 => Ok(Projection::Column(reader.string()?)),
        2 => Ok(Projection::Aggregate {
            function: reader.string()?,
            argument: reader.optional_string()?,
            alias: reader.string()?,
        }),
        _ => Err(CodecError("invalid projection tag".into())),
    }
}

fn encode_select(writer: &mut Writer, select: &Select) -> Result<(), CodecError> {
    writer.string(&select.table)?;
    writer.count(select.projections.len())?;
    for projection in &select.projections { encode_projection(writer, projection)?; }

    match &select.join {
        Some(join) => {
            writer.u8(1);
            writer.string(&join.table)?;
            writer.u8(u8::from(join.left));
            encode_expr(writer, &join.on)?;
        }
        None => writer.u8(0),
    }

    writer.optional_expr(select.filter.as_ref())?;
    writer.count(select.group.len())?;
    for group in &select.group { writer.string(group)?; }
    writer.optional_expr(select.having.as_ref())?;
    writer.optional_string(select.sort.as_deref())?;

    if select.skip > u32::MAX as usize { return Err(CodecError("skip exceeds snapshot limit".into())); }
    writer.u32(select.skip as u32);
    match select.take {
        Some(value) => {
            if value > u32::MAX as usize { return Err(CodecError("take exceeds snapshot limit".into())); }
            writer.u8(1);
            writer.u32(value as u32);
        }
        None => writer.u8(0),
    }
    Ok(())
}

fn decode_select(reader: &mut Reader<'_>) -> Result<Select, CodecError> {
    let table = reader.string()?;
    let projection_count = reader.count()?;
    let mut projections = Vec::with_capacity(projection_count);
    for _ in 0..projection_count { projections.push(decode_projection(reader)?); }
    if projections.is_empty() { return Err(CodecError("select has no projections".into())); }

    let join = match reader.u8()? {
        0 => None,
        1 => {
            let table = reader.string()?;
            let left = match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err(CodecError("invalid join type".into())),
            };
            let on = decode_expr(reader, 0)?;
            Some(Join { table, left, on })
        }
        _ => return Err(CodecError("invalid optional-join tag".into())),
    };

    let filter = reader.optional_expr(0)?;
    let group_count = reader.count()?;
    let mut group = Vec::with_capacity(group_count);
    for _ in 0..group_count { group.push(reader.string()?); }
    let having = reader.optional_expr(0)?;
    let sort = reader.optional_string()?;
    let skip = reader.u32()? as usize;
    let take = match reader.u8()? {
        0 => None,
        1 => Some(reader.u32()? as usize),
        _ => return Err(CodecError("invalid optional-take tag".into())),
    };

    Ok(Select { table, projections, join, filter, group, having, sort, skip, take })
}

fn encode_row(writer: &mut Writer, row: &Row) -> Result<(), CodecError> {
    writer.count(row.len())?;
    for (name, value) in row {
        writer.string(name)?;
        encode_value(writer, value)?;
    }
    Ok(())
}

fn decode_row(reader: &mut Reader<'_>) -> Result<Row, CodecError> {
    let count = reader.count()?;
    let mut row = BTreeMap::new();
    for _ in 0..count {
        let name = reader.string()?;
        if row.insert(name, decode_value(reader)?).is_some() {
            return Err(CodecError("duplicate row field".into()));
        }
    }
    Ok(row)
}

pub fn encode(database: &Database) -> Result<Vec<u8>, CodecError> {
    let mut writer = Writer::new();
    writer.bytes.extend_from_slice(MAGIC);
    writer.u32(PAYLOAD_VERSION);
    writer.u32(database.version);
    writer.optional_string(database.name.as_deref())?;

    writer.count(database.tables.len())?;
    for (name, table) in &database.tables {
        writer.string(name)?;
        writer.i64(table.next_auto);
        writer.count(table.fields.len())?;
        for field in &table.fields { encode_field(&mut writer, field)?; }
        writer.count(table.rows.len())?;
        for row in &table.rows { encode_row(&mut writer, row)?; }
    }

    writer.count(database.indexes.len())?;
    for (name, index) in &database.indexes {
        writer.string(name)?;
        writer.string(&index.table)?;
        writer.count(index.columns.len())?;
        for column in &index.columns { writer.string(column)?; }
    }

    writer.count(database.queries.len())?;
    for (name, query) in &database.queries {
        writer.string(name)?;
        writer.count(query.params.len())?;
        for param in &query.params {
            writer.string(&param.name)?;
            writer.u8(kind_tag(param.kind));
            writer.optional_expr(param.default.as_ref())?;
        }
        encode_select(&mut writer, &query.select)?;
    }

    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Database, CodecError> {
    let mut reader = Reader::new(bytes);
    if reader.take(8)? != MAGIC { return Err(CodecError("invalid Rust snapshot payload magic".into())); }
    if reader.u32()? != PAYLOAD_VERSION { return Err(CodecError("unsupported Rust snapshot payload version".into())); }
    let version = reader.u32()?;
    if version == 0 { return Err(CodecError("invalid database schema version".into())); }
    let name = reader.optional_string()?;

    let table_count = reader.count()?;
    let mut tables = BTreeMap::new();
    for _ in 0..table_count {
        let table_name = reader.string()?;
        let next_auto = reader.i64()?;
        if next_auto < 1 { return Err(CodecError("invalid auto sequence".into())); }

        let field_count = reader.count()?;
        let mut fields = Vec::with_capacity(field_count);
        let mut names = BTreeSet::new();
        for _ in 0..field_count {
            let field = decode_field(&mut reader)?;
            if !names.insert(field.name.clone()) { return Err(CodecError("duplicate schema field".into())); }
            fields.push(field);
        }

        let row_count = reader.count()?;
        let mut rows = Vec::with_capacity(row_count);
        for _ in 0..row_count { rows.push(decode_row(&mut reader)?); }

        if tables.insert(table_name, Table { fields, rows, next_auto }).is_some() {
            return Err(CodecError("duplicate table".into()));
        }
    }

    let index_count = reader.count()?;
    let mut indexes = BTreeMap::new();
    for _ in 0..index_count {
        let index_name = reader.string()?;
        let table = reader.string()?;
        let column_count = reader.count()?;
        if column_count == 0 || column_count > 16 { return Err(CodecError("invalid index column count".into())); }
        let mut columns = Vec::with_capacity(column_count);
        let mut seen = BTreeSet::new();
        for _ in 0..column_count {
            let column = reader.string()?;
            if !seen.insert(column.clone()) { return Err(CodecError("duplicate index column".into())); }
            columns.push(column);
        }
        if !tables.contains_key(&table) { return Err(CodecError("index targets missing table".into())); }
        if columns.iter().any(|column| !tables[&table].fields.iter().any(|field| &field.name == column)) {
            return Err(CodecError("index targets missing field".into()));
        }
        if indexes.insert(index_name, IndexDefinition { table, columns }).is_some() {
            return Err(CodecError("duplicate index".into()));
        }
    }

    let query_count = reader.count()?;
    let mut queries = BTreeMap::new();
    for _ in 0..query_count {
        let query_name = reader.string()?;
        let param_count = reader.count()?;
        let mut params = Vec::with_capacity(param_count);
        let mut seen = BTreeSet::new();
        for _ in 0..param_count {
            let name = reader.string()?;
            if !seen.insert(name.clone()) { return Err(CodecError("duplicate query parameter".into())); }
            let kind = decode_kind(reader.u8()?)?;
            if kind == ValueType::Ref { return Err(CodecError("query parameter cannot be ref".into())); }
            let default = reader.optional_expr(0)?;
            params.push(QueryParam { name, kind, default });
        }
        let select = decode_select(&mut reader)?;
        if !tables.contains_key(&select.table) { return Err(CodecError("query targets missing table".into())); }
        if queries.insert(query_name, NamedQuery { params, select }).is_some() {
            return Err(CodecError("duplicate named query".into()));
        }
    }

    if reader.at != bytes.len() { return Err(CodecError("trailing bytes in snapshot payload".into())); }
    Ok(Database { name, version, tables, indexes, queries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::Params;

    #[test]
    fn absurd_declared_counts_fail_before_allocating_large_vectors() {
        let mut corrupt = Vec::from(MAGIC.as_slice());
        corrupt.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        corrupt.extend_from_slice(&1u32.to_le_bytes()); // DB version
        corrupt.push(0); // no name
        corrupt.extend_from_slice(&(MAX_ITEMS as u32).to_le_bytes()); // impossible table count
        let error = decode(&corrupt).unwrap_err();
        assert!(error.0.contains("remaining payload"));
    }


    #[test]
    fn snapshot_round_trip_preserves_rows_catalog_and_named_queries() {
        let mut database = Database::default();
        database.execute(r#"db school
table users {
 id:int key auto
 name:text required
 active:bool required = true
}
index users_name on users(name)
query active_users(enabled:bool = true) {
 users get id,name where active == $enabled sort id
}
users add {name:"Amina"}
users add {name:"Brian",active:false}"#, &Params::new()).unwrap();

        let encoded = encode(&database).unwrap();
        let mut decoded = decode(&encoded).unwrap();

        assert_eq!(decoded.name.as_deref(), Some("school"));
        assert_eq!(decoded.indexes.len(), 1);
        assert_eq!(decoded.queries.len(), 1);
        assert_eq!(decoded.execute("users get id,name,active", &Params::new()).unwrap().rows.len(), 2);

        let rows = decoded.run_named("active_users", &Params::new()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name"), Some(&Value::Text("Amina".into())));

        decoded.execute("users add {name:\"Cara\"}", &Params::new()).unwrap();
        assert_eq!(
            decoded.execute("users get id sort id", &Params::new()).unwrap().rows[2].get("id"),
            Some(&Value::Int(3))
        );
    }

    #[test]
    fn corrupted_snapshot_is_rejected() {
        let mut payload = b"NQRP0001".to_vec();
        payload.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        payload.push(9);
        assert!(decode(&payload).is_err());
    }
}
