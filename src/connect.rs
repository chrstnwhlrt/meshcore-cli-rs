//! Finding and connecting to a device (serial, TCP, BLE).
//!
//! BLE goes through `BlueZ` and exists on Linux with the `ble` feature only;
//! elsewhere the BLE options are accepted and ignored.

use meshcore::MeshCore;

use crate::output;
use crate::util::text::pad_right;

/// Whether this build can use BLE.
pub const BLE: bool = cfg!(all(feature = "ble", target_os = "linux"));

/// How to reach the device.
#[derive(Debug, Clone, Default)]
pub struct Target {
    /// BLE address or part of the name (`-a`, `-d` or `default_address`);
    /// empty for the first device found.
    pub address: String,
    /// TCP host (`-t`).
    pub hostname: Option<String>,
    /// TCP port (`-p`), checked when connecting.
    pub port: i64,
    /// Serial port (`-s`).
    pub serial_port: Option<String>,
    /// Baud rate (`-b`), checked when connecting like pyserial does.
    pub baudrate: i64,
    /// Forced DTR level.
    pub dtr: Option<bool>,
    /// Forced RTS level.
    pub rts: Option<bool>,
    /// Pair with the device, asking for its PIN (`-P`).
    pub pair: bool,
    /// Take the first device found when connecting fails (`-f`).
    pub first_device: bool,
    /// BLE scan time in seconds (`-T`).
    pub scan_timeout: f64,
}

/// A serial port with the description shown by `-l` and `-S`.
#[derive(Debug, Clone)]
pub struct PortInfo {
    /// Device path.
    pub port: String,
    /// Description as pyserial gives it, `n/a` when unknown.
    pub desc: String,
    /// Hardware id like pyserial prints it.
    pub hwid: String,
}

/// Lists serial ports the way pyserial's `comports()` does in the
/// reference, in the order of its `sorted()`: names compared by their runs
/// of text and numbers, so `ttyUSB2` comes before `ttyUSB10`.
#[must_use]
pub fn serial_ports() -> Vec<PortInfo> {
    let mut ports = comports();
    ports.sort_by_cached_key(|p| natural_key(&p.port));
    ports
}

/// A part of a name in pyserial's `numsplit`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NamePart {
    /// A run of other characters, compared as text.
    Text(String),
    /// A run of digits, compared as a number: its length without leading
    /// zeros, then its digits.
    Number(usize, Vec<u32>),
}

/// The sort key of pyserial's `numsplit`. Names of ports start with text,
/// so text and numbers alternate alike in all keys.
fn natural_key(name: &str) -> Vec<NamePart> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut digits: Vec<u32> = Vec::new();
    let number = |digits: &mut Vec<u32>| {
        let start = digits.iter().position(|&d| d != 0).unwrap_or(digits.len());
        let value = digits.split_off(start);
        digits.clear();
        NamePart::Number(value.len(), value)
    };
    for c in name.chars() {
        if let Some(digit) = crate::util::py::decimal(c) {
            if !text.is_empty() {
                parts.push(NamePart::Text(std::mem::take(&mut text)));
            }
            digits.push(digit);
        } else {
            if !digits.is_empty() {
                parts.push(number(&mut digits));
            }
            text.push(c);
        }
    }
    if !text.is_empty() {
        parts.push(NamePart::Text(text));
    }
    if !digits.is_empty() {
        parts.push(number(&mut digits));
    }
    parts
}

/// pyserial's `comports()` on Linux: the files in `/dev` matching its name
/// patterns (links among them too), described from sysfs, without
/// platform devices.
#[cfg(target_os = "linux")]
fn comports() -> Vec<PortInfo> {
    const PATTERNS: [&str; 7] = [
        "ttyS", "ttyUSB", "ttyXRUSB", "ttyACM", "ttyAMA", "rfcomm", "ttyAP",
    ];
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| PATTERNS.iter().any(|p| name.starts_with(p)))
        .filter_map(|name| sysfs_port(&name))
        .collect()
}

/// A path with its links resolved like Python's `os.path.realpath`; for a
/// dangling link the target as far as it goes.
#[cfg(target_os = "linux")]
fn realpath(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::{Component, PathBuf};

    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    let target = std::fs::read_link(path).unwrap_or_else(|_| path.to_path_buf());
    let joined = path
        .parent()
        .map_or(target.clone(), |dir| dir.join(&target));
    let mut resolved = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
            _ => {}
        }
    }
    resolved
}

