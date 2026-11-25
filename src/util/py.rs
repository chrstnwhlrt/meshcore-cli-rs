//! Python behaviour the reference shows to users.
//!
//! `int`, `float` and `bytes.fromhex` accept the same inputs as in the
//! reference and fail with the same messages, which the reference shows as
//! `Unexpected error while processing '<cmd>': <message>`; [`os_error`]
//! writes I/O errors the way Python's `OSError` does. [`strip`], [`split`]
//! and [`lines`] cut text where Python's `str` methods and text files do,
//! and [`read_text`] reads a file as `open(path, "r")` does.

use serde_json::Value;

use crate::util::json::py_repr;

/// Whitespace skipped by `bytes.fromhex` (`Py_ISSPACE`).
fn is_ascii_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// Python's `str.isspace()` for one character: Unicode white space and the
/// separators U+001C to U+001F.
#[must_use]
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
#[must_use]
pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// Python's `str.lstrip()`.
#[must_use]
pub fn lstrip(text: &str) -> &str {
    text.trim_start_matches(is_space)
}

/// Python's `str.rstrip()`.
#[must_use]
pub fn rstrip(text: &str) -> &str {
    text.trim_end_matches(is_space)
}

/// Python's `str.split()`: the words between runs of white space.
pub fn split(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_space).filter(|word| !word.is_empty())
}

/// The lines of a file Python reads in text mode, without their ends:
/// universal newlines end a line at `\n`, `\r\n` and a lone `\r`.
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line = &rest[..end];
        let next = if rest[end..].starts_with("\r\n") {
            end + 2
        } else {
            (end + 1).min(rest.len())
        };
        rest = &rest[next..];
        Some(line)
    })
}

/// First code points of the Unicode decimal digit blocks above ASCII
/// (category Nd of Unicode 16.0, as in Python 3.14); each block holds the
/// digits 0 to 9 in order.
const DIGIT_ZEROS: [u32; 75] = [
    0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6,
    0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50,
    0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    0x104A0, 0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0,
    0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950, 0x11BF0, 0x11C50, 0x11D50,
    0x11DA0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE, 0x1D7D8,
    0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

/// The value of a decimal digit of any script (`Py_UNICODE_TODECIMAL`).
#[must_use]
pub fn decimal(c: char) -> Option<u32> {
    c.to_digit(10).or_else(|| {
        let code = u32::from(c);
        DIGIT_ZEROS
            .iter()
            .find(|&&zero| (zero..zero + 10).contains(&code))
            .map(|zero| code - zero)
    })
}

/// The text as `int()` and `float()` read it: every Unicode decimal digit
/// counts as its ASCII digit (`_PyUnicode_TransformDecimalAndSpaceToASCII`).
fn ascii_digits(text: &str) -> String {
    text.chars()
        .map(|c| {
            decimal(c)
                .and_then(|d| char::from_digit(d, 10))
                .unwrap_or(c)
        })
        .collect()
}

/// Python's `bytes.fromhex(text)`: pairs of hex digits, whitespace
/// allowed between the pairs.
///
/// # Errors
///
/// Returns Python's error message for invalid input.
pub fn fromhex(text: &str) -> Result<Vec<u8>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(chars.len() / 2);
    let mut i = 0;
    while i < chars.len() {
        if is_ascii_space(chars[i]) {
            while i < chars.len() && is_ascii_space(chars[i]) {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
        }
        let Some(top) = chars[i].to_digit(16) else {
            return Err(format!(
                "non-hexadecimal number found in fromhex() arg at position {i}"
            ));
        };
        i += 1;
        let Some(bottom) = chars.get(i).and_then(|c| c.to_digit(16)) else {
            return Err(if i >= chars.len() {
                "fromhex() arg must contain an even number of hexadecimal digits".to_string()
            } else {
                format!("non-hexadecimal number found in fromhex() arg at position {i}")
            });
        };
        i += 1;
        out.push(u8::try_from(top << 4 | bottom).unwrap_or_default());
    }
    Ok(out)
}

/// The C library's text for an error number (`os.strerror`).
#[must_use]
pub fn strerror(errno: i32) -> String {
    let text = std::io::Error::from_raw_os_error(errno).to_string();
    text.strip_suffix(&format!(" (os error {errno})"))
        .unwrap_or(&text)
        .to_string()
}

/// An I/O error as Python's `OSError` reads: `[Errno 2] No such file or
/// directory: 'name'` (Rust writes `No such file or directory (os error
/// 2)`). Errors without an OS error number keep their text.
#[must_use]
pub fn os_error(error: &std::io::Error, filename: Option<&str>) -> String {
    let Some(errno) = error.raw_os_error() else {
        return error.to_string();
    };
    let strerror = strerror(errno);
    match filename {
        Some(name) => format!(
            "[Errno {errno}] {strerror}: {}",
            crate::util::json::py_repr_str(name)
        ),
        None => format!("[Errno {errno}] {strerror}"),
    }
}

/// How Python reads a text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRead {
    /// `f.read()`: all bytes are decoded at once.
    All,
    /// `readline()`, `readlines()` or a loop over the file: the bytes go
    /// through an incremental decoder in chunks of 8192.
    Lines,
}

