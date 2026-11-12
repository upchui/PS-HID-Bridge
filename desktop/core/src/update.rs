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
pub const APPIMAGE_ASSET: &str = "PS-HID-Bridge-x86_64.AppImage";
pub const CLI_ASSET: &str = "pshb-cli-x86_64-linux";
pub const FIRMWARE_ASSET: &str = "ps-hid-bridge.uf2";
const SUMS_ASSET: &str = "SHA256SUMS";

/// How long to wait between automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

const OTA_KEY_LABEL: &[u8] = b"pshb-ota-key-v1";
const OTA_MAC_LABEL: &[u8] = b"pshb-ota-v1";

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

/// X-PSHB-OTA-Auth for uploading `image` after the board's challenge `nonce`.
pub fn ota_mac(psk: &[u8; 32], nonce: &[u8; 16], image: &[u8]) -> [u8; 32] {
    let len = (image.len() as u32).to_le_bytes();
    blake2b_mac(&ota_key(psk), &[OTA_MAC_LABEL, nonce, &len, image])
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

/// Install `image` (a UF2 file) on the board at `host` (web interface on
/// `http_port`), signed with the key. The board restarts into it afterwards.
pub fn board_ota(host: &str, http_port: u16, psk: &[u8; 32], image: &[u8]) -> Result<(), String> {
    let base = if host.contains(':') {
        format!("http://[{host}]:{http_port}") // IPv6
    } else {
        format!("http://{host}:{http_port}")
    };
    let agent = agent(Duration::from_secs(120), true);
    let mut resp = agent
        .get(format!("{base}/api/ota/challenge"))
        .call()
        .map_err(http_error)?;
    if resp.status() != 200 {
        return Err(board_error(&mut resp));
    }
    let text = resp.body_mut().read_to_string().map_err(http_error)?;
    let nonce_hex = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("nonce").and_then(|n| n.as_str()).map(str::to_string))
        .ok_or("Board: no challenge in the answer")?;
    let nonce: [u8; 16] = (0..16)
        .map(|i| u8::from_str_radix(nonce_hex.get(2 * i..2 * i + 2).unwrap_or("x"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("Board: invalid challenge")?;
    let mut resp = agent
        .post(format!("{base}/api/ota"))
        .header("Content-Type", "application/octet-stream")
        .header("X-PSHB-CRC32", format!("{:x}", crc32(image)))
        .header("X-PSHB-OTA-Auth", hex(&ota_mac(psk, &nonce, image)))
        .send(image)
        .map_err(http_error)?;
    if resp.status() != 200 {
        return Err(board_error(&mut resp));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
