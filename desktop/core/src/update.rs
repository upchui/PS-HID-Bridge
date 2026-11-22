//! Updates from the GitHub releases of upchui/PS-HID-Bridge: which version is
//! the newest, download a file checked against the release's SHA256SUMS,
//! replace this program, and install firmware on the board signed with the
//! key (protocol/SPEC.md, "Firmware update with the key").
//!
//! Only github.com and GitHub's download servers are contacted (HTTPS),
//! and the board in the local network (HTTP, signed).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use blake2::Blake2bMac;
use blake2::digest::common::KeyInit;
use blake2::digest::consts::U32;
use blake2::digest::{FixedOutput, Update};
use sha2::{Digest, Sha256};

pub const REPO: &str = "upchui/PS-HID-Bridge";

/// The app's release file for this processor: an update stays on the same
/// one ("PS-HID-Bridge-x86_64.AppImage", "PS-HID-Bridge-aarch64.AppImage").
pub fn appimage_asset() -> String {
    appimage_asset_for(std::env::consts::ARCH)
}

pub fn appimage_asset_for(arch: &str) -> String {
    format!("PS-HID-Bridge-{arch}.AppImage")
}

/// The command line tool's release file for this processor
/// ("pshb-cli-x86_64-linux", "pshb-cli-aarch64-linux").
pub fn cli_asset() -> String {
    cli_asset_for(std::env::consts::ARCH)
}

pub fn cli_asset_for(arch: &str) -> String {
    format!("pshb-cli-{arch}-linux")
}
pub const FIRMWARE_ASSET: &str = "ps-hid-bridge.uf2";
/// The Changelog page's data (tools/changelog.py), a file of every release
/// since it was added.
pub const CHANGELOG_ASSET: &str = "changelog.json";
const SUMS_ASSET: &str = "SHA256SUMS";

/// How long to wait between automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

const OTA_KEY_LABEL: &[u8] = b"pshb-ota-key-v1";
const OTA_MAC_LABEL: &[u8] = b"pshb-ota-v1";
const OTA_MAC2_LABEL: &[u8] = b"pshb-ota-v2";
const REQ_MAC_LABEL: &[u8] = b"pshb-req-v1";

/// MAJOR.MINOR.PATCH (the patch counts the commits since MAJOR.MINOR changed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// "0.9.27", "v0.9.27"; tags of older releases ("v0.9.0-build.15") count
    /// as their base version.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let core = s.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        let v = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(v)
    }

    /// Firmware version from a PONG (0xMMmmpppp).
    pub fn of_firmware(v: u32) -> Self {
        Self(v >> 24, (v >> 16) & 0xFF, v & 0xFFFF)
    }

    /// The version of this program; None for a development build, which is
    /// not compared with releases.
    pub fn of_this_program() -> Option<Self> {
        if crate::VERSION.contains("-dev") {
            None
        } else {
            Self::parse(crate::VERSION)
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Release page on GitHub.
    pub page: String,
    /// Git tag, e.g. "v1.0.3".
    pub tag: String,
}

impl Release {
    /// A file of the release (whether it exists shows when it is downloaded).
    pub fn asset(&self, name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            url: format!("https://github.com/{REPO}/releases/download/{}/{name}", self.tag),
        }
    }
}

/// The release behind a tag ("v1.0.89"), for installing that version (also
/// an older one); None if the tag names no version.
pub fn release_for_tag(tag: &str) -> Option<Release> {
    let tag = tag.trim();
    if !tag.starts_with('v') {
        return None;
    }
    Some(Release {
        version: Version::parse(tag)?,
        page: format!("https://github.com/{REPO}/releases/tag/{tag}"),
        tag: tag.to_string(),
    })
}

/// The changelog of `release` (its changelog.json, checked against its
/// SHA256SUMS), ready for the Changelog page: the section of commits after
/// the newest tag is the build the release was made of, so it gets the
/// release's tag (and can be installed from the page).
pub fn changelog_of(release: &Release) -> Result<String, String> {
    let data = download_verified(release, CHANGELOG_ASSET, &mut |_, _| {})?;
    let text = String::from_utf8(data).map_err(|_| format!("{CHANGELOG_ASSET} is not text"))?;
    tag_release_section(&text, release)
}

/// See [`changelog_of`]: the top section without a tag becomes `release`'s
/// if it has its version.
pub fn tag_release_section(text: &str, release: &Release) -> Result<String, String> {
    let mut v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("{CHANGELOG_ASSET}: {e}"))?;
    let Some(releases) = v.get_mut("releases").and_then(|r| r.as_array_mut()) else {
        return Err(format!("{CHANGELOG_ASSET}: no releases"));
    };
    if let Some(top) = releases.first_mut()
        && top["tag"].as_str() == Some("")
        && top["version"].as_str().and_then(Version::parse) == Some(release.version)
    {
        top["tag"] = serde_json::Value::from(release.tag.clone());
        top["build"] = serde_json::Value::from(false);
    }
    Ok(v.to_string())
}

