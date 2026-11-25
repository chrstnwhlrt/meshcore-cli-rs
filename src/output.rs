//! Output routing and logging.
//!
//! * [`print()`] writes to the terminal; while the chat prompt is active the
//!   text goes through rustyline's external printer and appears above the
//!   prompt, like `prompt_toolkit`'s `patch_stdout` in the reference.
//! * [`Sink`] is the destination of command output: stdout, a buffer, a
//!   file or the stdin of a child process (`>`, `>>` and `|` redirections).
//! * [`Logger`] prints log records like Python's `logging.basicConfig`:
//!   `INFO:meshcore:message`.

use std::fs::File;
use std::io::Write as _;
use std::process::{ChildStdin, Stdio};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};
use rustyline::ExternalPrinter;

/// rustyline's printer for output above an active prompt.
pub type BoxedPrinter = Box<dyn ExternalPrinter + Send>;

/// The printer used while the line editor is active.
static PRINTER: Mutex<Option<BoxedPrinter>> = Mutex::new(None);

/// Installs (or removes) the external printer of the line editor.
pub fn set_external_printer(printer: Option<BoxedPrinter>) {
    if let Ok(mut guard) = PRINTER.lock() {
        *guard = printer;
    }
}

/// Returns true while an external printer is installed.
#[must_use]
pub fn has_external_printer() -> bool {
    PRINTER.lock().is_ok_and(|g| g.is_some())
}

/// Writes text to the terminal (stdout).
pub fn print(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut guard) = PRINTER.lock()
        && let Some(printer) = guard.as_mut()
        && printer.print(text.to_string()).is_ok()
    {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Writes a line to the terminal.
pub fn println(text: &str) {
    print(&format!("{text}\n"));
}

/// Destination of command output.
#[derive(Debug)]
pub enum Sink {
    /// The terminal.
    Stdout,
    /// Collected in memory (`io.StringIO` in the reference).
    Buffer(String),
    /// A file opened by `>` or `>>`, with the first error writing to it.
    File(File, Option<std::io::Error>),
    /// The stdin of a process started by `|`, with the first error writing
    /// to it.
    Child(ChildStdin, Option<std::io::Error>),
    /// A sink whose handle was handed over to a child process.
    Closed,
}

impl Sink {
    /// Creates an empty buffer sink.
    #[must_use]
    pub const fn buffer() -> Self {
        Self::Buffer(String::new())
    }

    /// Writes text to the sink.
    pub fn write(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self {
            Self::Stdout => print(text),
            Self::Buffer(buf) => buf.push_str(text),
            // after the first error, like a full disk or a pipe the reader
            // closed, the rest goes nowhere
            Self::File(f, error @ None) => {
                if let Err(e) = f.write_all(text.as_bytes()) {
                    *error = Some(e);
                }
            }
            Self::Child(c, error @ None) => {
                if let Err(e) = c.write_all(text.as_bytes()) {
                    *error = Some(e);
                }
            }
            Self::File(_, Some(_)) | Self::Child(_, Some(_)) | Self::Closed => {}
        }
    }

    /// The error that broke a file or a pipe to a process, which ends the
    /// commands writing to it like the exception does in the reference.
    #[must_use]
    pub const fn broken(&self) -> Option<&std::io::Error> {
        match self {
            Self::File(_, error) | Self::Child(_, error) => error.as_ref(),
            _ => None,
        }
    }

    /// Flushes the sink.
    pub fn flush(&mut self) {
        match self {
            Self::File(f, _) => {
                let _ = f.flush();
            }
            Self::Child(c, _) => {
                let _ = c.flush();
            }
            Self::Stdout | Self::Buffer(_) | Self::Closed => {}
        }
    }

    /// Returns the collected text of a buffer sink.
    #[must_use]
    pub fn into_string(self) -> String {
        match self {
            Self::Buffer(buf) => buf,
            _ => String::new(),
        }
    }

    /// Converts the sink into the stdout of a child process.
    ///
    /// A child stdin is handed over (the sink becomes [`Sink::Closed`]),
    /// a file is duplicated, the terminal is inherited.
    ///
    /// # Errors
    ///
    /// Fails for buffers, which cannot back a process output.
    pub fn take_stdio(&mut self) -> std::io::Result<Stdio> {
        match self {
            Self::Stdout => Ok(Stdio::inherit()),
            Self::File(f, _) => Ok(Stdio::from(f.try_clone()?)),
            Self::Child(..) => match std::mem::replace(self, Self::Closed) {
                Self::Child(c, _) => Ok(Stdio::from(c)),
                _ => unreachable!("checked above"),
            },
            Self::Buffer(_) | Self::Closed => Err(std::io::Error::other(
                "output cannot be redirected to a process here",
            )),
        }
    }
}

/// Log level selected by the command line options.
static LOG_LEVEL: Mutex<LevelFilter> = Mutex::new(LevelFilter::Info);

/// A logger printing `LEVEL:meshcore:message` like Python's logging.
pub struct Logger;

static LOGGER: Logger = Logger;

impl Logger {
    /// Installs the logger with the default level (INFO).
    pub fn init() {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(LevelFilter::Trace);
    }

    /// Changes the level (`-D`, `-q`, `-j`).
    pub fn set_level(level: LevelFilter) {
        if let Ok(mut guard) = LOG_LEVEL.lock() {
            *guard = level;
        }
    }

    fn level() -> LevelFilter {
        LOG_LEVEL.lock().map_or(LevelFilter::Info, |g| *g)
    }
}

/// Python's name of a log level.
const fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARNING",
        Level::Info => "INFO",
        Level::Debug | Level::Trace => "DEBUG",
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // Like the reference, -D only raises the "meshcore" logger (the
        // library and this CLI); other crates stay at WARNING.
        let level = Self::level();
        if metadata.target().starts_with("meshcore") {
            metadata.level() <= level
        } else {
            metadata.level() <= level.min(LevelFilter::Warn)
        }
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{}:meshcore:{}\n",
            level_name(record.level()),
            record.args()
        );
        if has_external_printer() {
            print(&line);
        } else {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_sink_collects() {
        let mut sink = Sink::buffer();
        sink.write("a");
        sink.write("b\n");
        assert_eq!(sink.into_string(), "ab\n");
    }

    #[test]
    fn buffer_cannot_feed_a_process() {
        let mut sink = Sink::buffer();
        assert!(sink.take_stdio().is_err());
    }

    #[test]
    fn level_names_like_python() {
        assert_eq!(level_name(Level::Warn), "WARNING");
        assert_eq!(level_name(Level::Trace), "DEBUG");
    }
}
