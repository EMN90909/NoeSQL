//! Strict, bounded Noqeri native-web session-v2 JSON bindings.
//! No SQL interpolation, javascript runtime or external JSON dependency.
//! String JSON values stay strings; integers/bools/null keep their types.
//! Optional explicit typed bindings use {"kind":"decimal","value":"12.50"}.
use crate::decimal::Decimal;
use crate::reference::Params;
use crate::value::{Value, ValueType};

const MAX_BIND_BYTES: usize = 32 * 1024;
const MAX_BIND_VALUE: usize = 4096;
const MAX_BIND_COUNT: usize = 32;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> { self.bytes.get(self.pos).copied() }
    fn advance(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }
    fn space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) { self.pos += 1; }
    }
    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.advance() == Some(byte) { Ok(()) }
        else { Err(format!("invalid binding JSON at offset {}", self.pos)) }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let b = self.advance().ok_or("truncated Unicode escape")?;
            let digit = (b as char).to_digit(16).ok_or("invalid Unicode escape")?;
            value = (value << 4) | digit;
        }
        Ok(value)
    }
    fn json_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut result = String::new();
        loop {
            let b = self.advance().ok_or("unterminated JSON string")?;
            if b == b'"' { break; }
            if b < 0x20 { return Err("unescaped control character in JSON".into()); }
            if b == b'\\' {
                let esc = self.advance().ok_or("unterminated JSON escape")?;
                match esc {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'b' => result.push('\u{0008}'),
                    b'f' => result.push('\u{000c}'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'u' => {
                        let high = self.hex4()?;
                        let scalar = if (0xD800..=0xDBFF).contains(&high) {
                            self.expect(b'\\')?;
                            self.expect(b'u')?;
                            let low = self.hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err("invalid low Unicode surrogate".into());
                            }
                            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                        } else { high };
                        let ch = char::from_u32(scalar)
                            .ok_or("invalid Unicode surrogate or code point")?;
                        result.push(ch);
                    },
                    _ => return Err("unsupported JSON string escape".into()),
                }
            } else if b.is_ascii() {
                result.push(b as char);
            } else {
                // The full buffer is validated as UTF-8 before parsing.
                let tail = std::str::from_utf8(&self.bytes[self.pos-1..])
                    .map_err(|_| "invalid binding UTF-8")?;
                let ch = tail.chars().next().ok_or("invalid binding UTF-8")?;
                result.push(ch);
                self.pos += ch.len_utf8()-1;
            }
            if result.len() > MAX_BIND_VALUE {
                return Err("JSON binding string exceeds 4096-byte limit".into());
            }
        }
        Ok(result)
    }
    fn literal(&mut self, literal: &[u8]) -> Result<(), String> {
        let end = self.pos.checked_add(literal.len()).ok_or("binding overflow")?;
        if self.bytes.get(self.pos..end) != Some(literal) {
            return Err("invalid binding JSON literal".into());
        }
        self.pos = end;
        Ok(())
    }
    fn numeric(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') { self.pos += 1; }
        match self.advance() {
            Some(b'0') => {
                if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    return Err("JSON number may not have leading zeroes".into());
                }
            },
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|b| b.is_ascii_digit()) { self.pos += 1; }
            },
            _ => return Err("invalid JSON number".into()),
        }
        let mut decimal = false;
        if self.peek() == Some(b'.') {
            decimal = true;
            self.pos += 1;
            let at = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) { self.pos += 1; }
            if at == self.pos { return Err("missing decimal digits".into()); }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            return Err("JSON exponent requires a separately qualified numeric conversion".into());
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid number encoding")?;
        if decimal {
            Decimal::parse(raw).map(Value::Decimal)
                .ok_or_else(|| "decimal outside exact NoeSQL 38,6 range".into())
        } else {
            raw.parse::<i64>().map(Value::Int)
                .map_err(|_| "integer outside int64 range".into())
        }
    }
    fn typed(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut kind: Option<String> = None;
        let mut value: Option<String> = None;
        for i in 0..2 {
            self.space();
            let name = self.json_string()?;
            self.space();
            self.expect(b':')?;
            self.space();
            let data = self.json_string()?;
            match name.as_str() {
                "kind" if kind.is_none() => kind = Some(data),
                "value" if value.is_none() => value = Some(data),
                _ => return Err("typed binding only accepts unique kind and value strings".into()),
            }
            self.space();
            if i == 0 { self.expect(b',')?; }
        }
        self.space();
        self.expect(b'}')?;
        let kind = kind.ok_or("missing typed binding kind")?;
        let raw = value.ok_or("missing typed binding value")?;
        let field = ValueType::parse(&kind).ok_or("unknown typed binding kind")?;
        let value = match field {
            ValueType::Int | ValueType::Ref => raw.parse::<i64>().ok().map(Value::Int),
            ValueType::Bool => match raw.as_str() {
                "true" => Some(Value::Bool(true)), "false" => Some(Value::Bool(false)), _ => None
            },
            ValueType::Decimal => Decimal::parse(&raw).map(Value::Decimal),
            ValueType::Text | ValueType::Date | ValueType::DateTime | ValueType::Uuid
                | ValueType::Json | ValueType::Binary => Some(Value::Text(raw)),
        }.ok_or("invalid typed binding value")?;
        if !field.accepts(&value) {
            return Err("typed binding failed NoeSQL field validation".into());
        }
        Ok(value)
    }
    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'"') => self.json_string().map(Value::Text),
            Some(b'{') => self.typed(),
            Some(b't') => { self.literal(b"true")?; Ok(Value::Bool(true)) },
            Some(b'f') => { self.literal(b"false")?; Ok(Value::Bool(false)) },
            Some(b'n') => { self.literal(b"null")?; Ok(Value::Null) },
            Some(b'-' | b'0'..=b'9') => self.numeric(),
            _ => Err("invalid or unsupported JSON binding value".into()),
        }
    }
}