/// A port described from sysfs like pyserial's `SysFS` class; `None` for a
/// platform device, which pyserial leaves out.
#[cfg(target_os = "linux")]
fn sysfs_port(name: &str) -> Option<PortInfo> {
    use std::path::Path;

    // the first line of a sysfs attribute (`f.readline().strip()` in
    // pyserial), None when it cannot be read
    let read = |dir: &Path, attr: &str| {
        std::fs::read_to_string(dir.join(attr)).ok().map(|s| {
            crate::util::py::lines(&s)
                .next()
                .map_or_else(String::new, |l| crate::util::py::strip(l).to_string())
        })
    };
    let port = format!("/dev/{name}");
    // a link keeps its own name, which sysfs does not know, and gets its
    // target added to the hardware id
    let link = std::fs::symlink_metadata(&port)
        .is_ok_and(|m| m.file_type().is_symlink())
        .then(|| realpath(Path::new(&port)).display().to_string());
    let mut desc = "n/a".to_string();
    let mut hwid = link
        .as_ref()
        .map_or_else(|| "n/a".to_string(), |target| format!("LINK={target}"));
    let device = std::fs::canonicalize(format!("/sys/class/tty/{name}/device")).ok();
    let subsystem = device.as_ref().map(|device| {
        realpath(&device.join("subsystem"))
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let file_name = |path: &Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match (device, subsystem.as_deref()) {
        (_, Some("platform")) => return None,
        (Some(device), Some(subsystem @ ("usb" | "usb-serial"))) => {
            let interface = if subsystem == "usb" {
                device
            } else {
                device.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            let usb = interface
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let interfaces = read(&usb, "bNumInterfaces")
                .and_then(|n| crate::util::py::int(&n, 10).ok())
                .unwrap_or(1);
            let id = |attr| {
                read(&usb, attr)
                    .and_then(|v| u16::from_str_radix(&v, 16).ok())
                    .unwrap_or(0)
            };
            // multi interface devices are told apart by their interface
            let location = if interfaces > 1 { &interface } else { &usb };
            let product = read(&usb, "product");
            desc = match (read(&interface, "interface"), product) {
                (Some(name_of_interface), product) => format!(
                    "{} - {name_of_interface}",
                    product.as_deref().unwrap_or("None")
                ),
                (None, Some(product)) => product,
                (None, None) => name.to_string(),
            };
            hwid = format!("USB VID:PID={:04X}:{:04X}", id("idVendor"), id("idProduct"));
            if let Some(serial) = read(&usb, "serial") {
                hwid.push_str(" SER=");
                hwid.push_str(&serial);
            }
            hwid.push_str(" LOCATION=");
            hwid.push_str(&file_name(location));
        }
        (Some(device), Some("pnp")) => {
            desc = name.to_string();
            hwid = read(&device, "id").unwrap_or_else(|| "None".to_string());
        }
        (Some(device), Some("amba")) => {
            desc = name.to_string();
            hwid = file_name(&device);
        }
        _ => {}
    }
    if let Some(target) = link {
        hwid.push_str(" LINK=");
        hwid.push_str(&target);
    }
    Some(PortInfo { port, desc, hwid })
}

/// The ports the serialport crate finds, described from what it knows.
#[cfg(not(target_os = "linux"))]
fn comports() -> Vec<PortInfo> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let (desc, hwid) = match &p.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    let mut hwid = format!("USB VID:PID={:04X}:{:04X}", usb.vid, usb.pid);
                    if let Some(serial) = &usb.serial_number {
                        hwid.push_str(" SER=");
                        hwid.push_str(serial);
                    }
                    let desc = usb.product.clone().unwrap_or_else(|| "n/a".to_string());
                    (desc, hwid)
                }
                _ => ("n/a".to_string(), "n/a".to_string()),
            };
            PortInfo {
                port: p.port_name,
                desc,
                hwid,
            }
        })
        .collect()
}

/// A `MeshCore` device found by a BLE scan.
#[derive(Debug, Clone)]
pub struct BleEntry {
    /// Address used to connect.
    pub address: String,
    /// Advertised name.
    pub name: String,
}

