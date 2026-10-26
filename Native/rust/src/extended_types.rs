//! Strict bounded validation for portable, text-backed extended column types.
//! Date/time and UUID use a canonical encoding so equality and lexical sort
//! agree with chronological/byte order. JSON uses a depth-limited validator.
//! Decimal and native binary arithmetic/storage are not yet provided.

pub const MAX_JSON_BYTES: usize = 1024 * 1024;

pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' { return false; }
    fn digits(b: &[u8]) -> Option<u32> {
        if b.iter().any(|c| !c.is_ascii_digit()) { return None; }
        Some(b.iter().fold(0u32, |n, c| n * 10 + u32::from(c - b'0')))
    }
    let (Some(year), Some(month), Some(day)) =
        (digits(&b[..4]), digits(&b[5..7]), digits(&b[8..])) else { return false; };
    if year == 0 || !(1..=12).contains(&month) { return false; }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    (1..=days[month as usize - 1]).contains(&day)
}

pub fn valid_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 24 || b[10] != b'T' || b[13] != b':' || b[16] != b':' ||
        b[19] != b'.' || b[23] != b'Z' || !s.is_ascii() || !valid_date(&s[..10]) { return false; }
    if [11, 12, 14, 15, 17, 18, 20, 21, 22]
        .iter().any(|i| !b[*i].is_ascii_digit()) { return false; }
    let hh = (b[11] - b'0') * 10 + b[12] - b'0';
    let mm = (b[14] - b'0') * 10 + b[15] - b'0';
    let ss = (b[17] - b'0') * 10 + b[18] - b'0';
    hh < 24 && mm < 60 && ss < 60
}

pub fn valid_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 { return false; }
    for (i, c) in b.iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            if *c != b'-' { return false; }
        } else if !matches!(*c, b'0'..=b'9' | b'a'..=b'f') {
            return false;
        }
    }
    true
}

pub fn valid_binary_hex(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b.len() <= 2 * 1024 * 1024 + 2 &&
        b.starts_with(b"0x") && (b.len() - 2) % 2 == 0 &&
        b[2..].iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

struct Json<'a> { src: &'a [u8], at: usize }
impl Json<'_> {
    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.src.get(self.at) {
            self.at += 1;
        }
    }
    fn one(&mut self, ch: u8) -> bool {
        self.ws();
        if self.src.get(self.at) == Some(&ch) { self.at += 1; true } else { false }
    }
    fn hex4(&mut self) -> Option<u16> {
        let bytes = self.src.get(self.at..self.at + 4)?;
        let mut result = 0u16;
        for &ch in bytes {
            let n = match ch {
                b'0'..=b'9' => u16::from(ch - b'0'),
                b'a'..=b'f' => u16::from(ch - b'a' + 10),
                b'A'..=b'F' => u16::from(ch - b'A' + 10),
                _ => return None,
            };
            result = (result << 4) | n;
        }
        self.at += 4;
        Some(result)
    }
    fn string(&mut self) -> bool {
        if !self.one(b'"') { return false; }
        while let Some(&c) = self.src.get(self.at) {
            self.at += 1;
            match c {
                b'"' => return true,
                b'\\' => {
                    let Some(&escaped) = self.src.get(self.at) else { return false };
                    self.at += 1;
                    match escaped {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {},
                        b'u' => {
                            let Some(code) = self.hex4() else { return false };
                            if (0xD800..=0xDBFF).contains(&code) {
                                if self.src.get(self.at..self.at + 2) != Some(&b"\\u"[..]) { return false }
                                self.at += 2;
                                let Some(low) = self.hex4() else { return false };
                                if !(0xDC00..=0xDFFF).contains(&low) { return false }
                            } else if (0xDC00..=0xDFFF).contains(&code) { return false }
                        },
                        _ => return false,
                    }
                },
                c if c < 0x20 => return false,
                _ => {},
            }
        }
        false
    }
    fn number(&mut self) -> bool {
        let start = self.at;
        if self.src.get(self.at) == Some(&b'-') { self.at += 1; }
        match self.src.get(self.at).copied() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.at += 1;
                while matches!(self.src.get(self.at).copied(), Some(b'0'..=b'9')) { self.at += 1; }
            },
            _ => return false,
        }
        if self.src.get(self.at) == Some(&b'.') {
            self.at += 1;
            if !matches!(self.src.get(self.at), Some(b'0'..=b'9')) { return false; }
            while matches!(self.src.get(self.at), Some(b'0'..=b'9')) { self.at += 1; }
        }
        if matches!(self.src.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.src.get(self.at), Some(b'+' | b'-')) { self.at += 1; }
            if !matches!(self.src.get(self.at), Some(b'0'..=b'9')) { return false; }
            while matches!(self.src.get(self.at), Some(b'0'..=b'9')) { self.at += 1; }
        }
        self.at > start
    }
    fn word(&mut self, text: &[u8]) -> bool {
        if self.src.get(self.at..self.at + text.len()) == Some(text) {
            self.at += text.len();
            true
        } else { false }
    }
    fn value(&mut self, depth: u8) -> bool {
        if depth > 32 { return false; }
        self.ws();
        match self.src.get(self.at) {
            Some(b'"') => self.string(),
            Some(b'{') => {
                self.at += 1;
                if self.one(b'}') { return true; }
                loop {
                    if !self.string() || !self.one(b':') || !self.value(depth + 1) { return false; }
                    if self.one(b'}') { return true; }
                    if !self.one(b',') { return false; }
                }
            },
            Some(b'[') => {
                self.at += 1;
                if self.one(b']') { return true; }
                loop {
                    if !self.value(depth + 1) { return false; }
                    if self.one(b']') { return true; }
                    if !self.one(b',') { return false; }
                }
            },
            Some(b't') => self.word(b"true"),
            Some(b'f') => self.word(b"false"),
            Some(b'n') => self.word(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => false,
        }
    }
}

pub fn valid_json(src: &str) -> bool {
    if src.len() > MAX_JSON_BYTES { return false; }
    let mut cursor = Json { src: src.as_bytes(), at: 0 };
    if !cursor.value(0) { return false; }
    cursor.ws();
    cursor.at == cursor.src.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_timezone_uuid_and_binary_bounds() {
        assert!(valid_date("2000-02-29"));
        assert!(!valid_date("1900-02-29"));
        assert!(!valid_date("2026-11-31"));
        assert!(valid_datetime("2026-10-26T12:45:10.999Z"));
        assert!(!valid_datetime("2026-10-26T25:45:10.999Z"));
        assert!(!valid_datetime("2026-10-26T12:45:10+03:00"));
        assert!(valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!valid_uuid("550E8400-e29b-41d4-a716-446655440000"));
        assert!(valid_binary_hex("0xdeadbeef"));
        assert!(!valid_binary_hex("0xdeadbee"));
        assert!(!valid_binary_hex("0xDEAD"));
    }
    #[test]
    fn json_rejects_control_bytes_invalid_surrogates_and_deep_nesting() {
        assert!(valid_json(r#"{"a":[1,2,{"x":true}],"escaped":"\uD83D\uDE00"}"#));
        assert!(valid_json("[null,false,-0.5e+2]"));
        assert!(!valid_json(r#"{"a":01}"#));
        assert!(!valid_json(r#"{"s":"\uD800"}"#));
        assert!(!valid_json("{\"x\":\n}"));
        assert!(!valid_json(&"[".repeat(40)));
        assert!(!valid_json("true trailing"));
    }
}
