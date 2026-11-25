//! Line editing with history and nested completion.
//!
//! The reference uses `prompt_toolkit` with a `NestedCompleter`; this module
//! provides the same behaviour on top of rustyline:
//!
//! * completion candidates come from a [`CompletionTree`]; the first word is
//!   completed from the top level, later words from the subtree of the first
//!   word, recursively,
//! * history is read from and appended to a `prompt_toolkit` compatible file,
//! * output from other tasks goes through an external printer so that it
//!   appears above the prompt (`prompt_toolkit`'s `patch_stdout`),
//! * a password prompt shows a star for each character typed, and Ctrl-C
//!   at any prompt ends only that prompt, as it does there,
//! * a line pasted along with a command answers the command's prompt.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Mutex;

use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::config::Configurer as _;
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{ColorMode, CompletionType, Config, Context, Editor, Helper, Prompt};

use crate::history::FileHistory;
use crate::output;
use crate::util::lock;
use crate::util::text::escape_ansi;

/// A prompt with a plain and a colored variant of the same width.
#[derive(Debug, Clone)]
pub struct StyledPrompt {
    raw: String,
    styled: String,
}

impl StyledPrompt {
    /// Builds a prompt from its colored form; the plain form strips ANSI.
    #[must_use]
    pub fn new(styled: &str, color: bool) -> Self {
        let raw = escape_ansi(styled);
        let styled = if color {
            styled.to_string()
        } else {
            raw.clone()
        };
        Self { raw, styled }
    }
}

impl Prompt for StyledPrompt {
    fn raw(&self) -> &str {
        &self.raw
    }

    fn styled(&self) -> &str {
        &self.styled
    }
}

/// Completion candidates organised like `prompt_toolkit`'s nested dicts.
#[derive(Debug, Clone, Default)]
pub struct CompletionTree {
    entries: Vec<(String, Option<CompletionTree>)>,
}

impl CompletionTree {
    /// Creates an empty tree.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Builds a tree whose entries are all leaves.
    #[must_use]
    pub fn leaves<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut tree = Self::new();
        for key in keys {
            tree.insert(key, None);
        }
        tree
    }

    /// Adds or replaces an entry, keeping the position of an existing key.
    pub fn insert(&mut self, key: impl Into<String>, child: Option<Self>) {
        let key = key.into();
        if let Some(entry) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = child;
        } else {
            self.entries.push((key, child));
        }
    }

    /// Adds a leaf entry.
    pub fn leaf(&mut self, key: impl Into<String>) {
        self.insert(key, None);
    }

    /// Adds an entry with a subtree.
    pub fn node(&mut self, key: impl Into<String>, child: Self) {
        self.insert(key, Some(child));
    }

    /// Merges all entries of `other` into this tree (`dict.update`).
    pub fn update(&mut self, other: &Self) {
        for (key, child) in &other.entries {
            self.insert(key.clone(), child.clone());
        }
    }

    /// Returns the subtree of an entry, if the entry exists and has one.
    #[must_use]
    pub fn child(&self, key: &str) -> Option<&Self> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, c)| c.as_ref())
    }

    /// Mutable access to the subtree of an entry.
    pub fn child_mut(&mut self, key: &str) -> Option<&mut Self> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .and_then(|(_, c)| c.as_mut())
    }

    /// Iterates over the keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Completes `text` (the line before the cursor).
    ///
    /// Returns the byte offset in `text` where the completed word starts and
    /// the candidates. With `slash_paths`, a first word like `/Ali` is
    /// completed to `/Alice` from keys of the form `/Alice/cmd`, like the
    /// reference's `MyNestedCompleter`.
    #[must_use]
    pub fn complete(&self, text: &str, slash_paths: bool) -> (usize, Vec<String>) {
        // prompt_toolkit's NestedCompleter: the first word goes to the
        // next level once the line holds a space
        let trimmed = crate::util::py::lstrip(text);
        let offset = text.len() - trimmed.len();

        if trimmed.contains(' ') {
            let first = crate::util::py::split(trimmed).next().unwrap_or_default();
            let Some(child) = self.child(first) else {
                return (text.len(), Vec::new());
            };
            let (start, candidates) = child.complete(&trimmed[first.len()..], false);
            return (offset + first.len() + start, candidates);
        }

        let word = trimmed;
        let keys: Vec<String> =
            if slash_paths && word.starts_with('/') && word.matches('/').count() == 1 {
                let mut options: Vec<String> = Vec::new();
                for key in self.keys().filter(|k| k.starts_with('/')) {
                    let head = key.split('/').nth(1).unwrap_or_default();
                    let option = format!("/{head}");
                    if !options.contains(&option) {
                        options.push(option);
                    }
                }
                options
            } else {
                self.keys().map(str::to_string).collect()
            };
        let lower = word.to_lowercase();
        let candidates = keys
            .into_iter()
            .filter(|k| k.to_lowercase().starts_with(&lower))
            .collect();
        (offset, candidates)
    }
}

