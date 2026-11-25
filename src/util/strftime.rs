//! `datetime.fromtimestamp(t).strftime(pattern)`, the way the reference
//! formats times, including patterns users give with `print_timestamp`.
//!
//! Python formats a naive local time: it replaces `%f` with the
//! microseconds (`000000` for whole seconds) and `%z`, `%:z` and `%Z` with
//! nothing, then hands the pattern to the C library in ASCII runs, with a
//! buffer that limits how long the text may get. The rest follows glibc's
//! `strftime` in the C locale: the flags `_ - 0 ^ #`, a width, the
//! modifiers `E` and `O`, and directives it does not know copied as
//! written. jiff supplies the values.

use jiff::Timestamp;
use jiff::fmt::strtime::{BrokenDownTime, Config, PosixCustom};
use jiff::tz::TimeZone;

/// The directives glibc takes with the modifier `E`, and with `O`.
const WITH_E: &str = "cCnprRstTuxXyYPzZ%";
const WITH_O: &str = "bBCdegGhHIjklmMnprRsStTuUVwWyPzZ%";

/// Formats `timestamp` as local time in `tz` with a strftime `pattern`.
#[must_use]
pub fn strftime(timestamp: i64, pattern: &str, tz: &TimeZone) -> String {
    let Ok(ts) = Timestamp::from_second(timestamp) else {
        return timestamp.to_string();
    };
    let mut tm = BrokenDownTime::from(tz.to_datetime(ts));
    // for %s, which glibc gets back from the local time
    tm.set_timestamp(Some(ts));
    // time.strftime copies a NUL or non-ASCII character and the text after
    // it up to the next "%" itself; the ASCII runs from the start or from a
    // "%" go to the C library one by one, through one growing buffer
    let pattern = python_pattern(pattern);
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut buffer = 1024;
    let mut rest = pattern.as_str();
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| c == '\0' || !c.is_ascii())
            .unwrap_or(rest.len());
        let end = if end == 0 {
            let copied = rest.find('%').unwrap_or(rest.len());
            out.push_str(&rest[..copied]);
            copied
        } else {
            out.push_str(&run(&rest[..end], &tm, &mut buffer));
            end
        };
        rest = &rest[end..];
    }
    out
}

/// The pattern as `datetime.strftime` hands it to `time.strftime`: `%f`
/// becomes the microseconds (none for whole seconds), and `%z`, `%:z` and
/// `%Z` of a naive time become nothing.
fn python_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut rest = pattern;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (text, len) = if after.starts_with('f') {
            ("000000", 2)
        } else if after.starts_with(['z', 'Z']) {
            ("", 2)
        } else if after.starts_with(":z") {
            ("", 3)
        } else {
            // a pair the C library gets as it is, so "%%f" stays "%%f"
            let len = 1 + after.chars().next().map_or(0, char::len_utf8);
            (&rest[at..at + len], len)
        };
        out.push_str(text);
        rest = &rest[at + len..];
    }
    out.push_str(rest);
    out
}

/// One run through the C library the way `time.strftime` calls it: the
/// buffer (shared by all runs, at first 1024 bytes) doubles until the text
/// fits or it holds 256 times the run; a text that does not fit by then
/// comes out empty, and so does an empty one after growing the buffer.
fn run(run: &str, tm: &BrokenDownTime, buffer: &mut usize) -> String {
    let mut limit = *buffer;
    while limit < 256 * run.len() {
        limit *= 2;
    }
    let mut out = String::new();
    let mut rest = run;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let (text, len) = directive(&rest[at..], tm, limit);
        out.push_str(&text);
        rest = &rest[at + len..];
        if out.len() >= limit {
            break;
        }
    }
    out.push_str(rest);
    if out.is_empty() || out.len() >= limit {
        *buffer = limit;
        return String::new();
    }
    while out.len() >= *buffer {
        *buffer *= 2;
    }
    out
}

/// Formats the directive `spec` starts with (at its `%`); returns the text
/// and how many bytes of `spec` the directive takes. Widths are cut at
/// `limit`, which the text of the run must stay below anyway.
fn directive(spec: &str, tm: &BrokenDownTime, limit: usize) -> (String, usize) {
    let bytes = spec.as_bytes();
    let mut i = 1;
    let mut pad = None;
    let (mut upcase, mut change_case) = (false, false);
    while let Some(&b) = bytes.get(i) {
        match b {
            b'_' | b'-' | b'0' => pad = Some(b),
            b'^' => upcase = true,
            b'#' => change_case = true,
            _ => break,
        }
        i += 1;
    }
    let mut width = 0;
    while let Some(&b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        width = (width * 10 + usize::from(b - b'0')).min(limit);
        i += 1;
    }
    let modifier = match bytes.get(i) {
        Some(&m @ (b'E' | b'O')) => {
            i += 1;
            Some(m)
        }
        _ => None,
    };
    // A directive glibc does not know is copied as written; one the run
    // ends in ends there.
    let literal = |len: usize, upcase: bool| {
        let text = padded(&spec[..len], width, pad);
        let text = if upcase {
            text.to_ascii_uppercase()
        } else {
            text
        };
        (text, len)
    };
    let Some(conv) = spec[i..].chars().next() else {
        return literal(i, upcase);
    };
    let len = i + 1;
    let allowed = match modifier {
        Some(b'E') => WITH_E.contains(conv),
        Some(_) => WITH_O.contains(conv),
        None => true,
    };
    match allowed.then(|| convert(conv, tm, pad, width, upcase, change_case)) {
        Some(Some(text)) => (text, len),
        // glibc turns "#" into upper case for %b and %h before it finds the
        // modifier wrong, so "%#Eb" is copied as "%#EB"
        _ => literal(len, upcase || (change_case && matches!(conv, 'b' | 'h'))),
    }
}