/// The newest version a changelog lists (its top section).
pub fn changelog_version(text: &str) -> Option<Version> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    Version::parse(v["releases"].get(0)?["version"].as_str()?)
}

/// Of the changelog built into the app and a fetched one, the one that
/// goes further (each lists every version before its newest); the built-in
/// one if they go as far.
pub fn newest_changelog<'a>(built_in: &'a str, fetched: &'a str) -> &'a str {
    match (changelog_version(built_in), changelog_version(fetched)) {
        (Some(a), Some(b)) if b > a => fetched,
        (None, Some(_)) => fetched,
        _ => built_in,
    }
}

/// The oldest app that may be installed over this one: older ones drop the
/// settings they do not know when they save (profiles, remapped keys, ...);
/// from this version on they keep them for a newer one (config `extra`).
pub const OLDEST_APP_INSTALL: Version = Version(1, 0, 32);
/// The oldest firmware that may be installed: older firmware never confirms
/// itself after an update over the network (its boot ROM call failed on
/// core 1), so the board would be back at the newer one after its next
/// restart. From this version on, an older firmware stays: the boot ROM
/// erases the start of the newer one's slot when the older one confirms
/// itself (a "version downgrade" of an A/B pair).
pub const OLDEST_FIRMWARE_INSTALL: Version = Version(1, 0, 64);

/// What is installed: this program, or the firmware on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    App,
    Firmware,
}

/// Whether `version` may be installed as `target`; the reason if not.
pub fn installable(target: Target, version: Version) -> Result<(), String> {
    match target {
        Target::App if version < OLDEST_APP_INSTALL => Err(format!(
            "App versions before {OLDEST_APP_INSTALL} would lose settings they do not know; install {OLDEST_APP_INSTALL} or newer."
        )),
        Target::Firmware if version < OLDEST_FIRMWARE_INSTALL => Err(format!(
            "Firmware before {OLDEST_FIRMWARE_INSTALL} cannot stay installed (the board would return to the newer one); install {OLDEST_FIRMWARE_INSTALL} or newer."
        )),
        _ => Ok(()),
    }
}

/// Upper limit for a download whose size the server does not announce.
const MAX_DOWNLOAD: u64 = 256 << 20;

/// The release a `Location` of `https://github.com/{REPO}/releases/latest`
/// points to (`…/releases/tag/v1.0.3`); None for anything else (no release
/// yet: GitHub then sends the list of releases).
pub fn release_from_location(location: &str) -> Option<Release> {
    let prefix = format!("https://github.com/{REPO}/releases/tag/");
    let tag = location.strip_prefix(&prefix)?;
    let version = Version::parse(tag)?;
    Some(Release {
        version,
        page: location.to_string(),
        tag: tag.to_string(),
    })
}

fn agent(timeout: Duration, follow_redirects: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(timeout))
        .user_agent(format!("ps-hid-bridge/{}", crate::VERSION))
        .http_status_as_error(false)
        .max_redirects(if follow_redirects { 10 } else { 0 })
        .build()
        .into()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Io(e) => format!("Network: {e}"),
        e => e.to_string(),
    }
}

/// The newest release on GitHub. Asks github.com where "latest" points to
/// instead of using api.github.com, whose limit of 60 requests per hour is
/// shared by everything behind the same internet address.
pub fn latest_release() -> Result<Release, String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let resp = agent(Duration::from_secs(30), false)
        .get(&url)
        .call()
        .map_err(http_error)?;
    if !resp.status().is_redirection() {
        return Err(format!("GitHub answered {}", resp.status()));
    }
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    release_from_location(location).ok_or_else(|| "No release found on GitHub.".into())
}

/// Download an asset; `progress(done, total)` while it arrives (total 0 if
/// the server does not say).
pub fn download(asset: &Asset, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let mut resp = agent(Duration::from_secs(600), true)
        .get(&asset.url)
        .call()
        .map_err(http_error)?;
    if resp.status() == 404 {
        return Err(format!("{} is missing in the release.", asset.name));
    }
    if resp.status() != 200 {
        return Err(format!("Download of {} failed: {}", asset.name, resp.status()));
    }
    let total = resp.body().content_length().unwrap_or(0);
    let limit = if total > 0 {
        total.saturating_mul(2)
    } else {
        MAX_DOWNLOAD
    };
    let mut data = Vec::with_capacity(total.min(MAX_DOWNLOAD) as usize);
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download of {}: {e}", asset.name))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() as u64 > limit {
            return Err(format!("Download of {}: more data than announced", asset.name));
        }
        progress(data.len() as u64, total);
    }
    Ok(data)
}

