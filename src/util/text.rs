//! Text helpers ported from the reference implementation.

use std::collections::HashMap;
use std::fmt::Write as _;

/// Splits a line at unquoted `|` characters (`split_pipeline`).
///
/// Quotes and backslashes are kept in the parts so that each part can be
/// tokenized again later; every part is trimmed.
#[must_use]
pub fn split_pipeline(pipeline: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for c in pipeline.chars() {
        if escaped {
            buf.push(c);
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            buf.push(c);
            escaped = true;
        } else if let Some(q) = quote {
            buf.push(c);
            if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
            buf.push(c);
        } else if c == '|' {
            lines.push(crate::util::py::strip(&buf).to_string());
            buf.clear();
        } else {
            buf.push(c);
        }
    }
    lines.push(crate::util::py::strip(&buf).to_string());
    lines
}

/// Removes one pair of matching outer quotes (`strip_outer_quotes`).
#[must_use]
pub fn strip_outer_quotes(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if (first == b'"' || first == b'\'') && bytes[bytes.len() - 1] == first {
            return &text[1..text.len() - 1];
        }
    }
    text
}

/// Removes ANSI escape sequences (`escape_ansi`).
///
/// Equivalent to the regular expression
/// `(?:\x1B[@-_]|[\x80-\x9F])[0-?]*[ -/]*[@-~]` used by the reference.
#[must_use]
pub fn escape_ansi(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = ansi_sequence_end(&chars, i) {
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Returns the end index of an escape sequence starting at `start`.
fn ansi_sequence_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    match chars[i] {
        '\u{1b}' => {
            let next = *chars.get(i + 1)?;
            if !('@'..='_').contains(&next) {
                return None;
            }
            i += 2;
        }
        '\u{80}'..='\u{9f}' => i += 1,
        _ => return None,
    }
    while i < chars.len() && ('0'..='?').contains(&chars[i]) {
        i += 1;
    }
    while i < chars.len() && (' '..='/').contains(&chars[i]) {
        i += 1;
    }
    let last = *chars.get(i)?;
    ('@'..='~').contains(&last).then_some(i + 1)
}

/// Pads `s` with spaces to `width` characters, left aligned; longer text
/// stays as it is, like Python's `f"{s:<width}"`.
#[must_use]
pub fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

/// Pads `s` to `width` characters, right aligned.
#[must_use]
pub fn pad_left(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - len))
    }
}

/// The Python exception behind a [`FormatError`]; callers of the
/// reference catch them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatErrorKind {
    /// `IndexError`: a positional argument is missing.
    Index,
    /// `ValueError`: the template is malformed.
    Value,
    /// `MemoryError`: the result does not fit into memory.
    Memory,
}

/// Error raised by [`SafeFormatter`], with the message Python gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    /// The exception Python raises.
    pub kind: FormatErrorKind,
    /// Its message.
    pub message: String,
}

