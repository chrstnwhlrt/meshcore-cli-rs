//! Time and distance helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use jiff::tz::TimeZone;

/// Current Unix time in seconds.
#[must_use]
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Current Unix time with its fraction of a second, like `time.time()`.
#[must_use]
pub fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Current Unix time as the 32-bit value the protocol uses.
#[must_use]
pub fn now_u32() -> u32 {
    u32::try_from(now_secs()).unwrap_or(u32::MAX)
}

/// Formats a Unix timestamp in local time with a strftime pattern, like
/// the reference's `datetime.fromtimestamp(timestamp).strftime(pattern)`
/// (see [`crate::util::strftime`]).
#[must_use]
pub fn format_local(timestamp: i64, pattern: &str) -> String {
    crate::util::strftime::strftime(timestamp, pattern, &local_zone())
}

/// The local time zone, chosen once. A set `TZ` names a file first, as
/// glibc's `tzset` reads it for the reference: see [`zone_file`]. The rest
/// is left to jiff: an unset `TZ` (the zone of `/etc/localtime`), POSIX
/// rules, and names glibc does not find (in another case, or in a
/// zoneinfo directory other than `TZDIR` and `/usr/share/zoneinfo`).
#[must_use]
pub fn local_zone() -> TimeZone {
    static ZONE: std::sync::OnceLock<TimeZone> = std::sync::OnceLock::new();
    ZONE.get_or_init(|| {
        zone_file(
            std::env::var("TZ").ok().as_deref(),
            std::env::var_os("TZDIR").as_deref(),
        )
        .unwrap_or_else(TimeZone::system)
    })
    .clone()
}

/// The zone of the file a `TZ` value names, without a leading `:`: an
/// absolute path, or a path under `tzdir` (`/usr/share/zoneinfo` when
/// unset or empty) taken as written, so that `posix/Europe/Berlin` or
/// `./Europe/Berlin` work as in glibc. Only regular files are read, at
/// most 1 MiB of them. Anything else there, like a directory or
/// `/dev/zero`, gives UTC as in glibc; jiff would read a device until the
/// memory runs out.
#[cfg(unix)]
fn zone_file(tz: Option<&str>, tzdir: Option<&std::ffi::OsStr>) -> Option<TimeZone> {
    use std::io::Read;
    let name = tz?;
    let name = name.strip_prefix(':').unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    let path = if name.starts_with('/') {
        std::path::PathBuf::from(name)
    } else {
        let dir = tzdir
            .filter(|dir| !dir.is_empty())
            .unwrap_or_else(|| "/usr/share/zoneinfo".as_ref());
        std::path::Path::new(dir).join(name)
    };
    if !std::fs::metadata(&path).ok()?.is_file() {
        return Some(TimeZone::UTC);
    }
    let mut data = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(1 << 20)
        .read_to_end(&mut data)
        .ok()?;
    TimeZone::tzif(name, &data).ok()
}

/// Outside Unix the reference's C library reads `TZ` its own way; jiff
/// decides there.
#[cfg(not(unix))]
fn zone_file(_tz: Option<&str>, _tzdir: Option<&std::ffi::OsStr>) -> Option<TimeZone> {
    None
}

/// Formats a message or advert timestamp the way `print_timestamp` asks for.
///
/// `setting` is the value of the `print_timestamp` option: `on` selects
/// `%H:%M` for the last 24 hours and `%y-%m-%d %H:%M` otherwise, anything
/// else is used as strftime pattern. Returns `None` when timestamps are off.
#[must_use]
pub fn format_print_timestamp(setting: &str, timestamp: i64) -> Option<String> {
    if setting.is_empty() || setting == "off" {
        return None;
    }
    let pattern = if setting == "on" {
        // abs(time.time() - ts), with the fraction of the current second
        #[allow(clippy::cast_precision_loss)]
        if (now_f64() - timestamp as f64).abs() < 86_400.0 {
            "%H:%M"
        } else {
            "%y-%m-%d %H:%M"
        }
    } else {
        setting
    };
    Some(format_local(timestamp, pattern))
}

/// Short relative age like `5m`, `3d` or `+2h` for future values
/// (`time_ago_from_timestamp`).
#[must_use]
pub fn time_ago(timestamp: i64, now: i64) -> String {
    if timestamp == 0 {
        return "0".to_string();
    }
    let mut duration = now - timestamp;
    let in_future = duration < 0;
    if in_future {
        duration = -duration;
        if duration > 86_400 {
            return String::new();
        }
    }
    let text = if duration >= 86_400 {
        let days = duration / 86_400;
        if days >= 365 {
            format!("{}y", days / 365)
        } else {
            format!("{days}d")
        }
    } else if duration >= 3600 {
        format!("{}h", duration / 3600)
    } else if duration >= 60 {
        format!("{}m", duration / 60)
    } else {
        format!("{duration}s")
    };
    if in_future { format!("+{text}") } else { text }
}

/// Great-circle distance in kilometres between two coordinates.
#[must_use]
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;
    let (lat1, lon1, lat2, lon2) = (
        lat1.to_radians(),
        lon1.to_radians(),
        lat2.to_radians(),
        lon2.to_radians(),
    );
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let a = a.clamp(0.0, 1.0);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_KM * c
}

/// Longest wait in seconds, so that `now + duration` cannot overflow.
const MAX_WAIT_SECS: f64 = 1e9;