/// Scans for `MeshCore` BLE devices for `timeout` seconds.
#[cfg(all(feature = "ble", target_os = "linux"))]
async fn ble_scan(timeout: f64) -> Result<Vec<BleEntry>, meshcore::Error> {
    let devices =
        meshcore::transport::ble::scan(crate::util::time::duration_from_secs(timeout.max(0.1)))
            .await?;
    Ok(devices
        .into_iter()
        .map(|d| BleEntry {
            address: d.address,
            name: d.name,
        })
        .collect())
}

#[cfg(not(all(feature = "ble", target_os = "linux")))]
#[allow(clippy::unused_async)]
async fn ble_scan(_timeout: f64) -> Result<Vec<BleEntry>, meshcore::Error> {
    Ok(Vec::new())
}

/// `-l`: prints BLE devices and serial ports.
pub async fn list_devices(timeout: f64, repeater_mode: bool) {
    if BLE && !repeater_mode {
        output::println("BLE devices:");
        match ble_scan(timeout).await {
            Ok(devices) if devices.is_empty() => output::println(" No ble device found"),
            Ok(devices) => {
                for d in devices {
                    output::println(&format!(" {}  {}", d.address, d.name));
                }
            }
            Err(e) => {
                log::debug!("BLE scan: {e}");
                output::println(" No BLE HW");
            }
        }
        output::println("");
    }
    output::println("Serial ports:");
    for p in serial_ports() {
        let port = pad_right(&p.port, 18);
        output::println(&format!(" {port} {} [{}]", p.desc, p.hwid));
    }
}

/// Choice made with `-S`.
#[derive(Debug, Clone)]
pub enum Selection {
    /// A BLE device address.
    Ble(String),
    /// A serial port.
    Serial(String),
}

/// `-S`: lets the user choose a device.
pub async fn select_device(timeout: f64, repeater_mode: bool) -> Option<Selection> {
    let mut choices: Vec<(Selection, String)> = Vec::new();
    if BLE && !repeater_mode {
        match ble_scan(timeout).await {
            Ok(devices) => {
                for d in devices {
                    choices.push((
                        Selection::Ble(d.address.clone()),
                        format!("{} {}", pad_right(&d.address, 22), d.name),
                    ));
                }
            }
            Err(e) => {
                log::debug!("BLE scan: {e}");
                log::info!("No BLE Device");
            }
        }
    }
    for p in serial_ports() {
        choices.push((
            Selection::Serial(p.port.clone()),
            format!("{} {}", pad_right(&p.port, 22), p.desc),
        ));
    }
    if choices.is_empty() {
        log::error!("No device found, exiting");
        return None;
    }
    output::println("MeshCore-cli device selector");
    output::println("Choose the device to connect to :");
    for (i, (_, label)) in choices.iter().enumerate() {
        output::println(&format!(" {:>2}) {label}", i + 1));
    }
    output::print("Number (Enter to cancel): ");
    let mut line = String::new();
    let read =
        tokio::task::spawn_blocking(move || std::io::stdin().read_line(&mut line).map(|_| line))
            .await;
    let choice = read
        .ok()
        .and_then(Result::ok)
        .and_then(|l| l.trim().parse::<usize>().ok())
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| choices.get(i).map(|(s, _)| s.clone()));
    if choice.is_none() {
        log::info!("No choice made, exiting");
    }
    choice
}

/// Connects over TCP, serial or BLE (in this order of preference).
pub async fn connect(target: &Target) -> Option<MeshCore> {
    if let Some(host) = &target.hostname {
        let Ok(port) = u16::try_from(target.port) else {
            // the OverflowError of Python's socket, which the reference prints
            output::println("Error: connect(): port must be 0-65535.");
            return None;
        };
        return connect_tcp(host, port).await;
    }
    if let Some(port) = &target.serial_port {
        let Some(baud_rate) = serial_baudrate(target.baudrate) else {
            // pyserial's ValueError, which the reference prints and ends with
            let baudrate = target.baudrate;
            output::println(&format!("Error: Not a valid baudrate: {baudrate}"));
            return None;
        };
        let config = meshcore::SerialConfig {
            baud_rate,
            dtr: target.dtr,
            rts: target.rts,
            ..meshcore::SerialConfig::new(port.clone())
        };
        return match MeshCore::create_serial(config, meshcore::Options::default()).await {
            Ok(mc) => Some(mc),
            // the port opened but the node did not answer like a companion
            Err(
                meshcore::Error::Timeout
                | meshcore::Error::Device(_)
                | meshcore::Error::Protocol(_)
                | meshcore::Error::ConnectionLost,
            ) => {
                log::error!("To connect to a repeater, use -r option.");
                None
            }
            // pyserial's exception, which the reference prints and ends with
            Err(e) => {
                let message = pyserial_open_error(port).unwrap_or_else(|| e.to_string());
                output::println(&format!("Error: {message}"));
                None
            }
        };
    }
    connect_ble(target).await
}

