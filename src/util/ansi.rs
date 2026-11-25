//! ANSI escape sequences, named as in the reference implementation.

pub const END: &str = "\x1b[0m";
pub const INVERT: &str = "\x1b[7m";
pub const NORMAL: &str = "\x1b[27m";
pub const GREEN: &str = "\x1b[0;32m";
pub const BGREEN: &str = "\x1b[1;32m";
pub const DGREEN: &str = "\x1b[0;38;5;22m";
pub const BLUE: &str = "\x1b[0;34m";
pub const BBLUE: &str = "\x1b[1;34m";
pub const RED: &str = "\x1b[0;31m";
pub const BRED: &str = "\x1b[1;31m";
pub const MAGENTA: &str = "\x1b[0;35m";
pub const BMAGENTA: &str = "\x1b[1;35m";
pub const CYAN: &str = "\x1b[0;36m";
pub const BCYAN: &str = "\x1b[1;36m";
pub const LIGHT_GRAY: &str = "\x1b[0;38;5;247m";
pub const BGRAY: &str = "\x1b[1;38;5;247m";
pub const GRAY_BACK: &str = "\x1b[48;5;247m";
pub const RESET_BACK: &str = "\x1b[49m";
pub const ORANGE: &str = "\x1b[0;38;5;214m";
pub const YELLOW: &str = "\x1b[0;33m";
pub const BYELLOW: &str = "\x1b[1;33m";

/// Start of a control sequence, used for cursor positioning (`\x1b[34G`).
pub const START: &str = "\x1b[";

/// Moves the cursor to the given column.
#[must_use]
pub fn column(col: usize) -> String {
    format!("{START}{col}G")
}
