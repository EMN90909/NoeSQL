//! Exact, fixed-scale decimal arithmetic for financial data. No floats.
//!
//! `decimal` has 38 total precision digits (including six fractional ones).
//! Binary snapshot representation is signed i128 little-endian micro-units;
//! conversion to JSON uses strings to avoid loss of precision in JavaScript.
//! Division rounds ties to even. Every operation is checked; a temporary
//! i128 overflow is reported rather than silently changing the result.
use std::fmt;
pub const SCALE: i128 = 1_000_000;
pub const MAX_UNITS: i128 = 99_999_999_999_999_999_999_999_999_999_999_999_999;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal(i128);

impl Decimal {
    pub fn from_scaled(units: i128) -> Option<Self> {
        if (-MAX_UNITS..=MAX_UNITS).contains(&units) { Some(Self(units)) } else { None }
    }
    pub fn scaled(self) -> i128 { self.0 }
    /// No scientific notation, locale separators, implicit rounding or
    /// precision truncation. Leading zeros are rejected except for zero.
    pub fn parse(source: &str) -> Option<Self> {
        let (negative, s) = match source.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, source),
        };
        let (whole, fraction) = match s.split_once('.') {
            Some((whole, frac)) => (whole, Some(frac)),
            None => (s, None),
        };
        if whole.is_empty() || !whole.bytes().all(|ch| ch.is_ascii_digit())
            || (whole.len() > 1 && whole.starts_with('0')) || whole.len() > 32 {
            return None;
        }
        let integer: i128 = whole.parse().ok()?;
        let fractional = match fraction {
            Some(frac) => {
                if frac.is_empty() || frac.len() > 6 ||
                    !frac.bytes().all(|ch| ch.is_ascii_digit()) { return None; }
                let n: i128 = frac.parse().ok()?;
                n.checked_mul(10i128.pow((6 - frac.len()) as u32))?
            },
            None => 0,
        };
        let units = integer.checked_mul(SCALE)?.checked_add(fractional)?;
        let units = if negative { units.checked_neg()? } else { units };
        Self::from_scaled(units)
    }
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        Self::from_scaled(self.0.checked_add(rhs.0)?)
    }
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        Self::from_scaled(self.0.checked_sub(rhs.0)?)
    }
    pub fn checked_neg(self) -> Option<Self> {
        Self::from_scaled(self.0.checked_neg()?)
    }
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        let numerator = self.0.checked_mul(rhs.0)?;
        Self::from_scaled(round_quotient_even(numerator, SCALE)?)
    }
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.0 == 0 { return None; }
        let numerator = self.0.checked_mul(SCALE)?;
        Self::from_scaled(round_quotient_even(numerator, rhs.0)?)
    }
}
// Calculate a signed quotient rounded to nearest, with exact ties to even,
// without overflowing when doubling the remainder.
fn round_quotient_even(a: i128, b: i128) -> Option<i128> {
    if b == 0 { return None; }
    let q = a.checked_div(b)?;
    let remainder = a.checked_rem(b)?.unsigned_abs();
    let divisor = b.unsigned_abs();
    let half = divisor / 2;
    let round = remainder > half || (divisor % 2 == 0 && remainder == half && q % 2 != 0);
    if !round { return Some(q); }
    if (a < 0) ^ (b < 0) { q.checked_sub(1) } else { q.checked_add(1) }
}
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let magnitude = self.0.unsigned_abs();
        write!(f, "{}{}.{:06}", if self.0 < 0 { "-" } else { "" },
            magnitude / SCALE as u128, magnitude % SCALE as u128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dec(src: &str) -> Decimal { Decimal::parse(src).unwrap() }
    #[test]
    fn exact_parse_and_roundtrip_at_boundaries() {
        for src in ["0", "-0.000000", "0.1", "1.000001", "-2.010000",
            "99999999999999999999999999999999.999999"] {
            let v = dec(src);
            assert_eq!(dec(&v.to_string()), v);
        }
        for bad in ["", ".", ".1", "01.20", "1.", "1.0000001", "1e3",
            "123,456", "1.2.3", "100000000000000000000000000000000.000000"] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn exact_arithmetic_and_bankers_rounding() {
        assert_eq!(dec("0.1").checked_add(dec("0.2")).unwrap().to_string(), "0.300000");
        assert_eq!(dec("5.0").checked_sub(dec("6.5")).unwrap().to_string(), "-1.500000");
        assert_eq!(dec("0.000001").checked_mul(dec("0.5")).unwrap().to_string(), "0.000000");
        assert_eq!(dec("0.000003").checked_mul(dec("0.5")).unwrap().to_string(), "0.000002");
        assert_eq!(dec("-0.000003").checked_mul(dec("0.5")).unwrap().to_string(), "-0.000002");
        assert_eq!(dec("1").checked_div(dec("8")).unwrap().to_string(), "0.125000");
        assert_eq!(dec("1").checked_div(dec("0")), None);
        assert!(dec("99999999999999999999999999999999").checked_add(dec("1")).is_none());
    }
}