/// Lines "<sha256 hex>  <file name>" (also "*<file name>").
pub fn parse_sums(text: &str) -> Vec<(String, [u8; 32])> {
    text.lines()
        .filter_map(|l| {
            let (hex, name) = l.split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            let bytes = crate::crypto::parse_key_hex(hex).ok()?;
            Some((name.to_string(), bytes))
        })
        .collect()
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Download `name` from `release` and check it against the release's
/// SHA256SUMS.
pub fn download_verified(
    release: &Release,
    name: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    let sums = download(&release.asset(SUMS_ASSET), &mut |_, _| {})?;
    let sums = String::from_utf8(sums).map_err(|_| "SHA256SUMS is not text")?;
    let want = parse_sums(&sums)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, h)| h)
        .ok_or_else(|| format!("{name} is not listed in SHA256SUMS."))?;
    let data = download(&release.asset(name), progress)?;
    if sha256(&data) != want {
        return Err(format!("{name}: checksum does not match, download discarded."));
    }
    Ok(data)
}

/// Replace `path` with `data` (made executable): written next to it first,
/// then renamed over it. A running program keeps its old file open.
pub fn replace_file(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("update");
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.set_permissions(fs::Permissions::from_mode(0o755))?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// ---------------------------------------------------------------- board

fn blake2b_mac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Blake2bMac::<U32>::new_from_slice(key).expect("key of at most 64 bytes");
    for p in parts {
        m.update(p);
    }
    m.finalize_fixed().into()
}

/// Key for signed firmware uploads, derived from the pre-shared key.
pub fn ota_key(psk: &[u8; 32]) -> [u8; 32] {
    blake2b_mac(psk, &[OTA_KEY_LABEL])
}

/// X-PSHB-OTA-Auth: the old signature over the whole file, the only one
/// firmware before 1.0.9 checks (only after it has written the file).
pub fn ota_mac(psk: &[u8; 32], nonce: &[u8; 16], image: &[u8]) -> [u8; 32] {
    let len = (image.len() as u32).to_le_bytes();
    blake2b_mac(&ota_key(psk), &[OTA_MAC_LABEL, nonce, &len, image])
}

/// X-PSHB-OTA-Hash: BLAKE2b-256 of the file.
pub fn ota_hash(image: &[u8]) -> [u8; 32] {
    <blake2::Blake2b<U32> as Digest>::digest(image).into()
}

/// X-PSHB-OTA-Auth2: signature over the file's hash, which the board checks
/// before it writes anything (protocol/SPEC.md).
pub fn ota_mac2(psk: &[u8; 32], nonce: &[u8; 16], image: &[u8]) -> [u8; 32] {
    let len = (image.len() as u32).to_le_bytes();
    blake2b_mac(&ota_key(psk), &[OTA_MAC2_LABEL, nonce, &len, &ota_hash(image)])
}

/// X-PSHB-Auth of a request signed with the key (status, diagnostics,
/// clock; protocol/SPEC.md "Signed requests").
pub fn request_mac(psk: &[u8; 32], nonce: &[u8; 16], method: &str, path: &str, body: &[u8]) -> [u8; 32] {
    blake2b_mac(
        &ota_key(psk),
        &[
            REQ_MAC_LABEL,
            nonce,
            method.as_bytes(),
            b" ",
            path.as_bytes(),
            b"\n",
            body,
        ],
    )
}

/// CRC-32 (IEEE), as the board checks it (X-PSHB-CRC32).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn board_error(resp: &mut ureq::http::Response<ureq::Body>) -> String {
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(text);
    format!("Board answered {}: {msg}", resp.status())
}

fn board_base(host: &str, http_port: u16) -> String {
    if host.contains(':') {
        format!("http://[{host}]:{http_port}") // IPv6
    } else {
        format!("http://{host}:{http_port}")
    }
}

fn read_json(resp: &mut ureq::http::Response<ureq::Body>) -> Result<serde_json::Value, String> {
    if resp.status() != 200 {
        return Err(board_error(resp));
    }
    let text = resp.body_mut().read_to_string().map_err(http_error)?;
    serde_json::from_str(&text).map_err(|e| format!("Board: unexpected answer ({e})"))
}

/// Why one try at the board failed, and whether trying again is safe.
struct Failure {
    msg: String,
    /// Nothing happened on the board: both of its HTTP sockets were busy
    /// (connection refused), or another program took the one-time challenge.
    again: bool,
}

