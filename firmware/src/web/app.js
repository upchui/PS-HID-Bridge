// PS HID Bridge – board web interface.
"use strict";

const $ = (id) => document.getElementById(id);
let statusTimer = null;
let lastStatus = null;
// Newest release on GitHub (asked by this browser, not by the board).
const RELEASES = "https://api.github.com/repos/upchui/PS-HID-Bridge/releases?per_page=10";
let updateChecked = false;
// Tabs live in the address (#network), so reloading keeps the tab.
const TABS = ["status", "network", "security", "system"];
const tabName = () => (TABS.includes(location.hash.slice(1)) ? location.hash.slice(1) : "status");

async function api(path, body) {
  const opt = { credentials: "same-origin", headers: {} };
  if (body !== undefined) {
    opt.method = "POST";
    opt.headers["Content-Type"] = "application/json";
    opt.headers["X-PSHB"] = "1";
    opt.body = JSON.stringify(body);
  }
  let res;
  try {
    res = await fetch(path, opt);
  } catch (e) {
    throw new Error("Board not reachable.");
  }
  let data = {};
  try { data = await res.json(); } catch (e) { /* empty body */ }
  if (res.status === 401 && path !== "/api/login") {
    showLogin();
    throw new Error(data.error || "Please log in.");
  }
  if (!res.ok) throw new Error(data.error || "Error " + res.status);
  return data;
}

function message(text, ok) {
  const m = $("msg");
  m.textContent = text;
  m.className = "msg " + (ok ? "ok" : "bad");
  m.hidden = !text;
  if (text) window.scrollTo({ top: 0, behavior: "smooth" });
}

function show(view) {
  for (const v of ["v-login", "v-first", "v-main"]) $(v).hidden = v !== view;
  $("logout").hidden = view === "v-login";
  if (view !== "v-main") stopStatus();
}

function showLogin() {
  show("v-login");
  $("pill").hidden = true;
}

