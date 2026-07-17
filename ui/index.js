// Sotto app shell — page routing, settings, insights, and all UI wiring.

const T = window.__TAURI__;
const hasTauri = !!(T && T.core);

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
  tone: "",
  appTones: [],
  history: [
    { time: "2:14 PM", text: "Let's ship the overlay states first." },
    { time: "1:58 PM", text: "dev@sotto.app" },
    { time: "11:02 AM", text: "Refactor the polish tier." },
  ],
  models: [
    { id: "parakeet-v3", name: "Parakeet v3", variant: "· English", meta: "NVIDIA · int8 quantized", state: "installed", size: "639 MB", selected: true },
    { id: "whisper-turbo", name: "Whisper turbo", variant: "· 99 languages", meta: "OpenAI · large-v3-turbo q5_0", state: "download", size: "547 MB", selected: false },
    { id: "egyptian-small", name: "Egyptian Arabic", variant: "· عامية + English", meta: "Whisper small · code-switch tuned", state: "download", size: "465 MB", selected: false },
  ],
  asrModel: "parakeet-v3",
  asrLanguage: "auto",
};

async function invoke(cmd, args) {
  if (hasTauri) return T.core.invoke(cmd, args);
  console.log("[mock invoke]", cmd, args || "");
}
async function getSettings() {
  if (hasTauri) return T.core.invoke("get_settings");
  return mock;
}

let HOTKEY_LABELS = {};
let HOTKEY_RISKY = {};

const $ = (id) => document.getElementById(id);

// ── settings modal ──
// Settings opens over the app (shell dimmed behind) rather than replacing the
// content area, so you never lose your place in Home/Insights/etc.
function openSettings() {
  $("settings-scrim").hidden = false;
  document.querySelector('.nav-item[data-page="settings"]')?.classList.add("active");
}
function closeSettings() {
  $("settings-scrim").hidden = true;
  document.querySelector('.nav-item[data-page="settings"]')?.classList.remove("active");
}
const settingsOpen = () => !$("settings-scrim").hidden;

// ── page routing ──
function navigate(page) {
  if (page === 'settings') { openSettings(); return; }
  document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
  document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
  const pg = $(`page-${page}`);
  if (pg) pg.classList.add('active');
  const nav = document.querySelector(`.nav-item[data-page="${page}"]`);
  if (nav) nav.classList.add('active');
  if (page === 'insights') loadInsights();
  if (page === 'history') loadHistory();
  if (page === 'home') loadHome();
}
document.querySelectorAll('.nav-item').forEach(item => {
  item.onclick = (e) => { e.preventDefault(); navigate(item.dataset.page); };
});
$("settings-modal-close").onclick = closeSettings;
$("settings-scrim").onclick = (e) => { if (e.target === $("settings-scrim")) closeSettings(); };
// Escape closes the topmost layer only — the hotkey picker sits above settings.
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (!$("hotkey-modal").hidden) return; // its own handler deals with it
  if (settingsOpen()) { e.preventDefault(); closeSettings(); }
});

// ── window controls ──
if (hasTauri && T.window) {
  const w = T.window.getCurrentWindow();
  $("win-min").onclick = () => w.minimize();
  $("win-max").onclick = () => w.toggleMaximize();
  $("win-close").onclick = () => w.hide();
} else {
  $("win-close").onclick = () => window.close();
}

// ── greetings ──
function setGreeting() {
  const h = new Date().getHours();
  const g = h < 12 ? "Good morning." : h < 17 ? "Good afternoon." : "Good evening.";
  $("greeting").textContent = g;
}
setGreeting();

// ── segmented control helper ──
function initSegmented(el, onPick) {
  el.querySelectorAll("button").forEach(b => {
    b.onclick = () => {
      if (b.dataset.value === el.dataset.value) return;
      el.dataset.value = b.dataset.value;
      el.querySelectorAll("button").forEach(x => x.classList.toggle("active", x === b));
      onPick(b.dataset.value);
    };
  });
}
function selectSegment(el, value) {
  el.dataset.value = value;
  el.querySelectorAll("button").forEach(x => x.classList.toggle("active", x.dataset.value === value));
}

function initSwitch(el, onToggle) {
  el.onclick = () => {
    const on = el.getAttribute("aria-checked") !== "true";
    el.setAttribute("aria-checked", String(on));
    onToggle(on);
  };
}

// ── insights period toggle ──
const periodToggle = $("insights-period");
periodToggle.querySelectorAll("button").forEach(b => {
  b.onclick = () => {
    if (b.dataset.value === periodToggle.dataset.value) return;
    periodToggle.dataset.value = b.dataset.value;
    periodToggle.querySelectorAll("button").forEach(x => x.classList.toggle("active", x === b));
    loadInsights();
  };
});

// ── home stats ──
async function loadHome() {
  try {
    const stats = hasTauri ? await invoke("get_stats") : null;
    if (stats) {
      $("stat-words").textContent = stats.wordsThisWeek?.toLocaleString() || "0";
      $("stat-wpm").textContent = stats.avgWpm30d || "0";
      $("stat-streak").innerHTML = (stats.currentStreak || 0) + '<span class="stat-suffix">d</span>';
    }
    const s = await getSettings();
    if (s && s.hotkey) {
      $("keycap-display").textContent = HOTKEY_LABELS[s.hotkey] || s.hotkey;
    }
    renderRecent(s?.history || []);
    updateStatusBar(s);
  } catch {}
}
function updateStatusBar(s) {
  const parts = [];
  if (s?.models?.length) {
    const sel = s.models.find(m => m.selected);
    if (sel) parts.push(sel.name);
  }
  if (s?.polish === "ai") parts.push("AI polish on");
  else if (s?.polish === "rules") parts.push("rules polish");
  else parts.push("polish off");
  parts.push("mic: " + (s?.microphone || "system default"));
  $("status-text").textContent = parts.join(" · ");
}