impl From<String> for Failure {
    fn from(msg: String) -> Self {
        Self { msg, again: false }
    }
}

impl From<&str> for Failure {
    fn from(msg: &str) -> Self {
        msg.to_string().into()
    }
}

/// A request to the board that did not get through. A refused connection
/// never reached the board's web server, so trying again is safe.
fn ureq_failure(e: ureq::Error) -> Failure {
    let refused = matches!(&e, ureq::Error::Io(err) if err.kind() == io::ErrorKind::ConnectionRefused);
    let msg = if refused {
        "The board refused the connection: it is restarting, or its web server is busy \
         (it takes two connections at a time, an open web interface uses them too). \
         Try again in a moment."
            .to_string()
    } else {
        http_error(e)
    };
    Failure { msg, again: refused }
}

/// Tries per request while the board is only busy.
const BOARD_TRIES: u32 = 6;

/// Runs `f` up to `BOARD_TRIES` times (about 2 s) while the board was only
/// busy. The board serves two HTTP connections at a time: the app's status
/// requests, an open web interface and pshb-cli can find both in use, or two
/// programs take each other's challenge.
fn retry<T>(f: impl FnMut() -> Result<T, Failure>) -> Result<T, String> {
    retry_pausing(
        |attempt| std::thread::sleep(Duration::from_millis(150 * u64::from(attempt))),
        f,
    )
}

fn retry_pausing<T>(pause: impl Fn(u32), mut f: impl FnMut() -> Result<T, Failure>) -> Result<T, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match f() {
            Err(fail) if fail.again && attempt < BOARD_TRIES => pause(attempt),
            result => return result.map_err(|fail| fail.msg),
        }
    }
}

/// A fresh one-time challenge for a signed request or upload.
fn board_challenge(agent: &ureq::Agent, base: &str) -> Result<[u8; 16], Failure> {
    let mut resp = agent
        .get(format!("{base}/api/ota/challenge"))
        .call()
        .map_err(ureq_failure)?;
    let v = read_json(&mut resp)?;
    let nonce_hex = v
        .get("nonce")
        .and_then(|n| n.as_str())
        .ok_or("Board: no challenge in the answer")?;
    (0..16)
        .map(|i| u8::from_str_radix(nonce_hex.get(2 * i..2 * i + 2).unwrap_or("x"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| "Board: invalid challenge".into())
}

/// The board's web API with the key instead of the web password: status,
/// latency diagnostics and the clock speeds (protocol/SPEC.md "Signed
/// requests"). Every signed request fetches its own challenge first.
pub struct BoardApi {
    base: String,
    psk: [u8; 32],
    agent: ureq::Agent,
}

/// Clock speeds of the board: what the settings ask for and what runs.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct BoardClock {
    pub cpu_mhz: u32,
    pub spi_fast: bool,
    pub cpu_hz: u32,
    pub spi_hz: u32,
    pub cpu_fell_back: bool,
    pub spi_fast_failed: bool,
}

impl BoardApi {
    pub fn new(host: &str, http_port: u16, psk: &[u8; 32]) -> Self {
        Self {
            base: board_base(host, http_port),
            psk: *psk,
            agent: agent(Duration::from_secs(5), false),
        }
    }

    fn signed(&self, method: &str, path: &str, body: &[u8]) -> Result<serde_json::Value, String> {
        retry(|| {
            let nonce = board_challenge(&self.agent, &self.base)?;
            let mac = hex(&request_mac(&self.psk, &nonce, method, path, body));
            let url = format!("{}{path}", self.base);
            let mut resp = if method == "POST" {
                self.agent
                    .post(url)
                    .header("X-PSHB", "1")
                    .header("Content-Type", "application/json")
                    .header("X-PSHB-Auth", mac)
                    .header("X-PSHB-Nonce", hex(&nonce))
                    .send(body)
            } else {
                self.agent
                    .get(url)
                    .header("X-PSHB-Auth", mac)
                    .header("X-PSHB-Nonce", hex(&nonce))
                    .call()
            }
            .map_err(ureq_failure)?;
            if resp.status() == 401 {
                // Firmware before 1.0.25 knows no signed requests: it asks for the login.
                return Err(
                    "The board's firmware is too old for this: update it (Firmware & updates)".into(),
                );
            }
            if resp.status() == 403 {
                // The board checks the challenge before it does anything.
                let msg = board_error(&mut resp);
                let again = msg.contains("Challenge missing or expired");
                return Err(Failure { msg, again });
            }
            Ok(read_json(&mut resp)?)
        })
    }

    /// Board status (network, packets, firmware, ...), as the web interface shows it.
    pub fn status(&self) -> Result<serde_json::Value, String> {
        self.signed("GET", "/api/status", b"")
    }

    /// Latency of the input path, stage by stage (docs/latency.md).
    pub fn diag(&self) -> Result<serde_json::Value, String> {
        self.signed("GET", "/api/diag", b"")
    }

    /// Start the latency statistics over.
    pub fn diag_reset(&self) -> Result<(), String> {
        self.signed("POST", "/api/diag/reset", b"{}").map(drop)
    }

    /// The clock speeds (readable without the key).
    pub fn clock(&self) -> Result<BoardClock, String> {
        let v = retry(|| {
            let mut resp = self
                .agent
                .get(format!("{}/api/clock", self.base))
                .call()
                .map_err(ureq_failure)?;
            Ok(read_json(&mut resp)?)
        })?;
        let num = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        Ok(BoardClock {
            cpu_mhz: num("cpu_mhz"),
            spi_fast: flag("spi_fast"),
            cpu_hz: num("cpu_hz"),
            spi_hz: num("spi_hz"),
            cpu_fell_back: flag("cpu_fell_back"),
            spi_fast_failed: flag("spi_fast_failed"),
        })
    }

    /// Store new clock speeds; returns true if the board restarts for them.
    pub fn set_clock(&self, cpu_mhz: u32, spi_fast: bool) -> Result<bool, String> {
        let body = serde_json::json!({ "cpu_mhz": cpu_mhz, "spi_fast": spi_fast }).to_string();
        let v = self.signed("POST", "/api/clock", body.as_bytes())?;
        Ok(v.get("reboot").and_then(|r| r.as_bool()).unwrap_or(false))
    }

    /// Give the board a name ("" = its default name); returns the name it
    /// shows and its network name (router, browser).
    pub fn set_name(&self, name: &str) -> Result<(String, String), String> {
        let body = serde_json::json!({ "name": name }).to_string();
        let v = match self.signed("POST", "/api/name", body.as_bytes()) {
            // Firmware before names answers like for any other path.
            Err(e) if e.contains("Not allowed with a signature") => {
                return Err(
                    "The board's firmware is too old to be named: update it (Firmware & updates)".into(),
                );
            }
            r => r?,
        };
        let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        Ok((text("name"), text("hostname")))
    }
}

/// Why pairing did not work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairError {
    WrongPassword,
    /// Too many wrong passwords: the board's answer ("Please wait 8 s").
    LockedOut(String),
    /// The board still has its factory password: a new one is needed.
    NeedsNewPassword,
    /// The new password was refused (too short, the factory password).
    NewPasswordRefused(String),
    Other(String),
}

