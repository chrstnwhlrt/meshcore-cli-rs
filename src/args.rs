//! Command line arguments, parsed like Python's `getopt.getopt`.
//!
//! The reference uses `getopt.getopt(argv, "a:d:s:ht:p:b:fjDhvSlT:Pc:Crqi")`:
//! short options may be combined (`-jD`), an option argument is either the
//! rest of the word (`-s/dev/ttyUSB0`) or the next word, and parsing stops at
//! the first word that is not an option. Everything after that is the list
//! of commands, which keep their own dash options (`msg -a Bob hi`).

use std::fmt;

/// Option letters that take an argument.
const WITH_ARGUMENT: &[char] = &['a', 'd', 's', 't', 'p', 'b', 'T', 'c'];
/// Option letters without argument.
const FLAGS: &[char] = &['h', 'f', 'j', 'D', 'v', 'S', 'l', 'P', 'C', 'r', 'q', 'i'];

/// A parsing error, worded like Python's `GetoptError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetoptError(pub String);

impl fmt::Display for GetoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GetoptError {}

/// Parsed options (letter and argument, in order) and remaining arguments.
pub type Parsed = (Vec<(char, String)>, Vec<String>);

/// Parses `argv` into options (in order) and the remaining arguments.
///
/// # Errors
///
/// Returns an error for unknown options or a missing option argument.
pub fn getopt(argv: &[String]) -> Result<Parsed, GetoptError> {
    let mut opts = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        if arg.starts_with("--") {
            return Err(GetoptError(format!("option {arg} not recognized")));
        }
        let letters: Vec<char> = arg[1..].chars().collect();
        let mut j = 0;
        while j < letters.len() {
            let letter = letters[j];
            if WITH_ARGUMENT.contains(&letter) {
                let rest: String = letters[j + 1..].iter().collect();
                let value = if rest.is_empty() {
                    i += 1;
                    argv.get(i)
                        .cloned()
                        .ok_or_else(|| GetoptError(format!("option -{letter} requires argument")))?
                } else {
                    rest
                };
                opts.push((letter, value));
                break;
            } else if FLAGS.contains(&letter) {
                opts.push((letter, String::new()));
                j += 1;
            } else {
                return Err(GetoptError(format!("option -{letter} not recognized")));
            }
        }
        i += 1;
    }
    Ok((opts, argv[i.min(argv.len())..].to_vec()))
}

/// Serial port options given as `-s <port>:rts=..,dtr=..,baud=..`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SerialSpec {
    /// Device path.
    pub port: String,
    /// Forced DTR level.
    pub dtr: Option<bool>,
    /// Forced RTS level.
    pub rts: Option<bool>,
    /// Baud rate override, any whole number like the reference's `int()`;
    /// one no serial port takes fails when connecting.
    pub baudrate: Option<i64>,
    /// Problems found while parsing, reported by the caller.
    pub errors: Vec<String>,
}

/// Parses the argument of `-s`.
#[must_use]
pub fn parse_serial_spec(arg: &str) -> SerialSpec {
    let Some((port, options)) = arg.split_once(':') else {
        return SerialSpec {
            port: arg.to_string(),
            ..SerialSpec::default()
        };
    };
    let mut spec = SerialSpec {
        port: port.to_string(),
        ..SerialSpec::default()
    };
    for opt in options.split(',') {
        // opt.split('=')[1] in the reference: the text up to a second '='
        let mut parts = opt.split('=');
        let name = crate::util::py::strip(&parts.next().unwrap_or("").to_lowercase()).to_string();
        let value = parts.next();
        let flag = |v: &str| matches!(v.to_lowercase().as_str(), "true" | "1" | "on");
        match (name.as_str(), value) {
            ("dtr", Some(v)) => spec.dtr = Some(flag(v)),
            ("rts", Some(v)) => spec.rts = Some(flag(v)),
            ("baudrate" | "baud" | "b", Some(v)) => match crate::util::py::int(v, 10) {
                Ok(b) => spec.baudrate = Some(b),
                Err(e) => spec.errors.push(format!("Bad option format {opt}: {e}")),
            },
            ("dtr" | "rts" | "baudrate" | "baud" | "b", None) => {
                spec.errors
                    .push(format!("Bad option format {opt}: list index out of range"));
            }
            _ => spec
                .errors
                .push(format!("Unrecognized serial option {opt}")),
        }
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn combined_flags_and_arguments() {
        let (opts, rest) = getopt(&v(&["-jD", "-s/dev/ttyUSB0", "-b", "9600", "infos"])).unwrap();
        assert_eq!(
            opts,
            vec![
                ('j', String::new()),
                ('D', String::new()),
                ('s', "/dev/ttyUSB0".to_string()),
                ('b', "9600".to_string())
            ]
        );
        assert_eq!(rest, v(&["infos"]));
    }

    #[test]
    fn stops_at_first_command() {
        let (opts, rest) = getopt(&v(&["-s", "x", "msg", "-a", "Bob", "hi"])).unwrap();
        assert_eq!(opts, vec![('s', "x".to_string())]);
        assert_eq!(rest, v(&["msg", "-a", "Bob", "hi"]));
    }

    #[test]
    fn flag_followed_by_argument_option() {
        let (opts, rest) = getopt(&v(&["-js", "/dev/ttyACM0"])).unwrap();
        assert_eq!(
            opts,
            vec![('j', String::new()), ('s', "/dev/ttyACM0".to_string())]
        );
        assert_eq!(rest, Vec::<String>::new());
    }

    #[test]
    fn double_dash_ends_options() {
        let (opts, rest) = getopt(&v(&["-i", "--", "-x"])).unwrap();
        assert_eq!(opts, vec![('i', String::new())]);
        assert_eq!(rest, v(&["-x"]));
    }

    #[test]
    fn errors() {
        assert_eq!(
            getopt(&v(&["-x"])).unwrap_err().0,
            "option -x not recognized"
        );
        assert_eq!(
            getopt(&v(&["-s"])).unwrap_err().0,
            "option -s requires argument"
        );
        assert_eq!(
            getopt(&v(&["--long"])).unwrap_err().0,
            "option --long not recognized"
        );
    }

    #[test]
    fn serial_options() {
        let spec = parse_serial_spec("/dev/ttyUSB0:dtr=off,rts=1,baud=57600");
        assert_eq!(spec.port, "/dev/ttyUSB0");
        assert_eq!(spec.dtr, Some(false));
        assert_eq!(spec.rts, Some(true));
        assert_eq!(spec.baudrate, Some(57600));
        assert_eq!(spec.errors, Vec::<String>::new());

        let spec = parse_serial_spec("/dev/x:foo=1");
        assert_eq!(
            spec.errors,
            vec!["Unrecognized serial option foo=1".to_string()]
        );

        assert_eq!(parse_serial_spec("COM3").port, "COM3");

        // messages of the reference's ValueError and IndexError
        assert_eq!(
            parse_serial_spec("/dev/x:baud=fast").errors,
            vec!["Bad option format baud=fast: invalid literal for int() with base 10: 'fast'"]
        );
        assert_eq!(
            parse_serial_spec("/dev/x:dtr").errors,
            vec!["Bad option format dtr: list index out of range"]
        );
        assert_eq!(parse_serial_spec("/dev/x:dtr=1=2").dtr, Some(true));
        // any whole number, like int(); the port refuses it when connecting
        let spec = parse_serial_spec("/dev/x:baud=-5");
        assert_eq!((spec.baudrate, spec.errors.len()), (Some(-5), 0));
    }
}