function renderRecent(entries) {
  const host = $("recent-list");
  host.innerHTML = "";
  if (!entries || !entries.length) {
    host.innerHTML = '<div class="recent-empty">Nothing dictated yet this session</div>';
    return;
  }
  entries.forEach((e, i) => {
    const row = document.createElement("div");
    row.className = "recent-item";
    row.innerHTML = `
      <span class="recent-time">${e.time}</span>
      <span class="recent-text">${escapeHtml(e.text)}</span>
      <span class="recent-copy" title="Copy">⧉</span>
      <span class="recent-retry" title="Re-polish &amp; copy">↻</span>`;
    row.querySelector(".recent-copy").onclick = (ev) => { ev.stopPropagation(); copyText(e.text); };
    row.querySelector(".recent-retry").onclick = (ev) => { ev.stopPropagation(); invoke("repolish_copy", { text: e.text }); };
    host.appendChild(row);
  });
}

// ── insights ──
async function loadInsights() {
  try {
    const stats = hasTauri ? await invoke("get_stats") : null;
    if (!stats) { renderMockInsights(); return; }
    const period = $("insights-period").dataset.value;
    const wpm = stats.avgWpm30d || 0;
    $("speed-val").textContent = wpm;
    const offset = Math.max(0, 169.6 - (wpm / 200) * 169.6);
    $("speed-arc").setAttribute("stroke-dashoffset", offset);
    $("speed-pb").textContent = stats.bestWpm || "—";
    
    $("fixes-val").textContent = (stats.fixesTotal || 0).toLocaleString();
    const dictHits = stats.dictHitsTotal || 0;
    const wordCorrected = (stats.fixesTotal || 0) - dictHits;
    if ($("fixes-words-val")) $("fixes-words-val").textContent = Math.max(0, wordCorrected).toLocaleString();
    if ($("fixes-dict-val")) $("fixes-dict-val").textContent = dictHits.toLocaleString();

    $("total-words-val").textContent = (stats.totalWords || 0).toLocaleString();
    const periodWords = period === "week" ? (stats.wordsThisWeek || 0) : (stats.wordsThisMonth || 0);
    if ($("total-period-val")) $("total-period-val").textContent = periodWords.toLocaleString();
    const minSaved = Math.round((stats.totalWords || 0) * 0.02);
    if ($("total-saved-val")) $("total-saved-val").textContent = minSaved.toLocaleString();

    renderAppBreakdown(stats.topApps || []);
    renderStreakCalendar(stats.daily || [], stats.currentStreak || 0, stats.longestStreak || 0);
  } catch { renderMockInsights(); }
}
function renderMockInsights() {
  $("speed-val").textContent = "112";
  $("speed-arc").setAttribute("stroke-dashoffset", "48.4"); // (112/200) wpm offset
  $("fixes-val").textContent = "127";
  if ($("fixes-words-val")) $("fixes-words-val").textContent = "93";
  if ($("fixes-dict-val")) $("fixes-dict-val").textContent = "34";
  $("total-words-val").textContent = "8,420";
  const period = $("insights-period").dataset.value;
  if ($("total-period-val")) $("total-period-val").textContent = period === "week" ? "1,240" : "5,420";
  if ($("total-saved-val")) $("total-saved-val").textContent = "168";
  renderAppBreakdown([
    { name: "VS Code", words: 3420, pct: 41 },
    { name: "Chrome", words: 2180, pct: 26 },
    { name: "Word", words: 1520, pct: 18 },
    { name: "Slack", words: 840, pct: 10 },
  ]);
  renderStreakCalendar(mockStreakData(), 12, 21);
}
function mockStreakData() {
  const data = [];
  const today = localDayNum();
  for (let i = 0; i < 120; i++) {
    if (Math.random() > 0.35) {
      const words = Math.floor(Math.random() * 200) + 10;
      const d = new Date();
      d.setDate(d.getDate() - i);
      data.push({ date: d.toISOString().slice(0, 10), day: today - i, words });
    }
  }
  return data;
}
function renderAppBreakdown(apps) {
  const host = $("app-list");
  host.innerHTML = "";
  if (!apps.length) { host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No data yet</div>'; return; }
  apps.forEach(a => {
    const row = document.createElement("div");
    row.className = "app-row";
    row.innerHTML = `
      <span class="app-name">${escapeHtml(a.name)}</span>
      <div class="app-bar-wrap"><div class="app-bar-fill" style="width:${a.pct}%"></div></div>
      <span class="app-pct">${a.pct}%</span>
      <span class="app-words">${(a.words || 0).toLocaleString()}</span>`;
    host.appendChild(row);
  });
}
// Day number = days since 1970-01-01 for a *civil local* date. Mirrors
// stats.rs::local_today (days_from_civil over GetLocalTime) exactly —
// Date.now()/86400000 is UTC-based and drifts a day off it depending on
// timezone and time of day, which silently shifted every cell.
function localDayNum(date = new Date()) {
  return Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86400000);
}
// 1970-01-01 was a Thursday, so day 0 has weekday index 4 (0 = Sunday).
const dowOf = (day) => (((day % 7) + 4) % 7 + 7) % 7;
const MONTH_ABBR = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
const CAL_WEEKS = 19; // 7×19 = 133 cells, per the design doc

