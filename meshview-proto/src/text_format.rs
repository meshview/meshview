// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The protobuf text format, written the way Python's protobuf writes it
//! (`text_format.MessageToString` with its defaults), which is how the web pages
//! have always shown a packet's payload.
//!
//! The rules, for the messages Meshtastic has:
//!
//! - fields in the order of their numbers, and only those present: a field
//!   without explicit presence when it is not its default, one with it (a
//!   message, a oneof, an `optional`) when it is set; unknown fields not at all;
//! - `name: value` a line, a repeated field a line per element, a message as
//!   `name {`, its fields two spaces further in, and `}`;
//! - an enum by its name, or its number when it has none;
//! - a float as the shortest decimal that reads back as the same 32 bit float,
//!   then printed as Python prints a float; a double as Python prints it;
//! - strings and bytes quoted and escaped in C's way, with octal for control
//!   characters, and in bytes for every byte from 0x80 up.

use prost_reflect::{
    DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, ReflectMessage, Value,
};
use std::fmt::Write;

/// The message `name` encoded in `bytes`, in the text format, or `None` when the
/// bytes are not such a message.
pub fn decode_to_text(name: &str, bytes: &[u8]) -> Option<String> {
    let descriptor = crate::descriptors().get_message_by_name(name)?;
    let bytes = crate::wire::sanitize(name, bytes)?;
    let message = DynamicMessage::decode(descriptor, bytes.as_slice()).ok()?;
    Some(to_text(&message))
}

/// A message in the text format, as Python's protobuf writes it.
pub fn to_text(message: &DynamicMessage) -> String {
    let mut out = String::new();
    write_message(&mut out, message, 0);
    out
}

fn write_message(out: &mut String, message: &DynamicMessage, indent: usize) {
    let mut fields: Vec<FieldDescriptor> = message.descriptor().fields().collect();
    fields.sort_by_key(FieldDescriptor::number);

    for field in fields {
        if !message.has_field(&field) {
            continue;
        }
        let value = message.get_field(&field);
        match value.as_ref() {
            Value::List(items) => {
                for item in items {
                    write_field(out, &field, item, indent);
                }
            }
            other => write_field(out, &field, other, indent),
        }
    }
}

fn write_field(out: &mut String, field: &FieldDescriptor, value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Message(message) => {
            let _ = writeln!(out, "{pad}{} {{", field.name());
            write_message(out, message, indent + 2);
            let _ = writeln!(out, "{pad}}}");
        }
        scalar => {
            let _ = writeln!(out, "{pad}{}: {}", field.name(), scalar_text(field, scalar));
        }
    }
}

fn scalar_text(field: &FieldDescriptor, value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::I32(n) => n.to_string(),
        Value::I64(n) => n.to_string(),
        Value::U32(n) => n.to_string(),
        Value::U64(n) => n.to_string(),
        Value::F32(f) => float32_text(*f),
        Value::F64(f) => python_float(*f),
        Value::String(text) => format!("\"{}\"", escape(text.as_bytes(), true)),
        Value::Bytes(bytes) => format!("\"{}\"", escape(bytes, false)),
        Value::EnumNumber(number) => match field.kind() {
            Kind::Enum(descriptor) => descriptor
                .get_value(*number)
                .map(|value| value.name().to_string())
                .unwrap_or_else(|| number.to_string()),
            _ => number.to_string(),
        },
        Value::Message(_) | Value::List(_) | Value::Map(_) => String::new(),
    }
}

/// C's escapes, as Python's `text_encoding.CEscape`. Text keeps what is not
/// ASCII as it is; bytes get it in octal too.
fn escape(bytes: &[u8], text: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        match byte {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'"' => out.push_str("\\\""),
            b'\'' => out.push_str("\\'"),
            b'\\' => out.push_str("\\\\"),
            0..0x20 | 0x7f => {
                let _ = write!(out, "\\{byte:03o}");
            }
            0x80.. if !text => {
                let _ = write!(out, "\\{byte:03o}");
            }
            0x80.. => {
                // A whole UTF-8 sequence, kept as it is. The bytes come from a
                // string, which prost has already checked is UTF-8.
                let length = match byte {
                    0xf0.. => 4,
                    0xe0.. => 3,
                    _ => 2,
                };
                let end = (i + length).min(bytes.len());
                out.push_str(&String::from_utf8_lossy(&bytes[i..end]));
                i = end;
                continue;
            }
            _ => out.push(byte as char),
        }
        i += 1;
    }
    out
}

/// A 32 bit float as Python's protobuf prints one: the shortest decimal of at
/// least six significant digits that reads back as the same float, printed as
/// the double it is.
fn float32_text(value: f32) -> String {
    if !value.is_finite() {
        return python_float(f64::from(value));
    }
    for precision in 6..=9 {
        let candidate: f64 = format!("{:.*e}", precision - 1, f64::from(value))
            .parse()
            .expect("a float formats as a float");
        if candidate as f32 == value {
            return python_float(candidate);
        }
    }
    python_float(f64::from(value))
}

/// A double as Python's `repr` prints it: the shortest digits that read back
/// as the same double, in positional notation from 1e-4 up to 1e16 and in
/// scientific notation outside that, always with a `.` or an exponent.
pub fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_string();
    }

    // Rust's `{:e}` gives the same shortest digits; only the layout differs.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("{:e} has an exponent");
    let exponent: i32 = exponent.parse().expect("an exponent is a number");
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();

    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let text = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            format!("{whole}.{fraction}")
        };
        format!("{sign}{text}")
    } else {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!("{sign}{mantissa}e{exponent_sign}{:02}", exponent.abs())
    }
}

/// The descriptor of a message, by its full name.
pub fn message(name: &str) -> Option<MessageDescriptor> {
    crate::descriptors().get_message_by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_as_python_prints_them() {
        for (value, text) in [
            (5.0, "5.0"),
            (-7.25, "-7.25"),
            (0.1, "0.1"),
            (1e-5, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (0.0001, "0.0001"),
            (123.456, "123.456"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(python_float(value), text, "{value}");
        }
    }

    #[test]
    fn floats_of_32_bits_are_shortened_first() {
        assert_eq!(float32_text(4.1), "4.1");
        assert_eq!(float32_text(3.3), "3.3");
        assert_eq!(float32_text(-7.0), "-7.0");
        assert_eq!(float32_text(0.000_012_3), "1.23e-05");
    }

    #[test]
    fn escapes_are_c_escapes() {
        assert_eq!(
            escape(b"a\"b'c\\d\ne\x01\x7f", true),
            "a\\\"b\\'c\\\\d\\ne\\001\\177"
        );
        assert_eq!(escape("Ñandú".as_bytes(), true), "Ñandú");
        assert_eq!(escape(&[0x00, 0xc8, b'A'], false), "\\000\\310A");
    }
}