/// The text of one known directive, `None` for one glibc does not know.
fn convert(
    conv: char,
    tm: &BrokenDownTime,
    pad: Option<u8>,
    width: usize,
    upcase: bool,
    change_case: bool,
) -> Option<String> {
    let text = match conv {
        'C' | 'd' | 'e' | 'G' | 'g' | 'H' | 'I' | 'j' | 'k' | 'l' | 'M' | 'm' | 'S' | 's' | 'U'
        | 'u' | 'V' | 'W' | 'w' | 'Y' | 'y' => return number(conv, tm, pad, width),
        'a' | 'A' | 'b' | 'B' | 'h' => {
            let name = value(tm, conv)?;
            if upcase || change_case {
                name.to_uppercase()
            } else {
                name
            }
        }
        // "#" turns AM/PM to lower case, "^" leaves it; %P stays lower case
        'p' => {
            let meridiem = value(tm, conv)?;
            if change_case {
                meridiem.to_lowercase()
            } else {
                meridiem
            }
        }
        'P' => value(tm, conv)?,
        'c' | 'D' | 'F' | 'r' | 'R' | 'T' | 'x' | 'X' => {
            let text = value(tm, conv)?;
            if upcase { text.to_uppercase() } else { text }
        }
        'n' => "\n".to_string(),
        't' => "\t".to_string(),
        '%' => "%".to_string(),
        // a naive time has no offset; glibc writes nothing, not even padding
        'z' => return Some(String::new()),
        // and an empty zone name, padded to the width unless "-" is given
        'Z' if pad == Some(b'-') => return Some(String::new()),
        'Z' => String::new(),
        _ => return None,
    };
    Some(padded(&text, width, pad))
}

/// A number padded like glibc: to its natural size (or the width) with
/// zeros, or spaces for `%e`, `%k`, `%l` and `%s` and with the `_` flag;
/// `-` leaves it unpadded, a width still pads it with spaces.
fn number(conv: char, tm: &BrokenDownTime, pad: Option<u8>, width: usize) -> Option<String> {
    let mut digits = tm
        .to_string_with_config(&config(), format!("%-{conv}"))
        .ok()?;
    let natural = match conv {
        'j' => 3,
        'G' | 'Y' => 4,
        's' | 'u' | 'w' => 1,
        _ => 2,
    };
    let pad = pad.or_else(|| matches!(conv, 'e' | 'k' | 'l' | 's').then_some(b'_'));
    let size = natural.max(width);
    if pad != Some(b'-') && digits.len() < size {
        let fill = if pad == Some(b'_') { " " } else { "0" };
        digits.insert_str(0, &fill.repeat(size - digits.len()));
    }
    Some(padded(&digits, width, pad))
}

/// jiff's text for a directive without flags, in the C locale.
fn value(tm: &BrokenDownTime, conv: char) -> Option<String> {
    tm.to_string_with_config(&config(), format!("%{conv}")).ok()
}

fn config() -> Config<PosixCustom> {
    Config::new().custom(PosixCustom::new())
}

