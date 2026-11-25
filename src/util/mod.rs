//! Helpers that do not depend on the device connection.

pub mod ansi;
pub mod json;
pub mod printable;
pub mod py;
pub mod shlex;
pub mod strftime;
pub mod text;
pub mod time;

/// Locks a mutex, also after a panic of another thread.
pub fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