/// Connects over TCP like asyncio's `create_connection` in the reference:
/// the host is looked up and its addresses are tried in turn, and a failure
/// is printed in Python's words. A node that does not answer once the
/// connection stands is given up at once, as `meshcore_py` does.
async fn connect_tcp(host: &str, port: u16) -> Option<MeshCore> {
    let addresses: Vec<std::net::SocketAddr> =
        match tokio::net::lookup_host((host.to_string(), port)).await {
            Ok(addresses) => addresses.collect(),
            Err(e) => {
                output::println(&format!("Error: {}", lookup_error(&e)));
                return None;
            }
        };
    let mut errors = Vec::new();
    for address in &addresses {
        let transport = meshcore::TcpTransport::new(address.ip().to_string(), address.port());
        let client = MeshCore::new(transport, meshcore::Options::default());
        match client.connect().await {
            Ok(_) => return Some(client),
            Err(_) if client.connection_info().is_some() => {
                log::error!("No response from meshcore node, disconnecting");
                return None;
            }
            Err(e) => errors.push(connect_error(address, &e)),
        }
    }
    // asyncio's summary of the failed addresses
    let message = match errors.as_slice() {
        [] => "getaddrinfo() returned empty list".to_string(),
        [first, rest @ ..] if rest.iter().all(|e| e == first) => first.clone(),
        all => format!("Multiple exceptions: {}", all.join(", ")),
    };
    output::println(&format!("Error: {message}"));
    None
}

/// The `OSError` asyncio raises when connecting to one address fails:
/// `[Errno 111] Connect call failed ('127.0.0.1', 5000)`. A connection
/// that takes too long fails here where the system would end it later
/// with `ETIMEDOUT`.
fn connect_error(address: &std::net::SocketAddr, error: &meshcore::Error) -> String {
    let errno = match error {
        meshcore::Error::Io(e) => e.raw_os_error(),
        meshcore::Error::Timeout => Some(110),
        _ => None,
    };
    let Some(errno) = errno else {
        return error.to_string();
    };
    let sockaddr = match address {
        std::net::SocketAddr::V4(v4) => format!("('{}', {})", v4.ip(), v4.port()),
        std::net::SocketAddr::V6(v6) => format!(
            "('{}', {}, {}, {})",
            v6.ip(),
            v6.port(),
            v6.flowinfo(),
            v6.scope_id()
        ),
    };
    format!("[Errno {errno}] Connect call failed {sockaddr}")
}

/// The `socket.gaierror` of a failed lookup: `[Errno -2] Name or service
/// not known`, with the error codes of glibc.
fn lookup_error(error: &std::io::Error) -> String {
    const CODES: [(i32, &str); 12] = [
        (-1, "Bad value for ai_flags"),
        (-2, "Name or service not known"),
        (-3, "Temporary failure in name resolution"),
        (-4, "Non-recoverable failure in name resolution"),
        (-5, "No address associated with hostname"),
        (-6, "ai_family not supported"),
        (-7, "ai_socktype not supported"),
        (-8, "Servname not supported for ai_socktype"),
        (-9, "Address family for hostname not supported"),
        (-10, "Memory allocation failure"),
        (-11, "System error"),
        (-12, "Argument buffer overflow"),
    ];
    if error.raw_os_error().is_some() {
        return crate::util::py::os_error(error, None);
    }
    let text = error.to_string();
    let detail = text
        .strip_prefix("failed to lookup address information: ")
        .unwrap_or(&text);
    CODES
        .iter()
        .find(|(_, message)| *message == detail)
        .map_or_else(
            || detail.to_string(),
            |(code, message)| format!("[Errno {code}] {message}"),
        )
}

