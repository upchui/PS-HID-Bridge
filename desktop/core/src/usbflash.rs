//! Flashing the board over USB: with BOOTSEL held while plugging in, the
//! RP2350 shows up as a drive labelled "RP2350". A UF2 file copied onto it is
//! written to flash by the boot ROM itself (it knows the partitions), which
//! then restarts the board. The drive is found and mounted through UDisks2
//! (D-Bus), as a file manager would do it: no root, no udev rule.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
/// Volume label of the RP2350 boot drive.
const LABEL: &str = "RP2350";

/// How long to wait for the board to show up as a drive.
pub const WAIT_FOR_BOARD: Duration = Duration::from_secs(120);
/// How long the board may take to process a file and restart.
const WAIT_FOR_RESTART: Duration = Duration::from_secs(30);

/// What to put on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    /// Partition table, then the firmware (first installation).
    FirstInstall,
    Firmware,
    HidTest,
    HidTestMouse8,
    /// Erase the whole flash.
    Erase,
}

impl Image {
    pub const ALL: [Image; 5] = [
        Image::FirstInstall,
        Image::Firmware,
        Image::HidTest,
        Image::HidTestMouse8,
        Image::Erase,
    ];

    /// Name on the command line and in the app.
    pub fn id(self) -> &'static str {
        match self {
            Image::FirstInstall => "first-install",
            Image::Firmware => "firmware",
            Image::HidTest => "hidtest",
            Image::HidTestMouse8 => "hidtest-mouse8",
            Image::Erase => "erase",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.id() == id)
    }

    pub fn description(self) -> &'static str {
        match self {
            Image::FirstInstall => "First installation: partition table, then the firmware",
            Image::Firmware => "Firmware only (the partition table is on the board already)",
            Image::HidTest => "HID test: no network, the mouse moves in a square",
            Image::HidTestMouse8 => "HID test with 8-bit mouse axes",
            Image::Erase => "Erase the whole flash (settings and firmware)",
        }
    }

    /// What to do once it is on the board.
    pub fn done_message(self) -> &'static str {
        match self {
            Image::FirstInstall | Image::Firmware => {
                "The firmware is installed. Connect the board to the PlayStation (USB) and to the network; \
                 your router shows it as ps-hid-bridge-…"
            }
            Image::HidTest | Image::HidTestMouse8 => {
                "The test image runs. Connect the board to the PlayStation: 5 s later the mouse moves in a square."
            }
            Image::Erase => {
                "The flash is erased. To use the board again, install with \"first installation\"."
            }
        }
    }

    /// Release files, in the order they are written.
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Image::FirstInstall => &["ps-hid-bridge-pt.uf2", "ps-hid-bridge.uf2"],
            Image::Firmware => &["ps-hid-bridge.uf2"],
            Image::HidTest => &["ps-hid-bridge-hidtest.uf2"],
            Image::HidTestMouse8 => &["ps-hid-bridge-hidtest-mouse8.uf2"],
            Image::Erase => &["ps-hid-bridge-nuke.uf2"],
        }
    }
}

/// The files to write for `image`: from the newest release (checked against
/// its SHA256SUMS), or with `own` (name, contents) instead of the last one,
/// e.g. a firmware of your own after the release's partition table.
pub fn files_for(
    image: Image,
    own: Option<(String, Vec<u8>)>,
    progress: &mut dyn FnMut(&str),
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let names = image.files();
    let from_release = if own.is_some() {
        &names[..names.len() - 1]
    } else {
        names
    };
    let mut files = Vec::new();
    if !from_release.is_empty() {
        progress("Looking for the newest release …");
        let release = crate::update::latest_release()?;
        for name in from_release {
            progress(&format!("Downloading {name} ({}) …", release.version));
            let data = crate::update::download_verified(&release, name, &mut |_, _| {})?;
            files.push((name.to_string(), data));
        }
    }
    if let Some((name, data)) = own {
        check_uf2(&data).map_err(|e| format!("{name}: {e}"))?;
        files.push((name, data));
    }
    Ok(files)
}

// ---------------------------------------------------------------- UF2

