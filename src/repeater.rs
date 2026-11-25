//! Repeater mode (`-r`): a line editor in front of a repeater's serial CLI.
//!
//! The repeater firmware offers a plain text console on its serial port.
//! This mode adds history, completion, clock syncing, and region upload and
//! download from files, like the reference implementation.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use serialport::{ClearBuffer, SerialPort};

use crate::editor::{Completion, CompletionTree, LineEditor, StyledPrompt};
use crate::help;
use crate::paths;
use crate::util::text::escape_ansi;
use crate::util::time::{format_local, now_secs};
use crate::util::{ansi, py};

/// Read timeout of the serial port, like `pyserial.Serial(timeout=1)`.
const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// An open connection to a repeater console.
pub struct Repeater {
    port: Box<dyn SerialPort>,
    color: bool,
}

impl Repeater {
    /// Opens the serial port and wakes up the console.
    ///
    /// # Errors
    ///
    /// Returns a printable message when the port cannot be opened.
    pub fn open(port_name: &str, baudrate: i64, color: bool) -> Result<Self, String> {
        log::info!(
            "{}",
            paint(
                color,
                &format!(
                    "{}Connecting to repeater at {port_name} ({baudrate} baud)...{}",
                    ansi::BCYAN,
                    ansi::END
                )
            )
        );
        // pyserial's ValueError for a rate it does not take
        let Some(rate) = crate::connect::serial_baudrate(baudrate) else {
            return Err(format!(
                "{}Error opening serial port: Not a valid baudrate: {baudrate}{}",
                ansi::BRED,
                ansi::END
            ));
        };
        let port = serialport::new(port_name, rate)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| {
                format!(
                    "{}Error opening serial port: {}{}",
                    ansi::BRED,
                    crate::connect::pyserial_open_error(port_name).unwrap_or_else(|| e.to_string()),
                    ansi::END
                )
            })?;
        let mut repeater = Self { port, color };
        std::thread::sleep(Duration::from_millis(500));
        repeater.clear_input();
        repeater.write("\r");
        std::thread::sleep(Duration::from_millis(200));
        repeater.clear_input();
        Ok(repeater)
    }

    fn clear_input(&mut self) {
        let _ = self.port.clear(ClearBuffer::Input);
    }

    fn write(&mut self, text: &str) {
        let _ = self.port.write_all(text.as_bytes());
        let _ = self.port.flush();
    }

    /// Prints a line, removing colors when they are disabled.
    fn print(&self, text: &str) {
        println!("{}", paint(self.color, text));
    }

    /// Reads up to `size` bytes, waiting at most one second.
    fn read(&mut self, size: usize) -> Vec<u8> {
        let deadline = Instant::now() + READ_TIMEOUT;
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while out.len() < size && Instant::now() < deadline {
            let want = (size - out.len()).min(buf.len());
            match self.port.read(&mut buf[..want]) {
                Ok(0) => {}
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
        out
    }

    /// Reads one line (up to `\n`), waiting at most one second.
    fn read_line(&mut self) -> String {
        let deadline = Instant::now() + READ_TIMEOUT;
        let mut out = Vec::new();
        let mut byte = [0u8; 1];
        while Instant::now() < deadline {
            match self.port.read(&mut byte) {
                Ok(1) => {
                    out.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
        decode_ignore(&out)
    }

    /// Asks the repeater for its name.
    pub fn name(&mut self) -> String {
        self.write("get name\r");
        let mut line = String::new();
        let mut tries = 0;
        while !line.starts_with("get name") {
            line = py::rstrip(&self.read_line()).to_string();
            tries += 1;
            if tries > 30 {
                log::error!("Could not retrieve name");
                return "UNKNOWN".to_string();
            }
        }
        let line = py::rstrip(&self.read_line()).to_string();
        line.strip_prefix("  -> > ")
            .map_or_else(|| "Repeater".to_string(), str::to_string)
    }

    /// Asks the repeater for its firmware version.
    pub fn version(&mut self) -> String {
        self.write("ver\r");
        let mut line = py::rstrip(&self.read_line()).to_string();
        let mut tries = 0;
        while line != "ver" {
            line = py::rstrip(&self.read_line()).to_string();
            tries += 1;
            if tries > 30 {
                log::error!("Error reading version");
                return "unknown".to_string();
            }
        }
        let line = py::rstrip(&self.read_line()).to_string();
        if line.starts_with("  -> ") {
            line.chars().skip(6).collect()
        } else {
            "unknown".to_string()
        }
    }

    /// Processes one command line; returns false when the repeater reported
    /// an error (used to stop scripts).
    pub fn process_line(&mut self, input: &str, echo: bool, name: Option<&str>) -> bool {
        let lower = input.to_lowercase();
        let mut cmd = input.to_string();

        if lower == "help" {
            self.print(&help::repeater_help());
            return true;
        }

        if lower == "clock sync" || lower == "st" || lower == "sync_time" {
            let now = now_secs();
            self.print(&format!(
                "{}Syncing clock to {} ({now}){}",
                ansi::GREEN,
                format_local(now, "%Y-%m-%d %H:%M:%S"),
                ansi::END
            ));
            cmd = format!("time {now}");
        }

        if lower.starts_with("region upload") || lower.starts_with("region load") {
            let Some(file) = file_argument(input, 2) else {
                log::info!("Region upload canceled");
                return false;
            };
            let path = paths::expand_user(&file);
            let content = match py::read_text(&path, py::TextRead::Lines) {
                Ok(content) => content,
                Err(py::TextError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    log::error!("File not found");
                    return false;
                }
                // the reference fails with these errors, with a decode
                // error only after sending part of the file
                Err(e) => {
                    log::error!("{}", e.message(&path.display().to_string()));
                    return false;
                }
            };
            self.write("region load\r");
            for line in py::lines(&content) {
                if py::strip(line).is_empty() {
                    break;
                }
                if !line.starts_with(';') {
                    self.write(&format!("{}\r", py::rstrip(line)));
                }
            }
            self.write("\r");
            // send nothing more, just collect the answer below
            cmd = String::new();
        }

        if lower.starts_with("region download") {
            let Some(file) = file_argument(input, 2) else {
                log::info!("Region download canceled");
                return false;
            };
            return self.download_regions(&file, name);
        }

        if lower.starts_with("script") {
            let Some(file) = file_argument(input, 1) else {
                log::info!("Script canceled");
                return false;
            };
            return self.run_script(&paths::expand_user(&file));
        }

        if !cmd.is_empty() {
            self.write(&format!("{cmd}\r"));
        }
        std::thread::sleep(Duration::from_millis(300));

        let waiting = self
            .port
            .bytes_to_read()
            .ok()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n > 0)
            .unwrap_or(4096);
        let response = decode_ignore(&self.read(waiting));
        let mut result = true;
        if !response.is_empty() {
            for line in py::rstrip(&response).split('\n') {
                let line = py::rstrip(line);
                if !line.is_empty() && (echo || line != cmd) {
                    let stripped = py::strip(line);
                    if stripped.starts_with("OK") || stripped.starts_with("ok") {
                        self.print(&format!("{}{line}{}", ansi::GREEN, ansi::END));
                    } else if stripped.starts_with("Error") || stripped.starts_with("ERR") {
                        self.print(&format!("{}{line}{}", ansi::RED, ansi::END));
                    } else if stripped.starts_with("->") {
                        self.print(&format!("{}{line}{}", ansi::CYAN, ansi::END));
                    } else if !stripped.starts_with("DEBUG") {
                        self.print(line);
                    }
                }
                if line.contains("-> Unknown command") || line.contains("-> Error") {
                    result = false;
                }
            }
        }
        result
    }

    fn download_regions(&mut self, file: &str, name: Option<&str>) -> bool {
        let path = paths::expand_user(file);
        let mut out = match std::fs::File::create(&path) {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::error!("File not found");
                return false;
            }
            // the reference fails with this error
            Err(e) => {
                log::error!("{}", py::os_error(&e, Some(&path.display().to_string())));
                return false;
            }
        };
        let name = name.map_or_else(|| self.name(), str::to_string);
        let header = format!(
            "; Regions spec downloaded from {name}\n; On {}\n",
            format_local(now_secs(), "%a %d %b %Y %H:%M %z")
        );
        let _ = out.write_all(header.as_bytes());

        self.write("region\r");
        let mut line = self.read_line();
        let mut tries = 0;
        while !line.starts_with("  ->") {
            line = self.read_line();
            tries += 1;
            if tries > 30 {
                log::error!("Error while communicating with target for downloading region");
                return false;
            }
        }
        let mut line: String = line.chars().skip(5).collect();
        while !py::rstrip(&line).is_empty() {
            let _ = out.write_all(line.as_bytes());
            line = self.read_line();
        }
        log::info!(
            "{}",
            paint(
                self.color,
                &format!(
                    "{}Wrote regions spec to {}{}",
                    ansi::CYAN,
                    path.display(),
                    ansi::END
                )
            )
        );
        true
    }

    fn run_script(&mut self, file: &Path) -> bool {
        if !file.exists() {
            log::info!("file {} not found", file.display());
            return false;
        }
        let content = match py::read_text(file, py::TextRead::Lines) {
            Ok(content) => content,
            // the reference fails with these errors
            Err(e) => {
                log::error!("{}", e.message(&file.display().to_string()));
                return false;
            }
        };
        for line in py::lines(&content) {
            let line = py::strip(line);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            log::debug!("processing {line}");
            if !self.process_line(line, true, None) {
                log::info!("Error during script execution, exiting");
                return false;
            }
        }
        true
    }

    /// Runs the interactive loop.
    pub fn interactive(&mut self) {
        let name = self.name();
        let version = self.version();
        self.print(&format!(
            "{}Connected!{} Device: {}{name}{} version {}{version}{}",
            ansi::BGREEN,
            ansi::END,
            ansi::BMAGENTA,
            ansi::END,
            ansi::BMAGENTA,
            ansi::END
        ));
        self.print(&format!(
            "Type {c}help{e} for commands, {c}quit{e} to exit, {c}Tab{e} for completion",
            c = ansi::BCYAN,
            e = ansi::END
        ));
        println!("{}", "-".repeat(50));

        let history = paths::existing_config_dir().map(|_| paths::repeater_history_file());
        let mut editor = match LineEditor::new(history) {
            Ok(e) => e,
            Err(e) => {
                log::error!("Cannot start line editor: {e}");
                return;
            }
        };
        editor.set_completion(Completion::Nested {
            tree: completion_tree(),
            slash_paths: false,
        });
        let prompt = StyledPrompt::new(
            &format!("{}{name}{}>{} ", ansi::BGRAY, ansi::MAGENTA, ansi::END),
            self.color,
        );

        while let Ok(line) = editor.readline(&prompt) {
            // prompt_toolkit keeps the line as typed in the history
            editor.remember(&line);
            let cmd = py::strip(&line).to_string();
            if cmd.is_empty() {
                continue;
            }
            if matches!(cmd.to_lowercase().as_str(), "quit" | "exit" | "q") {
                break;
            }
            self.process_line(&cmd, false, Some(&name));
        }
    }
}

/// Returns the file name argument (everything from word `index` on, keeping
/// its case so that paths with capitals and spaces work) or asks for one.
fn file_argument(input: &str, index: usize) -> Option<String> {
    let words: Vec<&str> = input.splitn(index + 1, ' ').collect();
    match words.get(index) {
        Some(file) if !py::strip(file).is_empty() => Some(py::strip(file).to_string()),
        _ => prompt_for_file(),
    }
}

/// Asks for a file name with path completion (`prompt_for_file`).
#[must_use]
pub fn prompt_for_file() -> Option<String> {
    let history = paths::existing_config_dir().map(|_| paths::file_prompt_history());
    let mut editor = LineEditor::new(history).ok()?;
    editor.set_completion(Completion::Path);
    let prompt = StyledPrompt::new("Enter filename (Tab to complete CTRL+C to cancel): ", false);
    let line = editor.readline(&prompt).ok()?;
    editor.remember(&line);
    Some(line)
}

/// Decodes bytes as UTF-8, dropping invalid sequences (`errors='ignore'`).
fn decode_ignore(bytes: &[u8]) -> String {
    bytes.utf8_chunks().map(|chunk| chunk.valid()).collect()
}

/// Completion candidates of the repeater console.
#[must_use]
pub fn completion_tree() -> CompletionTree {
    let mut root = CompletionTree::new();
    for key in [
        "ver",
        "board",
        "reboot",
        "advert",
        "time",
        "neighbors",
        "neighbor.remove",
        "discover.neighbors",
        "stats-core",
        "stats-radio",
        "stats-packets",
    ] {
        root.leaf(key);
    }
    root.node("clock", CompletionTree::leaves(["sync"]));
    root.node("clear", CompletionTree::leaves(["stats"]));
    root.node("log", CompletionTree::leaves(["start", "stop", "erase"]));

    let get = CompletionTree::leaves([
        "name",
        "radio",
        "tx",
        "freq",
        "public.key",
        "prv.key",
        "repeat",
        "role",
        "lat",
        "lon",
        "af",
        "rxdelay",
        "txdelay",
        "direct.txdelay",
        "flood.max",
        "flood.advert.interval",
        "advert.interval",
        "guest.password",
        "allow.read.only",
        "multi.acks",
        "int.thresh",
        "agc.reset.interval",
        "bridge.enabled",
        "bridge.delay",
        "bridge.source",
        "bridge.baud",
        "bridge.channel",
        "bridge.secret",
        "bridge.type",
        "adc.multiplier",
        "acl",
        "owner.info",
        "path.hash.mode",
    ]);
    root.node("get", get);

    let on_off = || CompletionTree::leaves(["on", "off"]);
    let mut set = CompletionTree::new();
    for key in ["name", "radio", "tx", "freq", "prv.key"] {
        set.leaf(key);
    }
    set.node("repeat", on_off());
    for key in [
        "lat",
        "lon",
        "af",
        "rxdelay",
        "txdelay",
        "direct.txdelay",
        "flood.max",
        "flood.advert.interval",
        "advert.interval",
        "guest.password",
    ] {
        set.leaf(key);
    }
    set.node("allow.read.only", on_off());
    for key in ["multi.acks", "int.thresh", "agc.reset.interval"] {
        set.leaf(key);
    }
    set.node("bridge.enabled", on_off());
    for key in [
        "bridge.delay",
        "bridge.source",
        "bridge.baud",
        "bridge.channel",
        "bridge.secret",
        "adc.multiplier",
        "owner.info",
    ] {
        set.leaf(key);
    }
    set.node("path.hash.mode", CompletionTree::leaves(["0", "1", "2"]));
    root.node("set", set);

    root.node("powersaving", on_off());
    root.leaf("password");
    root.leaf("erase");
    let mut gps = CompletionTree::leaves(["on", "off", "sync", "setloc"]);
    gps.node("advert", CompletionTree::leaves(["none", "share", "prefs"]));
    root.node("gps", gps);
    root.node("sensor", CompletionTree::leaves(["list", "get", "set"]));
    root.node(
        "region",
        CompletionTree::leaves([
            "get", "put", "remove", "save", "load", "home", "allowf", "denyf", "upload",
            "download", "list",
        ]),
    );
    for key in ["setperm", "tempradio", "script", "quit", "q", "help"] {
        root.leaf(key);
    }
    root
}

/// Runs repeater mode: one command from the command line, or the loop.
pub fn run(port: &str, baudrate: i64, color: bool, args: &[String]) {
    let mut repeater = match Repeater::open(port, baudrate, color) {
        Ok(r) => r,
        Err(message) => {
            println!("{}", paint(color, &message));
            return;
        }
    };
    log::debug!("Serial port opened: {port}");
    if args.is_empty() {
        repeater.interactive();
    } else {
        repeater.process_line(&args.join(" "), false, None);
    }
    log::info!(
        "{}",
        paint(
            color,
            &format!("{}Disconnected from repeater.{}", ansi::BGRAY, ansi::END)
        )
    );
}

/// Removes the colors of a text when colors are off (`-c off`).
fn paint(color: bool, text: &str) -> String {
    if color {
        text.to_string()
    } else {
        escape_ansi(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_argument_keeps_case() {
        assert_eq!(
            file_argument("region upload ~/Regions/Home.txt", 2),
            Some("~/Regions/Home.txt".to_string())
        );
        assert_eq!(
            file_argument("script My Script.txt", 1),
            Some("My Script.txt".to_string())
        );
    }

    #[test]
    fn decode_drops_invalid_bytes() {
        assert_eq!(decode_ignore(b"ok\xff\xfe done"), "ok done");
        // a replacement character sent as such stays
        assert_eq!(decode_ignore(b"a\xef\xbf\xbdb\xe2\x82"), "a\u{fffd}b");
    }

    #[test]
    fn completion_has_nested_entries() {
        let tree = completion_tree();
        let (_, c) = tree.complete("region d", false);
        assert_eq!(c, vec!["denyf", "download"]);
        let (_, c) = tree.complete("region do", false);
        assert_eq!(c, vec!["download"]);
        let (_, c) = tree.complete("set bridge.enabled o", false);
        assert_eq!(c, vec!["on", "off"]);
    }
}