/// Why a serial port cannot be opened, in the words of pyserial's
/// exception, which the reference prints: the error of `os.open` (also for
/// a missing permission), or the one of configuring a file that is no
/// terminal. The serialport crate does not tell the error number, so the
/// port is opened once more the way pyserial does it to find out; `None`
/// when that gives no answer either.
#[cfg(unix)]
#[must_use]
pub fn pyserial_open_error(port_name: &str) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let opened = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(port_name);
    match opened {
        Err(e) => e.raw_os_error().map(|errno| {
            format!(
                "[Errno {errno}] could not open port {port_name}: {}",
                crate::util::py::os_error(&e, Some(port_name))
            )
        }),
        // the termios.error of tcgetattr
        Ok(file) if !std::io::IsTerminal::is_terminal(&file) => Some(format!(
            "Could not configure port: ({}, {})",
            libc::ENOTTY,
            crate::util::json::py_repr_str(&crate::util::py::strerror(libc::ENOTTY))
        )),
        Ok(_) => None,
    }
}

#[cfg(not(unix))]
#[must_use]
pub fn pyserial_open_error(_port_name: &str) -> Option<String> {
    None
}

/// A baud rate a serial port can be opened with: pyserial refuses negative
/// ones, and a port takes no rate beyond 32 bits.
#[must_use]
pub fn serial_baudrate(baudrate: i64) -> Option<u32> {
    u32::try_from(baudrate).ok()
}

#[cfg(not(all(feature = "ble", target_os = "linux")))]
#[allow(clippy::unused_async)]
async fn connect_ble(_target: &Target) -> Option<MeshCore> {
    output::println("No device given: use -s <serial port> or -t <host> (no BLE in this build)");
    output::print(&crate::help::command_usage());
    None
}

/// The message of the reference when BLE cannot be used at all.
#[cfg(all(feature = "ble", target_os = "linux"))]
fn no_ble_hardware() {
    output::println("BLE connection asked (default behaviour), but no BLE HW found");
    output::println(&format!(
        "Call {} with -h for some more help (on commands)",
        crate::help::PROGRAM
    ));
    output::print(&crate::help::command_usage());
}

/// A full Bluetooth address (`AA:BB:CC:DD:EE:FF`) needs no scan.
#[cfg(all(feature = "ble", target_os = "linux"))]
fn is_full_address(address: &str) -> bool {
    address.split(':').count() == 6
}

/// Finds the device to connect to like the reference: the first `MeshCore`
/// device whose name contains `address` (any for an empty one), or the
/// device with exactly this address.
#[cfg(all(feature = "ble", target_os = "linux"))]
fn pick_device(devices: &[BleEntry], address: &str) -> Option<BleEntry> {
    devices
        .iter()
        .find(|d| {
            (d.name.starts_with("MeshCore-") && d.name.contains(address)) || d.address == address
        })
        .cloned()
}

/// Scans for the device; `Err(())` when BLE is unusable (message printed).
#[cfg(all(feature = "ble", target_os = "linux"))]
async fn scan_for(address: &str, timeout: f64) -> Result<Option<BleEntry>, ()> {
    if address.is_empty() {
        log::info!("Searching first MC BLE device");
    } else {
        log::info!("Scanning BLE for device matching {address}");
    }
    let devices = match ble_scan(timeout).await {
        Ok(devices) => devices,
        Err(e) => {
            log::debug!("BLE scan: {e}");
            no_ble_hardware();
            return Err(());
        }
    };
    let found = pick_device(&devices, address);
    match &found {
        Some(d) => log::info!("Found device {} {}", d.name, d.address),
        None => log::info!("Couldn't find device {address}"),
    }
    Ok(found)
}

/// Asks for the PIN to pair with (`-P`).
#[cfg(all(feature = "ble", target_os = "linux"))]
async fn ask_pin() -> Option<u32> {
    let read = tokio::task::spawn_blocking(|| crate::editor::read_password("BLE PIN: ")).await;
    let Ok(Ok(text)) = read else {
        log::info!("Canceled");
        return None;
    };
    let pin = text.trim().parse::<u32>().ok();
    if pin.is_none() {
        log::error!("The PIN has to be a number");
    }
    pin
}

/// Reports a failure that connecting again would not change: another
/// program on this computer uses the device, pairing failed (wrong PIN, or
/// the device needs its PIN and none was given with `-P`), or a setting is
/// invalid (a PIN of more than six digits).
#[cfg(all(feature = "ble", target_os = "linux"))]
fn give_up(error: &meshcore::Error, pin_given: bool) -> Option<MeshCore> {
    log::error!("Can't connect to node, {error}");
    if matches!(error, meshcore::Error::PairingFailed(_)) && !pin_given {
        log::error!("To pair with the device, use -P option.");
    }
    None
}