/// What the editor offers on Tab.
#[derive(Debug, Clone, Default)]
pub enum Completion {
    /// No completion.
    #[default]
    None,
    /// Nested word completion.
    Nested {
        /// Candidates.
        tree: CompletionTree,
        /// Complete `/name` from `/name/cmd` keys for the first word.
        slash_paths: bool,
    },
    /// File name completion.
    Path,
}

/// rustyline helper providing completion, and the stars of a password.
struct CliHelper {
    completion: Completion,
    files: FilenameCompleter,
    masked: bool,
}

impl Completer for CliHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        match &self.completion {
            Completion::None => Ok((pos, Vec::new())),
            Completion::Path => self.files.complete(line, pos, ctx),
            Completion::Nested { tree, slash_paths } => {
                let (start, candidates) = tree.complete(&line[..pos], *slash_paths);
                Ok((
                    start,
                    candidates
                        .into_iter()
                        .map(|c| Pair {
                            display: c.clone(),
                            replacement: c,
                        })
                        .collect(),
                ))
            }
        }
    }
}

impl Hinter for CliHelper {
    type Hint = String;
}

impl Highlighter for CliHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }

    /// Shows a star for each character of a password, like `prompt_toolkit`
    /// does (`is_password=True`); a wide character gets as many stars as it
    /// takes columns, which rustyline needs to place the cursor.
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        use unicode_width::UnicodeWidthStr as _;
        if self.masked {
            Cow::Owned("*".repeat(line.width()))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
        self.masked && !matches!(kind, CmdKind::MoveCursor)
    }
}

impl Validator for CliHelper {}

impl Helper for CliHelper {}

/// The line editor while no prompt is shown. Every prompt uses the same one:
/// what was pasted after the line that answered the last prompt is in it, as
/// `prompt_toolkit` hands text typed ahead to the next prompt, whichever it
/// is.
static IDLE: Mutex<Option<LineEditor>> = Mutex::new(None);

/// Asks for a password (or PIN) without showing it. Ctrl-C and Ctrl-D end
/// the prompt like any other line editor prompt, instead of the program.
///
/// # Errors
///
/// Returns `Interrupted` on Ctrl-C and `Eof` on Ctrl-D.
pub fn read_password(prompt: &str) -> Result<String, ReadlineError> {
    ask(prompt, Completion::None, &[], true)
}

/// Reads one line without history or completion, like a bare
/// `PromptSession().prompt_async()` of the reference.
///
/// # Errors
///
/// Returns `Interrupted` on Ctrl-C and `Eof` on Ctrl-D.
pub fn read_line(prompt: &str) -> Result<String, ReadlineError> {
    ask(prompt, Completion::None, &[], false)
}