/// Seconds from user input as a duration, without panicking: negative and
/// NaN values give zero, very large or infinite ones are capped.
#[must_use]
pub fn duration_from_secs(secs: f64) -> std::time::Duration {
    if secs.is_nan() || secs <= 0.0 {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f64(secs.min(MAX_WAIT_SECS))
}

/// Parses a time offset like `2d`, `5h`, `30m` (relative) or an absolute
/// Unix timestamp, as used by the `apply_to` filters `u` and `a`.
///
/// Returns the absolute timestamp the filter compares against.
///
/// # Errors
///
/// `None` for an empty value (the reference's `IndexError`), otherwise the
/// message of Python's `float()` or `int()`.
pub fn parse_filter_time(value: &str, now: f64) -> Result<f64, Option<String>> {
    let mut chars = value.chars();
    let unit = chars.next_back().ok_or(None)?;
    let number = chars.as_str();
    let factor = match unit {
        'd' => 86_400.0,
        'h' => 3600.0,
        'm' => 60.0,
        #[allow(clippy::cast_precision_loss)]
        _ => {
            return crate::util::py::int(value, 10)
                .map(|v| v as f64)
                .map_err(Some);
        }
    };
    crate::util::py::float(number)
        .map(|n| now - n * factor)
        .map_err(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        let now = 1_000_000;
        assert_eq!(time_ago(0, now), "0");
        assert_eq!(time_ago(now - 5, now), "5s");
        assert_eq!(time_ago(now - 120, now), "2m");
        assert_eq!(time_ago(now - 7200, now), "2h");
        assert_eq!(time_ago(now - 3 * 86_400, now), "3d");
        assert_eq!(time_ago(now - 800 * 86_400, now), "2y");
        assert_eq!(time_ago(now + 120, now), "+2m");
        assert_eq!(time_ago(now + 2 * 86_400, now), "");
    }

    #[test]
    fn haversine_known_distance() {
        // Hamburg -> Berlin, roughly 255 km.
        let d = haversine_km(53.5511, 9.9937, 52.5200, 13.4050);
        assert!((d - 255.0).abs() < 5.0, "{d}");
        assert!(haversine_km(1.0, 1.0, 1.0, 1.0).abs() < 1e-9);
    }

    #[test]
    fn filter_times() {
        assert_eq!(
            parse_filter_time("2d", 1_000_000.0),
            Ok(1_000_000.0 - 172_800.0)
        );
        assert_eq!(parse_filter_time("1.5h", 10_000.0), Ok(10_000.0 - 5400.0));
        assert_eq!(parse_filter_time("1700000000", 0.0), Ok(1_700_000_000.0));
        assert_eq!(
            parse_filter_time("x", 0.0),
            Err(Some(
                "invalid literal for int() with base 10: 'x'".to_string()
            ))
        );
        assert_eq!(
            parse_filter_time("xd", 0.0),
            Err(Some("could not convert string to float: 'x'".to_string()))
        );
        assert_eq!(parse_filter_time("", 0.0), Err(None));
    }

    #[test]
    fn durations_never_panic() {
        assert_eq!(duration_from_secs(-1.0), std::time::Duration::ZERO);
        assert_eq!(duration_from_secs(f64::NAN), std::time::Duration::ZERO);
        assert_eq!(duration_from_secs(1.5).as_millis(), 1500);
        assert_eq!(duration_from_secs(f64::INFINITY).as_secs(), 1_000_000_000);
    }

    #[test]
    fn unknown_directives_are_copied() {
        assert_eq!(format_local(0, "%Q%"), "%Q%");
    }

    /// A `TZif` file of a fixed zone 5:30 east of UTC (`<+0530>-5:30`):
    /// the version 1 block, the same as version 2 block, the rule.
    fn tzif_plus_0530() -> Vec<u8> {
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(b"TZif2");
            data.extend_from_slice(&[0; 15]);
            // isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt
            for count in [0u32, 0, 0, 0, 1, 6] {
                data.extend_from_slice(&count.to_be_bytes());
            }
            data.extend_from_slice(&19_800_i32.to_be_bytes());
            data.extend_from_slice(&[0, 0]);
            data.extend_from_slice(b"+0530\0");
        }
        data.extend_from_slice(b"\n<+0530>-5:30\n");
        data
    }

    #[cfg(unix)]
    #[test]
    fn zone_files_are_found_like_glibc() {
        let dir = std::env::temp_dir().join(format!("mccli-tz-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("posix/Asia")).unwrap();
        std::fs::write(dir.join("posix/Asia/Kolkata"), tzif_plus_0530()).unwrap();
        std::fs::write(dir.join("broken"), b"not a zone").unwrap();
        let at = |tz: &str, tzdir: Option<&std::path::Path>| {
            zone_file(Some(tz), tzdir.map(std::path::Path::as_os_str))
                .map(|zone| crate::util::strftime::strftime(0, "%H:%M", &zone))
        };
        let some = Some(dir.as_path());
        assert_eq!(at("posix/Asia/Kolkata", some).as_deref(), Some("05:30"));
        assert_eq!(at(":./posix//Asia/Kolkata", some).as_deref(), Some("05:30"));
        let absolute = dir.join("posix/Asia/Kolkata");
        assert_eq!(
            at(absolute.to_str().unwrap(), None).as_deref(),
            Some("05:30")
        );
        // UTC for what is there but no regular file
        assert_eq!(at("posix", some).as_deref(), Some("00:00"));
        assert_eq!(at("/dev/zero", some).as_deref(), Some("00:00"));
        // left to jiff: no file, not a zone file, empty
        assert_eq!(at("posix/Asia/Delhi", some), None);
        assert_eq!(at("broken", some), None);
        assert_eq!(at(":", some), None);
        assert_eq!(at("", some), None);
        assert!(zone_file(None, None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
