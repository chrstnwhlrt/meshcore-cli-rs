//! JSON output that matches Python's `json.dumps` byte for byte.
//!
//! The reference prints most results with `json.dumps(obj)` or
//! `json.dumps(obj, indent=4)`. Python uses `", "` and `": "` as separators
//! in compact mode, escapes every non-ASCII character (`ensure_ascii`) and
//! keeps the key order of the dictionary. Matching this keeps scripts that
//! post-process the output (jq, the sxmo wrapper) working unchanged.
//!
//! [`py_repr`] renders the same values like Python's `repr()`, for the
//! places where the reference prints a dictionary or list with `str()`, and
//! [`py_loads`] reads JSON like `json.loads`, failing with the messages of
//! its `JSONDecodeError`.

use std::fmt::Write as _;

use serde_json::Value;

/// Serializes like `json.dumps(value)`.
#[must_use]
pub fn dumps(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, None, 0);
    out
}

/// Serializes like `json.dumps(value, indent=indent)`.
#[must_use]
pub fn dumps_indent(value: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, value, Some(indent), 0);
    out
}

fn write_value(out: &mut String, value: &Value, indent: Option<usize>, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                separator(out, i, indent, level + 1);
                write_value(out, item, indent, level + 1);
            }
            closing(out, indent, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                separator(out, i, indent, level + 1);
                write_string(out, key);
                out.push_str(": ");
                write_value(out, item, indent, level + 1);
            }
            closing(out, indent, level);
            out.push('}');
        }
    }
}

fn separator(out: &mut String, index: usize, indent: Option<usize>, level: usize) {
    match indent {
        Some(width) => {
            if index > 0 {
                out.push(',');
            }
            out.push('\n');
            out.push_str(&" ".repeat(width * level));
        }
        None => {
            if index > 0 {
                out.push_str(", ");
            }
        }
    }
}

fn closing(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(width) = indent {
        out.push('\n');
        out.push_str(&" ".repeat(width * level));
    }
}

fn write_number(out: &mut String, n: &serde_json::Number) {
    match n.as_f64().filter(|_| n.is_f64()) {
        Some(f) => out.push_str(&py_float_repr(f)),
        None => out.push_str(&n.to_string()),
    }
}

/// Python's `repr(float)`, which `json.dumps` and `str()` use: the
/// shortest digits that read back as the same value, with an exponent
/// (`1e-05`, `1.5e+16`) when the decimal point would be more than four
/// places left of the first digit or after the sixteenth, and `.0` for
/// whole numbers.
#[must_use]
pub fn py_float_repr(f: f64) -> String {
    if !f.is_finite() {
        // json.dumps writes these as Infinity, -Infinity and NaN
        return if f.is_nan() {
            "NaN".to_string()
        } else if f > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    let (digits, exponent) = shortest_digits(f);
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let point = exponent + 1;
    if point <= -4 || point > 16 {
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        return format!(
            "{sign}{first}{fraction}e{exp_sign}{:02}",
            exponent.unsigned_abs()
        );
    }
    let len = digits.len();
    let body = match usize::try_from(point) {
        Err(_) | Ok(0) => format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize)),
        Ok(p) if p < len => format!("{}.{}", &digits[..p], &digits[p..]),
        Ok(p) => format!("{digits}{}.0", "0".repeat(p - len)),
    };
    format!("{sign}{body}")
}

/// The shortest digits of `|f|` and the exponent of the first one. Of two
/// equally near candidates Python takes the one ending in an even digit;
/// Rust takes the upper one, so a tie is looked for and decided here.
fn shortest_digits(f: f64) -> (String, i32) {
    let f = f.abs();
    let split = |text: &str| -> (String, i32) {
        let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
        (mantissa.replace('.', ""), exponent.parse().unwrap_or(0))
    };
    let (digits, exponent) = split(&format!("{f:e}"));
    let n = digits.len();
    // at a tie the exact value, one digit longer, ends in 5 ...
    let (longer, longer_exponent) = split(&format!("{f:.n$e}"));
    if longer_exponent != exponent || !longer.ends_with('5') {
        return (digits, exponent);
    }
    // ... and nothing follows (a double has at most 767 digits)
    let (exact, _) = split(&format!("{f:.800e}"));
    if exact
        .get(n + 1..)
        .is_none_or(|rest| rest.bytes().any(|b| b != b'0'))
    {
        return (digits, exponent);
    }
    let lower = &exact[..n];
    let Some(upper) = increment(lower) else {
        return (digits, exponent);
    };
    let even = if lower.ends_with(['0', '2', '4', '6', '8']) {
        lower.to_string()
    } else {
        upper
    };
    let even = even.trim_end_matches('0');
    let candidate = format!("{}.{}e{exponent}", &even[..1], &even[1..]);
    if even.is_empty() || candidate.parse::<f64>() != Ok(f) {
        return (digits, exponent);
    }
    (even.to_string(), exponent)
}