/// Why a text file could not be read.
#[derive(Debug)]
pub enum TextError {
    /// Opening or reading failed.
    Io(std::io::Error),
    /// The text of the `UnicodeDecodeError` Python raises.
    Decode(String),
}

impl TextError {
    /// The message Python shows, for the file named as it was opened.
    #[must_use]
    pub fn message(&self, filename: &str) -> String {
        match self {
            Self::Io(e) => os_error(e, Some(filename)),
            Self::Decode(text) => text.clone(),
        }
    }
}

/// Reads a text file like Python's `open(path, "r")` in a UTF-8 locale.
/// The line ends stay as they are; [`lines`] splits like Python's
/// universal newlines.
///
/// # Errors
///
/// The I/O error, or Python's `UnicodeDecodeError` for bytes that are not
/// UTF-8, at the position Python reports for this way of reading.
pub fn read_text(path: &std::path::Path, how: TextRead) -> Result<String, TextError> {
    let bytes = std::fs::read(path).map_err(TextError::Io)?;
    decode_text(&bytes, how).map_err(TextError::Decode)
}

/// Decodes the bytes of a text file as Python does; see [`read_text`].
///
/// # Errors
///
/// The text of Python's `UnicodeDecodeError`.
pub fn decode_text(bytes: &[u8], how: TextRead) -> Result<String, String> {
    if how == TextRead::All {
        return match std::str::from_utf8(bytes) {
            Ok(text) => Ok(text.to_string()),
            Err(e) => Err(decode_error(bytes, e.valid_up_to(), e.error_len())),
        };
    }
    // TextIOWrapper decodes 8192 bytes at a time; an incomplete character
    // at the end of a chunk waits for the next one
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new();
    for chunk in bytes.chunks(8192) {
        let mut input = std::mem::take(&mut pending);
        input.extend_from_slice(chunk);
        match std::str::from_utf8(&input) {
            Ok(text) => out.push_str(text),
            Err(e) => {
                let (valid, rest) = input.split_at(e.valid_up_to());
                // CPython also waits with the first two bytes of a
                // surrogate (ED A0..BF), which only "surrogatepass" takes
                let waits = e.error_len().is_none() || matches!(rest, [0xED, 0xA0..=0xBF]);
                if !waits {
                    return Err(decode_error(&input, e.valid_up_to(), e.error_len()));
                }
                out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                pending = rest.to_vec();
            }
        }
    }
    // the final decode sees only what was left over
    match std::str::from_utf8(&pending) {
        Ok(text) => {
            out.push_str(text);
            Ok(out)
        }
        Err(e) => Err(decode_error(&pending, e.valid_up_to(), e.error_len())),
    }
}