function renderStreakCalendar(daily, currentStreak, longestStreak) {
  const wrap = $("calendar-wrap");
  wrap.innerHTML = "";
  const today = localDayNum();
  const dayMap = {};
  daily.forEach(d => { dayMap[d.day] = Math.min(4, Math.ceil(d.words / 50)); });

  if ($("streak-title")) $("streak-title").textContent = `${currentStreak || 0}-day streak`;
  if ($("longest-streak-title")) {
    $("longest-streak-title").textContent = `longest ${longestStreak || 0} days`;
  }

  // Each column is a real calendar week (Sun→Sat top→bottom) and each row is
  // a fixed weekday, so today sits in the last column at its own weekday —
  // the previous version just chunked the last 371 days into arbitrary
  // 7-cell columns, which is why the grid read as starting nowhere sensible.
  const lastSunday = today - dowOf(today);
  const firstSunday = lastSunday - (CAL_WEEKS - 1) * 7;

  const months = document.createElement("div");
  months.className = "cal-months";
  const body = document.createElement("div");
  body.className = "cal-body";
  const dows = document.createElement("div");
  dows.className = "cal-dows";
  ["", "Mon", "", "Wed", "", "Fri", ""].forEach(l => {
    const s = document.createElement("span");
    s.textContent = l;
    dows.appendChild(s);
  });
  const grid = document.createElement("div");
  grid.className = "cal-grid";

  let prevMonth = -1;
  for (let w = 0; w < CAL_WEEKS; w++) {
    const colSunday = firstSunday + w * 7;
    // Month label when the month of this column's Sunday changes.
    const m = new Date(colSunday * 86400000).getUTCMonth();
    const label = document.createElement("span");
    label.textContent = m !== prevMonth ? MONTH_ABBR[m] : "";
    months.appendChild(label);
    prevMonth = m;

    for (let d = 0; d < 7; d++) {
      const day = colSunday + d;
      const cell = document.createElement("div");
      cell.className = "cal-cell";
      if (day > today) {
        // Future days in the current week: hold the grid shape, draw nothing.
        cell.classList.add("future");
      } else {
        const level = dayMap[day] || 0;
        cell.style.background = `var(--mm-cal-${level})`;
        const iso = new Date(day * 86400000).toISOString().slice(0, 10);
        const words = daily.find(x => x.day === day)?.words || 0;
        cell.title = words ? `${iso} · ${words} words` : iso;
      }
      grid.appendChild(cell);
    }
  }
  body.appendChild(dows);
  body.appendChild(grid);
  wrap.appendChild(months);
  wrap.appendChild(body);
}

// Heuristic to separate dictionary corrections from snippet text expansions
const isSnippet = (e) =>
  e.replacement.includes(" ") ||
  e.replacement.includes("\n") ||
  e.replacement.includes("@") ||
  e.replacement.length > 15;

// ── dictionary (main page) ──
let dictEntries = [];
function renderDictPage(entries) {
  const host = $("dict-entries");
  host.innerHTML = "";
  const q = ($("dict-search").value || "").toLowerCase();
  const filtered = q ? entries.filter(e => e.spoken.toLowerCase().includes(q) || e.replacement.toLowerCase().includes(q)) : entries;
  
  if (!filtered.length) {
    host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No dictionary words found</div>';
    return;
  }

  filtered.forEach((e, idx) => {
    const row = document.createElement("div");
    row.className = "dict-row-view";
    
    let isEditing = (e.spoken === "");
    
    const renderRowContent = () => {
      if (isEditing) {
        row.innerHTML = `
          <input class="spoken" value="${escapeHtml(e.spoken)}" placeholder="spoken" style="flex:1; margin-right:4px;" />
          <span class="arrow" style="margin:0 4px; color:var(--mm-muted-3);">&rarr;</span>
          <input class="replacement" value="${escapeHtml(e.replacement)}" placeholder="replacement" style="flex:1; margin-right:8px;" />
          <div class="actions" style="display:flex; gap:10px; align-items:center;">
            <span class="action-btn save-btn" title="Save" style="color:var(--mm-status-green); font-size:14px; font-weight:bold;">&#10003;</span>
            <span class="action-btn cancel-btn" title="Cancel" style="color:var(--mm-coral); font-size:14px; font-weight:bold;">&#10005;</span>
          </div>
        `;
        row.querySelector(".save-btn").onclick = (ev) => {
          ev.stopPropagation();
          const spoken = row.querySelector(".spoken").value.trim();
          const replacement = row.querySelector(".replacement").value.trim();
          if (spoken) {
            e.spoken = spoken;
            e.replacement = replacement;
            isEditing = false;
            saveDictPage();
            renderRowContent();
          }
        };
        row.querySelector(".cancel-btn").onclick = (ev) => {
          ev.stopPropagation();
          if (e.spoken === "") {
            dictEntries.splice(dictEntries.indexOf(e), 1);
            renderDictPage(dictEntries);
          } else {
            isEditing = false;
            renderRowContent();
          }
        };
        row.querySelector(".spoken").focus();
      } else {
        row.innerHTML = `
          <span class="term">${escapeHtml(e.spoken)}</span>
          <span class="arrow">&rarr;</span>
          <span class="replace">${escapeHtml(e.replacement)}</span>
          <div class="actions">
            <span class="action-btn edit-btn" title="Edit">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M13 4 L16 7 L7 16 H4 V13 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn del-btn" title="Remove">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        `;
        row.querySelector(".edit-btn").onclick = (ev) => {
          ev.stopPropagation();
          isEditing = true;
          renderRowContent();
        };
        row.querySelector(".del-btn").onclick = (ev) => {
          ev.stopPropagation();
          dictEntries.splice(dictEntries.indexOf(e), 1);
          saveDictPage();
        };
      }
    };
    
    renderRowContent();
    host.appendChild(row);
  });
}

function saveDictPage() {
  dictEntries = dictEntries.filter(e => e.spoken.trim() !== "");
  const combined = [...dictEntries, ...snipEntries];
  invoke("set_dictionary", { entries: combined });
  renderDictPage(dictEntries);
}
$("dict-search").oninput = () => renderDictPage(dictEntries);
$("dict-add").onclick = () => {
  const newEntry = { spoken: "", replacement: "" };
  dictEntries.push(newEntry);
  renderDictPage(dictEntries);
};

