//! Handler processes fed with events (`handler_attach`).
//!
//! A handler is a shell command that receives one json line per event on
//! its stdin. Its output is relayed to the terminal. Each handler has a
//! bounded queue; when it is full, events are dropped with a warning.

use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::output;

/// Queue size of a handler (`HANDLER_QUEUE_SIZE`).
const QUEUE_SIZE: usize = 100;

/// Event streams a handler can attach to.
pub const KINDS: [&str; 3] = ["rxlog", "msgs", "chan_data"];

/// One attached process.
struct Handler {
    id: u32,
    kind: String,
    cmd: String,
    pid: Option<u32>,
    closed: Arc<AtomicBool>,
    queue: mpsc::Sender<String>,
    child: Child,
    tasks: Vec<JoinHandle<()>>,
}

impl Handler {
    fn describe(&self) -> Value {
        json!({
            "id": self.id,
            "type": self.kind,
            "cmd": self.cmd,
            "process": format!("<Process {}>", self.pid.map_or_else(|| "?".to_string(), |p| p.to_string())),
            "closed": self.closed.load(Ordering::Relaxed),
        })
    }
}

#[derive(Default)]
struct Inner {
    last_id: u32,
    handlers: Vec<Handler>,
}

/// Why a handler could not be attached.
#[derive(Debug)]
pub enum AttachError {
    /// Reported by `handler_attach`, which goes on.
    Logged(String),
    /// An exception the reference does not catch there; it ends the command
    /// like any other.
    Raised(String),
}

/// The error of starting a program with an empty name the way Python's
/// `subprocess` does: it tries `<dir>/` for each directory of `PATH`, and
/// exec refuses a directory, where exec would call the empty name missing.
pub fn empty_program_error() -> std::io::Error {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/bin:/usr/bin".to_string());
    let separator = if cfg!(windows) { ';' } else { ':' };
    if path
        .split(separator)
        .any(|dir| !dir.is_empty() && std::path::Path::new(dir).is_dir())
    {
        std::io::Error::from(std::io::ErrorKind::PermissionDenied)
    } else {
        std::io::Error::from(std::io::ErrorKind::NotFound)
    }
}

/// All attached handlers.
#[derive(Default)]
pub struct Handlers {
    inner: Mutex<Inner>,
}

