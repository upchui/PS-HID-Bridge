// PS5 HID Bridge – Weboberfläche des Boards.
"use strict";

const $ = (id) => document.getElementById(id);
let statusTimer = null;
let lastStatus = null;

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
    throw new Error("Board nicht erreichbar.");
  }
  let data = {};
  try { data = await res.json(); } catch (e) { /* empty body */ }
  if (res.status === 401 && path !== "/api/login") {
    showLogin();
    throw new Error(data.error || "Bitte anmelden.");
  }
  if (!res.ok) throw new Error(data.error || "Fehler " + res.status);
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

function selectTab(name) {
  for (const b of document.querySelectorAll(".tab")) {
    b.setAttribute("aria-selected", b.dataset.tab === name ? "true" : "false");
  }
  for (const p of document.querySelectorAll(".tabpane")) p.hidden = p.id !== "t-" + name;
  message("");
}

function fmtUptime(ms) {
  let s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400); s %= 86400;
  const h = Math.floor(s / 3600); s %= 3600;
  const m = Math.floor(s / 60); s %= 60;
  if (d) return `${d} T ${h} Std`;
  if (h) return `${h} Std ${m} Min`;
  return `${m} Min ${s} s`;
}

function setPill(st) {
  const p = $("pill");
  p.hidden = false;
  if (!st.usb_mounted) { p.textContent = "PS5 nicht verbunden"; p.className = "pill bad"; }
  else if (st.usb_suspended) { p.textContent = "PS5 im Ruhemodus"; p.className = "pill warn"; }
  else if (st.input_active) { p.textContent = "Eingaben aktiv"; p.className = "pill ok"; }
  else { p.textContent = "Bereit"; p.className = "pill ok"; }
}

function renderStatus(st) {
  lastStatus = st;
  setPill(st);
  $("s-input").textContent = st.input_active ? "Aktiv" : "Keine";
  $("s-usb").textContent = !st.usb_mounted ? "USB: nicht verbunden"
    : st.usb_suspended ? "USB: Ruhemodus" : "USB: verbunden";
  $("s-ip").textContent = st.ip;
  $("s-net").textContent = `${st.net_state} · ${st.ip_mode === "static" ? "feste Adresse" : "DHCP"}`;
  $("s-fw").textContent = st.firmware;
  $("s-uptime").textContent = `Läuft seit ${fmtUptime(st.uptime_ms)} · Slot ${st.partition}`;
  $("s-ok").textContent = st.rx_ok;
  $("s-auth").textContent = st.rx_auth_fail;
  $("s-replay").textContent = st.rx_replay;
  $("s-filtered").textContent = st.rx_filtered;
  $("s-malformed").textContent = st.rx_malformed;
  $("s-mac").textContent = st.mac;
  $("s-gw").textContent = `${st.gateway} / ${st.dns}`;
  $("s-port").textContent = st.udp_port;
  $("s-psk").textContent = st.psk_set ? "gesetzt" : "fehlt – unter „Sicherheit“ erzeugen";
  $("s-allowed").textContent = st.allowed_sender || "alle Absender";
  $("psk-state").textContent = st.psk_set ? "gesetzt" : "noch kein Schlüssel";
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
  for (const el of $("static-fields").querySelectorAll("input")) el.disabled = !isStatic;
}

async function refreshStatus(fill) {
  try {
    const st = await api("/api/status");
    renderStatus(st);
    if (fill) fillForms(st);
  } catch (e) {
    $("pill").textContent = "keine Verbindung";
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
  selectTab("status");
  await refreshStatus(true);
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
      message("Board ist wieder da.", true);
      if (st.logged_in) enterMain(); else showLogin();
    } catch (e) {
      if (tries > 30) {
        clearInterval(poll);
        message("Das Board antwortet nicht mehr unter dieser Adresse. Bei geänderter IP die neue Adresse im Browser öffnen.", false);
      }
    }
  }, 2000);
}

