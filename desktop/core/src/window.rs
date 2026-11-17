//! Which window has the focus: for the window rule (bridge on while a
//! certain window has the focus) and for profiles that follow a window.
//!
//! Wayland gives applications no common way to learn the focused window, so
//! each desktop has its own backend ([`Backend`], picked by [`detect`]):
//! * KDE Plasma (KWin): a small KWin script reports every activation over
//!   D-Bus (same approach as kdotool).
//! * Hyprland: its event socket (`activewindow>>class,title`).
//! * Sway and i3: the i3 IPC (window and workspace events, then the tree).
//! * niri: its event stream.
//! * X11, any desktop: `_NET_ACTIVE_WINDOW` on the root window.
//!
//! GNOME on Wayland offers nothing of the kind without a Shell extension.
//! Backends only report the window; [`matches`] decides what a pattern
//! means, the same for all of them.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

/// The focused window as a backend sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    pub title: String,
    /// Window class (X11, KWin, Hyprland) or app id (Wayland).
    pub app_id: String,
    /// Desktop file name (KWin), or the X11 class's instance part.
    pub desktop_file: String,
}

impl WindowInfo {
    /// None when every part is empty (no window).
    pub fn new(title: &str, app_id: &str, desktop_file: &str) -> Option<Self> {
        let w = Self {
            title: title.to_string(),
            app_id: app_id.to_string(),
            desktop_file: desktop_file.to_string(),
        };
        (!w.title.is_empty() || !w.app_id.is_empty() || !w.desktop_file.is_empty()).then_some(w)
    }
}

/// The window's title, class or desktop file contains `pattern`, ignoring
/// case. An empty pattern matches nothing.
pub fn matches(pattern: &str, w: &WindowInfo) -> bool {
    let p = pattern.trim().to_lowercase();
    !p.is_empty()
        && [&w.title, &w.app_id, &w.desktop_file]
            .iter()
            .any(|s| !s.is_empty() && s.to_lowercase().contains(&p))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    KWin,
    Hyprland,
    Sway,
    Niri,
    X11,
}

impl Backend {
    /// For the user: where the window is learned from.
    pub fn name(self) -> &'static str {
        match self {
            Self::KWin => "KDE Plasma",
            Self::Hyprland => "Hyprland",
            Self::Sway => "Sway / i3",
            Self::Niri => "niri",
            Self::X11 => "X11",
        }
    }
}

/// The backend for this session, or why there is none. `kwin`: KWin is on
/// the session bus; `var`: the environment.
pub fn detect_from(kwin: bool, var: &dyn Fn(&str) -> Option<String>) -> Result<Backend, String> {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    if kwin {
        return Ok(Backend::KWin);
    }
    if set("HYPRLAND_INSTANCE_SIGNATURE") {
        return Ok(Backend::Hyprland);
    }
    if set("SWAYSOCK") || set("I3SOCK") {
        return Ok(Backend::Sway);
    }
    if set("NIRI_SOCKET") {
        return Ok(Backend::Niri);
    }
    // Under Wayland, DISPLAY belongs to Xwayland: it knows X11 windows only.
    let session = var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase();
    let wayland = session == "wayland" || (session.is_empty() && set("WAYLAND_DISPLAY"));
    if !wayland && set("DISPLAY") {
        return Ok(Backend::X11);
    }
    let desktop = var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop.to_lowercase().contains("gnome") {
        return Err("GNOME on Wayland does not tell applications which window has the focus".into());
    }
    Err(format!(
        "this desktop ({}) does not tell applications which window has the focus",
        if desktop.is_empty() { "unknown" } else { &desktop }
    ))
}

/// The backend for this session (see [`detect_from`]).
pub fn detect() -> Result<Backend, String> {
    detect_from(kwin_available(), &|k| std::env::var(k).ok())
}

/// Is KWin reachable on the session bus?
pub fn kwin_available() -> bool {
    #[cfg(feature = "focus")]
    {
        kwin::available()
    }
    #[cfg(not(feature = "focus"))]
    {
        false
    }
}

/// Calls the callback only when the window changed.
struct Reporter {
    last: Mutex<Option<Option<WindowInfo>>>,
    on_change: Box<dyn Fn(Option<WindowInfo>) + Send + Sync>,
}