impl FormatError {
    fn value(message: impl Into<String>) -> Self {
        Self {
            kind: FormatErrorKind::Value,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FormatError {}

/// One replacement field of a format string.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    name: String,
    conversion: Option<char>,
    spec: String,
}

/// Python's `MarkupIterator`: the literal texts and fields of a format
/// string in order. Each field is parsed when it is reached, so an error
/// further on only surfaces once the fields before it are done.
struct Markup {
    chars: Vec<char>,
    pos: usize,
}

impl Markup {
    fn new(template: &str) -> Self {
        Self {
            chars: template.chars().collect(),
            pos: 0,
        }
    }

    /// The next literal text and the field after it, `None` at the end.
    fn next_piece(&mut self) -> Result<Option<(String, Option<Field>)>, FormatError> {
        let end = self.chars.len();
        if self.pos >= end {
            return Ok(None);
        }
        let start = self.pos;
        let mut c = '\0';
        let mut markup = false;
        while self.pos < end {
            c = self.chars[self.pos];
            self.pos += 1;
            if c == '{' || c == '}' {
                markup = true;
                break;
            }
        }
        let at_end = self.pos >= end;
        let mut len = self.pos - start;
        if c == '}' && (at_end || self.chars[self.pos] != c) {
            return Err(FormatError::value(
                "Single '}' encountered in format string",
            ));
        }
        if at_end && c == '{' {
            return Err(FormatError::value(
                "Single '{' encountered in format string",
            ));
        }
        if !at_end {
            if self.chars[self.pos] == c {
                // a doubled brace stands for itself
                self.pos += 1;
                markup = false;
            } else {
                len -= 1;
            }
        }
        let literal = self.chars[start..start + len].iter().collect();
        if !markup {
            return Ok(Some((literal, None)));
        }
        Ok(Some((literal, Some(self.field()?))))
    }

    /// Parses a field after its `{` (`parse_field`): the name runs up to
    /// `!`, `:` or `}` (brackets may hold them), then come the conversion
    /// and the format spec with balanced braces.
    fn field(&mut self) -> Result<Field, FormatError> {
        let end = self.chars.len();
        let name_start = self.pos;
        let mut c = '\0';
        while self.pos < end {
            c = self.chars[self.pos];
            self.pos += 1;
            match c {
                '{' => return Err(FormatError::value("unexpected '{' in field name")),
                '[' => {
                    while self.pos < end && self.chars[self.pos] != ']' {
                        self.pos += 1;
                    }
                }
                '}' | ':' | '!' => break,
                _ => {}
            }
        }
        let name: String = self.chars[name_start..self.pos - 1].iter().collect();
        if c != '!' && c != ':' {
            return if c == '}' {
                Ok(Field {
                    name,
                    conversion: None,
                    spec: String::new(),
                })
            } else {
                Err(FormatError::value("expected '}' before end of string"))
            };
        }
        let mut conversion = None;
        if c == '!' {
            let Some(&conv) = self.chars.get(self.pos) else {
                return Err(FormatError::value(
                    "end of string while looking for conversion specifier",
                ));
            };
            self.pos += 1;
            // Python takes a NUL as no conversion
            conversion = (conv != '\0').then_some(conv);
            if let Some(&next) = self.chars.get(self.pos) {
                self.pos += 1;
                if next == '}' {
                    return Ok(Field {
                        name,
                        conversion,
                        spec: String::new(),
                    });
                }
                if next != ':' {
                    return Err(FormatError::value(
                        "expected ':' after conversion specifier",
                    ));
                }
            }
        }
        let spec_start = self.pos;
        let mut depth = 1;
        while let Some(&ch) = self.chars.get(self.pos) {
            self.pos += 1;
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let spec = self.chars[spec_start..self.pos - 1].iter().collect();
                        return Ok(Field {
                            name,
                            conversion,
                            spec,
                        });
                    }
                }
                _ => {}
            }
        }
        Err(FormatError::value("unmatched '{' in format spec"))
    }
}

/// Adds a digit to a number like Python's `get_integer`, which refuses
/// numbers beyond `Py_ssize_t`.
fn push_digit(value: usize, digit: u32) -> Result<usize, FormatError> {
    let max = isize::MAX.unsigned_abs();
    let digit = usize::try_from(digit).unwrap_or(max);
    if value > (max - digit) / 10 {
        return Err(FormatError::value(
            "Too many decimal digits in format string",
        ));
    }
    Ok(value * 10 + digit)
}

/// The number a field name stands for: `Some` when it is all decimal
/// digits of any script, like `get_integer` in Python's format parser.
fn field_index(name: &str) -> Result<Option<usize>, FormatError> {
    if name.is_empty() {
        return Ok(None);
    }
    let mut value = 0;
    for c in name.chars() {
        let Some(digit) = crate::util::py::decimal(c) else {
            return Ok(None);
        };
        value = push_digit(value, digit)?;
    }
    Ok(Some(value))
}