const UF2_BLOCK: usize = 512;
const UF2_MAGIC_START0: u32 = 0x0A32_4655;
const UF2_MAGIC_START1: u32 = 0x9E5D_5157;
const UF2_MAGIC_END: u32 = 0x0AB1_6F30;
const UF2_FLAG_FAMILY: u32 = 0x0000_2000;
/// ABSOLUTE (partition table), DATA, RP2350 ARM secure, RISC-V, ARM non-secure.
const RP2350_FAMILIES: std::ops::RangeInclusive<u32> = 0xE48B_FF57..=0xE48B_FF5B;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A UF2 file for the RP2350 (every block valid, with an RP2350 family).
pub fn check_uf2(data: &[u8]) -> Result<(), String> {
    if data.is_empty() || !data.len().is_multiple_of(UF2_BLOCK) {
        return Err("This is not a UF2 file.".into());
    }
    for (i, b) in data.chunks(UF2_BLOCK).enumerate() {
        if u32_at(b, 0) != UF2_MAGIC_START0
            || u32_at(b, 4) != UF2_MAGIC_START1
            || u32_at(b, 508) != UF2_MAGIC_END
        {
            return Err(format!("This is not a UF2 file (block {i})."));
        }
        let flags = u32_at(b, 8);
        if flags & UF2_FLAG_FAMILY == 0 || !RP2350_FAMILIES.contains(&u32_at(b, 28)) {
            return Err(format!("The UF2 file is not for the RP2350 (block {i})."));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- drive

/// The RP2350 boot drive as UDisks2 sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct BootDrive {
    object: OwnedObjectPath,
    /// e.g. /dev/sdb1
    pub device: String,
    pub mount: Option<PathBuf>,
}

/// Byte strings from UDisks end with a NUL.
fn bytes_to_string(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn prop<T: TryFrom<OwnedValue>>(props: &HashMap<String, OwnedValue>, name: &str) -> Option<T> {
    props
        .get(name)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| T::try_from(v).ok())
}

fn system_bus() -> Result<Connection, String> {
    Connection::system().map_err(|e| format!("System D-Bus: {e}"))
}

/// The boot drive, if one is connected (None also without UDisks).
pub fn find_drive() -> Result<Option<BootDrive>, String> {
    let conn = system_bus()?;
    let om = ObjectManagerProxy::builder(&conn)
        .destination(UDISKS)
        .and_then(|b| b.path("/org/freedesktop/UDisks2"))
        .and_then(|b| b.build())
        .map_err(|e| format!("UDisks2: {e}"))?;
    let objects = om
        .get_managed_objects()
        .map_err(|e| format!("UDisks2 (is udisks2 installed and running?): {e}"))?;
    for (path, ifaces) in objects {
        let (mut block, mut fs) = (None, None);
        for (name, props) in &ifaces {
            match name.as_str() {
                BLOCK => block = Some(props),
                FILESYSTEM => fs = Some(props),
                _ => {}
            }
        }
        let (Some(block), Some(fs)) = (block, fs) else {
            continue;
        };
        if prop::<String>(block, "IdLabel").as_deref() != Some(LABEL) {
            continue;
        }
        let device = prop::<Vec<u8>>(block, "Device")
            .map(|d| bytes_to_string(&d))
            .unwrap_or_default();
        let mount = prop::<Vec<Vec<u8>>>(fs, "MountPoints")
            .and_then(|m| m.first().map(|p| PathBuf::from(bytes_to_string(p))));
        return Ok(Some(BootDrive {
            object: path,
            device,
            mount,
        }));
    }
    Ok(None)
}

/// Mount the drive (if it is not mounted yet) and return the folder.
pub fn mount(drive: &BootDrive) -> Result<PathBuf, String> {
    if let Some(m) = &drive.mount {
        return Ok(m.clone());
    }
    let conn = system_bus()?;
    let proxy = zbus::blocking::Proxy::new(&conn, UDISKS, drive.object.as_str(), FILESYSTEM)
        .map_err(|e| format!("UDisks2: {e}"))?;
    let options: HashMap<&str, Value> = HashMap::new();
    let path: String = proxy
        .call("Mount", &(options,))
        .map_err(|e| format!("Mounting {}: {e}", drive.device))?;
    Ok(PathBuf::from(path))
}

/// INFO_UF2.TXT of an RP2350 boot drive names the chip.
pub fn is_rp2350_info(text: &str) -> bool {
    text.lines()
        .any(|l| (l.starts_with("Board-ID:") || l.starts_with("Model:")) && l.contains("RP2350"))
}

fn check_drive(dir: &Path) -> Result<(), String> {
    let info = fs::read_to_string(dir.join("INFO_UF2.TXT")).unwrap_or_default();
    if is_rp2350_info(&info) {
        Ok(())
    } else {
        Err(format!("{} is not an RP2350 boot drive.", dir.display()))
    }
}

/// Copy a UF2 file onto the mounted drive. The board often restarts before
/// the file system is finished, so an error after all data was written is
/// expected; whether it worked shows when the drive disappears.
pub fn write_uf2(dir: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    let mut f = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(data)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = f.sync_all();
    Ok(())
}

/// Stopped from outside (the app's cancel button).
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled.".into())
    } else {
        Ok(())
    }
}