/// Python's text of a `UnicodeDecodeError` for the sequence at `start` of
/// `input` (`len`: its length, `None` when the input ends inside it).
fn decode_error(input: &[u8], start: usize, len: Option<usize>) -> String {
    let end = len.map_or(input.len(), |n| start + n);
    let reason = match len {
        None => "unexpected end of data",
        Some(_) if matches!(input[start], 0x80..=0xC1 | 0xF5..=0xFF) => "invalid start byte",
        Some(_) => "invalid continuation byte",
    };
    if end - start == 1 {
        format!(
            "'utf-8' codec can't decode byte 0x{:02x} in position {start}: {reason}",
            input[start]
        )
    } else {
        format!(
            "'utf-8' codec can't decode bytes in position {start}-{}: {reason}",
            end - 1
        )
    }
}

/// The text of a file Python reads in text mode, with its line ends made
/// `\n` (universal newlines).
#[must_use]
pub fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Python's `int(text)` (`base` 10) or `int(text, 0)` (`base` 0, with
/// `0x`, `0o` and `0b` prefixes).
///
/// # Errors
///
/// Returns Python's error message for invalid input. A number beyond 64
/// bits fails with "int too big to convert", the message Python gives when
/// it packs such a number for the device; where Python takes it as it is
/// (a count, a time), this port refuses it.
pub fn int(text: &str, base: u32) -> Result<i64, String> {
    let error = || {
        format!(
            "invalid literal for int() with base {base}: {}",
            py_repr(&Value::String(text.to_string()))
        )
    };
    let ascii = ascii_digits(text);
    let trimmed = ascii.trim();
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let lower = unsigned.to_ascii_lowercase();
    let prefixed = |prefix: &str| lower.strip_prefix(prefix);
    let (radix, digits, after_prefix) = if base == 0 {
        if let Some(d) = prefixed("0x") {
            (16, d, true)
        } else if let Some(d) = prefixed("0o") {
            (8, d, true)
        } else if let Some(d) = prefixed("0b") {
            (2, d, true)
        } else {
            (10, lower.as_str(), false)
        }
    } else {
        // like Python, a base may come with its own prefix: int("0x1f", 16)
        let prefix = match base {
            16 => Some("0x"),
            8 => Some("0o"),
            2 => Some("0b"),
            _ => None,
        };
        match prefix.and_then(prefixed) {
            Some(d) => (base, d, true),
            None => (base, lower.as_str(), false),
        }
    };
    // underscores may separate digits, and follow a base prefix
    let digits = if after_prefix {
        digits.strip_prefix('_').unwrap_or(digits)
    } else {
        digits
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return Err(error());
    }
    let digits = digits.replace('_', "");
    if !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(error());
    }
    // base 0 rejects decimal literals with leading zeros, except zero
    if base == 0 && radix == 10 && digits.starts_with('0') && digits.chars().any(|c| c != '0') {
        return Err(error());
    }
    // the digits are valid here, so parsing can only overflow; the sign
    // stays with them so that i64::MIN fits
    let signed = if negative {
        format!("-{digits}")
    } else {
        digits
    };
    i64::from_str_radix(&signed, radix).map_err(|_| "int too big to convert".to_string())
}