// ── snippets ──
let snipEntries = [];
function renderSnipPage(entries) {
  const host = $("snip-entries");
  host.innerHTML = "";
  const q = ($("snip-search").value || "").toLowerCase();
  const filtered = q ? entries.filter(e => e.spoken.toLowerCase().includes(q) || e.replacement.toLowerCase().includes(q)) : entries;
  
  if (!filtered.length) {
    host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No snippets found</div>';
    return;
  }

  filtered.forEach((e, idx) => {
    const row = document.createElement("div");
    row.className = "snip-row-view";
    
    let isEditing = (e.spoken === "");
    
    const renderRowContent = () => {
      if (isEditing) {
        row.innerHTML = `
          <input class="spoken" value="${escapeHtml(e.spoken)}" placeholder="phrase" style="width:150px; flex:none; margin-right:4px;" />
          <span class="arrow" style="margin:0 4px; color:var(--mm-muted-3);">&rarr;</span>
          <input class="replacement" value="${escapeHtml(e.replacement)}" placeholder="expansion" style="flex:1; margin-right:8px;" />
          <div class="actions" style="display:flex; gap:10px; align-items:center;">
            <span class="action-btn save-btn" title="Save" style="color:var(--mm-status-green); font-size:14px; font-weight:bold;">&#10003;</span>
            <span class="action-btn cancel-btn" title="Cancel" style="color:var(--mm-coral); font-size:14px; font-weight:bold;">&#10005;</span>
          </div>
        `;
        row.querySelector(".save-btn").onclick = (ev) => {
          ev.stopPropagation();
          const spoken = row.querySelector(".spoken").value.trim();
          const replacement = row.querySelector(".replacement").value.trim();
          if (spoken) {
            e.spoken = spoken;
            e.replacement = replacement;
            isEditing = false;
            saveSnipPage();
            renderRowContent();
          }
        };
        row.querySelector(".cancel-btn").onclick = (ev) => {
          ev.stopPropagation();
          if (e.spoken === "") {
            snipEntries.splice(snipEntries.indexOf(e), 1);
            renderSnipPage(snipEntries);
          } else {
            isEditing = false;
            renderRowContent();
          }
        };
        row.querySelector(".spoken").focus();
      } else {
        row.innerHTML = `
          <span class="trigger">${escapeHtml(e.spoken)}</span>
          <span class="preview">${escapeHtml(e.replacement)}</span>
          <div class="actions">
            <span class="action-btn edit-btn" title="Edit">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M13 4 L16 7 L7 16 H4 V13 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn del-btn" title="Remove">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        `;
        row.querySelector(".edit-btn").onclick = (ev) => {
          ev.stopPropagation();
          isEditing = true;
          renderRowContent();
        };
        row.querySelector(".del-btn").onclick = (ev) => {
          ev.stopPropagation();
          snipEntries.splice(snipEntries.indexOf(e), 1);
          saveSnipPage();
        };
      }
    };
    
    renderRowContent();
    host.appendChild(row);
  });
}

function saveSnipPage() {
  snipEntries = snipEntries.filter(e => e.spoken.trim() !== "");
  const combined = [...dictEntries, ...snipEntries];
  invoke("set_dictionary", { entries: combined });
  renderSnipPage(snipEntries);
}
$("snip-search").oninput = () => renderSnipPage(snipEntries);
$("snip-add").onclick = () => {
  const newEntry = { spoken: "", replacement: "" };
  snipEntries.push(newEntry);
  renderSnipPage(snipEntries);
};

// ── tone ──
// Presets map to instruction strings; the backend just stores whatever
// string it's given (see set_tone / set_app_tones in main.rs), no enum.
const TONE_PRESETS = {
  professional: "Write in a professional, polished tone.",
  friendly: "Write in a warm, friendly tone.",
  concise: "Keep it concise and to the point.",
  casual: "Write in a casual, relaxed tone.",
};
function toneKeyForValue(tone) {
  if (!tone) return "";
  const hit = Object.entries(TONE_PRESETS).find(([, v]) => v === tone);
  return hit ? hit[0] : "custom";
}
// Lets a per-app tone row's free-text field suggest the same presets as the
// default-tone select, without a second select+custom widget.
if ($("tone-preset-datalist")) {
  $("tone-preset-datalist").innerHTML =
    Object.values(TONE_PRESETS).map(v => `<option value="${escapeHtml(v)}"></option>`).join("");
}
// Tone rewrites voice, which only the AI polish tier can do — Rules just
// strips/fixes. Greyed out (not hidden) so it reads as "needs a setting",
// not "broken".
function updateToneDisabled(polishMode) {
  const card = $("tone-card");
  if (!card) return;
  const enabled = polishMode === "ai";
  card.classList.toggle("disabled", !enabled);
  if ($("tone-sub")) {
    $("tone-sub").textContent = enabled
      ? "Casual in Slack, professional in email — the Wispr Flow model."
      : "Needs AI polish — switch Cleanup to AI above to use tones.";
  }
}
function setToneSelectUI(tone) {
  const key = toneKeyForValue(tone);
  if ($("tone-select")) $("tone-select").value = key;
  const isCustom = key === "custom";
  if ($("tone-custom-row")) $("tone-custom-row").hidden = !isCustom;
  if ($("tone-custom-input")) $("tone-custom-input").value = isCustom ? tone : "";
}
// Suggests apps the user has actually dictated into (reusing the Insights
// per-app breakdown) via a native <datalist> — free text still works for
// apps not seen yet.
async function populateToneAppDatalist() {
  const dl = $("tone-app-datalist");
  if (!dl) return;
  try {
    const stats = await invoke("get_stats");
    const apps = (stats?.topApps || []).map(a => a.name).filter(Boolean);
    dl.innerHTML = apps.map(a => `<option value="${escapeHtml(a)}"></option>`).join("");
  } catch {}
}

