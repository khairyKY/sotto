// Sotto settings — wires the UI to the Rust backend over Tauri commands, with a
// self-contained browser-preview fallback (mock data, no persistence) so the
// page can be eyeballed without the app running.

const T = window.__TAURI__;
const hasTauri = !!(T && T.core);

// Mock state used only in browser preview.
const mock = {
  hotkey: "ControlRight",
  activation: "hold",
  polish: "ai",
  threshold: 18,
  launchLogin: true,
  startHidden: true,
  dictionary: [
    { spoken: "gee pee tee", replacement: "GPT" },
    { spoken: "my email", replacement: "dev@sotto.app" },
    { spoken: "arrow", replacement: "→" },
  ],
  history: [
    { time: "2:14 PM", text: "Let's ship the overlay states first, then wire up the settings window." },
    { time: "1:58 PM", text: "dev@sotto.app" },
    { time: "11:02 AM", text: "Refactor the polish tier so short clips skip the LLM round-trip." },
  ],
  models: [
    { name: "Parakeet v3", variant: "· English", meta: "NVIDIA · int8 quantized", state: "installed", size: "670 MB", selected: true },
    { name: "Parakeet v3", variant: "· multilingual", meta: "25 languages · int8", state: "download", size: "720 MB" },
    { name: "Whisper large-v3", variant: "", meta: "", state: "downloading", progress: 62 },
  ],
};

async function invoke(cmd, args) {
  if (hasTauri) return T.core.invoke(cmd, args);
  console.log("[mock invoke]", cmd, args || "");
}
async function getSettings() {
  if (hasTauri) return T.core.invoke("get_settings");
  return mock;
}

const HOTKEY_LABEL = {
  ControlRight: "Right Ctrl", ControlLeft: "Left Ctrl", AltGr: "Right Alt",
  Alt: "Left Alt", ShiftRight: "Right Shift", ShiftLeft: "Left Shift", CapsLock: "Caps Lock",
};
// Browser keydown event.code → our config key name.
const CODE_TO_KEY = {
  ControlRight: "ControlRight", ControlLeft: "ControlLeft", AltRight: "AltGr",
  AltLeft: "Alt", ShiftRight: "ShiftRight", ShiftLeft: "ShiftLeft", CapsLock: "CapsLock",
};

const $ = (id) => document.getElementById(id);

// ── window controls ────────────────────────────────────────────
if (hasTauri && T.window) {
  const w = T.window.getCurrentWindow();
  $("win-min").onclick = () => w.minimize();
  $("win-max").onclick = () => w.toggleMaximize();
  $("win-close").onclick = () => w.hide(); // hide, not quit — app stays in tray
} else {
  $("win-close").onclick = () => window.close();
}

// ── segmented control helper ───────────────────────────────────
function initSegmented(el, onPick) {
  el.querySelectorAll("button").forEach((b) => {
    b.onclick = () => {
      if (b.dataset.value === el.dataset.value) return;
      el.dataset.value = b.dataset.value;
      el.querySelectorAll("button").forEach((x) => x.classList.toggle("active", x === b));
      onPick(b.dataset.value);
    };
  });
}
function selectSegment(el, value) {
  el.dataset.value = value;
  el.querySelectorAll("button").forEach((x) => x.classList.toggle("active", x.dataset.value === value));
}

// ── switch helper ──────────────────────────────────────────────
function initSwitch(el, onToggle) {
  el.onclick = () => {
    const on = el.getAttribute("aria-checked") !== "true";
    el.setAttribute("aria-checked", String(on));
    onToggle(on);
  };
}

// ── renderers ──────────────────────────────────────────────────
function renderModels(models) {
  const host = $("model-list");
  host.innerHTML = "";
  models.forEach((m, i) => {
    if (i) host.insertAdjacentHTML("beforeend", '<div class="divider"></div>');
    const row = document.createElement("div");
    row.className = "model-row";
    let right = "";
    if (m.state === "installed") right = `<span class="pill">Installed</span><span class="size">${m.size}</span>`;
    else if (m.state === "download") right = `<button class="btn">Download</button><span class="size">${m.size}</span>`;
    else if (m.state === "downloading")
      right = `<div style="display:flex;align-items:center;gap:9px"><span style="display:block;width:150px;height:4px;border-radius:2px;background:#33323a;overflow:hidden"><span style="display:block;width:${m.progress}%;height:100%;background:var(--accent)"></span></span><span class="mono" style="width:auto">downloading · ${m.progress}%</span></div>`;
    row.innerHTML = `
      <span class="radio ${m.selected ? "on" : ""}"></span>
      <div class="grow"><div class="name">${m.name} <span class="sub">${m.variant}</span></div>${m.meta ? `<div class="meta">${m.meta}</div>` : ""}</div>
      ${right}`;
    host.appendChild(row);
  });
}

