use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Int,
    Bool,
    Text,
    Ref,
    Date,
    DateTime,
    Uuid,
    Json,
    Binary,
}

impl ValueType {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "int" => Some(Self::Int),
            "bool" => Some(Self::Bool),
            "text" => Some(Self::Text),
            "ref" => Some(Self::Ref),
            "date" => Some(Self::Date),
            "datetime" => Some(Self::DateTime),
            "uuid" => Some(Self::Uuid),
            "json" => Some(Self::Json),
            "binary" => Some(Self::Binary),
            _ => None,
        }
    }

    pub fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (_, Value::Null) => true,
            (Self::Int | Self::Ref, Value::Int(_)) => true,
            (Self::Bool, Value::Bool(_)) => true,
            (Self::Text, Value::Text(_)) => true,
            (Self::Date, Value::Text(s)) => crate::extended_types::valid_date(s),
            (Self::DateTime, Value::Text(s)) => crate::extended_types::valid_datetime(s),
            (Self::Uuid, Value::Text(s)) => crate::extended_types::valid_uuid(s),
            (Self::Json, Value::Text(s)) => crate::extended_types::valid_json(s),
            (Self::Binary, Value::Text(s)) => crate::extended_types::valid_binary_hex(s),
            _ => false,
        }
    }
}

impl Value {
    pub fn truthy(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Null => Some(false),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Text(_) => "text",
        }
    }

    pub fn compare(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Null, Self::Null) => Some(Ordering::Equal),
            (Self::Bool(a), Self::Bool(b)) => Some(a.cmp(b)),
            (Self::Int(a), Self::Int(b)) => Some(a.cmp(b)),
            (Self::Text(a), Self::Text(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    pub fn encode_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::Int(value) => out.push_str(&value.to_string()),
            Self::Text(value) => {
                out.push('"');
                for ch in value.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
        }
    }
}
