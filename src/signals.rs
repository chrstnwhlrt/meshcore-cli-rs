//! Signals that end the program.

/// Number of the signal Ctrl-C sends.
pub const INTERRUPT: u8 = 2;

/// Waits for a signal that ends the program and returns its number: Ctrl-C,
/// or on Unix also SIGTERM (`kill`, `timeout`, a service that is stopped) or
/// SIGHUP (the terminal is gone). Without a handler the last two would end
/// the program before it closes its connection.
#[cfg(unix)]
pub async fn ending() -> u8 {
    use tokio::signal::unix::{Signal, SignalKind, signal};

    async fn received(signal: &mut Option<Signal>) {
        match signal {
            Some(signal) => {
                signal.recv().await;
            }
            None => std::future::pending().await,
        }
    }

    let ignored = ignored();
    let listen = |kind: SignalKind| {
        (ignored >> (kind.as_raw_value() - 1) & 1 == 0)
            .then(|| signal(kind).ok())
            .flatten()
    };
    let mut interrupt = listen(SignalKind::interrupt());
    let mut terminate = listen(SignalKind::terminate());
    let mut hangup = listen(SignalKind::hangup());
    tokio::select! {
        () = received(&mut interrupt) => INTERRUPT,
        () = received(&mut terminate) => 15,
        () = received(&mut hangup) => 1,
    }
}

/// Waits for Ctrl-C and returns its number.
#[cfg(not(unix))]
pub async fn ending() -> u8 {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
    INTERRUPT
}

/// The signals the program was started to ignore, signal 1 in the lowest
/// bit: `nohup` does that with SIGHUP, a shell without job control with
/// SIGINT for what it runs in the background. They stay ignored, as in
/// Python. Only Linux tells them in a file; elsewhere none are taken for
/// ignored.
#[cfg(unix)]
fn ignored() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| ignored_mask(&status))
        .unwrap_or(0)
}

/// The mask of ignored signals in `/proc/self/status`.
#[cfg(unix)]
fn ignored_mask(status: &str) -> Option<u64> {
    let mask = status.lines().find_map(|l| l.strip_prefix("SigIgn:"))?;
    u64::from_str_radix(mask.trim(), 16).ok()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ignored_signals() {
        let status = "Name:\tsh\nSigBlk:\t0000000000000000\nSigIgn:\t0000000000001001\n";
        assert_eq!(ignored_mask(status), Some(0x1001));
        assert_eq!(ignored_mask("Name:\tsh\n"), None);
        assert_eq!(ignored_mask("SigIgn:\tx\n"), None);
    }
}