/// The board's web interface with its password, as the browser uses it
/// (plain HTTP, like the browser: see docs/security.md). Pairing takes the
/// key from the board this way. Logs out when dropped: the board keeps only
/// two sessions, the user's browser may hold the other.
pub struct BoardWeb {
    base: String,
    agent: ureq::Agent,
    session: Option<String>,
}

impl BoardWeb {
    pub fn new(host: &str, http_port: u16) -> Self {
        Self {
            base: board_base(host, http_port),
            // The board checks passwords with Argon2id: a login takes a while.
            agent: agent(Duration::from_secs(15), false),
            session: None,
        }
    }

    fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<ureq::http::Response<ureq::Body>, Failure> {
        let mut req = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("X-PSHB", "1")
            .header("Content-Type", "application/json");
        if let Some(s) = &self.session {
            req = req.header("Cookie", format!("pshb_session={s}"));
        }
        req.send(body.to_string().as_bytes()).map_err(ureq_failure)
    }

    /// Log in; Ok(true) if the board still has its factory password.
    pub fn login(&mut self, password: &str) -> Result<bool, PairError> {
        let body = serde_json::json!({ "password": password });
        let mut resp = retry(|| self.post("/api/login", &body)).map_err(PairError::Other)?;
        match resp.status().as_u16() {
            401 => return Err(PairError::WrongPassword),
            429 => return Err(PairError::LockedOut(board_error(&mut resp))),
            _ => {}
        }
        let cookie = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(session_cookie);
        let v = read_json(&mut resp).map_err(PairError::Other)?;
        self.session =
            Some(cookie.ok_or_else(|| PairError::Other("Board: no session in the answer".into()))?);
        Ok(v.get("must_change_password")
            .and_then(|m| m.as_bool())
            .unwrap_or(false))
    }