/// Pads `text` on the left to `width` bytes: with zeros for the `0` flag,
/// spaces otherwise.
fn padded(text: &str, width: usize, pad: Option<u8>) -> String {
    let fill = if pad == Some(b'0') { "0" } else { " " };
    let mut out = fill.repeat(width.saturating_sub(text.len()));
    out.push_str(text);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_python_on_glibc() {
        // datetime.datetime.fromtimestamp(ts).strftime(pattern), Python
        // 3.14 on glibc with TZ='XXX-1' (one hour east of UTC, no DST)
        let tz = TimeZone::fixed(jiff::tz::offset(1));
        let cases: [(i64, &str, &str); 28] = [
            (1_795_557_777, "%Y-%m-%d %H:%M:%S", "2026-11-24 23:02:57"),
            (1_795_557_777, "%y-%m-%d %H:%M", "26-11-24 23:02"),
            (
                1_795_557_777,
                "%Y-%m-%d at %H:%M:%S",
                "2026-11-24 at 23:02:57",
            ),
            (
                1_795_557_777,
                "%a %d %b %Y %H:%M %z",
                "Tue 24 Nov 2026 23:02 ",
            ),
            (1_795_557_777, "%f|%z|%:z|%Z|%%f", "000000||||%f"),
            (
                1_795_557_777,
                "%c|%x|%X|%r|%D|%F|%R|%T",
                "Tue Nov 24 23:02:57 2026|11/24/26|23:02:57|11:02:57 PM|11/24/26|2026-11-24|23:02|23:02:57",
            ),
            (0, "%-d.%-m.%Y %k:%M %l%P", "1.1.1970  1:00  1am"),
            (0, "%e|%_d|%0e|%-H|%3d|%-3e", " 1| 1|01|1|001|  1"),
            (
                1_795_557_777,
                "%10A|%-10B|%010a|%^a|%#b|%#p|%^P|%^c",
                "   Tuesday|  November|0000000Tue|TUE|NOV|pm|pm|TUE NOV 24 23:02:57 2026",
            ),
            (
                1_795_557_777,
                "%5Y|%-5Y|%_15s|%5s|%3u|%03w|%3C|%6G",
                "02026| 2026|     1795557777|1795557777|002|002|020|002026",
            ),
            (0, "%5s|%05s|%-5s", "    0|00000|    0"),
            (
                1_704_067_199,
                "%G-W%V-%u|%g|%U|%W|%j|%w",
                "2024-W01-1|24|00|01|001|1",
            ),
            (1_704_070_799, "%G-W%V-%u|%U|%W|%j", "2024-W01-1|00|01|001"),
            (1_095_379_199, "%U|%W|%V|%j", "37|37|38|261"),
            (
                1_795_557_777,
                "%Ey|%Od|%Ec|%OB|%EY",
                "26|24|Tue Nov 24 23:02:57 2026|November|2026",
            ),
            (
                1_795_557_777,
                "%Ed|%5Ed|%E5y|%Oa|%-5Ew",
                "%Ed| %5Ed|%E5y|%Oa|%-5Ew",
            ),
            (
                1_795_557_777,
                "%q|%N|%Q|%.f|%3f|%+|%^v|%^5v|%i",
                "%q|%N|%Q|%.f|%3f|%+|%^V| %^5V|%i",
            ),
            (1_795_557_777, "%5|%^|%", "  %5|%^|%"),
            (1_795_557_777, "100%", "100%"),
            (
                1_795_557_777,
                "%5%|%05n|%5t|%5Z|%-5Z|%05Z|%-5z",
                "    %|0000\n|    \t|     ||00000|",
            ),
            (
                1_795_557_777,
                "ü %H ß|%ü|%5é|%#10ü|%^ü",
                "ü 23 ß|%ü|   %5é|      %#10ü|%^ü",
            ),
            (1_795_557_777, "%:|%:x|%::z|%%%H", "%:|%:x|%::z|%23"),
            (1_795_557_777, "%#Eb|%#5Eh|%^Ob|%#EB", "%#EB|%#5EH|NOV|%#EB"),
            (43200, "%I %p %l %P", "01 PM  1 pm"),
            (0, "%I %p|%#p", "01 AM|am"),
            (4_102_444_800, "%y %C %Y", "00 21 2100"),
            (
                2_147_483_647,
                "%s %Y-%m-%d %H:%M:%S",
                "2147483647 2038-01-19 04:14:07",
            ),
            (1_795_557_777, "%n%t%%", "\n\t%"),
        ];
        for (ts, pattern, want) in cases {
            assert_eq!(strftime(ts, pattern, &tz), want, "{pattern:?} at {ts}");
        }
    }

    #[test]
    fn long_texts_like_python() {
        // Python 3.14: a run whose text does not fit 256 times its length
        // (at least 1024 bytes) comes out empty; runs end at NUL and at
        // non-ASCII characters, and %f, %z and %Z count as replaced
        let tz = TimeZone::fixed(jiff::tz::offset(1));
        let len = |pattern: &str| strftime(0, pattern, &tz).len();
        assert_eq!(len("%1024H"), 1024);
        assert_eq!(len("%2047H"), 2047);
        assert_eq!(len("%2048H"), 0);
        assert_eq!(len("%3000Habcdefghij"), 3010);
        assert_eq!(len("%4090Hyyyyyyyyyy"), 0);
        assert_eq!(len("%f%3000H"), 3006);
        assert_eq!(len("%Z%Z%Z%Z%3000H"), 0);
        assert_eq!(len("%99999999999999999999H"), 0);
        assert_eq!(strftime(0, "é%3000H", &tz), "é");
        assert_eq!(strftime(0, "%3000H\0abc", &tz), "\0abc");
        assert_eq!(strftime(0, "%5\0", &tz), "   %5\0");
        // the text after a non-ASCII character is copied up to the next "%"
        assert_eq!(strftime(0, "éab%3000Hz", &tz), "éab");
        assert_eq!(len("éx%3000Hy%Mz"), 3007);
        // the buffer keeps the size an earlier run needed
        assert_eq!(len("%3000Habcdefghijé%2100H"), 5112);
        assert_eq!(len("%2047Hé%2100H"), 2049);
        assert_eq!(strftime(0, "", &tz), "");
        assert_eq!(strftime(i64::MAX, "%Y", &tz), i64::MAX.to_string());
    }
}
