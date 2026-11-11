// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Query and path parameters, read the way the Python meshview reads them.
//!
//! The first value of a repeated parameter is the one that counts, an empty
//! value is the same as none for numbers, and integers are what Python's `int()`
//! accepts: surrounding space, a sign, and underscores between digits.

use super::error::ApiError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use std::convert::Infallible;

/// The query string, decoded, in order.
#[derive(Debug, Clone, Default)]
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn parse(query: &str) -> Self {
        Self(
            form_urlencoded::parse(query.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }

    /// The first value given for `name`, `""` included.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// An integer parameter, as the Python version's `_query_int` reads it:
    /// absent or empty is `default`, one that is not an integer is refused, and
    /// one out of range is brought into it rather than refused.
    pub fn int(
        &self,
        name: &str,
        default: Option<i64>,
        minimum: Option<i64>,
        maximum: Option<i64>,
    ) -> Result<Option<i64>, ApiError> {
        let value = match self.get(name) {
            None | Some("") => return Ok(default),
            Some(raw) => parse_db_int(raw).ok_or_else(|| must_be_integer(name))?,
        };
        let value = minimum.map_or(value, |minimum| value.max(minimum));
        Ok(Some(maximum.map_or(value, |maximum| value.min(maximum))))
    }
}

impl Params {
    /// A boolean parameter, in any of the spellings the Python version
    /// accepts, or `None` when absent or empty.
    pub fn bool(&self, name: &str) -> Result<Option<bool>, ApiError> {
        match self.get(name) {
            None | Some("") => Ok(None),
            Some(raw) => match raw.trim().to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(Some(true)),
                "0" | "false" | "no" | "off" => Ok(Some(false)),
                _ => Err(ApiError::bad_request(format!(
                    "{name} must be true or false"
                ))),
            },
        }
    }

    /// A text parameter, with an empty value taken as none: a filter on
    /// nothing filters nothing.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.get(name).filter(|value| !value.is_empty())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Params {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::parse(parts.uri.query().unwrap_or("")))
    }
}

/// `{name} must be an integer`, the Python version's words.
pub fn must_be_integer(name: &str) -> ApiError {
    ApiError::bad_request(format!("{name} must be an integer"))
}

/// An integer as Python's `int(text)` reads it, limited to the signed 64 bit
/// range the database columns have.
pub fn parse_db_int(text: &str) -> Option<i64> {
    i64::try_from(parse_python_int(text)?).ok()
}

/// An integer as Python's `int(text)` reads it, in base 10: surrounding white
/// space, an optional sign, and digits with single underscores between them. A
/// number too large even for this saturates, which only matters for clamping.
pub fn parse_python_int(text: &str) -> Option<i128> {
    let text = text.trim();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    {
        return None;
    }

    let mut value: i128 = 0;
    for digit in digits.bytes().filter(u8::is_ascii_digit) {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(digit - b'0'));
    }
    Some(if negative { -value } else { value })
}

/// An integer as Python's `int(text, 0)` reads it: decimal without leading
/// zeros, or with a `0x`, `0o` or `0b` prefix in any case, a sign and
/// underscores between digits allowed; limited to 64 signed bits.
pub fn parse_db_int_any_base(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, rest) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let lower = rest.to_ascii_lowercase();
    let (radix, digits) = match lower.get(..2) {
        Some("0x") => (16, &lower[2..]),
        Some("0o") => (8, &lower[2..]),
        Some("0b") => (2, &lower[2..]),
        _ => {
            // Decimal: a leading zero only in a number that is all zeros.
            let all_zeros = lower.bytes().all(|b| b == b'0' || b == b'_');
            if lower.starts_with('0') && !all_zeros {
                return None;
            }
            (10, lower.as_str())
        }
    };
    // After a prefix a single underscore may come first, as in 0x_ff.
    let digits = if radix != 10 {
        digits.strip_prefix('_').unwrap_or(digits)
    } else {
        digits
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let digits: String = digits.chars().filter(|&c| c != '_').collect();
    let value = i128::from_str_radix(&digits, radix).ok()?;
    i64::try_from(if negative { -value } else { value }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_read_as_python_reads_them() {
        assert_eq!(parse_db_int("42"), Some(42));
        assert_eq!(parse_db_int("  -7 "), Some(-7));
        assert_eq!(parse_db_int("+1_000"), Some(1000));
        assert_eq!(parse_db_int("007"), Some(7));
        for bad in [
            "", " ", "1.0", "1e3", "0x10", "1__0", "_1", "1_", "--1", "12a",
        ] {
            assert_eq!(parse_db_int(bad), None, "{bad:?}");
        }
        assert_eq!(parse_db_int("9223372036854775807"), Some(i64::MAX));
        assert_eq!(parse_db_int("9223372036854775808"), None);
    }

    #[test]
    fn any_base_reads_as_python_reads_it() {
        assert_eq!(parse_db_int_any_base("0x1A"), Some(26));
        assert_eq!(parse_db_int_any_base("0X_ff"), Some(255));
        assert_eq!(parse_db_int_any_base(" 42 "), Some(42));
        assert_eq!(parse_db_int_any_base("0"), Some(0));
        assert_eq!(parse_db_int_any_base("00"), Some(0));
        assert_eq!(parse_db_int_any_base("-0o17"), Some(-15));
        for bad in ["010", "0x", "0xg", "", "1__0", "!abcd"] {
            assert_eq!(parse_db_int_any_base(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_first_value_counts_and_empty_numbers_are_absent() {
        let params = Params::parse("length=5&length=7&empty=&word=a+b%21");
        assert_eq!(params.get("length"), Some("5"));
        assert_eq!(params.get("word"), Some("a b!"));
        assert_eq!(params.int("empty", Some(24), None, None).unwrap(), Some(24));
        assert_eq!(params.int("missing", None, None, None).unwrap(), None);
    }

    #[test]
    fn integers_out_of_range_are_brought_into_it() {
        let params = Params::parse("big=99999&small=-3&bad=x");
        assert_eq!(
            params.int("big", None, Some(1), Some(3650)).unwrap(),
            Some(3650)
        );
        assert_eq!(
            params.int("small", None, Some(1), Some(3650)).unwrap(),
            Some(1)
        );
        assert!(params.int("bad", None, None, None).is_err());
    }
}
