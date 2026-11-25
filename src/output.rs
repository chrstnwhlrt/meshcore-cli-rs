//! Output routing and logging.
//!
//! * [`print()`] writes to the terminal; while the chat prompt is shown the
//!   text appears above the prompt, printed by a thread of its own with
//!   rustyline's external printer, like `prompt_toolkit`'s `patch_stdout` in
//!   the reference.
//! * [`Sink`] is the destination of command output: stdout, a buffer, a
//!   file or the stdin of a child process (`>`, `>>` and `|` redirections).
//! * [`Logger`] prints log records like Python's `logging.basicConfig`:
//!   `INFO:meshcore:message`.

use std::fs::File;
use std::io::{IsTerminal as _, Write as _};
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};

use log::{Level, LevelFilter, Log, Metadata, Record};
use rustyline::ExternalPrinter;

/// rustyline's printer for output above an active prompt.
pub type BoxedPrinter = Box<dyn ExternalPrinter + Send>;

/// The texts for the thread that prints above the prompt.
static PRINTER: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);

/// True while a prompt is shown that texts are printed above.
static AT_PROMPT: AtomicBool = AtomicBool::new(false);

/// The first error writing to stdout.
static STDOUT_ERROR: Mutex<Option<std::io::Error>> = Mutex::new(None);

/// Locks a mutex, also after a panic of another thread.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the thread that prints above the prompt. rustyline's printer
/// waits while the line editor takes no text, as from Enter until the next
/// prompt; the thread waits for it alone, so that nothing else does.
pub fn start_printer(mut printer: BoxedPrinter) {
    let (queue, texts) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for text in texts {
            if printer.print(text.clone()).is_err() {
                write_stdout(&text);
            }
        }
    });
    *lock(&PRINTER) = Some(queue);
}

/// Tells that a prompt is shown as long as this lives.
#[derive(Debug)]
pub struct PromptShown;

impl PromptShown {
    /// Starts printing above the prompt.
    #[must_use]
    pub fn new() -> Self {
        AT_PROMPT.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for PromptShown {
    fn drop(&mut self) {
        AT_PROMPT.store(false, Ordering::SeqCst);
    }
}

/// Hands a text to the thread that prints above the prompt; false when no
/// prompt is shown.
fn print_above_prompt(text: &str) -> bool {
    AT_PROMPT.load(Ordering::SeqCst)
        && lock(&PRINTER)
            .as_ref()
            .is_some_and(|queue| queue.send(text.to_string()).is_ok())
}

/// Writes text to stdout; the first error is kept for [`stdout_error`].
fn write_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        lock(&STDOUT_ERROR).get_or_insert(e);
    }
}

/// Writes text to the terminal (stdout).
pub fn print(text: &str) {
    if !text.is_empty() && !print_above_prompt(text) {
        write_stdout(text);
    }
}

/// Writes a line to the terminal.
pub fn println(text: &str) {
    print(&format!("{text}\n"));
}

/// Writes a line to stderr.
pub fn eprintln(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(format!("{text}\n").as_bytes());
}

/// Why stdout could not be written (a full disk, a pipe whose reader is
/// gone), in the words of Python's `OSError`, with which the reference ends.
#[must_use]
pub fn stdout_error() -> Option<String> {
    lock(&STDOUT_ERROR)
        .as_ref()
        .map(|e| crate::util::py::os_error(e, None))
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

    /// The error that broke the sink, in the words of Python's `OSError`,
    /// which ends the commands writing to it like the exception does in the
    /// reference.
    #[must_use]
    pub fn broken(&self) -> Option<String> {
        match self {
            Self::Stdout => stdout_error(),
            Self::File(_, error) | Self::Child(_, error) => {
                error.as_ref().map(|e| crate::util::py::os_error(e, None))
            }
            Self::Buffer(_) | Self::Closed => None,
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
        *lock(&LOG_LEVEL) = level;
    }

    fn level() -> LevelFilter {
        *lock(&LOG_LEVEL)
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
        let line = format!("{}:meshcore:{}", level_name(record.level()), record.args());
        // a terminal shows the records where the prompt is
        if !(std::io::stderr().is_terminal() && print_above_prompt(&format!("{line}\n"))) {
            eprintln(&line);
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

    /// A printer that hands over what it prints only when asked, like
    /// rustyline's while the line editor takes no text.
    struct SlowPrinter(mpsc::SyncSender<String>);

    impl ExternalPrinter for SlowPrinter {
        fn print(&mut self, msg: String) -> rustyline::Result<()> {
            let _ = self.0.send(msg);
            Ok(())
        }
    }

    #[test]
    fn printing_does_not_wait_for_the_prompt() {
        let (printed, shown) = mpsc::sync_channel(0);
        start_printer(Box::new(SlowPrinter(printed)));
        let prompt = PromptShown::new();
        for text in ["a", "b", "c"] {
            print(text);
        }
        drop(prompt);
        // the printer still waits, printing goes on without it
        print("");
        assert!(!print_above_prompt("d"));
        let texts: Vec<String> = shown.iter().take(3).collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    #[test]
    fn level_names_like_python() {
        assert_eq!(level_name(Level::Warn), "WARNING");
        assert_eq!(level_name(Level::Trace), "DEBUG");
    }
}