// "0.9.27" or "v0.9.27" -> [0, 9, 27] (older tags "v0.9.0-build.N" -> [0, 9, 0]).
function parseVersion(s) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)/.exec(s || "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

function newer(a, b) {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

// Once per page load, when the System tab is opened: is there a newer
// firmware? Without internet access the hint simply stays hidden.
async function checkForUpdate() {
  if (updateChecked || !lastStatus) return;
  updateChecked = true;
  const current = parseVersion(lastStatus.firmware);
  try {
    const r = await fetch(RELEASES, { headers: { Accept: "application/vnd.github+json" } });
    if (!r.ok || !current) return;
    let best = null;
    for (const rel of await r.json()) {
      const v = parseVersion(rel.tag_name);
      const asset = (rel.assets || []).find((a) => a.name === "ps-hid-bridge.uf2");
      if (!rel.draft && v && asset && (!best || newer(v, best.v))) best = { v, rel, asset };
    }
    if (!best || !newer(best.v, current)) return;
    const p = $("fw-update");
    p.textContent = `Version ${best.v.join(".")} is available: `;
    const a = document.createElement("a");
    a.href = best.asset.browser_download_url;
    a.textContent = "download ps-hid-bridge.uf2";
    p.append(a, " and upload it below, or update from the desktop app (Firmware & updates).");
    p.hidden = false;
  } catch (e) {
    /* offline or blocked: no hint */
  }
}

function selectTab(name) {
  for (const a of document.querySelectorAll(".tab")) {
    if (a.getAttribute("href") === "#" + name) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
  for (const p of document.querySelectorAll(".tabpane")) p.hidden = p.id !== "t-" + name;
  if (lastStatus && name !== "status") fillForms(lastStatus);
  if (name === "system") checkForUpdate();
}

function fmtUptime(ms) {
  let s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400); s %= 86400;
  const h = Math.floor(s / 3600); s %= 3600;
  const m = Math.floor(s / 60); s %= 60;
  if (d) return `${d} d ${h} h`;
  if (h) return `${h} h ${m} min`;
  return `${m} min ${s} s`;
}

// [title, explanation, state colour]
function boardState(st) {
  if (!st.usb_mounted) return ["PlayStation not connected", "Connect the board to the PlayStation's USB port.", "bad"];
  if (st.usb_suspended) return ["PlayStation in rest mode", "USB is connected, the PlayStation sleeps.", "warn"];
  if (st.input_active) return ["Input active", "The app is sending input.", "ok"];
  return ["Ready", "Waiting for input from the app.", "accent"];
}

function renderStatus(st) {
  lastStatus = st;
  const [title, text, level] = boardState(st);
  const p = $("pill");
  p.hidden = false;
  p.textContent = title;
  p.className = "pill " + level;
  $("s-input").textContent = title;
  $("s-usb").textContent = text;
  $("h-icon").className = "badge " + level;
  $("s-nokey").hidden = st.psk_set;
  $("s-ip").textContent = st.ip;
  $("s-net").textContent = `${st.net_state} · ${st.ip_mode === "static" ? "static address" : "DHCP"}`;
  $("s-fw").textContent = st.firmware;
  $("s-uptime").textContent = `up ${fmtUptime(st.uptime_ms)} · slot ${st.partition}`;
  $("fw-current").textContent = `${st.firmware} · slot ${st.partition}`;
  const ms = (us) => (us / 1000).toFixed(2) + " ms";
  $("s-lat").textContent = st.lat_usb_count ? ms(st.lat_usb_p50_us) : "–";
  $("s-lat-sub").textContent = st.lat_usb_count
    ? `median · 95 % below ${ms(st.lat_usb_p95_us)}` : "no input yet";
  $("s-lat-n").textContent = st.lat_usb_count ? `${st.lat_usb_count}, slowest ${ms(st.lat_usb_max_us)}` : "–";
  $("s-lat-rx").textContent = st.lat_rx_max_us ? `median ${st.lat_rx_p50_us} µs, max ${st.lat_rx_max_us} µs` : "–";
  $("s-lat-waits").textContent = st.mouse_waits;
  for (const [id, n, bad] of [["s-ok", st.rx_ok, false], ["s-auth", st.rx_auth_fail, true],
    ["s-replay", st.rx_replay, true], ["s-filtered", st.rx_filtered, true], ["s-malformed", st.rx_malformed, true]]) {
    $(id).textContent = n;
    $(id).parentElement.className = bad && n > 0 ? "warn" : "";
  }
  $("s-mac").textContent = st.mac;
  $("s-gw").textContent = `${st.gateway} / ${st.dns}`;
  $("s-port").textContent = st.udp_port;
  $("s-psk").textContent = st.psk_set ? "set" : "missing – generate one under “Security”";
  $("s-allowed").textContent = st.allowed_sender || "any sender";
  $("psk-state").textContent = st.psk_set ? "set" : "no key yet";
  $("psk-state").className = "pill " + (st.psk_set ? "ok" : "warn");
}

function fillForms(st) {
  const f = $("f-network");
  f.mode.value = st.ip_mode;
  f.ip.value = st.ip_mode === "static" ? st.static_ip : st.ip;
  f.mask.value = st.ip_mode === "static" ? st.static_mask : st.mask;
  f.gateway.value = st.ip_mode === "static" ? st.static_gateway : st.gateway;
  f.dns.value = st.ip_mode === "static" ? st.static_dns : st.dns;
  f.udp_port.value = st.udp_port;
  updateStaticFields();
  $("f-allowed").ip.value = st.allowed_sender;
}

function updateStaticFields() {
  const isStatic = $("f-network").mode.value === "static";
  $("static-fields").hidden = !isStatic;
  for (const el of $("static-fields").querySelectorAll("input")) el.disabled = !isStatic;
}

async function refreshStatus(fill) {
  try {
    const st = await api("/api/status");
    renderStatus(st);
    if (fill) fillForms(st);
  } catch (e) {
    $("pill").textContent = "no connection";
    $("pill").className = "pill bad";
  }
}

function startStatus() {
  stopStatus();
  statusTimer = setInterval(() => refreshStatus(false), 2000);
}

function stopStatus() {
  if (statusTimer) clearInterval(statusTimer);
  statusTimer = null;
}

async function enterMain() {
  show("v-main");
  selectTab(tabName());
  await refreshStatus(true);
  if (tabName() === "system") checkForUpdate();
  startStatus();
}

function formBusy(form, busy) {
  for (const b of form.querySelectorAll("button")) b.disabled = busy;
}

function onSubmit(id, handler) {
  $(id).addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const form = ev.target;
    formBusy(form, true);
    try {
      await handler(form);
    } catch (e) {
      message(e.message, false);
    } finally {
      formBusy(form, false);
    }
  });
}

function waitForReboot(text) {
  stopStatus();
  message(text, true);
  let tries = 0;
  const poll = setInterval(async () => {
    tries++;
    try {
      const st = await api("/api/state");
      clearInterval(poll);
      message("The board is back.", true);
      if (st.logged_in) enterMain(); else showLogin();
    } catch (e) {
      if (tries > 30) {
        clearInterval(poll);
        message("The board no longer answers at this address. If the IP changed, open the new address in the browser.", false);
      }
    }
  }, 2000);
}

// CRC-32 (IEEE) -- crypto.subtle is not available on plain-HTTP pages.
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(bytes) {
  let c = 0xFFFFFFFF;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xFF] ^ (c >>> 8);
  return (c ^ 0xFFFFFFFF) >>> 0;
}

function uploadFirmware(data, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", "/api/ota");
    xhr.setRequestHeader("X-PSHB", "1");
    xhr.setRequestHeader("X-PSHB-CRC32", crc32(data).toString(16));
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => { if (e.lengthComputable) onProgress(100 * e.loaded / e.total); };
    xhr.onload = () => {
      let body = {};
      try { body = JSON.parse(xhr.responseText); } catch (e) { /* ignore */ }
      if (xhr.status === 200) resolve(body); else reject(new Error(body.error || "Error " + xhr.status));
    };
    xhr.onerror = () => reject(new Error("Connection lost."));
    xhr.send(data);
  });
}