    /// Replace the password (logged in).
    pub fn change_password(&self, old: &str, new: &str) -> Result<(), PairError> {
        let body = serde_json::json!({ "old": old, "new": new });
        let mut resp = retry(|| self.post("/api/password", &body)).map_err(PairError::Other)?;
        match resp.status().as_u16() {
            200 => Ok(()),
            400 => Err(PairError::NewPasswordRefused(board_error(&mut resp))),
            403 => Err(PairError::WrongPassword),
            429 => Err(PairError::LockedOut(board_error(&mut resp))),
            _ => Err(PairError::Other(board_error(&mut resp))),
        }
    }

    /// Make the board create a new key and return it (logged in, with an
    /// own password). Other PCs with the old key no longer get through.
    pub fn generate_key(&self) -> Result<[u8; 32], PairError> {
        let mut resp =
            retry(|| self.post("/api/psk/generate", &serde_json::json!({}))).map_err(PairError::Other)?;
        if resp.status() == 403 {
            return Err(PairError::NeedsNewPassword);
        }
        let v = read_json(&mut resp).map_err(PairError::Other)?;
        let hex = v
            .get("psk")
            .and_then(|k| k.as_str())
            .ok_or_else(|| PairError::Other("Board: no key in the answer".into()))?;
        crate::crypto::parse_key_hex(hex).map_err(PairError::Other)
    }

    pub fn logout(&mut self) {
        if self.session.is_some() {
            let _ = self.post("/api/logout", &serde_json::json!({}));
            self.session = None;
        }
    }
}

impl Drop for BoardWeb {
    fn drop(&mut self) {
        self.logout();
    }
}

/// "pshb_session=<token>; Path=/; …" -> the token.
fn session_cookie(header: &str) -> Option<String> {
    let first = header.split(';').next()?.trim();
    let token = first.strip_prefix("pshb_session=")?;
    (!token.is_empty()).then(|| token.to_string())
}

/// Pair with the board at `host`: log in with `password`, replace a factory
/// password by `new_password` (Err(NeedsNewPassword) without one), and let
/// the board create a new key, which is returned.
pub fn pair(
    host: &str,
    http_port: u16,
    password: &str,
    new_password: Option<&str>,
) -> Result<[u8; 32], PairError> {
    let mut web = BoardWeb::new(host, http_port);
    let factory = web.login(password)?;
    if factory {
        let new = new_password
            .filter(|p| !p.is_empty())
            .ok_or(PairError::NeedsNewPassword)?;
        web.change_password(password, new)?;
    }
    let key = web.generate_key()?;
    web.logout();
    Ok(key)
}

/// Install `image` (a UF2 file) on the board at `host` (web interface on
/// `http_port`), signed with the key. The board restarts into it afterwards.
pub fn board_ota(host: &str, http_port: u16, psk: &[u8; 32], image: &[u8]) -> Result<(), String> {
    let base = board_base(host, http_port);
    let agent = agent(Duration::from_secs(120), true);
    let nonce = retry(|| board_challenge(&agent, &base))?;
    // A refused upload never reached the board: its challenge is still
    // unused, and the same signed upload may go again.
    let mut resp = retry(|| {
        agent
            .post(format!("{base}/api/ota"))
            .header("Content-Type", "application/octet-stream")
            .header("X-PSHB-CRC32", format!("{:x}", crc32(image)))
            .header("X-PSHB-OTA-Hash", hex(&ota_hash(image)))
            .header("X-PSHB-OTA-Auth2", hex(&ota_mac2(psk, &nonce, image)))
            // For boards with firmware before 1.0.9; newer ones ignore it.
            .header("X-PSHB-OTA-Auth", hex(&ota_mac(psk, &nonce, image)))
            .header("X-PSHB-Nonce", hex(&nonce))
            .send(image)
            .map_err(ureq_failure)
    })?;
    if resp.status() != 200 {
        return Err(board_error(&mut resp));
    }
    Ok(())
}

#[cfg(test)]
mod install_tests {
    use super::*;

    #[test]
    fn releases_from_tags() {
        let r = release_for_tag("v1.0.89").unwrap();
        assert_eq!(r.version, Version(1, 0, 89));
        assert_eq!(r.tag, "v1.0.89");
        assert_eq!(r.page, format!("https://github.com/{REPO}/releases/tag/v1.0.89"));
        assert_eq!(
            r.asset(FIRMWARE_ASSET).url,
            format!("https://github.com/{REPO}/releases/download/v1.0.89/ps-hid-bridge.uf2")
        );
        assert_eq!(release_for_tag("1.0.89"), None);
        assert_eq!(release_for_tag("controller-mode-experiment"), None);
        assert_eq!(release_for_tag(""), None);
    }