// Per-app tone rows — same add/edit/remove list shape as the Dictionary page
// (reuses its dict-row-view / dict-entries-container markup and classes).
let appTones = [];
function renderToneAppsPage(entries) {
  const host = $("tone-apps-list");
  if (!host) return;
  host.innerHTML = "";
  if (!entries.length) {
    host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No per-app tones yet</div>';
    return;
  }
  entries.forEach((e) => {
    const row = document.createElement("div");
    row.className = "dict-row-view";

    let isEditing = (e.app === "");

    const renderRowContent = () => {
      if (isEditing) {
        row.innerHTML = `
          <input class="spoken" list="tone-app-datalist" value="${escapeHtml(e.app)}" placeholder="app (e.g. Slack)" style="flex:1; margin-right:4px;" />
          <span class="arrow" style="margin:0 4px; color:var(--mm-muted-3);">&rarr;</span>
          <input class="replacement" list="tone-preset-datalist" value="${escapeHtml(e.tone)}" placeholder="tone instruction" style="flex:1; margin-right:8px;" />
          <div class="actions" style="display:flex; gap:10px; align-items:center;">
            <span class="action-btn save-btn" title="Save" style="color:var(--mm-status-green); font-size:14px; font-weight:bold;">&#10003;</span>
            <span class="action-btn cancel-btn" title="Cancel" style="color:var(--mm-coral); font-size:14px; font-weight:bold;">&#10005;</span>
          </div>
        `;
        row.querySelector(".save-btn").onclick = (ev) => {
          ev.stopPropagation();
          const appName = row.querySelector(".spoken").value.trim();
          const tone = row.querySelector(".replacement").value.trim();
          if (appName) {
            e.app = appName;
            e.tone = tone;
            isEditing = false;
            saveToneApps();
            renderRowContent();
          }
        };
        row.querySelector(".cancel-btn").onclick = (ev) => {
          ev.stopPropagation();
          if (e.app === "") {
            appTones.splice(appTones.indexOf(e), 1);
            renderToneAppsPage(appTones);
          } else {
            isEditing = false;
            renderRowContent();
          }
        };
        row.querySelector(".spoken").focus();
      } else {
        row.innerHTML = `
          <span class="term">${escapeHtml(e.app)}</span>
          <span class="arrow">&rarr;</span>
          <span class="replace">${escapeHtml(e.tone)}</span>
          <div class="actions">
            <span class="action-btn edit-btn" title="Edit">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M13 4 L16 7 L7 16 H4 V13 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn del-btn" title="Remove">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        `;
        row.querySelector(".edit-btn").onclick = (ev) => {
          ev.stopPropagation();
          isEditing = true;
          renderRowContent();
        };
        row.querySelector(".del-btn").onclick = (ev) => {
          ev.stopPropagation();
          appTones.splice(appTones.indexOf(e), 1);
          saveToneApps();
        };
      }
    };

    renderRowContent();
    host.appendChild(row);
  });
}
function saveToneApps() {
  appTones = appTones.filter(e => e.app.trim() !== "");
  invoke("set_app_tones", { tones: appTones });
  renderToneAppsPage(appTones);
}
if ($("tone-app-add")) $("tone-app-add").onclick = () => {
  appTones.push({ app: "", tone: "" });
  renderToneAppsPage(appTones);
};
if ($("tone-select")) $("tone-select").onchange = () => {
  const key = $("tone-select").value;
  const isCustom = key === "custom";
  if ($("tone-custom-row")) $("tone-custom-row").hidden = !isCustom;
  const tone = key === "" ? "" : (isCustom ? ($("tone-custom-input")?.value.trim() || "") : TONE_PRESETS[key]);
  invoke("set_tone", { tone });
  if (isCustom && $("tone-custom-input")) $("tone-custom-input").focus();
};
if ($("tone-custom-input")) $("tone-custom-input").onchange = () => {
  invoke("set_tone", { tone: $("tone-custom-input").value.trim() });
};
function initToneUI(s) {
  setToneSelectUI(s.tone || "");
  updateToneDisabled(s.polish);
  appTones = (s.appTones || []).map(e => ({ ...e }));
  renderToneAppsPage(appTones);
  populateToneAppDatalist();
}

// ── history page ──
let historyEntries = [];
function renderHistoryPage(entries) {
  const host = $("history-list");
  host.innerHTML = "";
  if (!entries.length) {
    host.innerHTML = '<div class="hist-empty">Nothing dictated yet this session</div>';
    return;
  }
  entries.forEach((e, i) => {
    const row = document.createElement("div");
    row.className = "hist-row";
    row.innerHTML = `<span class="time">${e.time}</span><span class="txt">${escapeHtml(e.text)}</span><span class="copy" title="Copy">⧉</span><span class="retry" title="Re-polish &amp; copy">↻</span>`;
    row.querySelector(".copy").onclick = (ev) => { ev.stopPropagation(); copyText(e.text); };
    row.querySelector(".retry").onclick = (ev) => { ev.stopPropagation(); invoke("repolish_copy", { text: e.text }); };
    row.onclick = () => copyText(e.text);
    host.appendChild(row);
  });
}
function loadHistory() { renderHistoryPage(historyEntries); }

// ── settings page wiring ──
// `selected` (which engine set_asr_model chose) and `state` (installed vs.
// download, i.e. is it actually on disk) are independent — a model can be
// selected but not yet downloaded, mid-download-and-restart. Only
// installed + selected is truly ACTIVE; installed-but-not-selected offers a
// switch, not-installed always offers Download regardless of selection.
function renderModels(models) {
  const host = $("model-list");
  host.innerHTML = "";
  models.forEach((m, i) => {
    if (i) host.insertAdjacentHTML("beforeend", '<div class="divider"></div>');
    const row = document.createElement("div");
    row.className = "model-row";

    let rightStatus = "";
    if (m.selected && m.state === "installed") {
      rightStatus = `<span class="model-badge">ACTIVE</span>`;
    } else if (m.state === "installed") {
      rightStatus = `<button class="btn btn-outline model-select-btn" style="font-size:11px; padding:4px 10px; border-radius:6px;">Use this</button>`;
    } else if (m.state === "download") {
      rightStatus = `<button class="btn btn-primary model-download-btn" style="font-size:11px; padding:4px 10px; border-radius:6px;">Download</button>`;
    } else if (m.state === "downloading") {
      rightStatus = `<span class="mono" style="font-size:11px;">downloading &middot; ${m.progress}%</span>`;
    }

    row.innerHTML = `
      <div style="display:flex;align-items:center;gap:12px;">
        <span class="model-icon-box">
          <svg viewBox="0 0 20 20" width="17" height="17">
            <path d="M10 3 V13 M10 13 A2.4 2.4 0 1 0 7.6 15.4 A2.4 2.4 0 0 0 10 13 M10 3 L15 4.6 V8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </span>
        <div>
          <div class="name" style="font-weight: 500; color: var(--mm-ink);">${escapeHtml(m.name)} <span class="sub" style="color: var(--mm-muted-2); font-size: 11.5px; font-weight: 400;">${escapeHtml(m.variant)}</span></div>
          <div class="meta" style="font: 400 11.5px 'Hanken Grotesk'; color: var(--mm-muted-3); margin-top: 2px;">${m.meta || (m.state === "installed" ? `Installed &middot; ${m.size} &middot; on-device` : `Not installed &middot; ${m.size}`)}</div>
        </div>
      </div>
      <div style="flex:1"></div>
      ${rightStatus}
    `;
    const selectBtn = row.querySelector(".model-select-btn");
    if (selectBtn) selectBtn.onclick = () => selectAsrModel(m.id);
    const downloadBtn = row.querySelector(".model-download-btn");
    if (downloadBtn) downloadBtn.onclick = () => selectAsrModel(m.id, true);
    host.appendChild(row);
  });
  updateLanguageDisabled(models.find(m => m.selected)?.id);
}