/// Wait until the boot drive is there (or `timeout` passed).
pub fn wait_present(timeout: Duration, cancel: &AtomicBool) -> Result<BootDrive, String> {
    let end = Instant::now() + timeout;
    loop {
        cancelled(cancel)?;
        if let Some(d) = find_drive()? {
            return Ok(d);
        }
        if Instant::now() > end {
            return Err(
                "The board did not show up as a USB drive (hold BOOTSEL while plugging it in).".into(),
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_gone(timeout: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let end = Instant::now() + timeout;
    loop {
        cancelled(cancel)?;
        if find_drive()?.is_none() {
            return Ok(());
        }
        if Instant::now() > end {
            return Err("The board did not take the file (it is still a drive).".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Write `files` (name, contents) one after the other: wait for the drive,
/// mount it, check it, copy, and wait until the board restarted.
pub fn flash(
    files: &[(String, Vec<u8>)],
    progress: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<(), String> {
    for (i, (name, data)) in files.iter().enumerate() {
        check_uf2(data).map_err(|e| format!("{name}: {e}"))?;
        let drive = if i == 0 {
            progress("Waiting for the board: hold BOOTSEL and connect it to this PC via USB …");
            wait_present(WAIT_FOR_BOARD, cancel)?
        } else {
            progress("Waiting for the board to come back as a drive …");
            wait_present(WAIT_FOR_RESTART, cancel)?
        };
        let dir = mount(&drive)?;
        check_drive(&dir)?;
        cancelled(cancel)?;
        progress(&format!("Writing {name} ({} KiB) …", data.len() / 1024));
        write_uf2(&dir, name, data)?;
        progress("Waiting for the board to restart …");
        wait_gone(WAIT_FOR_RESTART, cancel)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(family: u32) -> Vec<u8> {
        let mut b = vec![0u8; UF2_BLOCK];
        b[0..4].copy_from_slice(&UF2_MAGIC_START0.to_le_bytes());
        b[4..8].copy_from_slice(&UF2_MAGIC_START1.to_le_bytes());
        b[8..12].copy_from_slice(&UF2_FLAG_FAMILY.to_le_bytes());
        b[28..32].copy_from_slice(&family.to_le_bytes());
        b[508..512].copy_from_slice(&UF2_MAGIC_END.to_le_bytes());
        b
    }

    #[test]
    fn uf2_files_are_checked() {
        let mut ok = block(0xE48B_FF57); // partition table (absolute)
        ok.extend(block(0xE48B_FF59)); // RP2350 ARM
        assert!(check_uf2(&ok).is_ok());
        assert!(check_uf2(&[]).is_err());
        assert!(check_uf2(&ok[..700]).is_err());
        assert!(check_uf2(&block(0xE48B_FF56)).unwrap_err().contains("RP2350")); // RP2040
        let mut bad = block(0xE48B_FF59);
        bad[508] ^= 1;
        assert!(check_uf2(&bad).is_err());
        let mut no_family = block(0xE48B_FF59);
        no_family[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(check_uf2(&no_family).is_err());
    }

    #[test]
    fn boot_drive_info() {
        assert!(is_rp2350_info(
            "UF2 Bootloader v1.0\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n"
        ));
        assert!(!is_rp2350_info(
            "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n"
        ));
        assert!(!is_rp2350_info(""));
    }

    #[test]
    fn images_and_files() {
        assert_eq!(
            Image::FirstInstall.files(),
            ["ps-hid-bridge-pt.uf2", "ps-hid-bridge.uf2"]
        );
        for i in Image::ALL {
            assert_eq!(Image::from_id(i.id()), Some(i));
            assert!(i.files().iter().all(|f| f.ends_with(".uf2")));
        }
        assert_eq!(Image::from_id("nope"), None);
    }

    #[test]
    fn udisks_byte_strings() {
        assert_eq!(bytes_to_string(b"/run/media/me/RP2350\0"), "/run/media/me/RP2350");
        assert_eq!(bytes_to_string(b"/dev/sdb1"), "/dev/sdb1");
    }

    #[test]
    fn file_is_written() {
        let dir = std::env::temp_dir().join(format!("pshb-usbflash-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_uf2(&dir, "x.uf2", b"data").unwrap();
        assert_eq!(fs::read(dir.join("x.uf2")).unwrap(), b"data");
        assert!(check_drive(&dir).is_err());
        fs::write(dir.join("INFO_UF2.TXT"), "Board-ID: RP2350\n").unwrap();
        assert!(check_drive(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