pub(crate) fn parse_bindings(bytes: &[u8]) -> Result<Params, String> {
    if bytes.len() > MAX_BIND_BYTES { return Err("binding JSON exceeds 32 KiB".into()); }
    // Do not reinterpret malformed bytes as lossy UTF-8.
    std::str::from_utf8(bytes).map_err(|_| "binding JSON is not UTF-8")?;
    let mut reader = Reader { bytes, pos: 0 };
    reader.space();
    reader.expect(b'{')?;
    reader.space();
    let mut params = Params::new();
    if reader.peek() == Some(b'}') {
        reader.advance();
    } else {
        loop {
            if params.len() >= MAX_BIND_COUNT { return Err("more than 32 NoeSQL bindings".into()); }
            let key = reader.json_string()?;
            if key.is_empty() || key.len() > 64 ||
                !key.as_bytes()[0].is_ascii_alphabetic() && key.as_bytes()[0] != b'_' ||
                !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err("binding name must be an ASCII identifier <=64 bytes".into());
            }
            reader.space();
            reader.expect(b':')?;
            reader.space();
            let value = reader.value()?;
            if params.insert(key, value).is_some() {
                return Err("duplicate NoeSQL binding name".into());
            }
            reader.space();
            match reader.advance() {
                Some(b'}') => break,
                Some(b',') => { reader.space(); },
                _ => return Err("expected comma or closing brace in binding object".into()),
            }
        }
    }
    reader.space();
    if reader.peek().is_some() { return Err("trailing bytes after JSON binding object".into()); }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noesql_web_string_bindings_preserve_sql_like_input() {
        let params = parse_bindings(
            br#"{"name":"A \"name\"; DROP TABLE people; -- <b>&","city":"Nairobi","emoji":"\ud83d\ude80"}"#
        ).unwrap();
        assert_eq!(params.get("name"), Some(&Value::Text(
            "A \"name\"; DROP TABLE people; -- <b>&".into()
        )));
        assert_eq!(params.get("emoji"), Some(&Value::Text("🚀".into())));
        assert_eq!(params.len(), 3);
    }
    #[test]
    fn typed_json_bindings_are_lossless_and_checked() {
        let params = parse_bindings(br#"{
            "on":true,"missing":null,"count":-8,"price":12.500001,
            "amount":{"kind":"decimal","value":"0.010000"},
            "id":{"kind":"uuid","value":"550e8400-e29b-41d4-a716-446655440000"}
        }"#).unwrap();
        assert_eq!(params.get("on"), Some(&Value::Bool(true)));
        assert_eq!(params.get("count"), Some(&Value::Int(-8)));
        assert_eq!(params.get("missing"), Some(&Value::Null));
        assert_eq!(params.get("price"), Some(&Value::Decimal(
            Decimal::parse("12.500001").unwrap()
        )));
        assert_eq!(params.get("amount"), Some(&Value::Decimal(
            Decimal::parse("0.01").unwrap()
        )));
        assert!(matches!(params.get("id"), Some(Value::Text(_))));
    }
    #[test]
    fn malformed_duplicate_overflow_and_surrogate_bindings_fail_closed() {
        for payload in [
            r#"{"x":1,"x":2}"#,
            r#"{"x":"\ud800"}"#,
            r#"{"x":"\ud800\ud800"}"#,
            r#"{"x":"oops""#,
            r#"{"x":01}"#,
            r#"{"x":1e5}"#,
            r#"{"x":1.2345678}"#,
            r#"{"x":{"kind":"bool","value":"True"}}"#,
            r#"{"x":[1]}"#,
            r#"{"x":true,}"#,
            r#"{"x":1} extra"#,
        ] {
            assert!(parse_bindings(payload.as_bytes()).is_err(), "{payload}");
        }
        assert!(parse_bindings(&vec![b'a'; MAX_BIND_BYTES + 1]).is_err());
    }
}