function renderDictionary(entries) {
  const host = $("dict-rows");
  host.innerHTML = "";
  entries.forEach((e, i) => {
    const row = document.createElement("div");
    row.className = "dict-row";
    row.innerHTML = `
      <input class="spoken" value="${escapeHtml(e.spoken)}" placeholder="spoken" />
      <span class="arrow">→</span>
      <input class="replacement" value="${escapeHtml(e.replacement)}" placeholder="replacement" />
      <span class="del" title="Remove">✕</span>`;
    const commit = () => saveDictionary();
    row.querySelector(".spoken").oninput = commit;
    row.querySelector(".replacement").oninput = commit;
    row.querySelector(".del").onclick = () => { row.remove(); saveDictionary(); };
    host.appendChild(row);
  });
}
function collectDictionary() {
  return [...document.querySelectorAll("#dict-rows .dict-row")]
    .map((r) => ({ spoken: r.querySelector(".spoken").value, replacement: r.querySelector(".replacement").value }))
    .filter((e) => e.spoken.trim() !== "");
}
function saveDictionary() { invoke("set_dictionary", { entries: collectDictionary() }); }

function renderHistory(entries) {
  const host = $("history-list");
  host.innerHTML = "";
  if (!entries.length) {
    host.innerHTML = '<div class="hist-empty">Nothing dictated yet this session</div>';
    return;
  }
  entries.forEach((e, i) => {
    if (i) host.insertAdjacentHTML("beforeend", '<div class="divider"></div>');
    const row = document.createElement("div");
    row.className = "hist-row";
    row.innerHTML = `<span class="time">${e.time}</span><span class="txt">${escapeHtml(e.text)}</span><span class="copy">⧉</span>`;
    row.onclick = () => copyHistory(e.text);
    host.appendChild(row);
  });
}
async function copyHistory(text) {
  if (hasTauri) invoke("copy_text", { text });
  else if (navigator.clipboard) navigator.clipboard.writeText(text).catch(() => {});
}

function escapeHtml(s) { return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }

// ── hotkey rebind (press-any-key capture) ──────────────────────
let capturing = false;
function setKeycap(key) { $("hotkey-keycap").textContent = HOTKEY_LABEL[key] || key; }
$("hotkey-rebind").onclick = () => {
  if (capturing) return;
  capturing = true;
  const cap = $("hotkey-keycap");
  cap.textContent = "Press a key…";
  cap.style.borderStyle = "dashed";
  const onKey = (ev) => {
    const key = CODE_TO_KEY[ev.code];
    window.removeEventListener("keydown", onKey, true);
    document.removeEventListener("click", onCancel, true);
    capturing = false;
    cap.style.borderStyle = "";
    if (key) { setKeycap(key); invoke("set_hotkey", { key }); }
    ev.preventDefault();
  };
  const onCancel = (ev) => {
    if (ev.target === $("hotkey-rebind")) return;
    window.removeEventListener("keydown", onKey, true);
    document.removeEventListener("click", onCancel, true);
    capturing = false;
    cap.style.borderStyle = "";
  };
  window.addEventListener("keydown", onKey, true);
  setTimeout(() => document.addEventListener("click", onCancel, true), 0);
};

// ── threshold slider ───────────────────────────────────────────
const slider = $("threshold");
function setThresholdUI(v) {
  slider.value = v;
  $("threshold-val").textContent = `${v} words`;
  slider.style.setProperty("--fill", `${(v / 60) * 100}%`);
}
slider.oninput = () => { setThresholdUI(+slider.value); };
slider.onchange = () => invoke("set_threshold", { words: +slider.value });