function init() {
  onSubmit("f-login", async (f) => {
    const r = await api("/api/login", { password: f.password.value });
    f.reset();
    message("");
    if (r.must_change_password) show("v-first"); else enterMain();
  });

  const pwChange = async (f) => {
    if (f.new.value !== f.new2.value) throw new Error("The two new passwords do not match.");
    await api("/api/password", { old: f.old.value, new: f.new.value });
    f.reset();
  };
  onSubmit("f-first", async (f) => {
    await pwChange(f);
    message("Password saved.", true);
    enterMain();
  });
  onSubmit("f-password", async (f) => {
    await pwChange(f);
    message("Password changed.", true);
  });

  onSubmit("f-network", async (f) => {
    const body = { mode: f.mode.value, udp_port: Number(f.udp_port.value) };
    if (body.mode === "static") {
      Object.assign(body, { ip: f.ip.value.trim(), mask: f.mask.value.trim(),
        gateway: f.gateway.value.trim(), dns: f.dns.value.trim() });
    }
    await api("/api/network", body);
    waitForReboot(body.mode === "static"
      ? `Saved. The board restarts and will then be reachable at ${body.ip}.`
      : "Saved. The board restarts and gets an address via DHCP.");
  });
  for (const r of $("f-network").querySelectorAll("input[name=mode]")) r.addEventListener("change", updateStaticFields);

  $("b-psk-gen").addEventListener("click", async () => {
    try {
      const r = await api("/api/psk/generate", {});
      $("psk-value").textContent = r.psk;
      $("psk-show").hidden = false;
      message("The new key is active. The app needs it, otherwise the board will no longer accept input.", true);
      refreshStatus(false);
    } catch (e) { message(e.message, false); }
  });
  $("b-psk-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("psk-value").textContent);
      message("Key copied.", true);
    } catch (e) {
      const range = document.createRange();
      range.selectNodeContents($("psk-value"));
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      message("Copying is not possible – the key is selected, please copy it with Ctrl+C.", false);
    }
  });
  onSubmit("f-psk", async (f) => {
    await api("/api/psk", { psk: f.psk.value.trim() });
    f.reset();
    message("Key applied.", true);
    refreshStatus(false);
  });
  onSubmit("f-allowed", async (f) => {
    await api("/api/allowed-sender", { ip: f.ip.value.trim() });
    message(f.ip.value.trim() ? "Sender filter saved." : "Sender filter off: any sender allowed.", true);
    // Show the stored form, e.g. "192.168.1.20/24" is saved as "192.168.1.0/24".
    const st = await api("/api/status");
    renderStatus(st);
    f.ip.value = st.allowed_sender;
  });

  onSubmit("f-ota", async (f) => {
    const file = f.file.files[0];
    if (!file) throw new Error("Please choose a UF2 file.");
    if (!file.name.toLowerCase().endsWith(".uf2")) throw new Error("Please choose the file ps-hid-bridge.uf2.");
    const data = new Uint8Array(await file.arrayBuffer());
    const bar = $("ota-progress");
    bar.hidden = false;
    bar.value = 0;
    stopStatus();
    try {
      await uploadFirmware(data, (p) => { bar.value = p; });
    } catch (e) {
      startStatus();  // the board keeps running the old firmware
      throw e;
    } finally {
      bar.hidden = true;
    }
    f.reset();
    waitForReboot("Firmware written. The board starts the new version …");
  });

  $("b-reboot").addEventListener("click", async () => {
    try {
      await api("/api/reboot", {});
      waitForReboot("The board is restarting …");
    } catch (e) { message(e.message, false); }
  });
  $("b-reset").addEventListener("click", () => { $("reset-confirm").hidden = false; });
  $("b-reset-no").addEventListener("click", () => { $("reset-confirm").hidden = true; });
  $("b-reset-yes").addEventListener("click", async () => {
    $("reset-confirm").hidden = true;
    try {
      await api("/api/factory-reset", {});
      waitForReboot("Restoring factory settings, the board is restarting …");
    } catch (e) { message(e.message, false); }
  });

  $("logout").addEventListener("click", async () => {
    try { await api("/api/logout", {}); } catch (e) { /* ignore */ }
    showLogin();
  });
  window.addEventListener("hashchange", () => {
    if ($("v-main").hidden) return;
    message("");
    selectTab(tabName());
  });

  api("/api/state").then((st) => {
    if (!st.logged_in) showLogin();
    else if (st.must_change_password) show("v-first");
    else enterMain();
  }).catch((e) => { showLogin(); message(e.message, false); });
}

document.addEventListener("DOMContentLoaded", init);
