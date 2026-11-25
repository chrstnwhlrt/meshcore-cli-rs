//! Locations of configuration files.
//!
//! Like the reference, everything lives in `~/.config/meshcore/` on every
//! platform, and files are only written when that directory already exists.

use std::ffi::OsStr;
use std::path::PathBuf;

/// Returns the home directory as `os.path.expanduser` finds it: `HOME` as
/// it is set, even when empty, else the user's entry in the password
/// database.
fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::home_dir()
}

/// The home directory of another user, from `/etc/passwd` as
/// `pwd.getpwnam` finds local users.
#[cfg(unix)]
fn user_home(name: &str) -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next() == Some(name) {
            fields.nth(4).map(PathBuf::from)
        } else {
            None
        }
    })
}

#[cfg(not(unix))]
fn user_home(_name: &str) -> Option<PathBuf> {
    None
}

/// Expands `~` and `~user` at the start of a path like Python's
/// `os.path.expanduser`: the home without trailing slashes followed by the
/// rest as written. An unknown user, or no home at all, leaves the path as
/// written.
#[must_use]
pub fn expand_user(path: &str) -> PathBuf {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let Some(after) = path.strip_prefix('~') else {
        return PathBuf::from(path);
    };
    let (user, rest) = after.split_at(after.find(is_sep).unwrap_or(after.len()));
    let home = if user.is_empty() {
        home_dir()
    } else {
        user_home(user)
    };
    let Some(home) = home else {
        return PathBuf::from(path);
    };
    match home.to_str() {
        Some(home) => {
            let expanded = format!("{}{rest}", home.trim_end_matches(is_sep));
            PathBuf::from(if expanded.is_empty() { "/" } else { &expanded })
        }
        // a home that is no UTF-8 text: joined, the same file
        None => home.join(rest.trim_start_matches(is_sep)),
    }
}

/// The configuration directory, `os.path.expanduser("~/.config/meshcore/")`
/// in the reference.
#[must_use]
pub fn config_dir() -> PathBuf {
    expand_user("~/.config/meshcore")
}

/// Returns the configuration directory if it exists.
#[must_use]
pub fn existing_config_dir() -> Option<PathBuf> {
    Some(config_dir()).filter(|d| d.is_dir())
}

/// Path of a file inside the configuration directory, built like the
/// reference's `MCCLI_CONFIG_DIR + name`: the name is appended as text, so
/// a node name or file name starting with `/` stays inside the directory
/// (`Path::join` would replace it).
#[must_use]
pub fn config_file(name: impl AsRef<OsStr>) -> PathBuf {
    let mut path = config_dir().into_os_string();
    path.push("/");
    path.push(name);
    PathBuf::from(path)
}

/// Address of the last BLE device (`default_address`).
#[must_use]
pub fn address_file() -> PathBuf {
    config_file("default_address")
}

/// Global init script.
#[must_use]
pub fn init_script() -> PathBuf {
    config_file("init")
}

/// Per-device init script `<name>.init`.
#[must_use]
pub fn device_init_script(name: &str) -> PathBuf {
    config_file(format!("{name}.init"))
}

/// Chat mode history.
#[must_use]
pub fn history_file() -> PathBuf {
    config_file("history")
}

/// Repeater mode history.
#[must_use]
pub fn repeater_history_file() -> PathBuf {
    config_file("repeater_history")
}

/// History of file name prompts.
#[must_use]
pub fn file_prompt_history() -> PathBuf {
    config_file("region_files_history")
}

/// Per channel scope overrides.
#[must_use]
pub fn scopes_file() -> PathBuf {
    config_file("scopes")
}

/// Stored password for a node, keyed by its public key.
#[must_use]
pub fn password_file(public_key: &str) -> PathBuf {
    config_file(format!("{public_key}.pass"))
}

/// Message log of a device (`/` removed from the name).
#[must_use]
pub fn message_log_file(device_name: &str) -> PathBuf {
    config_file(format!("{}.msgs", device_name.replace('/', "")))
}

/// Resolves a user supplied file name like `aliases_load`: stripped of
/// white space and with `~` expanded, as given or inside the configuration
/// directory as fallback.
#[must_use]
pub fn resolve_existing(name: &str) -> Option<PathBuf> {
    let direct = expand_user(crate::util::py::strip(name));
    if direct.exists() {
        return Some(direct);
    }
    Some(config_file(&direct)).filter(|p| p.exists())
}

/// Resolves a script name like the reference's `process_script`: stripped
/// of white space, as given or inside the configuration directory as
/// fallback. `~` is expanded before, by the `script` command only.
#[must_use]
pub fn resolve_script(name: &str) -> Option<PathBuf> {
    let name = crate::util::py::strip(name);
    let direct = PathBuf::from(name);
    if direct.exists() {
        return Some(direct);
    }
    Some(config_file(name)).filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home() {
        let home = std::env::home_dir().unwrap();
        assert_eq!(expand_user("~"), home);
        assert_eq!(expand_user("~/x/y"), home.join("x/y"));
        assert_eq!(expand_user("/abs"), PathBuf::from("/abs"));
        assert_eq!(expand_user("rel"), PathBuf::from("rel"));
        // os.path.expanduser glues the rest to the home as written
        let text = home.to_str().unwrap().trim_end_matches('/');
        assert_eq!(
            expand_user("~//x").to_str(),
            Some(format!("{text}//x").as_str())
        );
        // another user's home from the password file (in a build sandbox
        // root may live elsewhere than /root), an unknown user stays
        if let Some(root) = user_home("root") {
            assert_eq!(expand_user("~root/x"), root.join("x"));
        }
        assert_eq!(
            expand_user("~no-such-user-here/x"),
            PathBuf::from("~no-such-user-here/x")
        );
        assert_eq!(expand_user(" ~/x"), PathBuf::from(" ~/x"));
    }

    #[test]
    fn file_names() {
        assert!(message_log_file("a/b").ends_with("ab.msgs"));
        assert!(device_init_script("node").ends_with("node.init"));
        assert!(password_file("abcd").ends_with("abcd.pass"));
        assert!(config_dir().ends_with(".config/meshcore"));
        // a leading slash does not leave the directory
        assert_eq!(
            device_init_script("/tmp/x"),
            config_dir().join("tmp/x.init")
        );
    }
}