    fn log(top_version: &str, top_tag: &str) -> String {
        serde_json::json!({
            "repo": REPO,
            "releases": [
                { "version": top_version, "tag": top_tag, "build": top_tag.is_empty(), "date": "2026-11-20",
                  "compare": "", "changes": [] },
                { "version": "1.0.97", "tag": "v1.0.97", "build": false, "date": "2026-11-20",
                  "compare": "", "changes": [] }
            ]
        })
        .to_string()
    }

    #[test]
    fn a_fetched_changelog_names_its_release() {
        let release = release_for_tag("v1.0.99").unwrap();
        let tagged = tag_release_section(&log("1.0.99", ""), &release).unwrap();
        let v: serde_json::Value = serde_json::from_str(&tagged).unwrap();
        assert_eq!(v["releases"][0]["tag"], "v1.0.99");
        assert_eq!(v["releases"][0]["build"], false);
        assert_eq!(v["releases"][1]["tag"], "v1.0.97");
        // Another version on top (not this release's build): left alone.
        let other = tag_release_section(&log("1.0.98", ""), &release).unwrap();
        let v: serde_json::Value = serde_json::from_str(&other).unwrap();
        assert_eq!(v["releases"][0]["tag"], "");
        assert!(tag_release_section("not json", &release).is_err());
        assert!(tag_release_section("{}", &release).is_err());
    }

    #[test]
    fn the_changelog_that_goes_further_counts() {
        let built_in = log("1.0.99", "");
        let newer = log("1.0.105", "v1.0.105");
        let older = log("1.0.98", "v1.0.98");
        assert_eq!(newest_changelog(&built_in, &newer), newer);
        assert_eq!(newest_changelog(&built_in, &older), built_in);
        assert_eq!(newest_changelog(&built_in, &built_in), built_in);
        assert_eq!(newest_changelog(&built_in, "broken"), built_in);
        // A development build without a changelog: the fetched one.
        assert_eq!(newest_changelog("{}", &newer), newer);
        assert_eq!(changelog_version(&newer), Some(Version(1, 0, 105)));
    }

    #[test]
    fn oldest_versions_to_install() {
        assert!(installable(Target::App, Version(1, 0, 32)).is_ok());
        assert!(installable(Target::App, Version(1, 0, 31)).is_err());
        assert!(installable(Target::App, Version(0, 9, 0)).is_err());
        assert!(installable(Target::Firmware, Version(1, 0, 64)).is_ok());
        assert!(installable(Target::Firmware, Version(1, 0, 63)).is_err());
        assert!(installable(Target::Firmware, Version(1, 1, 0)).is_ok());
        assert!(installable(Target::App, Version(1, 0, 63)).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_files_follow_the_processor() {
        // x86_64 keeps the names every release had so far.
        assert_eq!(appimage_asset_for("x86_64"), "PS-HID-Bridge-x86_64.AppImage");
        assert_eq!(cli_asset_for("x86_64"), "pshb-cli-x86_64-linux");
        assert_eq!(appimage_asset_for("aarch64"), "PS-HID-Bridge-aarch64.AppImage");
        assert_eq!(cli_asset_for("aarch64"), "pshb-cli-aarch64-linux");
        assert_eq!(appimage_asset(), appimage_asset_for(std::env::consts::ARCH));
        assert_eq!(cli_asset(), cli_asset_for(std::env::consts::ARCH));
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("v0.9.27"), Some(Version(0, 9, 27)));
        assert_eq!(Version::parse("0.10.3"), Some(Version(0, 10, 3)));
        assert_eq!(Version::parse("v0.9.0-build.15"), Some(Version(0, 9, 0)));
        assert_eq!(Version::parse("0.9.27-dev"), Some(Version(0, 9, 27)));
        assert_eq!(Version::parse("0.9"), None);
        assert_eq!(Version::parse("0.9.1.2"), None);
        assert_eq!(Version::parse("latest"), None);
        assert!(Version(0, 10, 1) > Version(0, 9, 300));
        assert!(Version(1, 0, 0) > Version(0, 10, 1));
        assert_eq!(Version::of_firmware(0x0009_0123), Version(0, 9, 291));
        assert_eq!(Version(0, 9, 27).to_string(), "0.9.27");
    }

    #[test]
    fn latest_release_from_redirect() {
        let tag_url = format!("https://github.com/{REPO}/releases/tag/v1.0.3");
        let r = release_from_location(&tag_url).unwrap();
        assert_eq!(r.version, Version(1, 0, 3));
        assert_eq!(r.tag, "v1.0.3");
        assert_eq!(
            r.asset(FIRMWARE_ASSET).url,
            format!("https://github.com/{REPO}/releases/download/v1.0.3/ps-hid-bridge.uf2")
        );
        // No release yet: GitHub points to the list of releases.
        assert!(release_from_location(&format!("https://github.com/{REPO}/releases")).is_none());
        let nightly = format!("https://github.com/{REPO}/releases/tag/nightly");
        assert!(release_from_location(&nightly).is_none());
        assert!(release_from_location("https://example.com/releases/tag/v1.0.3").is_none());
        assert!(release_from_location("").is_none());
    }

    #[test]
    fn checksum_list() {
        let h = sha256(b"abc");
        let text = format!(
            "{}  ps-hid-bridge.uf2\n{} *pshb-cli-x86_64-linux\nnot a line\n",
            hex(&h),
            hex(&[1; 32])
        );
        let sums = parse_sums(&text);
        assert_eq!(sums.len(), 2);
        assert_eq!(sums[0], ("ps-hid-bridge.uf2".to_string(), h));
        assert_eq!(sums[1].0, "pshb-cli-x86_64-linux");
    }

    #[test]
    fn ota_mac_matches_the_reference() {
        // protocol/tools/pshb_proto.py: ota_key(bytes(range(32))),
        // ota_mac(bytes(range(32)), bytes(16), b"abc")
        let psk: [u8; 32] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            hex(&ota_key(&psk)),
            "c7ae5309d9256f7dde3c593ffb605101396e22c8d2d76c5b78788d9c79cc787e"
        );
        assert_eq!(
            hex(&ota_mac(&psk, &[0; 16], b"abc")),
            "2bcfe7c870d03d19006a4ed146195bd15a39d7ddb6cd25eaaf1e8ae9fa4906a1"
        );
        // ota_hash(b"abc"), ota_mac2(bytes(range(32)), bytes(16), b"abc")
        assert_eq!(
            hex(&ota_hash(b"abc")),
            "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"
        );
        assert_eq!(
            hex(&ota_mac2(&psk, &[0; 16], b"abc")),
            "8d5efd6a6ed49b245ad596bf92a29135162ba317b6e9d0ea470f7b46a397a7cd"
        );
    }