function init() {
  onSubmit("f-login", async (f) => {
    const r = await api("/api/login", { password: f.password.value });
    f.reset();
    message("");
    if (r.must_change_password) show("v-first"); else enterMain();
  });

  const pwChange = async (f) => {
    if (f.new.value !== f.new2.value) throw new Error("Die beiden neuen Passwörter stimmen nicht überein.");
    await api("/api/password", { old: f.old.value, new: f.new.value });
    f.reset();
  };
  onSubmit("f-first", async (f) => {
    await pwChange(f);
    message("Passwort gespeichert.", true);
    enterMain();
  });
  onSubmit("f-password", async (f) => {
    await pwChange(f);
    message("Passwort geändert.", true);
  });

  onSubmit("f-network", async (f) => {
    const body = { mode: f.mode.value, udp_port: Number(f.udp_port.value) };
    if (body.mode === "static") {
      Object.assign(body, { ip: f.ip.value.trim(), mask: f.mask.value.trim(),
        gateway: f.gateway.value.trim(), dns: f.dns.value.trim() });
    }
    await api("/api/network", body);
    waitForReboot(body.mode === "static"
      ? `Gespeichert. Das Board startet neu und ist dann unter ${body.ip} erreichbar.`
      : "Gespeichert. Das Board startet neu und holt sich eine Adresse per DHCP.");
  });
  for (const r of $("f-network").querySelectorAll("input[name=mode]")) r.addEventListener("change", updateStaticFields);

  $("b-psk-gen").addEventListener("click", async () => {
    try {
      const r = await api("/api/psk/generate", {});
      $("psk-value").textContent = r.psk;
      $("psk-show").hidden = false;
      message("Neuer Schlüssel ist aktiv. Die App braucht ihn, sonst nimmt das Board keine Eingaben mehr an.", true);
      refreshStatus(false);
    } catch (e) { message(e.message, false); }
  });
  $("b-psk-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("psk-value").textContent);
      message("Schlüssel kopiert.", true);
    } catch (e) {
      const range = document.createRange();
      range.selectNodeContents($("psk-value"));
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      message("Kopieren nicht möglich – der Schlüssel ist markiert, bitte mit Strg+C kopieren.", false);
    }
  });
  onSubmit("f-psk", async (f) => {
    await api("/api/psk", { psk: f.psk.value.trim() });
    f.reset();
    message("Schlüssel übernommen.", true);
    refreshStatus(false);
  });
  onSubmit("f-allowed", async (f) => {
    await api("/api/allowed-sender", { ip: f.ip.value.trim() });
    message(f.ip.value.trim() ? "Absenderfilter gespeichert." : "Absenderfilter aus: alle Absender erlaubt.", true);
    refreshStatus(false);
  });

  $("b-reboot").addEventListener("click", async () => {
    try {
      await api("/api/reboot", {});
      waitForReboot("Das Board startet neu …");
    } catch (e) { message(e.message, false); }
  });
  $("b-reset").addEventListener("click", () => { $("reset-confirm").hidden = false; });
  $("b-reset-no").addEventListener("click", () => { $("reset-confirm").hidden = true; });
  $("b-reset-yes").addEventListener("click", async () => {
    $("reset-confirm").hidden = true;
    try {
      await api("/api/factory-reset", {});
      waitForReboot("Werkseinstellungen werden wiederhergestellt, das Board startet neu …");
    } catch (e) { message(e.message, false); }
  });

  $("logout").addEventListener("click", async () => {
    try { await api("/api/logout", {}); } catch (e) { /* ignore */ }
    showLogin();
  });
  for (const b of document.querySelectorAll(".tab")) {
    b.addEventListener("click", () => {
      selectTab(b.dataset.tab);
      if (lastStatus && b.dataset.tab !== "status") fillForms(lastStatus);
    });
  }

  api("/api/state").then((st) => {
    if (!st.logged_in) showLogin();
    else if (st.must_change_password) show("v-first");
    else enterMain();
  }).catch((e) => { showLogin(); message(e.message, false); });
}

document.addEventListener("DOMContentLoaded", init);