// Picking a model (installed switch, or a not-yet-downloaded Download click)
// only ever changes which engine is *configured* — asr.rs caches the loaded
// model at startup, so this can't take effect until a restart either way.
async function selectAsrModel(id, alsoDownload) {
  await invoke("set_asr_model", { model: id });
  showAsrRestartNote();
  if (alsoDownload) invoke("download_assets");
  const s = await getSettings();
  renderModels(s.models || []);
}

function showAsrRestartNote() {
  const row = $("asr-restart-row");
  if (row) row.hidden = false;
}

// Parakeet is English-only and provably ignores the language setting — grey
// the picker out and say so, rather than let it silently do nothing.
function updateLanguageDisabled(modelId) {
  const wrapper = $("asr-language-wrapper");
  const select = $("asr-language-select");
  const sub = $("asr-language-sub");
  if (!wrapper || !select) return;
  const isParakeet = modelId === "parakeet-v3";
  select.disabled = isParakeet;
  wrapper.classList.toggle("disabled", isParakeet);
  if (sub) sub.textContent = isParakeet
    ? "Parakeet is English-only and ignores this."
    : "Which language to expect, or auto-detect";
}

async function copyText(text) {
  if (hasTauri) invoke("copy_text", { text });
  else if (navigator.clipboard) navigator.clipboard.writeText(text).catch(() => {});
}

function escapeHtml(s) { return s.replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }

// ── zoom ──
// Steps match Chrome's zoom ladder so the sizes feel familiar. Applying is
// the Rust side's job (native webview zoom, which scales the layout viewport
// — a CSS zoom/transform here would break the 100vh flex shell).
const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
let currentZoom = 1;

function applyZoom(factor, persist = true) {
  const clamped = Math.min(2, Math.max(0.5, factor));
  currentZoom = clamped;
  const el = $("zoom-val");
  if (el) el.textContent = Math.round(clamped * 100) + "%";
  if ($("zoom-out")) $("zoom-out").disabled = clamped <= ZOOM_STEPS[0];
  if ($("zoom-in")) $("zoom-in").disabled = clamped >= ZOOM_STEPS[ZOOM_STEPS.length - 1];
  if (persist) invoke("set_zoom", { factor: clamped });
}
function stepZoom(dir) {
  // Nearest step in the requested direction, so an odd saved value still
  // lands back on the ladder.
  const next = dir > 0
    ? ZOOM_STEPS.find(z => z > currentZoom + 0.001)
    : [...ZOOM_STEPS].reverse().find(z => z < currentZoom - 0.001);
  if (next) applyZoom(next);
}
function initZoom(saved) {
  applyZoom(saved, false); // reflect what Rust already applied at startup
  if ($("zoom-in")) $("zoom-in").onclick = () => stepZoom(1);
  if ($("zoom-out")) $("zoom-out").onclick = () => stepZoom(-1);
  if ($("zoom-reset")) $("zoom-reset").onclick = () => applyZoom(1);
}
// Ctrl +/-/0, including the numpad and the shift-less "=" key.
window.addEventListener("keydown", (e) => {
  if (!e.ctrlKey) return;
  if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") { e.preventDefault(); stepZoom(1); }
  else if (e.key === "-" || e.code === "NumpadSubtract") { e.preventDefault(); stepZoom(-1); }
  else if (e.key === "0" || e.code === "Numpad0") { e.preventDefault(); applyZoom(1); }
});

// ── hotkey picker ──
const CODE_ALIAS = {
  AltRight: "AltGr", AltLeft: "Alt", Enter: "Return",
  NumpadEnter: "NumpadEnter", NumpadAdd: "NumpadAdd", NumpadSubtract: "NumpadSubtract",
  NumpadMultiply: "NumpadMultiply", NumpadDivide: "NumpadDivide",
};
const MOUSE_BUTTON_TO_KEY = { 0: "MouseLeft", 1: "MouseMiddle", 2: "MouseRight", 3: "MouseX1", 4: "MouseX2" };
let currentHotkey = null;
let hotkeyOptions = [];

function setKeycap(key) { $("hotkey-keycap").textContent = HOTKEY_LABELS[key] || key; }
function eventCodeToName(code) {
  if (CODE_ALIAS[code]) return CODE_ALIAS[code];
  return code;
}
function populateMicPicker(options, current) {
  const sel = $("mic-select");
  if (!sel) return;
  sel.innerHTML = "";
  const def = document.createElement("option");
  def.value = "";
  def.textContent = "System default";
  sel.appendChild(def);
  for (const name of options) {
    const opt = document.createElement("option");
    opt.value = name;
    opt.textContent = name;
    sel.appendChild(opt);
  }
  sel.value = current && options.includes(current) ? current : "";
  sel.onchange = () => invoke("set_microphone", { name: sel.value });
}