/// Returns true if `template` has a replacement field named `name`
/// (`has_placeholder` of the reference). Like there, the template is read
/// only up to the first such field, so a malformed part after it does not
/// matter, and one before it is an error.
///
/// # Errors
///
/// Returns the error of a malformed template.
pub fn has_placeholder(template: &str, name: &str) -> Result<bool, FormatError> {
    let mut markup = Markup::new(template);
    while let Some((_, field)) = markup.next_piece()? {
        if field.is_some_and(|f| f.name == name) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A port of the reference's `SafeFormatter`, a restricted
/// `string.Formatter`.
///
/// * Positional fields (`{}`, `{0}`) are taken from `args`; a missing one is
///   an error, like Python's `IndexError`.
/// * Named fields come from `kwargs`; unknown names stay in the output
///   unchanged (`{name}`), so later substitutions can still fill them.
/// * Attribute and index access (`{a.b}`, `{a[0]}`) is refused.
///
/// Errors carry the exception and message Python gives, in the order
/// Python finds them.
pub struct SafeFormatter;

/// Python's `auto_arg_index`: the next automatic index, or `False` once a
/// field was numbered by hand.
#[derive(Clone, Copy)]
enum Numbering {
    Auto(usize),
    Manual,
}

impl SafeFormatter {
    /// Formats `template` with positional and named arguments.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed templates, missing positional
    /// arguments or forbidden field expressions.
    pub fn format(
        template: &str,
        args: &[String],
        kwargs: &HashMap<String, String>,
    ) -> Result<String, FormatError> {
        let mut numbering = Numbering::Auto(0);
        Self::vformat(template, args, kwargs, 2, &mut numbering)
    }

    /// `string.Formatter._vformat`: format specs may contain fields too.
    fn vformat(
        template: &str,
        args: &[String],
        kwargs: &HashMap<String, String>,
        depth: i8,
        numbering: &mut Numbering,
    ) -> Result<String, FormatError> {
        if depth < 0 {
            return Err(FormatError::value("Max string recursion exceeded"));
        }
        let mut markup = Markup::new(template);
        let mut out = String::new();
        while let Some((literal, field)) = markup.next_piece()? {
            out.push_str(&literal);
            let Some(field) = field else {
                continue;
            };
            let value = Self::field_value(&field.name, args, kwargs, numbering)?;
            let value = match field.conversion {
                None | Some('s') => value,
                Some('r') => crate::util::json::py_repr_str(&value),
                Some('a') => py_ascii(&crate::util::json::py_repr_str(&value)),
                Some(other) => {
                    return Err(FormatError::value(format!(
                        "Unknown conversion specifier {other}"
                    )));
                }
            };
            let spec = Self::vformat(&field.spec, args, kwargs, depth - 1, numbering)?;
            out.push_str(&apply_spec(&value, &spec)?);
        }
        Ok(out)
    }

    /// The value of a field: numbering as in `_vformat`, then
    /// `SafeFormatter.get_field` and `get_value`.
    fn field_value(
        name: &str,
        args: &[String],
        kwargs: &HashMap<String, String>,
        numbering: &mut Numbering,
    ) -> Result<String, FormatError> {
        // the part before an attribute or index (formatter_field_name_split)
        let first = &name[..name.find(['.', '[']).unwrap_or(name.len())];
        let mut field_name = name.to_string();
        if first.is_empty() {
            let Numbering::Auto(next) = *numbering else {
                return Err(FormatError::value(
                    "cannot switch from manual field specification to automatic field numbering",
                ));
            };
            field_name = format!("{next}{name}");
            *numbering = Numbering::Auto(next + 1);
        } else if field_index(first)?.is_some() {
            if matches!(*numbering, Numbering::Auto(n) if n > 0) {
                return Err(FormatError::value(
                    "cannot switch from automatic field numbering to manual field specification",
                ));
            }
            *numbering = Numbering::Manual;
        }
        if field_name.contains('.') || field_name.contains('[') {
            return Err(FormatError::value(format!(
                "Attribute/index access not allowed in format field: {}",
                crate::util::json::py_repr_str(&field_name)
            )));
        }
        match field_index(&field_name)? {
            Some(index) => args.get(index).cloned().ok_or_else(|| FormatError {
                kind: FormatErrorKind::Index,
                message: "tuple index out of range".to_string(),
            }),
            None => Ok(kwargs
                .get(&field_name)
                .cloned()
                .unwrap_or_else(|| format!("{{{field_name}}}"))),
        }
    }
}

/// Python's `ascii()` applied to a repr: non-ASCII characters escaped.
fn py_ascii(repr: &str) -> String {
    let mut out = String::new();
    for c in repr.chars() {
        let code = u32::from(c);
        if code < 0x80 {
            out.push(c);
        } else if code <= 0xff {
            let _ = write!(out, "\\x{code:02x}");
        } else if code <= 0xffff {
            let _ = write!(out, "\\u{code:04x}");
        } else {
            let _ = write!(out, "\\U{code:08x}");
        }
    }
    out
}

/// A presentation type in an error message, like Python prints it.
fn type_char(kind: char) -> String {
    let code = u32::from(kind);
    if code > 32 && code < 128 {
        kind.to_string()
    } else {
        format!("\\x{code:x}")
    }
}

/// Reads decimal digits of any script at `pos`, like `get_integer` of
/// Python's spec parser: the number and how many digits there were.
fn spec_integer(chars: &[char], pos: &mut usize) -> Result<(usize, usize), FormatError> {
    let (mut value, mut count) = (0, 0);
    while let Some(digit) = chars.get(*pos).and_then(|&c| crate::util::py::decimal(c)) {
        value = push_digit(value, digit)?;
        *pos += 1;
        count += 1;
    }
    Ok((value, count))
}

/// Applies a format spec to a string like Python's `str.__format__`:
/// `[[fill]align][sign]["z"]["#"]["0"][width][grouping]["." precision
/// [grouping]][type]`, parsed and checked in Python's order.
fn apply_spec(value: &str, spec: &str) -> Result<String, FormatError> {
    if spec.is_empty() {
        return Ok(value.to_string());
    }
    let chars: Vec<char> = spec.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let is_align = |c: Option<char>| matches!(c, Some('<' | '>' | '=' | '^'));
    let both = || FormatError::value("Cannot specify both ',' and '_'.");

    let (mut pos, mut fill, mut align) = (0, None, '<');
    if is_align(at(1)) {
        fill = at(0);
        align = chars[1];
        pos = 2;
    } else if is_align(at(0)) {
        align = chars[0];
        pos = 1;
    }
    let sign = at(pos).filter(|c| matches!(c, '+' | '-' | ' '));
    if sign.is_some() {
        pos += 1;
    }
    let no_neg_zero = at(pos) == Some('z');
    if no_neg_zero {
        pos += 1;
    }
    let alternate = at(pos) == Some('#');
    if alternate {
        pos += 1;
    }
    // the 0 flag pads with zeros unless a fill character was given; the
    // default alignment of str stays
    if fill.is_none() && at(pos) == Some('0') {
        fill = Some('0');
        pos += 1;
    }
    let (width, digits) = spec_integer(&chars, &mut pos)?;
    let width = (digits > 0).then_some(width);
    let mut grouping = None;
    if at(pos) == Some(',') {
        grouping = Some(',');
        pos += 1;
    }
    if at(pos) == Some('_') {
        if grouping.is_some() {
            return Err(both());
        }
        grouping = Some('_');
        pos += 1;
    }
    if at(pos) == Some(',') && grouping == Some('_') {
        return Err(both());
    }
    let mut precision = None;
    let mut fraction_grouping = None;
    if at(pos) == Some('.') {
        pos += 1;
        let (value, mut consumed) = spec_integer(&chars, &mut pos)?;
        precision = (consumed > 0).then_some(value);
        if at(pos) == Some(',') {
            fraction_grouping = Some(',');
            pos += 1;
            consumed += 1;
        }
        if at(pos) == Some('_') {
            if fraction_grouping.is_some() {
                return Err(both());
            }
            fraction_grouping = Some('_');
            pos += 1;
            consumed += 1;
        }
        if at(pos) == Some(',') && fraction_grouping == Some('_') {
            return Err(both());
        }
        if consumed == 0 {
            return Err(FormatError::value("Format specifier missing precision"));
        }
    }
    if chars.len() - pos > 1 {
        return Err(FormatError::value(format!(
            "Invalid format specifier '{spec}' for object of type 'str'"
        )));
    }
    let kind = at(pos).unwrap_or('s');
    let grouping_error = |sep: char| {
        FormatError::value(format!(
            "Cannot specify '{sep}' with '{}'.",
            type_char(kind)
        ))
    };
    if let Some(sep) = grouping {
        let allowed = matches!(kind, 'd' | 'e' | 'f' | 'g' | 'E' | 'G' | '%' | 'F' | '\0')
            || (sep == '_' && matches!(kind, 'b' | 'o' | 'x' | 'X'));
        if !allowed {
            return Err(grouping_error(sep));
        }
    }
    if kind != 's' {
        return Err(FormatError::value(format!(
            "Unknown format code '{}' for object of type 'str'",
            type_char(kind)
        )));
    }
    let refused = match sign {
        Some(' ') => Some("Space not allowed in string format specifier"),
        Some(_) => Some("Sign not allowed in string format specifier"),
        None if no_neg_zero => {
            Some("Negative zero coercion (z) not allowed in string format specifier")
        }
        None if alternate => Some("Alternate form (#) not allowed in string format specifier"),
        None if align == '=' => Some("'=' alignment not allowed in string format specifier"),
        None => None,
    };
    if let Some(message) = refused {
        return Err(FormatError::value(message));
    }

    let len = value.chars().count();
    let len = precision.map_or(len, |p| p.min(len));
    let total = width.map_or(len, |w| w.max(len));
    let left = match align {
        '>' => total - len,
        '^' => (total - len) / 2,
        _ => 0,
    };
    let right = total - len - left;
    let fill = fill.unwrap_or(' ');
    // a width beyond memory is Python's MemoryError, not an abort
    let mut out = String::new();
    let bytes = (left + right)
        .checked_mul(fill.len_utf8())
        .and_then(|b| b.checked_add(value.len()));
    if bytes.is_none_or(|b| out.try_reserve_exact(b).is_err()) {
        return Err(FormatError {
            kind: FormatErrorKind::Memory,
            message: String::new(),
        });
    }
    out.extend(std::iter::repeat_n(fill, left));
    out.extend(value.chars().take(len));
    out.extend(std::iter::repeat_n(fill, right));
    Ok(out)
}

/// Formats bytes as lowercase hex without separators.
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_respects_quotes_and_escapes() {
        assert_eq!(split_pipeline("lc | rn af"), vec!["lc", "rn af"]);
        assert_eq!(
            split_pipeline(r#"msg a "x|y" | ver"#),
            vec![r#"msg a "x|y""#, "ver"]
        );
        assert_eq!(split_pipeline(r"echo a\|b"), vec![r"echo a\|b"]);
        assert_eq!(split_pipeline("'a|b'"), vec!["'a|b'"]);
        assert_eq!(split_pipeline("single"), vec!["single"]);
    }

    #[test]
    fn strips_outer_quotes_only_when_matching() {
        assert_eq!(strip_outer_quotes("\"hi\""), "hi");
        assert_eq!(strip_outer_quotes("'hi'"), "hi");
        assert_eq!(strip_outer_quotes("\"hi'"), "\"hi'");
        assert_eq!(strip_outer_quotes("\""), "\"");
    }

    #[test]
    fn removes_ansi_sequences() {
        assert_eq!(escape_ansi("\x1b[0;32mgreen\x1b[0m"), "green");
        assert_eq!(escape_ansi("\x1b[1;38;5;247mX\x1b[27m"), "X");
        assert_eq!(escape_ansi("\x1b[42G: 01"), ": 01");
        assert_eq!(escape_ansi("plain"), "plain");
        assert_eq!(escape_ansi("dangling \x1b["), "dangling \x1b[");
    }

    #[test]
    fn formatter_positional_and_named() {
        let args = vec!["a".to_string(), "b".to_string()];
        let mut kw = HashMap::new();
        kw.insert("c".to_string(), "node".to_string());
        assert_eq!(SafeFormatter::format("{} {}", &args, &kw).unwrap(), "a b");
        assert_eq!(SafeFormatter::format("{1}{0}", &args, &kw).unwrap(), "ba");
        assert_eq!(
            SafeFormatter::format("to {c}", &args, &kw).unwrap(),
            "to node"
        );
        assert_eq!(
            SafeFormatter::format("{x}-{c}", &args, &kw).unwrap(),
            "{x}-node"
        );
        assert_eq!(
            SafeFormatter::format("{{lit}}", &args, &kw).unwrap(),
            "{lit}"
        );
        assert_eq!(SafeFormatter::format("{0:>3}", &args, &kw).unwrap(), "  a");
    }

    #[test]
    fn formatter_errors() {
        let kw = HashMap::new();
        assert!(SafeFormatter::format("{}", &[], &kw).is_err());
        assert!(SafeFormatter::format("{a.b}", &[], &kw).is_err());
        assert!(SafeFormatter::format("{0[1]}", &["x".into()], &kw).is_err());
        assert!(SafeFormatter::format("x}", &[], &kw).is_err());
        assert!(SafeFormatter::format("{x", &[], &kw).is_err());
        assert!(SafeFormatter::format("{}{0}", &["x".into()], &kw).is_err());
    }

    #[test]
    fn huge_widths_are_memory_errors() {
        let error = SafeFormatter::format("{0:99999999999999999}", &["a".into()], &HashMap::new())
            .unwrap_err();
        assert_eq!(error.kind, FormatErrorKind::Memory);
        let error = SafeFormatter::format("{1}", &["a".into()], &HashMap::new()).unwrap_err();
        assert_eq!(error.kind, FormatErrorKind::Index);
    }

    #[test]
    fn detects_placeholders() {
        assert_eq!(has_placeholder("login {c} {0}", "c"), Ok(true));
        assert_eq!(has_placeholder("login {0}", "c"), Ok(false));
        assert_eq!(has_placeholder("{{c}}", "c"), Ok(false));
        // read up to the first match, like Python's any() over parse()
        assert_eq!(has_placeholder("{c} {", "c"), Ok(true));
        assert!(has_placeholder("{ {c}", "c").is_err());
    }

    #[test]
    fn padding_counts_chars() {
        assert_eq!(pad_right("ab", 4), "ab  ");
        assert_eq!(pad_left("ab", 4), "  ab");
        assert_eq!(pad_right("abcdef", 4), "abcdef");
    }

    /// Template, positional and named arguments, success, result or error.
    type FormatCase<'a> = (
        &'a str,
        &'a [String],
        &'a [(&'a str, &'a str)],
        bool,
        &'a str,
    );

    /// Results of the reference's `SafeFormatter` (Python 3.14), see `meshcore_cli.py`.
    #[test]
    fn formatter_matches_python() {
        let cases: &[FormatCase] = &[
            ("path {c}", &[], &[("c", "Bob")], true, "path Bob"),
            (
                "msg {c} {0}",
                &["hi".to_string()],
                &[("c", "Bob")],
                true,
                "msg Bob hi",
            ),
            (
                "x {1}",
                &["a".to_string()],
                &[],
                false,
                "tuple index out of range",
            ),
            ("x {}", &[], &[], false, "tuple index out of range"),
            (
                "x {",
                &[],
                &[],
                false,
                "Single '{' encountered in format string",
            ),
            (
                "x }",
                &[],
                &[],
                false,
                "Single '}' encountered in format string",
            ),
            (
                "{a.b}",
                &[],
                &[],
                false,
                "Attribute/index access not allowed in format field: 'a.b'",
            ),
            (
                "{a[0]}",
                &[],
                &[],
                false,
                "Attribute/index access not allowed in format field: 'a[0]'",
            ),
            ("{0:>5}|", &["ab".to_string()], &[], true, "   ab|"),
            ("{unknown}", &[], &[], true, "{unknown}"),
            (
                "{} {}",
                &["a".to_string(), "b".to_string()],
                &[],
                true,
                "a b",
            ),
            (
                "{0} {}",
                &["a".to_string(), "b".to_string()],
                &[],
                false,
                "cannot switch from manual field specification to automatic field numbering",
            ),
            ("{!r}", &["a".to_string()], &[], true, "'a'"),
            ("{{lit}} {0}", &["v".to_string()], &[], true, "{lit} v"),
            (
                "{0:<4}|{0:^6}|",
                &["ab".to_string()],
                &[],
                true,
                "ab  |  ab  |",
            ),
            ("{0:.2}", &["abcdef".to_string()], &[], true, "ab"),
            ("{0:*>6}", &["ab".to_string()], &[], true, "****ab"),
            ("{c:>8}", &[], &[("c", "Bob")], true, "     Bob"),
            (
                "{0",
                &["a".to_string()],
                &[],
                false,
                "expected '}' before end of string",
            ),
            (
                "{0!x}",
                &["a".to_string()],
                &[],
                false,
                "Unknown conversion specifier x",
            ),
            (
                "{0:d}",
                &["a".to_string()],
                &[],
                false,
                "Unknown format code 'd' for object of type 'str'",
            ),
            ("{00}", &["a".to_string()], &[], true, "a"),
            ("{-1}", &["a".to_string()], &[], true, "{-1}"),
            (
                "{0!xy}",
                &["a".to_string()],
                &[],
                false,
                "expected ':' after conversion specifier",
            ),
            (
                "{0!}",
                &["a".to_string()],
                &[],
                false,
                "unmatched '{' in format spec",
            ),
            ("{0:}", &["a".to_string()], &[], true, "a"),
            (
                "{} {0}",
                &["a".to_string()],
                &[],
                false,
                "cannot switch from automatic field numbering to manual field specification",
            ),
            ("{0!r}", &["it's".to_string()], &[], true, "\"it's\""),
            (
                "{0:>5x}",
                &["a".to_string()],
                &[],
                false,
                "Unknown format code 'x' for object of type 'str'",
            ),
            (
                "{0:x}",
                &["a".to_string()],
                &[],
                false,
                "Unknown format code 'x' for object of type 'str'",
            ),
            ("{0:05}", &["a".to_string()], &[], true, "a0000"),
            (
                "{0:,}",
                &["a".to_string()],
                &[],
                false,
                "Cannot specify ',' with 's'.",
            ),
            (
                "{0:{1}}",
                &["a".to_string(), "5".to_string()],
                &[],
                true,
                "a    ",
            ),
            ("{0:<}", &["a".to_string()], &[], true, "a"),
            ("{0:>3.1}", &["abc".to_string()], &[], true, "  a"),
            ("{ 0}", &["a".to_string()], &[], true, "{ 0}"),
            ("{0!s:>3}", &["a".to_string()], &[], true, "  a"),
            ("{0!a}", &["\u{00e9}".to_string()], &[], true, "'\\xe9'"),
            // errors surface in reading order, like Python's lazy parser
            (
                "{1} {",
                &["a".to_string()],
                &[],
                false,
                "tuple index out of range",
            ),
            ("{c{", &[], &[], false, "unexpected '{' in field name"),
            (
                "{0!}x",
                &["a".to_string()],
                &[],
                false,
                "expected ':' after conversion specifier",
            ),
            (
                "{0!",
                &["a".to_string()],
                &[],
                false,
                "end of string while looking for conversion specifier",
            ),
            (
                "{a[:]}",
                &[],
                &[],
                false,
                "Attribute/index access not allowed in format field: 'a[:]'",
            ),
            (
                "{.x}",
                &[],
                &[],
                false,
                "Attribute/index access not allowed in format field: '0.x'",
            ),
            (
                "{\u{661}}",
                &["a".to_string(), "b".to_string()],
                &[],
                true,
                "b",
            ),
            (
                "{99999999999999999999}",
                &[],
                &[],
                false,
                "Too many decimal digits in format string",
            ),
            (
                "{0: }",
                &["a".to_string()],
                &[],
                false,
                "Space not allowed in string format specifier",
            ),
            (
                "{0:z}",
                &["a".to_string()],
                &[],
                false,
                "Negative zero coercion (z) not allowed in string format specifier",
            ),
            (
                "{0:,d}",
                &["a".to_string()],
                &[],
                false,
                "Unknown format code 'd' for object of type 'str'",
            ),
            ("{0:.2,}", &["abc".to_string()], &[], true, "ab"),
            (
                "{0:.2_,}",
                &["abc".to_string()],
                &[],
                false,
                "Cannot specify both ',' and '_'.",
            ),
            (
                "{0:\u{e9}}",
                &["a".to_string()],
                &[],
                false,
                "Unknown format code '\\xe9' for object of type 'str'",
            ),
            (
                "{0:{0:{0}}}",
                &["a".to_string()],
                &[],
                false,
                "Max string recursion exceeded",
            ),
        ];
        for (template, args, kwargs, ok, expected) in cases {
            let kwargs: HashMap<String, String> = kwargs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            let result = SafeFormatter::format(template, args, &kwargs);
            if *ok {
                assert_eq!(result.as_deref().ok(), Some(*expected), "{template}");
            } else {
                assert_eq!(
                    result.err().map(|e| e.to_string()).as_deref(),
                    Some(*expected),
                    "{template}"
                );
            }
        }
    }
}