/// Python's `float(text)`.
///
/// # Errors
///
/// Returns Python's error message for invalid input.
pub fn float(text: &str) -> Result<f64, String> {
    let error = || {
        format!(
            "could not convert string to float: {}",
            py_repr(&Value::String(text.to_string()))
        )
    };
    let ascii = ascii_digits(text);
    let trimmed = ascii.trim();
    // underscores are allowed between digits only
    let chars: Vec<char> = trimmed.chars().collect();
    let mut cleaned = String::with_capacity(trimmed.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == '_' {
            let digit_before = i > 0 && chars[i - 1].is_ascii_digit();
            let digit_after = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if !(digit_before && digit_after) {
                return Err(error());
            }
        } else {
            cleaned.push(*c);
        }
    }
    let lower = cleaned.to_ascii_lowercase();
    let unsigned = lower.trim_start_matches(['+', '-']);
    if lower.len() - unsigned.len() > 1 {
        return Err(error());
    }
    if matches!(unsigned, "inf" | "infinity" | "nan") {
        return lower.parse::<f64>().map_err(|_| error());
    }
    let valid = !unsigned.is_empty()
        && unsigned
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | '+' | '-'));
    if !valid {
        return Err(error());
    }
    lower.parse::<f64>().map_err(|_| error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_space_like_python_str() {
        // str.strip() and str.split() also cut at U+001C to U+001F
        assert_eq!(strip("\u{1c} a b\u{3000}\u{85}"), "a b");
        assert_eq!(lstrip("\t a "), "a ");
        assert_eq!(rstrip(" a \u{1f}\n"), " a");
        assert_eq!(strip("\u{feff}a\u{200b}"), "\u{feff}a\u{200b}");
        assert_eq!(
            split(" a\u{1d}b \u{a0} c ").collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(split(" \t ").count(), 0);
        let python: Vec<u32> = (0..0x11_0000)
            .filter_map(char::from_u32)
            .filter(|c| is_space(*c))
            .map(u32::from)
            .collect();
        assert_eq!(
            python,
            [
                0x9, 0xa, 0xb, 0xc, 0xd, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x85, 0xa0, 0x1680, 0x2000,
                0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a,
                0x2028, 0x2029, 0x202f, 0x205f, 0x3000
            ]
        );
    }

    #[test]
    fn decodes_like_python_text_files() {
        assert_eq!(
            decode_text(b"a\r\n\xc3\xa4", TextRead::All).unwrap(),
            "a\r\n\u{e4}"
        );
        assert_eq!(
            decode_text(b"abc\n\xe4x\n", TextRead::Lines).unwrap_err(),
            "'utf-8' codec can't decode byte 0xe4 in position 4: invalid continuation byte"
        );
        assert_eq!(
            decode_text(b"ab\xe2\x82A", TextRead::All).unwrap_err(),
            "'utf-8' codec can't decode bytes in position 2-3: invalid continuation byte"
        );
        assert_eq!(
            decode_text(b"\x80", TextRead::All).unwrap_err(),
            "'utf-8' codec can't decode byte 0x80 in position 0: invalid start byte"
        );
        // read() reports where the file ends, the line reader its last chunk
        let truncated = b"abc\nxy\xe2\x82";
        assert_eq!(
            decode_text(truncated, TextRead::All).unwrap_err(),
            "'utf-8' codec can't decode bytes in position 6-7: unexpected end of data"
        );
        assert_eq!(
            decode_text(truncated, TextRead::Lines).unwrap_err(),
            "'utf-8' codec can't decode bytes in position 0-1: unexpected end of data"
        );
        let mut long = vec![b'a'; 8191];
        long.extend_from_slice(b"\nzz\xe4z");
        assert_eq!(
            decode_text(&long, TextRead::Lines).unwrap_err(),
            "'utf-8' codec can't decode byte 0xe4 in position 2: invalid continuation byte"
        );
        let mut surrogate = vec![b'a'; 8190];
        surrogate.extend_from_slice(b"\xed\xa0\x80");
        assert_eq!(
            decode_text(&surrogate, TextRead::Lines).unwrap_err(),
            "'utf-8' codec can't decode byte 0xed in position 0: invalid continuation byte"
        );
    }

    #[test]
    fn lines_like_python_text_files() {
        let all = |text| lines(text).collect::<Vec<_>>();
        assert_eq!(all("a\nb\r\nc\rd"), ["a", "b", "c", "d"]);
        assert_eq!(all("a\n"), ["a"]);
        assert_eq!(all("a\n\n"), ["a", ""]);
        assert_eq!(all("\r\r\n"), ["", ""]);
        assert_eq!(all("a\u{2028}b\u{85}c"), ["a\u{2028}b\u{85}c"]);
        assert_eq!(lines("").count(), 0);
    }

    #[test]
    fn fromhex_like_python() {
        assert_eq!(fromhex("ab cd").unwrap(), vec![0xab, 0xcd]);
        assert_eq!(fromhex(" ab\t").unwrap(), vec![0xab]);
        assert_eq!(fromhex("ABcd").unwrap(), vec![0xab, 0xcd]);
        assert_eq!(fromhex("").unwrap(), Vec::<u8>::new());
        assert_eq!(
            fromhex("zz").unwrap_err(),
            "non-hexadecimal number found in fromhex() arg at position 0"
        );
        assert_eq!(
            fromhex("abc").unwrap_err(),
            "fromhex() arg must contain an even number of hexadecimal digits"
        );
        assert_eq!(
            fromhex("a b").unwrap_err(),
            "non-hexadecimal number found in fromhex() arg at position 1"
        );
        assert_eq!(
            fromhex("abc d").unwrap_err(),
            "non-hexadecimal number found in fromhex() arg at position 3"
        );
    }

    #[test]
    fn int_like_python() {
        assert_eq!(int("12", 10).unwrap(), 12);
        assert_eq!(int(" -7 ", 10).unwrap(), -7);
        assert_eq!(int("1_000", 10).unwrap(), 1000);
        assert_eq!(int("0x1F", 0).unwrap(), 31);
        assert_eq!(int("0b101", 0).unwrap(), 5);
        assert_eq!(int("0o17", 0).unwrap(), 15);
        assert_eq!(int("0x_ff", 0).unwrap(), 255);
        assert_eq!(int("00", 0).unwrap(), 0);
        assert_eq!(int("010", 10).unwrap(), 10);
        assert_eq!(
            int("010", 0).unwrap_err(),
            "invalid literal for int() with base 0: '010'"
        );
        assert_eq!(
            int("0.2", 10).unwrap_err(),
            "invalid literal for int() with base 10: '0.2'"
        );
        assert_eq!(
            int("", 10).unwrap_err(),
            "invalid literal for int() with base 10: ''"
        );
        assert!(int("1__0", 10).is_err());
        assert_eq!(int("0x1f", 16).unwrap(), 31);
        assert_eq!(int("1F", 16).unwrap(), 31);
        assert_eq!(
            int("zz", 16).unwrap_err(),
            "invalid literal for int() with base 16: 'zz'"
        );
        assert_eq!(float(" 2.5 ").unwrap(), 2.5);
        assert_eq!(float("1_000").unwrap(), 1000.0);
        assert_eq!(float("-1e3").unwrap(), -1000.0);
        assert!(float("inf").unwrap().is_infinite());
        assert_eq!(
            float("x").unwrap_err(),
            "could not convert string to float: 'x'"
        );
        assert_eq!(
            float("").unwrap_err(),
            "could not convert string to float: ''"
        );
        assert!(float("1__0").is_err());
        assert!(float("--1").is_err());
        assert!(int("_1", 10).is_err());
        assert!(int("0x", 0).is_err());
    }

    #[test]
    fn os_errors_like_python() {
        let error = std::io::Error::from_raw_os_error(2);
        assert_eq!(
            os_error(&error, Some("/x/y")),
            "[Errno 2] No such file or directory: '/x/y'"
        );
        assert_eq!(
            os_error(&error, None),
            "[Errno 2] No such file or directory"
        );
        let other = std::io::Error::other("plain");
        assert_eq!(os_error(&other, Some("f")), "plain");
    }

    #[test]
    fn unicode_digits_and_big_numbers_like_python() {
        // Arabic-Indic and fullwidth digits count like ASCII digits
        assert_eq!(int("\u{661}\u{662}", 10).unwrap(), 12);
        assert_eq!(int("\u{ff11}\u{ff12}", 16).unwrap(), 18);
        assert_eq!(int("\u{1d7ce}", 10).unwrap(), 0);
        assert_eq!(float("\u{ff11}.\u{ff15}").unwrap(), 1.5);
        // the message names the text as given
        assert_eq!(
            int("\u{661}x", 10).unwrap_err(),
            "invalid literal for int() with base 10: '\u{661}x'"
        );
        assert_eq!(int("-9223372036854775808", 10).unwrap(), i64::MIN);
        assert_eq!(int("9223372036854775807", 10).unwrap(), i64::MAX);
        assert_eq!(
            int("9223372036854775808", 10).unwrap_err(),
            "int too big to convert"
        );
        assert_eq!(
            int("-0x8000000000000001", 0).unwrap_err(),
            "int too big to convert"
        );
    }
}