/// A digit string plus one in its last place; `None` when that carries
/// over the first digit.
fn increment(digits: &str) -> Option<String> {
    let mut bytes = digits.as_bytes().to_vec();
    for b in bytes.iter_mut().rev() {
        if *b == b'9' {
            *b = b'0';
        } else {
            *b += 1;
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Renders a value like Python's `repr()` of the equivalent object.
///
/// Used where the reference prints a dictionary with `str(dict)`.
#[must_use]
pub fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => {
            let mut s = String::new();
            write_number(&mut s, n);
            s
        }
        Value::String(s) => py_repr_str(s),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(py_repr).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", py_repr_str(k), py_repr(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

/// Python `repr` of a list of strings, like `str(args)` in an f-string.
#[must_use]
pub fn py_str_list(items: &[String]) -> String {
    py_repr(&Value::Array(
        items.iter().cloned().map(Value::String).collect(),
    ))
}

pub(crate) fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            // other characters Python does not print as they are
            c if !c.is_ascii() && !crate::util::printable::is_printable(c) => {
                let code = u32::from(c);
                let _ = if code <= 0xff {
                    write!(out, "\\x{code:02x}")
                } else if code <= 0xffff {
                    write!(out, "\\u{code:04x}")
                } else {
                    write!(out, "\\U{code:08x}")
                };
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Containers nested deeper are read and checked by [`py_loads`] but kept
/// as `null`, so that no value too deep to drop is built.
const MAX_DEPTH: usize = 100;

/// One container being read by [`py_loads`] (`None`: one that is only
/// checked).
enum Open {
    Array(Option<Vec<Value>>),
    Object(Option<serde_json::Map<String, Value>>, String),
}

/// Python's `json.loads(text)` as its C scanner reads it: the value, or
/// the text of the `JSONDecodeError`
/// (`"Expecting value: line 1 column 1 (char 0)"`).
///
/// Python also takes `NaN` and the infinities, which a [`Value`] cannot
/// hold: they become `null`, as do floats beyond `f64` and containers
/// nested more than [`MAX_DEPTH`] levels deep. A lone surrogate escape
/// (`"\ud800"`), which a Python string keeps, becomes U+FFFD.
///
/// # Errors
///
/// The message Python gives for text that is not JSON.
pub fn py_loads(text: &str) -> Result<Value, String> {
    let s: Vec<char> = text.chars().collect();
    let error = |msg: &str, pos: usize| {
        let before = &s[..pos.min(s.len())];
        let line = before.iter().filter(|c| **c == '\n').count() + 1;
        let column = before
            .iter()
            .rposition(|c| *c == '\n')
            .map_or(pos + 1, |i| pos - i);
        format!("{msg}: line {line} column {column} (char {pos})")
    };
    if s.first() == Some(&'\u{feff}') {
        return Err(error("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0));
    }
    let skip_ws = |mut i: usize| {
        while matches!(s.get(i), Some(' ' | '\t' | '\n' | '\r')) {
            i += 1;
        }
        i
    };
    let starts_with = |i: usize, word: &str| {
        word.chars()
            .enumerate()
            .all(|(k, c)| s.get(i + k) == Some(&c))
    };
    // a property name and its colon; returns the name and where its value starts
    let key = |i: usize| -> Result<(String, usize), String> {
        if s.get(i) != Some(&'"') {
            return Err(error(
                "Expecting property name enclosed in double quotes",
                i,
            ));
        }
        let (name, next) = scan_string(&s, i + 1, &error)?;
        let colon = skip_ws(next);
        if s.get(colon) != Some(&':') {
            return Err(error("Expecting ':' delimiter", colon));
        }
        Ok((name, skip_ws(colon + 1)))
    };

    let mut open: Vec<Open> = Vec::new();
    let mut i = skip_ws(0);
    loop {
        // a value starts at i
        let (mut value, mut next) = match s.get(i) {
            Some('"') => {
                let (text, next) = scan_string(&s, i + 1, &error)?;
                (Value::String(text), next)
            }
            Some('{') => {
                let first = skip_ws(i + 1);
                if s.get(first) == Some(&'}') {
                    (Value::Object(serde_json::Map::new()), first + 1)
                } else {
                    let (name, start) = key(first)?;
                    let map = (open.len() < MAX_DEPTH).then(serde_json::Map::new);
                    open.push(Open::Object(map, name));
                    i = start;
                    continue;
                }
            }
            Some('[') => {
                let first = skip_ws(i + 1);
                if s.get(first) == Some(&']') {
                    (Value::Array(Vec::new()), first + 1)
                } else {
                    open.push(Open::Array((open.len() < MAX_DEPTH).then(Vec::new)));
                    i = first;
                    continue;
                }
            }
            Some('n') if starts_with(i, "null") => (Value::Null, i + 4),
            Some('t') if starts_with(i, "true") => (Value::Bool(true), i + 4),
            Some('f') if starts_with(i, "false") => (Value::Bool(false), i + 5),
            Some('N') if starts_with(i, "NaN") => (Value::Null, i + 3),
            Some('I') if starts_with(i, "Infinity") => (Value::Null, i + 8),
            Some('-') if starts_with(i, "-Infinity") => (Value::Null, i + 9),
            _ => scan_number(&s, i).ok_or_else(|| error("Expecting value", i))?,
        };
        // the value is complete: hand it to the open containers
        loop {
            match open.last_mut() {
                None => {
                    let end = skip_ws(next);
                    if end != s.len() {
                        return Err(error("Extra data", end));
                    }
                    return Ok(value);
                }
                Some(Open::Array(items)) => {
                    if let Some(items) = items {
                        items.push(value);
                    }
                    let after = skip_ws(next);
                    match s.get(after) {
                        Some(']') => {
                            let Some(Open::Array(items)) = open.pop() else {
                                unreachable!("an array is open");
                            };
                            value = items.map_or(Value::Null, Value::Array);
                            next = after + 1;
                        }
                        Some(',') => {
                            let start = skip_ws(after + 1);
                            if s.get(start) == Some(&']') {
                                return Err(error(
                                    "Illegal trailing comma before end of array",
                                    after,
                                ));
                            }
                            i = start;
                            break;
                        }
                        _ => return Err(error("Expecting ',' delimiter", after)),
                    }
                }
                Some(Open::Object(map, name)) => {
                    if let Some(map) = map {
                        map.insert(std::mem::take(name), value);
                    }
                    let after = skip_ws(next);
                    match s.get(after) {
                        Some('}') => {
                            let Some(Open::Object(map, _)) = open.pop() else {
                                unreachable!("an object is open");
                            };
                            value = map.map_or(Value::Null, Value::Object);
                            next = after + 1;
                        }
                        Some(',') => {
                            let first = skip_ws(after + 1);
                            if s.get(first) == Some(&'}') {
                                return Err(error(
                                    "Illegal trailing comma before end of object",
                                    after,
                                ));
                            }
                            let (new_name, start) = key(first)?;
                            *name = new_name;
                            i = start;
                            break;
                        }
                        _ => return Err(error("Expecting ',' delimiter", after)),
                    }
                }
            }
        }
    }
}

/// Reads a JSON string whose opening quote is just before `begin`;
/// returns it and the index after its closing quote.
fn scan_string(
    s: &[char],
    begin: usize,
    error: &impl Fn(&str, usize) -> String,
) -> Result<(String, usize), String> {
    let hex4 = |at: usize| -> Option<u32> {
        let digits = s.get(at..at + 4)?;
        digits
            .iter()
            .try_fold(0, |code, c| Some(code * 16 + c.to_digit(16)?))
    };
    let unterminated = || error("Unterminated string starting at", begin - 1);
    let mut out = String::new();
    let mut i = begin;
    loop {
        let c = *s.get(i).ok_or_else(unterminated)?;
        match c {
            '"' => return Ok((out, i + 1)),
            '\\' => {
                let escaped = *s.get(i + 1).ok_or_else(unterminated)?;
                let plain = match escaped {
                    '"' | '\\' | '/' => escaped,
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'u' => {
                        let code =
                            hex4(i + 2).ok_or_else(|| error("Invalid \\uXXXX escape", i + 1))?;
                        i += 6;
                        // a high surrogate pairs with a following low one
                        if (0xD800..0xDC00).contains(&code)
                            && s.get(i) == Some(&'\\')
                            && s.get(i + 1) == Some(&'u')
                        {
                            let low = hex4(i + 2)
                                .ok_or_else(|| error("Invalid \\uXXXX escape", i + 1))?;
                            if (0xDC00..0xE000).contains(&low) {
                                i += 6;
                                let pair = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                out.push(char::from_u32(pair).unwrap_or('\u{fffd}'));
                                continue;
                            }
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        continue;
                    }
                    _ => return Err(error("Invalid \\escape", i)),
                };
                out.push(plain);
                i += 2;
            }
            c if u32::from(c) < 0x20 => return Err(error("Invalid control character at", i)),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

/// Reads a number at `start` like the scanner's number pattern
/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`; `None` when there is
/// none.
fn scan_number(s: &[char], start: usize) -> Option<(Value, usize)> {
    let digit = |i: usize| s.get(i).is_some_and(char::is_ascii_digit);
    let mut i = start;
    if s.get(i) == Some(&'-') {
        i += 1;
    }
    match s.get(i) {
        Some('0') => i += 1,
        Some('1'..='9') => {
            while digit(i) {
                i += 1;
            }
        }
        _ => return None,
    }
    let mut is_float = false;
    if s.get(i) == Some(&'.') && digit(i + 1) {
        is_float = true;
        i += 1;
        while digit(i) {
            i += 1;
        }
    }
    if matches!(s.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some('+' | '-')) {
            j += 1;
        }
        if digit(j) {
            while digit(j) {
                j += 1;
            }
            is_float = true;
            i = j;
        }
    }
    let text: String = s[start..i].iter().collect();
    let value = if is_float {
        text.parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number)
    } else if let Ok(n) = text.parse::<i64>() {
        Value::from(n)
    } else if let Ok(n) = text.parse::<u64>() {
        Value::from(n)
    } else {
        text.parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number)
    };
    Some((value, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_like_python() {
        // json.loads of Python 3.14
        let ok = |text: &str| dumps(&py_loads(text).unwrap());
        let err = |text: &str| py_loads(text).unwrap_err();
        assert_eq!(
            ok(r#"{"a": "\u00e4\ud83d\ude00", "b": [1, 2.5, -3e2, true, null]}"#),
            r#"{"a": "\u00e4\ud83d\ude00", "b": [1, 2.5, -300.0, true, null]}"#
        );
        assert_eq!(
            ok("[NaN, -Infinity, \"\\ud800\"]"),
            r#"[null, null, "\ufffd"]"#
        );
        assert_eq!(ok(r#"{"a":1,"a":2}"#), r#"{"a": 2}"#);
        assert_eq!(err(""), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(
            err("{\"a\":1,}"),
            "Illegal trailing comma before end of object: line 1 column 7 (char 6)"
        );
        assert_eq!(
            err("[1 2]"),
            "Expecting ',' delimiter: line 1 column 4 (char 3)"
        );
        assert_eq!(
            err("\n{\n\"a\": tru}"),
            "Expecting value: line 3 column 6 (char 8)"
        );
        assert_eq!(
            err("\"a\\x\""),
            "Invalid \\escape: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("\"\\ud83d\\u12\""),
            "Invalid \\uXXXX escape: line 1 column 9 (char 8)"
        );
        assert_eq!(
            err("\"abc"),
            "Unterminated string starting at: line 1 column 1 (char 0)"
        );
        assert_eq!(
            err("\"a\u{1}\""),
            "Invalid control character at: line 1 column 3 (char 2)"
        );
        assert_eq!(err("01"), "Extra data: line 1 column 2 (char 1)");
        assert_eq!(
            err("\u{feff}{}"),
            "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"
        );
        // no depth limit to hit, as in Python 3.14; deep levels become null
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        let value = py_loads(&deep).unwrap();
        let text = dumps(&value);
        assert_eq!(text.matches('[').count(), MAX_DEPTH);
        assert!(text.contains("[null]"));
        assert_eq!(
            py_loads(&format!("{}1,]", "[".repeat(200))).unwrap_err(),
            "Illegal trailing comma before end of array: line 1 column 202 (char 201)"
        );
    }
    use serde_json::json;

    #[test]
    fn compact_uses_python_separators() {
        let v = json!({"a": 1, "b": [true, null, "x"]});
        assert_eq!(dumps(&v), r#"{"a": 1, "b": [true, null, "x"]}"#);
    }

    #[test]
    fn keeps_insertion_order() {
        let mut map = serde_json::Map::new();
        map.insert("z".into(), json!(1));
        map.insert("a".into(), json!(2));
        assert_eq!(dumps(&Value::Object(map)), r#"{"z": 1, "a": 2}"#);
    }

    #[test]
    fn indented_matches_python() {
        let v = json!({"name": "x", "list": [1, 2], "empty": {}});
        assert_eq!(
            dumps_indent(&v, 4),
            "{\n    \"name\": \"x\",\n    \"list\": [\n        1,\n        2\n    ],\n    \"empty\": {}\n}"
        );
    }

    #[test]
    fn escapes_non_ascii() {
        let bs = '\\';
        assert_eq!(dumps(&json!("é")), format!("\"{bs}u00e9\""));
        assert_eq!(dumps(&json!("😀")), format!("\"{bs}ud83d{bs}ude00\""));
        assert_eq!(dumps(&json!("a\"b\\c\n")), r#""a\"b\\c\n""#);
        assert_eq!(dumps(&json!("\u{1}")), r#""\u0001""#);
    }

    #[test]
    fn floats_like_python() {
        assert_eq!(dumps(&json!(1.0)), "1.0");
        assert_eq!(dumps(&json!(47.794)), "47.794");
        assert_eq!(dumps(&json!(-3.5)), "-3.5");
        assert_eq!(dumps(&json!(5)), "5");
        // repr() of Python 3.14 for each value
        let cases = [
            (1e-5, "1e-05"),
            (1.2e-5, "1.2e-05"),
            (0.0001, "0.0001"),
            (0.00012, "0.00012"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1e15, "1000000000000000.0"),
            (123_456_789_012_345.6, "123456789012345.6"),
            (0.5, "0.5"),
            (-1e-5, "-1e-05"),
            (-0.0, "-0.0"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (47.123_456, "47.123456"),
            (1e-7, "1e-07"),
            // a tie between two shortest candidates goes to the even digit
            (1_917_628_373_273_994.2, "1917628373273994.2"),
            (34_595_157_129_853.812, "34595157129853.812"),
            (0.3, "0.3"),
            (2.5, "2.5"),
            (100.0, "100.0"),
            (-0.000_123, "-0.000123"),
        ];
        for (value, want) in cases {
            assert_eq!(py_float_repr(value), want, "{value:e}");
            assert_eq!(dumps(&json!(value)), want);
        }
    }

    #[test]
    fn repr_like_python() {
        let v = json!({"channel_idx": 1, "channel_name": "it's", "ok": true, "n": null});
        assert_eq!(
            py_repr(&v),
            r#"{'channel_idx': 1, 'channel_name': "it's", 'ok': True, 'n': None}"#
        );
    }

    #[test]
    fn repr_escapes_what_python_does_not_print() {
        assert_eq!(py_repr_str("a\u{2028}b"), "'a\\u2028b'");
        assert_eq!(py_repr_str("\u{a0}\u{85}"), "'\\xa0\\x85'");
        assert_eq!(py_repr_str("\u{e9}\u{1f600}"), "'\u{e9}\u{1f600}'");
        assert_eq!(py_repr_str("\u{10ffff}"), "'\\U0010ffff'");
    }

    #[test]
    fn string_list_like_python() {
        let args = vec!["msg".to_string(), "Bob".to_string(), "it's".to_string()];
        assert_eq!(py_str_list(&args), r#"['msg', 'Bob', "it's"]"#);
        assert_eq!(py_str_list(&[]), "[]");
    }
}