/// Asks for a file name, with path completion and the names given before.
///
/// # Errors
///
/// Returns `Interrupted` on Ctrl-C and `Eof` on Ctrl-D.
pub fn read_file_name(prompt: &str, history: Option<PathBuf>) -> Result<String, ReadlineError> {
    let mut history = history.map(FileHistory::new);
    let names = history.as_mut().map(FileHistory::load).unwrap_or_default();
    let name = ask(prompt, Completion::Path, &names, false)?;
    if let Some(history) = &mut history {
        history.append(&name);
    }
    Ok(name)
}

/// Reads a line at a prompt of its own.
fn ask(
    prompt: &str,
    completion: Completion,
    history: &[String],
    masked: bool,
) -> Result<String, ReadlineError> {
    let mut editor = LineEditor::idle_or_new()?;
    let prompt = StyledPrompt::new(prompt, false);
    let result = editor.ask(&prompt, completion, history, masked);
    editor.lend();
    result
}

/// The settings of all line editors.
fn config() -> rustyline::Result<Config> {
    Ok(Config::builder()
        .completion_type(CompletionType::List)
        .auto_add_history(false)
        .history_ignore_dups(true)?
        .max_history_size(100_000)?
        .color_mode(ColorMode::Enabled)
        .build())
}

/// A line editor with `prompt_toolkit` compatible history.
pub struct LineEditor {
    editor: Editor<CliHelper, DefaultHistory>,
    history: Option<FileHistory>,
    /// Texts of other tasks are printed above the prompt.
    prints_above: bool,
}

impl LineEditor {
    /// An editor for the lines of a console; their history is loaded from
    /// `history_path` if given.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal cannot be initialised.
    pub fn new(history_path: Option<PathBuf>) -> rustyline::Result<Self> {
        let mut editor = Self::idle_or_new()?;
        editor.history = history_path.map(|path| {
            let mut file = FileHistory::new(path);
            for entry in file.load() {
                let _ = editor.editor.add_history_entry(entry);
            }
            file
        });
        Ok(editor)
    }

    /// The idle editor, or a new one without history.
    fn idle_or_new() -> rustyline::Result<Self> {
        if let Some(editor) = Self::take_back() {
            return Ok(editor);
        }
        let mut editor: Editor<CliHelper, DefaultHistory> = Editor::with_config(config()?)?;
        editor.set_helper(Some(CliHelper {
            completion: Completion::None,
            files: FilenameCompleter::new(),
            masked: false,
        }));
        Ok(Self {
            editor,
            history: None,
            prints_above: false,
        })
    }

    /// Sets the completion used for the next lines; returns the one before.
    pub fn set_completion(&mut self, completion: Completion) -> Completion {
        self.editor
            .helper_mut()
            .map(|helper| std::mem::replace(&mut helper.completion, completion))
            .unwrap_or_default()
    }

    /// Lets the prompts of a command use the editor until
    /// [`take_back`](Self::take_back).
    pub fn lend(self) {
        *lock(&IDLE) = Some(self);
    }

    /// Takes the editor back that was lent.
    #[must_use]
    pub fn take_back() -> Option<Self> {
        lock(&IDLE).take()
    }

    /// Reads a line at a prompt of its own: with other completion and
    /// history, or masked.
    fn ask(
        &mut self,
        prompt: &StyledPrompt,
        completion: Completion,
        history: &[String],
        masked: bool,
    ) -> Result<String, ReadlineError> {
        let no_history = DefaultHistory::with_config(&config()?);
        let console_history = std::mem::replace(self.editor.history_mut(), no_history);
        for entry in history {
            let _ = self.editor.add_history_entry(entry.as_str());
        }
        let console_completion = self.set_completion(completion);
        self.set_masked(masked);
        let result = self.readline(prompt);
        self.set_masked(false);
        self.set_completion(console_completion);
        *self.editor.history_mut() = console_history;
        result
    }