function populateHotkeyPicker(options, current) {
  hotkeyOptions = options;
  HOTKEY_LABELS = {};
  HOTKEY_RISKY = {};
  for (const o of options) { HOTKEY_LABELS[o.name] = o.label; HOTKEY_RISKY[o.name] = o.risky; }
  currentHotkey = current;
}
function groupOf(name) {
  if (/^Control|^Shift|^Alt|^Meta|CapsLock|Function/.test(name)) return "Modifiers";
  if (/^F\d+$/.test(name)) return "Function keys";
  if (/^Mouse/.test(name)) return "Mouse buttons";
  if (/^Key[A-Z]$/.test(name) || /^Digit\d$/.test(name)) return "Letters & digits";
  if (/^Arrow/.test(name)) return "Arrow keys";
  if (/^Numpad|^NumLock/.test(name)) return "Numpad";
  return "Other";
}
function renderHotkeyList() {
  const host = $("hotkey-list");
  host.innerHTML = "";
  const groups = new Map();
  for (const o of hotkeyOptions) {
    const g = groupOf(o.name);
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(o);
  }
  for (const [group, items] of groups) {
    const label = document.createElement("div");
    label.className = "hotkey-group-label";
    label.textContent = group;
    host.appendChild(label);
    for (const o of items) {
      const row = document.createElement("div");
      row.className = "hotkey-option" + (o.name === currentHotkey ? " active" : "");
      row.innerHTML = `<span>${o.label}</span>${o.risky ? '<span class="warn">risky</span>' : ""}`;
      row.onclick = () => pickHotkey(o.name);
      host.appendChild(row);
    }
  }
}
function openHotkeyModal() {
  renderHotkeyList();
  $("hotkey-modal").hidden = false;
  setTimeout(() => $("hotkey-capture").focus(), 0);
}
function closeHotkeyModal() {
  $("hotkey-modal").hidden = true;
  $("hotkey-capture").classList.remove("armed");
  $("capture-title").textContent = "Press any key or mouse button";
  $("capture-sub").textContent = "Focus this box, then tap the key you want.";
}
function pickHotkey(name) {
  if (!HOTKEY_LABELS[name]) {
    $("capture-title").textContent = "That key isn't bindable";
    $("capture-sub").textContent = "This build supports the keys shown below.";
    $("hotkey-capture").classList.remove("armed");
    return;
  }
  if (HOTKEY_RISKY[name] && !window.confirm(`Use "${HOTKEY_LABELS[name]}" as your dictation hotkey?\n\nThis key is often used elsewhere.`)) return;
  currentHotkey = name;
  setKeycap(name);
  invoke("set_hotkey", { key: name });
  closeHotkeyModal();
}
$("hotkey-open").onclick = openHotkeyModal;
$("hotkey-modal-close").onclick = closeHotkeyModal;
$("hotkey-modal").onclick = (ev) => { if (ev.target.id === "hotkey-modal") closeHotkeyModal(); };
document.addEventListener("keydown", (ev) => {
  if ($("hotkey-modal").hidden) return;
  if (ev.key === "Escape") { closeHotkeyModal(); return; }
  if (document.activeElement !== $("hotkey-capture")) return;
  ev.preventDefault(); ev.stopPropagation();
  $("hotkey-capture").classList.add("armed");
  $("capture-title").textContent = `Captured: ${ev.code}`;
  pickHotkey(eventCodeToName(ev.code));
});
$("hotkey-capture").addEventListener("mousedown", (ev) => {
  if (ev.button === 0) return;
  ev.preventDefault();
  const name = MOUSE_BUTTON_TO_KEY[ev.button];
  if (!name) return;
  $("hotkey-capture").classList.add("armed");
  $("capture-title").textContent = `Captured: ${name}`;
  pickHotkey(name);
});

// ── threshold slider ──
const slider = $("threshold");
function setThresholdUI(v) {
  slider.value = v;
  $("threshold-val").textContent = `${v} words`;
  slider.style.setProperty("--fill", `${(v / 60) * 100}%`);
}
slider.oninput = () => { setThresholdUI(+slider.value); };
slider.onchange = () => invoke("set_threshold", { words: +slider.value });

// ── updates ──
async function openReleases() {
  const url = "https://github.com/khairyKY/sotto/releases/latest";
  if (hasTauri) invoke("open_url", { url });
  else window.open(url, "_blank");
}
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
   
    $("update-install").disabled = false;
  };
  async function refresh(manual) {
    statusEl.textContent = "Checking for updates…";
    const latest = await invoke("check_update");
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
    try { await invoke("install_update"); }
    catch (e) {
      $("update-text").innerHTML = `Auto-update failed. <a href="#" id="update-manual-link">Download it manually from GitHub</a>`;
      const link = document.getElementById("update-manual-link");
      if (link) link.onclick = (ev) => { ev.preventDefault(); openReleases(); };
      $("update-install").disabled = false;
    }
  };
  $("update-close").onclick = () => { banner.hidden = true; };
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

// ── asset download banner ──
async function initAssets() {
  const banner = $("assets-banner");
  const fill = $("assets-fill");
  const text = $("assets-text");
  banner.hidden = true;
 
  if (hasTauri && T.event) {
    T.event.listen("asset-progress", (e) => {
      const p = e.payload || {};
      const pct = p.total ? Math.round((p.received / p.total) * 100) : 0;
      const mbNow = (p.received / 1048576).toFixed(0);
      const mbAll = p.total ? (p.total / 1048576).toFixed(0) : "?";
      banner.hidden = false;
      fill.style.width = pct + "%";
      text.textContent = `Downloading ${p.name}… ${pct}% (${mbNow} / ${mbAll} MB)`;
    });
    T.event.listen("assets-ready", () => {
      fill.style.width = "100%";
      text.textContent = "All models ready.";
      setTimeout(() => { banner.hidden = true; }, 1500);
    });
    T.event.listen("asset-error", (e) => {
      banner.hidden = false;
      text.textContent = "Download failed: " + e.payload + " — restart Sotto to retry.";
    });
  }
  const status = await invoke("assets_status");
  if (!status || status.ready) return;
  banner.hidden = false;
  const missing = (status.missing || []).join(", ");
  text.textContent = `Downloading ${missing || "voice models"}…`;
}

// ── alert card ──
// Shown only while the worker is actually holding an undelivered take. Both
// buttons drop that stash (retry consumes it, dismiss throws it away), so the
// card hides itself via the take-changed event rather than optimistically here.
function renderTakeAlert(info) {
  const card = $("alert-card");
  if (!card) return;
  if (!info) { card.hidden = true; return; }
  $("alert-title").textContent = `Last dictation wasn't delivered`;
  // words is 0 when the take never reached a transcript — fall back to how
  // much audio is sitting in the stash, which is all we actually know.
  const size = info.words > 0
    ? `${info.words} word${info.words === 1 ? "" : "s"}`
    : `${Math.max(1, Math.round(info.audio_ms / 1000))}s of audio`;
  $("alert-sub").textContent = `${info.reason} · ${size}`;
  card.hidden = false;
}
$("alert-dismiss").onclick = () => invoke("dismiss_take");
$("alert-retry").onclick = () => invoke("retry_last");