#[cfg(all(feature = "ble", target_os = "linux"))]
async fn connect_ble(target: &Target) -> Option<MeshCore> {
    // like the reference: a full address is used as is, anything else is
    // looked up with a scan first
    let mut address = target.address.clone();
    let mut scanned = false;
    if !is_full_address(&address) {
        let device = scan_for(&address, target.scan_timeout).await.ok()??;
        address = device.address;
        scanned = true;
    }
    // asked once the device is known, so it is not typed in vain
    let pin = if target.pair {
        Some(ask_pin().await?)
    } else {
        None
    };
    // -T only applies to the scans above; an address BlueZ does not know
    // yet is searched for as long as the library's default (10 s), as
    // bleak does when the reference connects to an address
    let connect = |address: String| async move {
        let config = meshcore::BleConfig::new(address).pin(pin);
        MeshCore::create_ble(config, meshcore::Options::default()).await
    };
    let mc = match connect(address.clone()).await {
        Ok(mc) => mc,
        Err(
            e @ (meshcore::Error::DeviceInUse(_)
            | meshcore::Error::PairingFailed(_)
            | meshcore::Error::InvalidArgument(_)),
        ) => {
            return give_up(&e, pin.is_some());
        }
        Err(e) => {
            log::info!("Error while connecting, retrying once ...");
            log::debug!("BLE connection: {e}");
            if !scanned {
                // a full address that failed: look for the device again
                let wanted = if target.first_device {
                    String::new()
                } else {
                    address.clone()
                };
                let device = scan_for(&wanted, target.scan_timeout).await.ok()??;
                address = device.address;
            }
            match connect(address.clone()).await {
                Ok(mc) => mc,
                Err(
                    e @ (meshcore::Error::DeviceInUse(_)
                    | meshcore::Error::PairingFailed(_)
                    | meshcore::Error::InvalidArgument(_)),
                ) => {
                    return give_up(&e, pin.is_some());
                }
                Err(e) => {
                    log::debug!("BLE connection: {e}");
                    log::error!("Can't connect to node, exiting");
                    return None;
                }
            }
        }
    };
    if crate::paths::existing_config_dir().is_some() {
        let _ = std::fs::write(crate::paths::address_file(), &address);
    }
    Some(mc)
}

/// Reads the stored BLE address (`~/.config/meshcore/default_address`).
#[must_use]
pub fn default_address() -> String {
    std::fs::read_to_string(crate::paths::address_file())
        .ok()
        .and_then(|c| {
            crate::util::py::lines(&c)
                .next()
                .map(|l| crate::util::py::strip(l).to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[cfg(all(feature = "ble", target_os = "linux"))]
mod tests {
    use super::*;

    fn entry(address: &str, name: &str) -> BleEntry {
        BleEntry {
            address: address.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn picks_devices_like_the_reference() {
        let devices = vec![
            entry("11:22:33:44:55:66", "MeshCore-Alpha"),
            entry("AA:BB:CC:DD:EE:FF", "MeshCore-Beta"),
        ];
        assert_eq!(pick_device(&devices, "").unwrap().name, "MeshCore-Alpha");
        assert_eq!(pick_device(&devices, "Beta").unwrap().name, "MeshCore-Beta");
        assert_eq!(
            pick_device(&devices, "AA:BB:CC:DD:EE:FF").unwrap().name,
            "MeshCore-Beta"
        );
        assert!(pick_device(&devices, "Gamma").is_none());
        assert!(is_full_address("AA:BB:CC:DD:EE:FF"));
        assert!(!is_full_address("Beta"));
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[test]
    fn sorts_ports_like_pyserial() {
        let mut names = vec![
            "/dev/ttyUSB10",
            "/dev/ttyS1",
            "/dev/ttyUSB2",
            "/dev/ttyACM0",
            "/dev/ttyUSB",
            "/dev/ttyUSB02x",
            "/dev/ttyUSB2a",
            "/dev/rfcomm0",
        ];
        names.sort_by_cached_key(|n| natural_key(n));
        assert_eq!(
            names,
            vec![
                "/dev/rfcomm0",
                "/dev/ttyACM0",
                "/dev/ttyS1",
                "/dev/ttyUSB",
                "/dev/ttyUSB2",
                "/dev/ttyUSB2a",
                "/dev/ttyUSB02x",
                "/dev/ttyUSB10",
            ]
        );
    }
}