impl Reporter {
    fn report(&self, w: Option<WindowInfo>) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() != Some(&w) {
            *last = Some(w.clone());
            (self.on_change)(w);
        }
    }
}

/// Watches the focused window until dropped.
pub struct Watcher {
    backend: Backend,
    stop: Arc<AtomicBool>,
    /// Shut down on drop: wakes a thread blocked reading from it.
    socket: Arc<Mutex<Option<UnixStream>>>,
    thread: Option<JoinHandle<()>>,
    #[cfg(feature = "focus")]
    _kwin: Option<kwin::KWinWatcher>,
}

/// How long a backend waits before connecting again after its compositor
/// went away (restarted).
const RECONNECT: Duration = Duration::from_secs(2);

impl Watcher {
    /// Report the focused window now and on every change (None: no window,
    /// or the backend lost its connection). The callback runs on a helper
    /// thread.
    pub fn start(
        backend: Backend,
        on_change: impl Fn(Option<WindowInfo>) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let reporter = Arc::new(Reporter {
            last: Mutex::new(None),
            on_change: Box::new(on_change),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let socket = Arc::new(Mutex::new(None));
        let mut w = Self {
            backend,
            stop: stop.clone(),
            socket: socket.clone(),
            thread: None,
            #[cfg(feature = "focus")]
            _kwin: None,
        };
        let run: RunFn = match backend {
            Backend::KWin => {
                #[cfg(feature = "focus")]
                {
                    let r = reporter.clone();
                    w._kwin = Some(kwin::KWinWatcher::start(move |info| r.report(info))?);
                    return Ok(w);
                }
                #[cfg(not(feature = "focus"))]
                return Err("built without KDE Plasma support".into());
            }
            Backend::Hyprland => {
                let dir = hyprland_dir().ok_or("Hyprland's sockets not found")?;
                Box::new(move |ctx| hyprland::run(&dir, ctx))
            }
            Backend::Sway => {
                let path = std::env::var_os("SWAYSOCK")
                    .or_else(|| std::env::var_os("I3SOCK"))
                    .map(PathBuf::from)
                    .ok_or("SWAYSOCK not set")?;
                Box::new(move |ctx| sway::run(&path, ctx))
            }
            Backend::Niri => {
                let path = std::env::var_os("NIRI_SOCKET")
                    .map(PathBuf::from)
                    .ok_or("NIRI_SOCKET not set")?;
                Box::new(move |ctx| niri::run(&path, ctx))
            }
            Backend::X11 => {
                #[cfg(feature = "x11")]
                {
                    Box::new(x11::run)
                }
                #[cfg(not(feature = "x11"))]
                return Err("built without X11 support".into());
            }
        };
        let ctx = Ctx {
            stop,
            socket,
            reporter,
        };
        let thread = thread::Builder::new()
            .name("pshb-window".into())
            .spawn(move || {
                while !ctx.stopped() {
                    if let Err(e) = run(&ctx)
                        && !ctx.stopped()
                    {
                        crate::logln!("PS HID Bridge: window watcher ({}): {e}", backend.name());
                    }
                    ctx.reporter.report(None);
                    // Wait for the compositor to come back, in small steps.
                    for _ in 0..20 {
                        if ctx.stopped() {
                            return;
                        }
                        thread::sleep(RECONNECT / 20);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        w.thread = Some(thread);
        Ok(w)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(s) = self.socket.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A backend's loop: watches until an error or the stop.
type RunFn = Box<dyn Fn(&Ctx) -> io::Result<()> + Send>;

/// What a backend's thread gets.
struct Ctx {
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<UnixStream>>>,
    reporter: Arc<Reporter>,
}

impl Ctx {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn report(&self, w: Option<WindowInfo>) {
        self.reporter.report(w);
    }

    /// Connect and remember the stream, so that dropping the watcher
    /// interrupts a blocking read.
    fn connect(&self, path: &Path) -> io::Result<UnixStream> {
        let s = UnixStream::connect(path)?;
        *self.socket.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.try_clone()?);
        if self.stopped() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        Ok(s)
    }
}

fn hyprland_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    // Hyprland 0.40 and newer, then older versions.
    [runtime.map(|r| r.join("hypr")), Some(PathBuf::from("/tmp/hypr"))]
        .into_iter()
        .flatten()
        .map(|d| d.join(&sig))
        .find(|d| d.join(".socket2.sock").exists())
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

mod hyprland {
    use super::*;

    /// One line of the event socket: Some(window) for a focus change.
    pub fn event(line: &str) -> Option<Option<WindowInfo>> {
        let data = line.strip_prefix("activewindow>>")?;
        // The class has no comma; the title may.
        let (class, title) = data.split_once(',').unwrap_or((data, ""));
        Some(WindowInfo::new(title, class, ""))
    }

    /// Answer to "j/activewindow" ("{}" without a window).
    pub fn active(json: &str) -> Option<WindowInfo> {
        let v: Value = serde_json::from_str(json).ok()?;
        WindowInfo::new(str_of(&v, "title"), str_of(&v, "class"), "")
    }

    pub fn run(dir: &Path, ctx: &Ctx) -> io::Result<()> {
        let events = ctx.connect(&dir.join(".socket2.sock"))?;
        // The current window (events only tell changes).
        if let Ok(mut q) = UnixStream::connect(dir.join(".socket.sock")) {
            q.set_read_timeout(Some(Duration::from_secs(2)))?;
            q.write_all(b"j/activewindow")?;
            let mut answer = String::new();
            q.read_to_string(&mut answer)?;
            ctx.report(active(&answer));
        }
        for line in BufReader::new(events).lines() {
            if let Some(w) = event(&line?) {
                ctx.report(w);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Hyprland closed the connection",
        ))
    }
}

mod sway {
    use super::*;

    const MAGIC: &[u8] = b"i3-ipc";
    pub const SUBSCRIBE: u32 = 2;
    pub const GET_TREE: u32 = 4;

    pub fn encode(kind: u32, payload: &[u8]) -> Vec<u8> {
        let mut m = MAGIC.to_vec();
        m.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        m.extend_from_slice(&kind.to_le_bytes());
        m.extend_from_slice(payload);
        m
    }

    /// One message: (type, payload).
    pub fn read(r: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
        let mut head = [0u8; 14];
        r.read_exact(&mut head)?;
        if &head[..6] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an i3 IPC message",
            ));
        }
        let len = u32::from_le_bytes(head[6..10].try_into().unwrap()) as usize;
        let kind = u32::from_le_bytes(head[10..14].try_into().unwrap());
        if len > 64 << 20 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "i3 IPC message too large",
            ));
        }
        let mut payload = vec![0; len];
        r.read_exact(&mut payload)?;
        Ok((kind, payload))
    }

    /// The focused window in a GET_TREE answer (None: an empty workspace
    /// has the focus).
    pub fn focused(tree: &Value) -> Option<WindowInfo> {
        let node = find_focused(tree)?;
        if !matches!(str_of(node, "type"), "con" | "floating_con") {
            return None;
        }
        let props = node.get("window_properties");
        let class = props.map(|p| str_of(p, "class")).unwrap_or("");
        let instance = props.map(|p| str_of(p, "instance")).unwrap_or("");
        let app_id = node.get("app_id").and_then(Value::as_str).unwrap_or(class);
        WindowInfo::new(str_of(node, "name"), app_id, instance)
    }

    fn find_focused(node: &Value) -> Option<&Value> {
        if node.get("focused").and_then(Value::as_bool) == Some(true) {
            return Some(node);
        }
        ["nodes", "floating_nodes"]
            .iter()
            .filter_map(|k| node.get(*k).and_then(Value::as_array))
            .flatten()
            .find_map(find_focused)
    }

    fn tree(q: &mut UnixStream) -> io::Result<Option<WindowInfo>> {
        q.write_all(&encode(GET_TREE, b""))?;
        let (_, payload) = read(q)?;
        let v: Value = serde_json::from_slice(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(focused(&v))
    }

    pub fn run(path: &Path, ctx: &Ctx) -> io::Result<()> {
        let mut events = ctx.connect(path)?;
        let mut query = UnixStream::connect(path)?;
        query.set_read_timeout(Some(Duration::from_secs(2)))?;
        events.write_all(&encode(SUBSCRIBE, br#"["window","workspace"]"#))?;
        let (_, reply) = read(&mut events)?;
        if !String::from_utf8_lossy(&reply).contains("true") {
            return Err(io::Error::other("subscribing to window events refused"));
        }
        ctx.report(tree(&mut query)?);
        loop {
            read(&mut events)?; // any window or workspace change
            ctx.report(tree(&mut query)?);
        }
    }
}

mod niri {
    use super::*;
    use std::collections::BTreeMap;

    /// Windows and which one has the focus, from the event stream.
    #[derive(Default)]
    pub struct State {
        windows: BTreeMap<u64, WindowInfo>,
        focused: Option<u64>,
    }

    impl State {
        pub fn focused(&self) -> Option<WindowInfo> {
            self.focused.and_then(|id| self.windows.get(&id)).cloned()
        }

        fn window(&mut self, w: &Value) {
            let Some(id) = w.get("id").and_then(Value::as_u64) else {
                return;
            };
            let info = WindowInfo {
                title: str_of(w, "title").into(),
                app_id: str_of(w, "app_id").into(),
                desktop_file: String::new(),
            };
            self.windows.insert(id, info);
            if w.get("is_focused").and_then(Value::as_bool) == Some(true) {
                self.focused = Some(id);
            } else if self.focused == Some(id) {
                self.focused = None;
            }
        }

        /// Apply one event line.
        pub fn apply(&mut self, event: &Value) {
            if let Some(e) = event.get("WindowsChanged") {
                self.windows.clear();
                self.focused = None;
                for w in e.get("windows").and_then(Value::as_array).into_iter().flatten() {
                    self.window(w);
                }
            } else if let Some(w) = event.get("WindowOpenedOrChanged").and_then(|e| e.get("window")) {
                self.window(w);
            } else if let Some(id) = event
                .get("WindowClosed")
                .and_then(|e| e.get("id"))
                .and_then(Value::as_u64)
            {
                self.windows.remove(&id);
                if self.focused == Some(id) {
                    self.focused = None;
                }
            } else if let Some(e) = event.get("WindowFocusChanged") {
                self.focused = e.get("id").and_then(Value::as_u64);
            }
        }
    }

    pub fn run(path: &Path, ctx: &Ctx) -> io::Result<()> {
        let mut s = ctx.connect(path)?;
        s.write_all(b"\"EventStream\"\n")?;
        let mut state = State::default();
        let mut lines = BufReader::new(s).lines();
        let first = lines.next().transpose()?.unwrap_or_default();
        if !first.contains("Ok") {
            return Err(io::Error::other(format!(
                "niri refused the event stream: {first}"
            )));
        }
        for line in lines {
            if let Ok(v) = serde_json::from_str::<Value>(&line?) {
                state.apply(&v);
                ctx.report(state.focused());
            }
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "niri closed the connection",
        ))
    }
}

#[cfg(feature = "x11")]
mod x11 {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, Window,
    };

    fn err(e: impl std::fmt::Display) -> io::Error {
        io::Error::other(e.to_string())
    }

    struct Atoms {
        active: Atom,
        name: Atom,
        utf8: Atom,
    }

    fn property(conn: &impl Connection, win: Window, prop: Atom, kind: Atom) -> Option<Vec<u8>> {
        let r = conn
            .get_property(false, win, prop, kind, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(r.value)
    }

    fn active(conn: &impl Connection, root: Window, a: &Atoms) -> Option<Window> {
        let r = conn
            .get_property(false, root, a.active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        r.value32().and_then(|mut v| v.next()).filter(|w| *w != 0)
    }

    fn info(conn: &impl Connection, win: Window, a: &Atoms) -> Option<WindowInfo> {
        let title = property(conn, win, a.name, a.utf8)
            .filter(|t| !t.is_empty())
            .or_else(|| property(conn, win, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
            .unwrap_or_default();
        // WM_CLASS: "instance\0class\0"
        let class =
            property(conn, win, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into()).unwrap_or_default();
        let mut parts = class
            .split(|b| *b == 0)
            .map(|p| String::from_utf8_lossy(p).into_owned());
        let instance = parts.next().unwrap_or_default();
        let class = parts.next().unwrap_or_default();
        WindowInfo::new(&String::from_utf8_lossy(&title), &class, &instance)
    }

    pub fn run(ctx: &Ctx) -> io::Result<()> {
        let (conn, screen) = x11rb::connect(None).map_err(err)?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &[u8]| -> io::Result<Atom> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(err)?
                .reply()
                .map_err(err)?
                .atom)
        };
        let a = Atoms {
            active: atom(b"_NET_ACTIVE_WINDOW")?,
            name: atom(b"_NET_WM_NAME")?,
            utf8: atom(b"UTF8_STRING")?,
        };
        let watch = |win: Window| {
            let _ = conn.change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            );
        };
        watch(root);
        let mut current = active(&conn, root, &a);
        if let Some(w) = current {
            watch(w); // its title changes too
        }
        conn.flush().map_err(err)?;
        ctx.report(current.and_then(|w| info(&conn, w, &a)));
        while !ctx.stopped() {
            match conn.poll_for_event().map_err(err)? {
                Some(Event::PropertyNotify(e)) if e.window == root && e.atom == a.active => {
                    current = active(&conn, root, &a);
                    if let Some(w) = current {
                        watch(w);
                        conn.flush().map_err(err)?;
                    }
                    ctx.report(current.and_then(|w| info(&conn, w, &a)));
                }
                Some(Event::PropertyNotify(e))
                    if Some(e.window) == current
                        && (e.atom == a.name || e.atom == u32::from(AtomEnum::WM_NAME)) =>
                {
                    ctx.report(current.and_then(|w| info(&conn, w, &a)));
                }
                Some(_) => {} // other events, errors of a window that is gone
                None => thread::sleep(Duration::from_millis(50)),
            }
        }
        Ok(())
    }
}

#[cfg(feature = "focus")]
mod kwin {
    //! A small KWin script reports every window activation over D-Bus.
    use super::WindowInfo;
    use zbus::blocking::{Connection, Proxy, connection};

    const OBJECT_PATH: &str = "/FocusWatcher";
    const INTERFACE: &str = "io.github.thealhu.pshidbridge.Focus";
    const SCRIPT_NAME: &str = "ps-hid-bridge-focus";

    struct FocusIface {
        on_change: Box<dyn Fn(Option<WindowInfo>) + Send + Sync>,
    }

    #[zbus::interface(name = "io.github.thealhu.pshidbridge.Focus")]
    impl FocusIface {
        fn activated(&self, caption: String, resource_class: String, desktop_file: String) {
            (self.on_change)(WindowInfo::new(&caption, &resource_class, &desktop_file));
        }
    }

    pub struct KWinWatcher {
        conn: Connection,
    }

    pub fn script(service: &str) -> String {
        format!(
            r#"// Generated by PS HID Bridge: reports the active window.
function report(w) {{
    if (!w) {{
        callDBus("{service}", "{OBJECT_PATH}", "{INTERFACE}", "Activated", "", "", "");
        return;
    }}
    callDBus("{service}", "{OBJECT_PATH}", "{INTERFACE}", "Activated",
             String(w.caption || ""), String(w.resourceClass || ""), String(w.desktopFileName || ""));
}}
if (workspace.windowActivated) {{
    workspace.windowActivated.connect(report);
    report(workspace.activeWindow);
}} else {{
    workspace.clientActivated.connect(report);
    report(workspace.activeClient);
}}
"#
        )
    }

    fn scripting(conn: &Connection) -> zbus::Result<Proxy<'_>> {
        Proxy::new(conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
    }

    pub fn available() -> bool {
        let Ok(conn) = Connection::session() else {
            return false;
        };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
            return false;
        };
        zbus::names::BusName::try_from("org.kde.KWin")
            .ok()
            .and_then(|n| dbus.name_has_owner(n).ok())
            .unwrap_or(false)
    }

    impl KWinWatcher {
        pub fn start(on_change: impl Fn(Option<WindowInfo>) + Send + Sync + 'static) -> Result<Self, String> {
            let iface = FocusIface {
                on_change: Box::new(on_change),
            };
            let conn = connection::Builder::session()
                .and_then(|b| b.serve_at(OBJECT_PATH, iface))
                .and_then(|b| b.build())
                .map_err(|e| format!("D-Bus: {e}"))?;
            let service = conn
                .unique_name()
                .map(|n| n.to_string())
                .ok_or("no D-Bus address")?;

            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = dir.join(format!("{SCRIPT_NAME}.js"));
            std::fs::write(&path, script(&service)).map_err(|e| format!("{}: {e}", path.display()))?;

            {
                let proxy = scripting(&conn).map_err(|e| format!("KWin: {e}"))?;
                let _: zbus::Result<bool> = proxy.call("unloadScript", &(SCRIPT_NAME,));
                let id: i32 = proxy
                    .call("loadScript", &(path.display().to_string(), SCRIPT_NAME))
                    .map_err(|e| format!("Loading the KWin script: {e}"))?;
                if id < 0 {
                    return Err("KWin refused the script.".into());
                }
                Proxy::new(
                    &conn,
                    "org.kde.KWin",
                    format!("/Scripting/Script{id}"),
                    "org.kde.kwin.Script",
                )
                .and_then(|p| p.call::<_, _, ()>("run", &()))
                .map_err(|e| format!("Starting the KWin script: {e}"))?;
            }
            // KWin reports the current window by itself shortly after.
            Ok(Self { conn })
        }
    }

    impl Drop for KWinWatcher {
        fn drop(&mut self) {
            if let Ok(p) = scripting(&self.conn) {
                let _: zbus::Result<bool> = p.call("unloadScript", &(SCRIPT_NAME,));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(title: &str, app: &str, desktop: &str) -> WindowInfo {
        WindowInfo::new(title, app, desktop).unwrap()
    }

    #[test]
    fn patterns() {
        let chiaki = w(
            "chiaki-ng | Stream",
            "chiaki-ng",
            "io.github.streetpea.Chiaki4deck",
        );
        assert!(matches("chiaki", &chiaki));
        assert!(matches("  CHIAKI-NG ", &chiaki));
        assert!(matches("streetpea", &chiaki));
        assert!(!matches("firefox", &chiaki));
        assert!(!matches("", &chiaki));
        assert!(!matches("  ", &chiaki));
        assert_eq!(WindowInfo::new("", "", ""), None);
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn backend_by_session() {
        let detect = |kwin, vars: &[(&str, &str)]| detect_from(kwin, &env(vars));
        assert_eq!(
            detect(true, &[("XDG_SESSION_TYPE", "wayland")]),
            Ok(Backend::KWin)
        );
        assert_eq!(
            detect(
                false,
                &[("HYPRLAND_INSTANCE_SIGNATURE", "abc"), ("DISPLAY", ":0")]
            ),
            Ok(Backend::Hyprland)
        );
        assert_eq!(
            detect(false, &[("SWAYSOCK", "/run/sway.sock")]),
            Ok(Backend::Sway)
        );
        assert_eq!(
            detect(false, &[("I3SOCK", "/run/i3.sock"), ("DISPLAY", ":0")]),
            Ok(Backend::Sway)
        );
        assert_eq!(
            detect(false, &[("NIRI_SOCKET", "/run/niri.sock")]),
            Ok(Backend::Niri)
        );
        assert_eq!(
            detect(false, &[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")]),
            Ok(Backend::X11)
        );
        assert_eq!(detect(false, &[("DISPLAY", ":0")]), Ok(Backend::X11));
        // Xwayland's DISPLAY does not count.
        let gnome = detect(
            false,
            &[
                ("XDG_SESSION_TYPE", "wayland"),
                ("DISPLAY", ":0"),
                ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
            ],
        );
        assert!(gnome.unwrap_err().contains("GNOME"));
        assert!(
            detect(false, &[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")])
                .unwrap_err()
                .contains("unknown")
        );
        assert!(detect(false, &[("HYPRLAND_INSTANCE_SIGNATURE", "")]).is_err());
    }

    #[test]
    fn hyprland_lines() {
        assert_eq!(
            hyprland::event("activewindow>>chiaki-ng,Stream, PS5"),
            Some(Some(w("Stream, PS5", "chiaki-ng", "")))
        );
        assert_eq!(hyprland::event("activewindow>>,"), Some(None));
        assert_eq!(hyprland::event("activewindowv2>>55d1e0"), None);
        assert_eq!(hyprland::event("workspace>>2"), None);
        assert_eq!(
            hyprland::active(r#"{"address":"0x1","class":"firefox","title":"Mozilla Firefox"}"#),
            Some(w("Mozilla Firefox", "firefox", ""))
        );
        assert_eq!(hyprland::active("{}"), None);
        assert_eq!(hyprland::active("Invalid"), None);
    }

    #[test]
    fn i3_ipc_framing() {
        let m = sway::encode(sway::SUBSCRIBE, br#"["window"]"#);
        assert_eq!(&m[..6], b"i3-ipc");
        let (kind, payload) = sway::read(&mut &m[..]).unwrap();
        assert_eq!(kind, sway::SUBSCRIBE);
        assert_eq!(payload, br#"["window"]"#);
        assert!(sway::read(&mut &b"x3-ipc\0\0\0\0\0\0\0\0"[..]).is_err());
        assert!(sway::read(&mut &m[..10]).is_err());
    }

    #[test]
    fn sway_tree() {
        let tree: Value = serde_json::from_str(
            r#"{"type":"root","focused":false,"nodes":[
                {"type":"output","focused":false,"nodes":[
                    {"type":"workspace","focused":false,"nodes":[
                        {"type":"con","name":"Terminal","app_id":"foot","focused":false,"nodes":[]},
                        {"type":"con","name":"chiaki-ng","app_id":"chiaki-ng","focused":true,"nodes":[]}
                    ],"floating_nodes":[]}
                ]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(sway::focused(&tree), Some(w("chiaki-ng", "chiaki-ng", "")));
        // i3 / Xwayland: class and instance from the X11 properties.
        let x: Value = serde_json::from_str(
            r#"{"type":"root","nodes":[{"type":"workspace","nodes":[],"floating_nodes":[
                {"type":"floating_con","nodes":[],"focused":true,"name":"Steam",
                 "window_properties":{"class":"Steam","instance":"steamwebhelper"}}]}]}"#,
        )
        .unwrap();
        assert_eq!(sway::focused(&x), Some(w("Steam", "Steam", "steamwebhelper")));
        // An empty workspace has the focus.
        let empty: Value = serde_json::from_str(
            r#"{"type":"root","nodes":[{"type":"workspace","focused":true,"nodes":[]}]}"#,
        )
        .unwrap();
        assert_eq!(sway::focused(&empty), None);
    }

    #[test]
    fn niri_events() {
        let mut s = niri::State::default();
        let ev = |t: &str| serde_json::from_str::<Value>(t).unwrap();
        s.apply(&ev(r#"{"WindowsChanged":{"windows":[
                {"id":1,"title":"Terminal","app_id":"foot","is_focused":false},
                {"id":2,"title":"Stream","app_id":"chiaki-ng","is_focused":true}]}}"#));
        assert_eq!(s.focused(), Some(w("Stream", "chiaki-ng", "")));
        s.apply(&ev(r#"{"WindowFocusChanged":{"id":1}}"#));
        assert_eq!(s.focused(), Some(w("Terminal", "foot", "")));
        s.apply(&ev(
            r#"{"WindowOpenedOrChanged":{"window":{"id":1,"title":"vim","app_id":"foot","is_focused":true}}}"#,
        ));
        assert_eq!(s.focused(), Some(w("vim", "foot", "")));
        s.apply(&ev(r#"{"WindowClosed":{"id":1}}"#));
        assert_eq!(s.focused(), None);
        s.apply(&ev(r#"{"WindowFocusChanged":{"id":2}}"#));
        assert_eq!(s.focused(), Some(w("Stream", "chiaki-ng", "")));
        s.apply(&ev(r#"{"WindowFocusChanged":{"id":null}}"#));
        assert_eq!(s.focused(), None);
        s.apply(&ev(r#"{"WorkspaceActivated":{"id":3,"focused":true}}"#)); // ignored
    }

    #[cfg(feature = "focus")]
    #[test]
    fn kwin_script_mentions_service_and_both_signal_names() {
        let s = kwin::script(":1.42");
        assert!(s.contains("\":1.42\""));
        assert!(s.contains("windowActivated") && s.contains("clientActivated"));
    }

    #[test]
    fn reporter_only_passes_changes() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let r = Reporter {
            last: Mutex::new(None),
            on_change: Box::new(move |w| s2.lock().unwrap().push(w)),
        };
        r.report(None);
        r.report(None);
        r.report(Some(w("a", "", "")));
        r.report(Some(w("a", "", "")));
        r.report(Some(w("b", "", "")));
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
}