    /// Shows stars for what is typed, also on a terminal without colors.
    fn set_masked(&mut self, masked: bool) {
        self.editor.set_color_mode(if masked {
            ColorMode::Forced
        } else {
            ColorMode::Enabled
        });
        if let Some(helper) = self.editor.helper_mut() {
            helper.masked = masked;
        }
    }

    /// Reads one line. Bytes that are no UTF-8, from a file or a terminal
    /// with another encoding, are reported and the prompt starts anew.
    ///
    /// # Errors
    ///
    /// Returns `Interrupted` on Ctrl-C and `Eof` on Ctrl-D.
    pub fn readline(&mut self, prompt: &StyledPrompt) -> Result<String, ReadlineError> {
        let _shown = self.prints_above.then(output::PromptShown::new);
        loop {
            match self.editor.readline(prompt) {
                Err(ReadlineError::Io(e)) if e.kind() == std::io::ErrorKind::InvalidData => {
                    log::error!("Unexpected error: {e}");
                }
                result => return result,
            }
        }
    }

    /// Adds a line to the in-memory and the file history.
    pub fn remember(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let _ = self.editor.add_history_entry(line);
        if let Some(history) = &mut self.history {
            history.append(line);
        }
    }

    /// Prints the texts of other tasks above the prompt while reading,
    /// unless input or output is no terminal.
    pub fn print_above_prompt(&mut self) {
        if let Ok(printer) = self.editor.create_external_printer() {
            output::start_printer(Box::new(printer));
            self.prints_above = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> CompletionTree {
        let mut set = CompletionTree::new();
        set.leaf("name");
        set.node("radio", CompletionTree::leaves(["f,bw,sf,cr"]));
        set.leaf("bridge.enabled");
        let mut root = CompletionTree::new();
        root.node("set", set);
        root.leaf("sync_msgs");
        root.leaf("script");
        root.leaf("/Alice/login");
        root.leaf("/Alice/logout");
        root.leaf("/Bob/cmd");
        root.leaf("/infos");
        root
    }

    #[test]
    fn completes_first_word() {
        let (start, c) = tree().complete("s", false);
        assert_eq!(start, 0);
        assert_eq!(c, vec!["set", "sync_msgs", "script"]);
        let (start, c) = tree().complete("  SY", false);
        assert_eq!(start, 2);
        assert_eq!(c, vec!["sync_msgs"]);
    }

    #[test]
    fn completes_nested_words() {
        let (start, c) = tree().complete("set ", false);
        assert_eq!(start, 4);
        assert_eq!(c, vec!["name", "radio", "bridge.enabled"]);
        let (start, c) = tree().complete("set bridge.e", false);
        assert_eq!(start, 4);
        assert_eq!(c, vec!["bridge.enabled"]);
        let (start, c) = tree().complete("set  radio f", false);
        assert_eq!(start, 11);
        assert_eq!(c, vec!["f,bw,sf,cr"]);
        let (_, c) = tree().complete("unknown x", false);
        assert_eq!(c, Vec::<String>::new());
        // the first word ends at any white space, the level changes at a space
        let (start, c) = tree().complete("set\tradio f", false);
        assert_eq!(start, 10);
        assert_eq!(c, vec!["f,bw,sf,cr"]);
    }

    #[test]
    fn slash_paths_collapse_to_destinations() {
        let (_, c) = tree().complete("/", true);
        assert_eq!(c, vec!["/Alice", "/Bob", "/infos"]);
        let (_, c) = tree().complete("/Alice/lo", true);
        assert_eq!(c, vec!["/Alice/login", "/Alice/logout"]);
    }

    #[test]
    fn styled_prompt_strips_ansi() {
        let p = StyledPrompt::new("\x1b[1;32mnode\x1b[0m> ", true);
        assert_eq!(p.raw(), "node> ");
        assert_eq!(p.styled(), "\x1b[1;32mnode\x1b[0m> ");
        let p = StyledPrompt::new("\x1b[1;32mnode\x1b[0m> ", false);
        assert_eq!(p.styled(), "node> ");
    }
}
