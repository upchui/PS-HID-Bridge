//! The app as an AppImage on a system that installs AppImages: Shelly,
//! CachyOS's package manager, copies one to ~/.local/bin, adds it to the
//! application menu and keeps a record with its version (read from
//! `X-AppImage-Version` in the AppImage's desktop entry).
//!
//! The app updates itself (crate::update); Shelly only installs it and is
//! told when the file changed ([`sync`]), so its record shows the version
//! that runs. Started from somewhere else (a download), the app offers to
//! install itself with Shelly ([`find`] tells whether it is installed
//! already, and where).
//!
//! Programs started from inside the AppImage must not see the paths into
//! its mount (libraries, Qt plugins), nor inherit files open in it:
//! [`host_command`].
//!
//! Shelly refreshes the desktop database but not KDE Plasma's menu cache
//! (ksycoca): after installing or syncing, [`refresh_menu`] does, so the app
//! shows in the menu (under Games) at once.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde_json::Value;

/// The desktop entry's name, as Shelly lists it.
const DESKTOP_NAME: &str = "PS HID Bridge";
/// The release files are called PS-HID-Bridge-<processor>.AppImage.
const FILE_PREFIX: &str = "ps-hid-bridge";
/// The desktop entry Shelly writes (it keeps the AppImage's desktop file ID).
const DESKTOP_FILE: &str = "io.github.thealhu.pshidbridge.desktop";

/// An AppImage in Shelly's record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Shelly's name for it (the file name it was installed from, without
    /// ".AppImage"): what `shelly sync appimage` and `remove` take.
    pub name: String,
    pub desktop_name: String,
    /// "Unknown" when the AppImage carried no version.
    pub version: String,
    pub path: PathBuf,
}

/// Where this app stands with Shelly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The running file is the installed one.
    ThisFile(Installed),
    /// The app is installed, but this is another file (e.g. a download).
    Elsewhere(Installed),
    NotInstalled,
}

/// The output of `shelly list appimage --json`.
pub fn parse_list(json: &str) -> Result<Vec<Installed>, String> {
    let v: Value = serde_json::from_str(json.trim()).map_err(|e| format!("Shelly's list: {e}"))?;
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    Ok(v.as_array()
        .ok_or("Shelly's list is no list")?
        .iter()
        .map(|item| Installed {
            name: text(item, "Name"),
            desktop_name: text(item, "DesktopName"),
            version: text(item, "Version"),
            path: PathBuf::from(text(item, "Path")),
        })
        .filter(|i| !i.name.is_empty())
        .collect())
}

fn is_this_app(i: &Installed) -> bool {
    i.desktop_name == DESKTOP_NAME || i.name.to_lowercase().starts_with(FILE_PREFIX)
}

/// Is the app (`running`: the AppImage that runs) in Shelly's record?
pub fn find(list: &[Installed], running: &Path) -> Found {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let me = real(running);
    if let Some(i) = list.iter().find(|i| real(&i.path) == me) {
        return Found::ThisFile(i.clone());
    }
    match list.iter().find(|i| is_this_app(i)) {
        Some(i) => Found::Elsewhere(i.clone()),
        None => Found::NotInstalled,
    }
}

/// Shelly's record shows another version than `own` (the one running):
/// "Unknown" from an AppImage without a version, or one from before an
/// update the app made itself.
pub fn record_outdated(recorded: &str, own: &str) -> bool {
    recorded.trim().trim_start_matches('v') != own
}

/// Shelly's program, if it is installed.
pub fn shelly() -> Option<PathBuf> {
    program("shelly")
}

/// A program in the PATH.
fn program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

/// Rebuild KDE Plasma's menu cache (kbuildsycoca6, or kbuildsycoca5), so
/// a new or changed desktop entry shows in its menu at once. Nothing on
/// other desktops (they read the desktop entries themselves).
pub fn refresh_menu() {
    let Some(tool) = program("kbuildsycoca6").or_else(|| program("kbuildsycoca5")) else {
        return;
    };
    match host_command(&tool).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => crate::logln!(
            "PS HID Bridge: {} failed: {}",
            tool.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => crate::logln!("PS HID Bridge: {}: {e}", tool.display()),
    }
}

/// KDE's menu cache is older than the app's desktop entry (installed or
/// changed by hand with Shelly while the cache was not rebuilt).
pub fn menu_outdated() -> bool {
    let entry = xdg_dir("XDG_DATA_HOME", ".local/share").map(|d| d.join("applications").join(DESKTOP_FILE));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(entry_time) = entry.as_deref().and_then(modified) else {
        return false; // no entry: nothing to show
    };
    let caches: Vec<SystemTime> = xdg_dir("XDG_CACHE_HOME", ".cache")
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("ksycoca"))
        .filter_map(|e| modified(&e.path()))
        .collect();
    cache_behind(entry_time, &caches)
}

