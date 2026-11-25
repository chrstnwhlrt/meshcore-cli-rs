//! Signals that end the program.

/// Number of the signal Ctrl-C sends.
pub const INTERRUPT: u8 = 2;

/// Waits for a signal that ends the program and returns its number: Ctrl-C
/// or one of [`terminated`].
pub async fn ending() -> u8 {
    tokio::select! {
        () = interrupted() => INTERRUPT,
        number = terminated() => number,
    }
}

/// Waits for Ctrl-C.
#[cfg(unix)]
async fn interrupted() {
    received(tokio::signal::unix::SignalKind::interrupt()).await;
}

/// Waits for Ctrl-C.
#[cfg(not(unix))]
async fn interrupted() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Waits for SIGTERM (`kill`, `timeout`, a service that is stopped) or SIGHUP
/// (the terminal is gone) and returns its number. Without a handler they
/// would end the program before it closes its connection.
#[cfg(unix)]
pub async fn terminated() -> u8 {
    use tokio::signal::unix::SignalKind;

    tokio::select! {
        () = received(SignalKind::terminate()) => 15,
        () = received(SignalKind::hangup()) => 1,
    }
}

/// There are no such signals here.
#[cfg(not(unix))]
pub async fn terminated() -> u8 {
    std::future::pending().await
}

/// Waits for a signal; for ever when the program was started to ignore it.
#[cfg(unix)]
async fn received(kind: tokio::signal::unix::SignalKind) {
    if ignored() >> (kind.as_raw_value() - 1) & 1 == 0
        && let Ok(mut signal) = tokio::signal::unix::signal(kind)
    {
        signal.recv().await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// The signals the program was started to ignore, signal 1 in the lowest
/// bit: `nohup` does that with SIGHUP, a shell without job control with
/// SIGINT for what it runs in the background. They stay ignored, as in
/// Python. Only Linux tells them in a file; elsewhere none are taken for
/// ignored.
#[cfg(unix)]
fn ignored() -> u64 {
    static IGNORED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *IGNORED.get_or_init(|| {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| ignored_mask(&status))
            .unwrap_or(0)
    })
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