    #[test]
    fn retry_only_while_the_board_was_busy() {
        let busy = || Failure {
            msg: "busy".into(),
            again: true,
        };
        let pauses = std::cell::RefCell::new(Vec::new());
        let pause = |attempt: u32| pauses.borrow_mut().push(attempt);
        let mut calls = 0;
        let r = retry_pausing(pause, || {
            calls += 1;
            if calls < 3 { Err(busy()) } else { Ok(calls) }
        });
        assert_eq!(r, Ok(3));
        assert_eq!(*pauses.borrow(), [1, 2]);

        pauses.borrow_mut().clear();
        calls = 0;
        let r: Result<(), _> = retry_pausing(pause, || {
            calls += 1;
            Err(busy())
        });
        assert_eq!((r, calls), (Err("busy".to_string()), BOARD_TRIES));
        assert_eq!(pauses.borrow().len() as u32, BOARD_TRIES - 1);

        pauses.borrow_mut().clear();
        calls = 0;
        let r: Result<(), _> = retry_pausing(pause, || {
            calls += 1;
            Err("Board answered 403 Forbidden: Signature does not match".into())
        });
        assert!(r.is_err());
        assert_eq!(calls, 1);
        assert!(pauses.borrow().is_empty());
    }

    #[test]
    fn request_mac_matches_the_reference() {
        // protocol/tools/pshb_proto.py: request_mac(bytes(range(32)), bytes(16), ...)
        let psk: [u8; 32] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            hex(&request_mac(&psk, &[0; 16], "GET", "/api/status", b"")),
            "7b405d8cd09b7f10d3756ec871ad11c575350e41ae7938fbcf646475d0aabbc4"
        );
        assert_eq!(
            hex(&request_mac(
                &psk,
                &[0; 16],
                "POST",
                "/api/clock",
                br#"{"cpu_mhz":200,"spi_fast":true}"#
            )),
            "4cbcb3b57c1ac62c0e9be0868a11009f462ff030d149d018e42380c80f513c64"
        );
    }

    #[test]
    fn session_cookie_from_the_header() {
        assert_eq!(
            session_cookie("pshb_session=0123abcd; Path=/; HttpOnly; SameSite=Strict"),
            Some("0123abcd".into())
        );
        assert_eq!(session_cookie("pshb_session=; Path=/; Max-Age=0"), None);
        assert_eq!(session_cookie("other=1"), None);
    }

    #[test]
    fn crc32_ieee() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn file_is_replaced_and_executable() {
        let dir = std::env::temp_dir().join(format!("pshb-update-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("program");
        fs::write(&path, b"old").unwrap();
        replace_file(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!path.with_extension("update").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