// ── boot ───────────────────────────────────────────────────────
async function boot() {
  const s = await getSettings();
  if (s.theme) document.documentElement.dataset.theme = s.theme;

  setKeycap(s.hotkey);
  selectSegment($("activation"), s.activation);
  selectSegment($("polish"), s.polish);
  setThresholdUI(s.threshold);
  renderModels(s.models || []);
  renderDictionary(s.dictionary || []);
  renderHistory(s.history || []);
  $("launch-login").setAttribute("aria-checked", String(!!s.launchLogin));
  $("start-hidden").setAttribute("aria-checked", String(!!s.startHidden));

  initSegmented($("activation"), (v) => invoke("set_activation", { mode: v }));
  initSegmented($("polish"), (v) => invoke("set_polish", { mode: v }));
  initSwitch($("launch-login"), (on) => invoke("set_launch_login", { enabled: on }));
  initSwitch($("start-hidden"), (on) => invoke("set_start_hidden", { enabled: on }));
  $("dict-add").onclick = () => {
    const host = $("dict-rows");
    const row = document.createElement("div");
    row.className = "dict-row";
    row.innerHTML = `<input class="spoken" placeholder="spoken" /><span class="arrow">→</span><input class="replacement" placeholder="replacement" /><span class="del" title="Remove">✕</span>`;
    row.querySelector(".spoken").oninput = saveDictionary;
    row.querySelector(".replacement").oninput = saveDictionary;
    row.querySelector(".del").onclick = () => { row.remove(); saveDictionary(); };
    host.appendChild(row);
    row.querySelector(".spoken").focus();
  };

  // Live overlay/history refresh from Rust (optional).
  if (hasTauri && T.event) {
    T.event.listen("history-updated", (e) => renderHistory(e.payload || []));
  }

  initUpdates();
  initAssets();
}

// ── app updates (toast + one-click install) ────────────────────
async function initUpdates() {
  const banner = $("update-banner");
  const statusEl = $("update-status");
  const verEl = $("app-version");
  try {
    const v = hasTauri && T.app ? await T.app.getVersion() : "0.1.0";
    verEl.textContent = "v" + v;
  } catch { verEl.textContent = ""; }

  const showUpdate = (version) => {
    $("update-text").textContent = `Sotto v${version} is available.`;
    banner.hidden = false;
  };
  async function refresh(manual) {
    statusEl.textContent = "Checking for updates…";
    const latest = await invoke("check_update"); // undefined in browser preview
    if (latest && latest.version) {
      showUpdate(latest.version);
      statusEl.textContent = `Update available: v${latest.version}`;
    } else {
      banner.hidden = true;
      statusEl.textContent = manual ? "You're on the latest version" : "Up to date";
    }
  }

  $("update-install").onclick = async () => {
    $("update-install").disabled = true;
    $("update-text").textContent = "Downloading update…";
    try {
      await invoke("install_update"); // app relaunches on success
    } catch (e) {
      $("update-text").textContent = "Update failed: " + e;
      $("update-install").disabled = false;
    }
  };
  $("update-dismiss").onclick = () => { banner.hidden = true; };
  $("check-update").onclick = () => refresh(true);

  if (hasTauri && T.event) {
    T.event.listen("update-available", (e) => showUpdate(e.payload));
    T.event.listen("update-progress", (e) => {
      const [d, t] = e.payload || [0, 0];
      const pct = t ? Math.round((d / t) * 100) : 0;
      $("update-text").textContent = `Downloading update… ${pct}%`;
    });
  }
  refresh(false);
}

// ── first-run model download ───────────────────────────────────
async function initAssets() {
  const banner = $("assets-banner");
  const fill = $("assets-fill");
  const text = $("assets-text");
  const status = await invoke("assets_status"); // undefined in browser preview
  if (!status || status.ready) { banner.hidden = true; return; }

  banner.hidden = false;
  text.textContent = "Downloading voice models… (first run)";

  if (hasTauri && T.event) {
    T.event.listen("asset-progress", (e) => {
      const p = e.payload || {};
      const pct = p.total ? Math.round((p.received / p.total) * 100) : 0;
      fill.style.width = pct + "%";
      text.textContent = `Downloading ${p.name}… ${pct}%`;
    });
    T.event.listen("assets-ready", () => {
      fill.style.width = "100%";
      text.textContent = "Models ready.";
      setTimeout(() => (banner.hidden = true), 1500);
    });
    T.event.listen("asset-error", (e) => {
      text.textContent = "Download failed: " + e.payload + " — restart Sotto to retry.";
    });
  }
}
boot();