impl Handlers {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts `cmd` as handler for `kind`; returns its description.
    ///
    /// # Errors
    ///
    /// Returns a message for unknown kinds or when the process cannot start.
    pub fn attach(&self, kind: &str, cmd: &str) -> Result<Value, AttachError> {
        use AttachError::{Logged, Raised};

        if !KINDS.contains(&kind) {
            return Err(Logged(format!("Value error Unknown handler type {kind}")));
        }
        let argv =
            crate::util::shlex::split(cmd).map_err(|e| Logged(format!("Value error {e}")))?;
        let Some((program, program_args)) = argv.split_first() else {
            // asyncio.create_subprocess_exec() called without a program
            return Err(Raised(
                "create_subprocess_exec() missing 1 required positional argument: 'program'"
                    .to_string(),
            ));
        };
        let spawn_error = |e: std::io::Error| {
            Logged(match e.kind() {
                std::io::ErrorKind::NotFound => format!("File not found {cmd}"),
                std::io::ErrorKind::PermissionDenied => format!("Permission denied: {cmd}"),
                _ => format!(
                    "Cannot pipe: {}",
                    crate::util::py::os_error(&e, Some(program))
                ),
            })
        };
        if program.is_empty() {
            return Err(spawn_error(empty_program_error()));
        }
        let pipe_error = |e: std::io::Error| {
            Logged(format!(
                "Cannot pipe: {}",
                crate::util::py::os_error(&e, None)
            ))
        };

        let mut command = Command::new(program);
        command
            .args(program_args)
            .stdin(Stdio::piped())
            .kill_on_drop(true);
        // stdout and stderr in one pipe, as the reference starts the process
        // (stderr=STDOUT), so that its lines keep their order
        #[cfg(unix)]
        let output = {
            let (reader, writer) = std::io::pipe().map_err(pipe_error)?;
            let errors = writer.try_clone().map_err(pipe_error)?;
            command.stdout(writer).stderr(errors);
            tokio::net::unix::pipe::Receiver::from_owned_fd(std::os::fd::OwnedFd::from(reader))
                .map_err(pipe_error)?
        };
        #[cfg(not(unix))]
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(spawn_error)?;
        // our copies of the write ends go, so the output ends with the process
        drop(command);

        let mut guard = self.lock();
        guard.last_id += 1;
        let id = guard.last_id;
        let closed = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel::<String>(QUEUE_SIZE);
        let mut tasks = Vec::new();

        if let Some(mut stdin) = child.stdin.take() {
            let closed = Arc::clone(&closed);
            tasks.push(tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    let written = async {
                        stdin.write_all(message.as_bytes()).await?;
                        stdin.flush().await
                    }
                    .await;
                    if written.is_err() {
                        closed.store(true, Ordering::Relaxed);
                        log::warn!("Handler {id} closed its input pipe");
                        return;
                    }
                }
            }));
        }
        #[cfg(unix)]
        tasks.push(tokio::spawn(relay(output)));
        #[cfg(not(unix))]
        {
            if let Some(stdout) = child.stdout.take() {
                tasks.push(tokio::spawn(relay(stdout)));
            }
            if let Some(stderr) = child.stderr.take() {
                tasks.push(tokio::spawn(relay(stderr)));
            }
        }

        let handler = Handler {
            id,
            kind: kind.to_string(),
            cmd: cmd.to_string(),
            pid: child.id(),
            closed,
            queue: tx,
            child,
            tasks,
        };
        let description = handler.describe();
        guard.handlers.push(handler);
        Ok(description)
    }

    /// Stops a handler; returns its description and command.
    ///
    /// # Errors
    ///
    /// Returns a message when there is no handler with this id.
    pub async fn detach(&self, id: u32) -> Result<(Value, String), String> {
        let handler = {
            let mut guard = self.lock();
            let pos = guard
                .handlers
                .iter()
                .position(|h| h.id == id)
                .ok_or_else(|| format!("{id}"))?;
            guard.handlers.remove(pos)
        };
        handler.closed.store(true, Ordering::Relaxed);
        let description = handler.describe();
        let Handler {
            cmd,
            queue,
            mut child,
            tasks,
            ..
        } = handler;
        drop(queue);
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        drop(child.stdin.take());
        // like the reference: wait, then terminate, then kill
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            terminate(&mut child).await;
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
        Ok((description, cmd))
    }

    /// Ends all handlers when the program does: each gets the events still
    /// queued for it, then the end of its input, and has time to finish
    /// until the caller stops waiting.
    pub async fn close(&self) {
        let handlers = std::mem::take(&mut self.lock().handlers);
        let mut endings = tokio::task::JoinSet::new();
        for handler in handlers {
            let Handler {
                queue,
                mut child,
                tasks,
                ..
            } = handler;
            drop(queue);
            endings.spawn(async move {
                let _ = child.wait().await;
                // what it printed last
                for task in tasks {
                    let _ = task.await;
                }
            });
        }
        while endings.join_next().await.is_some() {}
    }

    /// Queues an event for all handlers of `kind` (`enqueue_handler_event`).
    pub fn enqueue(&self, kind: &str, payload: &Value) {
        let guard = self.lock();
        if !guard.handlers.iter().any(|h| h.kind == kind) {
            return;
        }
        let message = format!("{}\n", crate::util::json::dumps(payload));
        for handler in guard.handlers.iter().filter(|h| h.kind == kind) {
            if handler.closed.load(Ordering::Relaxed) {
                continue;
            }
            if handler.queue.try_send(message.clone()).is_err() {
                log::warn!("Handler {} queue is full; dropping event", handler.id);
            }
        }
    }

    /// All handlers grouped by kind (`handlers`).
    #[must_use]
    pub fn list(&self) -> Value {
        let guard = self.lock();
        let mut out = serde_json::Map::new();
        for kind in KINDS {
            let mut map = serde_json::Map::new();
            for handler in guard.handlers.iter().filter(|h| h.kind == kind) {
                map.insert(handler.id.to_string(), handler.describe());
            }
            out.insert(kind.to_string(), Value::Object(map));
        }
        Value::Object(out)
    }
}

/// Asks a process to end (SIGTERM, `process.terminate()` in Python).
async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // the kill utility sends SIGTERM without unsafe code
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await;
        return;
    }
    // elsewhere terminating and killing are the same
    let _ = child.start_kill();
}

/// Copies a process output to the terminal.
async fn relay<R: tokio::io::AsyncRead + Unpin>(mut reader: R) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => output::print(&String::from_utf8_lossy(&buf[..n])),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_hands_over_the_last_events() {
        let file = std::env::temp_dir().join(format!("mccli-handler-{}", std::process::id()));
        let handlers = Handlers::default();
        let cmd = format!("sh -c 'cat > \"$1\"' sh '{}'", file.display());
        assert!(handlers.attach("msgs", &cmd).is_ok());
        for n in 0..3 {
            handlers.enqueue("msgs", &json!({ "n": n }));
        }
        let closed = tokio::time::timeout(Duration::from_secs(10), handlers.close()).await;
        assert!(closed.is_ok());
        let received = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        assert_eq!(received, "{\"n\": 0}\n{\"n\": 1}\n{\"n\": 2}\n");
    }
}
