//! Word splitting with the exact semantics of Python's `shlex` module.
//!
//! The reference implementation relies on two flavours of `shlex`:
//!
//! * [`split`] mirrors `shlex.split(line)`: POSIX mode, whitespace splitting
//!   and no comment characters, so `#channel` stays a normal word.
//! * [`split_first_token`] mirrors the custom helper of the reference that
//!   builds a `shlex.shlex(line, posix=True)` lexer (where `#` *does* start a
//!   comment), reads one token and returns the untouched rest of the input.
//!
//! Crates on crates.io follow POSIX shell rules instead (comments, `$`
//! escapes inside double quotes), which would silently drop hashtag channel
//! names, hence this small re-implementation.

use std::fmt;

/// Error raised for malformed input, with the messages Python uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShlexError {
    /// A quote was opened but never closed.
    NoClosingQuotation,
    /// The input ended right after a backslash.
    NoEscapedCharacter,
}

impl fmt::Display for ShlexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoClosingQuotation => f.write_str("No closing quotation"),
            Self::NoEscapedCharacter => f.write_str("No escaped character"),
        }
    }
}

impl std::error::Error for ShlexError {}

/// Lexer state, named after the states of Python's `shlex.read_token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Between tokens (`' '`).
    Whitespace,
    /// Inside a word (`'a'`).
    Word,
    /// Inside a quoted section opened by the given quote character.
    Quote(char),
    /// After a backslash; remembers the state to return to.
    Escape(Escaped),
    /// End of input reached.
    Eof,
}

/// State that was active when a backslash was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escaped {
    Word,
    DoubleQuote,
}

const WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// A `shlex.shlex` lexer in POSIX mode with `whitespace_split = True`.
struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    state: State,
    comments: bool,
}

impl<'a> Lexer<'a> {
    const fn new(input: &'a str, comments: bool) -> Self {
        Self {
            input,
            pos: 0,
            state: State::Whitespace,
            comments,
        }
    }