/// The newest cache is older than the entry (no cache: KDE builds one itself).
fn cache_behind(entry: SystemTime, caches: &[SystemTime]) -> bool {
    caches.iter().max().is_some_and(|newest| *newest < entry)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Run Shelly; its output, or what it said went wrong.
fn run_shelly(args: &[&OsStr]) -> Result<String, String> {
    let program = shelly().ok_or("Shelly is not installed")?;
    let out = host_command(&program)
        .args(args)
        .output()
        .map_err(|e| format!("Shelly: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let said = [stderr.as_ref(), stdout.as_str()]
        .iter()
        .flat_map(|s| s.lines().rev())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no message")
        .to_string();
    Err(format!("Shelly: {said}"))
}

/// The AppImages Shelly installed.
pub fn list() -> Result<Vec<Installed>, String> {
    parse_list(&run_shelly(&[
        "list".as_ref(),
        "appimage".as_ref(),
        "--json".as_ref(),
    ])?)
}

/// Install the AppImage at `path` with Shelly (a copy in its folder, the
/// application menu, the icon); returns the installed copy.
pub fn install(path: &Path) -> Result<Installed, String> {
    run_shelly(&[
        "install".as_ref(),
        "appimage".as_ref(),
        path.as_os_str(),
        "--no-confirm".as_ref(),
    ])?;
    let found = match find(&list()?, path) {
        Found::ThisFile(i) | Found::Elsewhere(i) => i,
        Found::NotInstalled => return Err("Shelly did not list the app after installing it".into()),
    };
    refresh_menu();
    Ok(found)
}

/// Shelly reads the installed AppImage `name` again (version, desktop
/// entry, icon): after the app replaced its own file with a new version.
pub fn sync(name: &str) -> Result<(), String> {
    run_shelly(&[
        "sync".as_ref(),
        "appimage".as_ref(),
        name.as_ref(),
        "--no-confirm".as_ref(),
    ])?;
    refresh_menu();
    Ok(())
}

/// A program on the host, started from inside the AppImage: without what
/// its AppRun set up (paths into its mount in PATH, LD_LIBRARY_PATH, the Qt
/// plugin paths …, which would load the AppImage's libraries or point into
/// a folder that disappears when it ends), without the app's open files
/// (one inside the mount would keep the old AppImage mounted, its runtime
/// waiting, after a restart into another AppImage), in the folder it was
/// started from, and in its own process group (not ended together with the
/// app).
pub fn host_command(program: impl AsRef<OsStr>) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(program);
    let appdir = std::env::var("APPDIR").ok().filter(|d| !d.is_empty());
    for (key, value) in host_env(appdir.as_deref(), std::env::vars()) {
        match value {
            Some(v) => cmd.env(key, v),
            None => cmd.env_remove(key),
        };
    }
    if let Some(dir) = std::env::var_os("OWD") {
        cmd.current_dir(dir);
    }
    cmd.process_group(0);
    // SAFETY: only system calls between fork and exec (async-signal-safe).
    // Close-on-exec, not closed: Rust's own pipe for exec errors stays
    // usable. Where close_range is missing (kernels before 5.11) or refused
    // (a container's seccomp filter), the first descriptors one by one.
    unsafe {
        cmd.pre_exec(|| {
            const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
            let done = libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            ) == 0;
            if !done {
                for fd in 3..1024 {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
    cmd
}

/// The changes to the environment for a program outside the AppImage at
/// `appdir`: list variables lose their parts inside it (and go if nothing
/// is left); the AppImage's own variables go.
pub fn host_env(
    appdir: Option<&str>,
    vars: impl Iterator<Item = (String, String)>,
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for (key, value) in vars {
        if ["APPDIR", "APPIMAGE", "ARGV0", "OWD"].contains(&key.as_str()) {
            out.push((key, None));
            continue;
        }
        let Some(appdir) = appdir.filter(|d| value.contains(d)) else {
            continue;
        };
        let kept: Vec<&str> = value.split(':').filter(|p| !p.contains(appdir)).collect();
        if kept.iter().all(|p| p.is_empty()) {
            out.push((key, None));
        } else {
            out.push((key, Some(kept.join(":"))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As Shelly 3.2.1 wrote it.
    const LIST: &str = r#"[{"Name":"PS-HID-Bridge-x86_64","DesktopName":"PS HID Bridge","Version":"Unknown","IconName":"ps-hid-bridge-x86_64","Description":"Forwards mouse and keyboard to the PlayStation over the network","SizeOnDisk":40184312,"UpdateURl":"","RawUpdateInfo":"","RepoOwner":null,"RepoName":null,"UpdateType":0,"AllowPrerelease":false,"EnvironmentVariables":[],"CommandLineArgs":"","Path":"/home/alex/.local/bin/PS-HID-Bridge-x86_64.AppImage"},
        {"Name":"Other","DesktopName":"Other App","Version":"2.0","Path":"/home/alex/.local/bin/Other.AppImage"}]"#;

    #[test]
    fn shellys_list() {
        let list = parse_list(LIST).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "PS-HID-Bridge-x86_64");
        assert_eq!(list[0].desktop_name, "PS HID Bridge");
        assert_eq!(list[0].version, "Unknown");
        assert_eq!(
            list[0].path,
            PathBuf::from("/home/alex/.local/bin/PS-HID-Bridge-x86_64.AppImage")
        );
        assert_eq!(parse_list("[]").unwrap(), vec![]);
        assert!(parse_list("No AppImages installed").is_err());
        assert!(parse_list("{}").is_err());
    }

    #[test]
    fn installed_here_elsewhere_or_not() {
        let dir = std::env::temp_dir().join(format!("pshb-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let installed = dir.join("PS-HID-Bridge-x86_64.AppImage");
        std::fs::write(&installed, b"").unwrap();
        let link = dir.join("link.AppImage");
        let _ = std::os::unix::fs::symlink(&installed, &link);
        let list = vec![
            Installed {
                name: "Other".into(),
                desktop_name: "Other App".into(),
                version: "2.0".into(),
                path: dir.join("Other.AppImage"),
            },
            Installed {
                name: "PS-HID-Bridge-x86_64".into(),
                desktop_name: "PS HID Bridge".into(),
                version: "1.0.90".into(),
                path: installed.clone(),
            },
        ];
        assert!(matches!(find(&list, &installed), Found::ThisFile(i) if i.name == "PS-HID-Bridge-x86_64"));
        // The same file through a symbolic link.
        assert!(matches!(find(&list, &link), Found::ThisFile(_)));
        let download = Path::new("/home/someone/Downloads/PS-HID-Bridge-x86_64.AppImage");
        assert!(matches!(find(&list, download), Found::Elsewhere(i) if i.version == "1.0.90"));
        assert_eq!(find(&list[..1], download), Found::NotInstalled);
        // Installed from a renamed download: found by its desktop entry's name.
        let renamed = vec![Installed {
            name: "PS-HID-Bridge-x86_64 (1)".into(),
            desktop_name: "PS HID Bridge".into(),
            version: "1.0.90".into(),
            path: dir.join("x.AppImage"),
        }];
        assert!(matches!(find(&renamed, download), Found::Elsewhere(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn programs_outside_inherit_no_files() {
        // Opened without close-on-exec (as some libraries do), at a number
        // the child's own files (ls opens /proc/self/fd as the lowest free
        // one) do not reach.
        let path = std::ffi::CString::new("/proc/self/stat").unwrap();
        let opened = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY) };
        assert!(opened > 2);
        let fd = unsafe { libc::dup2(opened, 200) };
        assert_eq!(fd, 200);
        unsafe { libc::close(opened) };
        let out = host_command("sh")
            .args(["-c", "ls /proc/self/fd"])
            .output()
            .unwrap();
        let fds: Vec<i32> = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect();
        unsafe { libc::close(fd) };
        assert!(!fds.contains(&fd), "fd {fd} inherited: {fds:?}");
        assert!(fds.contains(&1), "its own output is there: {fds:?}");
    }

    #[test]
    fn menu_cache_behind_the_entry() {
        let t = |s: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        assert!(cache_behind(t(100), &[t(50), t(90)]));
        assert!(!cache_behind(t(100), &[t(50), t(110)]));
        assert!(!cache_behind(t(100), &[]), "no cache: KDE builds one itself");
    }

    #[test]
    fn outdated_records() {
        assert!(record_outdated("Unknown", "1.0.90"));
        assert!(record_outdated("1.0.89", "1.0.90"));
        assert!(!record_outdated("1.0.90", "1.0.90"));
        assert!(!record_outdated("v1.0.90", "1.0.90"));
    }

    #[test]
    fn environment_for_programs_outside() {
        let vars = [
            ("PATH", "/tmp/.mount_PSabc/usr/bin:/usr/local/bin:/usr/bin"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_PSabc/usr/lib"),
            ("QT_PLUGIN_PATH", "/tmp/.mount_PSabc/usr/plugins"),
            ("HOME", "/home/alex"),
            ("APPDIR", "/tmp/.mount_PSabc"),
            ("APPIMAGE", "/home/alex/.local/bin/PS-HID-Bridge-x86_64.AppImage"),
            ("ARGV0", "PS-HID-Bridge-x86_64.AppImage"),
            ("OWD", "/home/alex"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let mut changes = host_env(Some("/tmp/.mount_PSabc"), vars.clone().into_iter());
        changes.sort();
        let change = |k: &str| changes.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(change("PATH"), Some(Some("/usr/local/bin:/usr/bin".into())));
        assert_eq!(change("LD_LIBRARY_PATH"), Some(None));
        assert_eq!(change("QT_PLUGIN_PATH"), Some(None));
        assert_eq!(change("HOME"), None, "untouched");
        for k in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
            assert_eq!(change(k), Some(None), "{k}");
        }
        // Not inside an AppImage: only its variables would go.
        assert!(host_env(None, vars[..4].iter().cloned()).is_empty());
    }
}