function formatTime(date) {
  let hours = date.getHours();
  let minutes = date.getMinutes();
  let ampm = hours >= 12 ? 'PM' : 'AM';
  hours = hours % 12;
  hours = hours ? hours : 12;
  minutes = minutes < 10 ? '0' + minutes : minutes;
  return `${hours}:${minutes} ${ampm}`;
}

// ── boot ──
async function boot() {
  const s = await getSettings();
  if (s.theme) document.documentElement.dataset.theme = s.theme;
  populateHotkeyPicker(s.hotkey_options || s.hotkeyOptions || [], s.hotkey);
  setKeycap(s.hotkey);

  // Home page initial data
  loadHome();
  renderTakeAlert(s.takeInfo || s.take_info || null);

  // Theme
  function applyTheme(theme) {
    if (theme === "system") {
      const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
      document.documentElement.dataset.theme = prefersDark ? "dark" : "light";
    } else {
      document.documentElement.dataset.theme = theme;
    }
  }
  const initTheme = s.theme || "system";
  applyTheme(initTheme);
  if (hasTauri && T.event) T.event.emit("theme-changed", initTheme);
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    const t = $("theme")?.dataset.value || "system";
    if (t === "system") { applyTheme("system"); T.event.emit("theme-changed", "system"); }
  });

  // Warning cards dismiss wiring
  if (localStorage.getItem("dict-warn-closed") === "true") {
    $("dict-warn-card").style.display = "none";
  }
  $("dict-warn-close").onclick = () => {
    $("dict-warn-card").style.display = "none";
    localStorage.setItem("dict-warn-closed", "true");
  };

  if (localStorage.getItem("snip-warn-closed") === "true") {
    $("snip-warn-card").style.display = "none";
  }
  $("snip-warn-close").onclick = () => {
    $("snip-warn-card").style.display = "none";
    localStorage.setItem("snip-warn-closed", "true");
  };

  // Settings: segmented controls & fields
  selectSegment($("activation"), s.activation);
  selectSegment($("polish"), s.polish);
  // The theme picker has no UI (theme follows the OS per the design doc), but
  // guard rather than assume: an unguarded null here killed the whole rest of
  // boot() — settings, dictionary, snippets, history — in one TypeError.
  if ($("theme")) selectSegment($("theme"), s.theme || "system");
  setThresholdUI(s.threshold);
  initToneUI(s);
  renderModels(s.models || []);
  if ($("asr-language-select")) {
    $("asr-language-select").value = s.asrLanguage || "auto";
    $("asr-language-select").onchange = () => {
      invoke("set_asr_language", { language: $("asr-language-select").value });
      showAsrRestartNote();
    };
  }
  populateMicPicker(s.microphone_options || s.microphoneOptions || [], s.microphone || "");
  $("launch-login").setAttribute("aria-checked", String(!!s.launchLogin));
  $("start-hidden").setAttribute("aria-checked", String(!!s.startHidden));
  
  if ($("stats-enabled-toggle")) {
    $("stats-enabled-toggle").setAttribute("aria-checked", String(!!s.statsEnabled));
    initSwitch($("stats-enabled-toggle"), (on) => invoke("set_stats_enabled", { enabled: on }));
  }
  if ($("sound-sounds")) {
    $("sound-sounds").setAttribute("aria-checked", String(!!s.soundEnabled));
    initSwitch($("sound-sounds"), (on) => invoke("set_sound_enabled", { enabled: on }));
  }
  if ($("open-folder")) {
    // `start` opens directories in Explorer just like URLs in the browser.
    $("open-folder").onclick = () => invoke("open_url", { url: s.dataDir || "" });
  }
  initZoom(s.zoom || 1);
  if ($("data-folder-path") && s.dataDir) {
    $("data-folder-path").textContent = s.dataDir;
  }
  // Only worth its own row when the models actually live somewhere else —
  // otherwise it'd just repeat the line above.
  if (s.assetsDir && s.assetsDir !== s.dataDir) {
    $("models-folder-path").textContent = s.assetsDir;
    $("models-folder-row").hidden = false;
    $("open-models-folder").onclick = () => invoke("open_url", { url: s.assetsDir });
  }

  // Clear stats button wiring
  if ($("clear-stats-btn")) {
    $("clear-stats-btn").onclick = async () => {
      const confirmClear = confirm("Are you sure you want to clear all usage statistics?");
      if (confirmClear) {
        await invoke("clear_stats");
        loadInsights();
        loadHome();
      }
    };
  }

  initSegmented($("activation"), (v) => invoke("set_activation", { mode: v }));
  initSegmented($("polish"), (v) => { invoke("set_polish", { mode: v }); updateToneDisabled(v); });
  if ($("theme")) initSegmented($("theme"), (v) => {
    applyTheme(v);
    invoke("set_theme", { theme: v });
    if (hasTauri && T.event) T.event.emit("theme-changed", v);
  });
  initSwitch($("launch-login"), (on) => invoke("set_launch_login", { enabled: on }));
  initSwitch($("start-hidden"), (on) => invoke("set_start_hidden", { enabled: on }));

  // Dictionary & Snippets page data partitioning
  dictEntries = (s.dictionary || []).filter(e => !isSnippet(e)).map(e => ({ ...e }));
  snipEntries = (s.dictionary || []).filter(e => isSnippet(e)).map(e => ({ ...e }));
  renderDictPage(dictEntries);
  renderSnipPage(snipEntries);

  // History data
  historyEntries = s.history || [];
  loadHistory();

  // Live event listeners
  if (hasTauri && T.event) {
    T.event.listen("history-updated", (e) => {
      historyEntries = e.payload || [];
      renderHistoryPage(historyEntries);
      renderRecent(historyEntries);
    });
    T.event.listen("navigate", (e) => {
      const page = e.payload;
      if (page && document.querySelector(`.nav-item[data-page="${page}"]`)) navigate(page);
    });
    // Fires on every worker outcome — null once a take is delivered, retried,
    // or dismissed, which is what actually takes the card off the screen.
    T.event.listen("take-changed", (e) => renderTakeAlert(e.payload || null));
  }

  initUpdates();
  initAssets();
}
boot();