    /// Reads one character, `None` at the end of the input.
    fn read_char(&mut self) -> Option<char> {
        let ch = self.input[self.pos..].chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    /// Skips the rest of the current line including the newline.
    fn skip_line(&mut self) {
        match self.input[self.pos..].find('\n') {
            Some(idx) => self.pos += idx + 1,
            None => self.pos = self.input.len(),
        }
    }

    /// Returns the part of the input that has not been consumed yet.
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    /// Reads the next token like `shlex.get_token()`.
    fn next_token(&mut self) -> Result<Option<String>, ShlexError> {
        let mut token = String::new();
        let mut quoted = false;

        loop {
            let next = self.read_char();
            match self.state {
                State::Eof => break,
                State::Whitespace => match next {
                    None => {
                        self.state = State::Eof;
                        break;
                    }
                    Some(c) if WHITESPACE.contains(&c) => {
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some('#') if self.comments => self.skip_line(),
                    Some('\\') => self.state = State::Escape(Escaped::Word),
                    Some(c @ ('"' | '\'')) => self.state = State::Quote(c),
                    Some(c) => {
                        token.push(c);
                        self.state = State::Word;
                    }
                },
                State::Quote(q) => {
                    quoted = true;
                    match next {
                        None => return Err(ShlexError::NoClosingQuotation),
                        Some(c) if c == q => self.state = State::Word,
                        Some('\\') if q == '"' => self.state = State::Escape(Escaped::DoubleQuote),
                        Some(c) => token.push(c),
                    }
                }
                State::Escape(escaped) => {
                    let Some(c) = next else {
                        return Err(ShlexError::NoEscapedCharacter);
                    };
                    // Inside double quotes a backslash only escapes `"` and `\`.
                    if escaped == Escaped::DoubleQuote && c != '\\' && c != '"' {
                        token.push('\\');
                    }
                    token.push(c);
                    self.state = match escaped {
                        Escaped::Word => State::Word,
                        Escaped::DoubleQuote => State::Quote('"'),
                    };
                }
                State::Word => match next {
                    None => {
                        self.state = State::Eof;
                        break;
                    }
                    Some(c) if WHITESPACE.contains(&c) => {
                        self.state = State::Whitespace;
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some('#') if self.comments => {
                        self.skip_line();
                        self.state = State::Whitespace;
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some(c @ ('"' | '\'')) => self.state = State::Quote(c),
                    Some('\\') => self.state = State::Escape(Escaped::Word),
                    Some(c) => token.push(c),
                },
            }
        }

        if token.is_empty() && !quoted {
            Ok(None)
        } else {
            Ok(Some(token))
        }
    }
}

/// Splits a line into words like Python's `shlex.split(line)`.
///
/// # Errors
///
/// Returns an error for an unclosed quote or a trailing backslash.
pub fn split(line: &str) -> Result<Vec<String>, ShlexError> {
    let mut lexer = Lexer::new(line, false);
    let mut words = Vec::new();
    while let Some(word) = lexer.next_token()? {
        words.push(word);
    }
    Ok(words)
}

/// Splits off the first shell word and returns it with the raw remainder.
///
/// This is the `split_first_token` helper of the reference: the remainder is
/// returned without any unquoting (only leading whitespace is removed), and
/// both parts must be non-empty.
///
/// # Errors
///
/// Returns a message when there is no target or no command after it.
pub fn split_first_token(line: &str) -> Result<(String, String), String> {
    let mut lexer = Lexer::new(line, true);
    let target = lexer.next_token().map_err(|e| e.to_string())?;
    let command = crate::util::py::lstrip(lexer.rest());
    match target {
        Some(target) if !command.is_empty() => Ok((target, command.to_string())),
        _ => Err("Expected a target and a command".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn splits_plain_words() {
        assert_eq!(
            split("msg Alice hello").unwrap(),
            s(&["msg", "Alice", "hello"])
        );
        assert_eq!(split("  a\t b  ").unwrap(), s(&["a", "b"]));
        assert_eq!(split("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn keeps_hash_words() {
        assert_eq!(
            split("set_channel 1 #test").unwrap(),
            s(&["set_channel", "1", "#test"])
        );
        assert_eq!(
            split("to #public%#scope").unwrap(),
            s(&["to", "#public%#scope"])
        );
    }

    #[test]
    fn handles_quotes() {
        assert_eq!(
            split(r#"msg "Bob Smith" "hi there""#).unwrap(),
            s(&["msg", "Bob Smith", "hi there"])
        );
        assert_eq!(split("a'b c'd").unwrap(), s(&["ab cd"]));
        assert_eq!(split(r#"alias key """#).unwrap(), s(&["alias", "key", ""]));
        assert_eq!(split("'it''s'").unwrap(), s(&["its"]));
    }

    #[test]
    fn handles_escapes() {
        assert_eq!(split(r"a\ b").unwrap(), s(&["a b"]));
        assert_eq!(split(r#""a\"b""#).unwrap(), s(&[r#"a"b"#]));
        assert_eq!(split(r#""a\nb""#).unwrap(), s(&[r"a\nb"]));
        assert_eq!(split(r#""a\\b""#).unwrap(), s(&[r"a\b"]));
        assert_eq!(split(r"'a\b'").unwrap(), s(&[r"a\b"]));
    }

    #[test]
    fn reports_errors() {
        assert_eq!(split(r#"msg "open"#), Err(ShlexError::NoClosingQuotation));
        assert_eq!(split(r"msg a\"), Err(ShlexError::NoEscapedCharacter));
    }

    #[test]
    fn first_token_keeps_raw_rest() {
        assert_eq!(
            split_first_token(r#"out.txt get "name" | x"#).unwrap(),
            ("out.txt".to_string(), r#"get "name" | x"#.to_string())
        );
        assert_eq!(
            split_first_token(r#""contact markers.sh 18" get coords"#).unwrap(),
            (
                "contact markers.sh 18".to_string(),
                "get coords".to_string()
            )
        );
        assert!(split_first_token("onlyone").is_err());
        assert!(split_first_token("").is_err());
        assert!(split_first_token("#comment rest").is_err());
    }
}
