// Sotto app shell — page routing, settings, insights, and all UI wiring.

const T = window.__TAURI__;
const hasTauri = !!(T && T.core);
// The backend hands a window it builds its theme and first page (app_windows.rs):
// one built on demand (#13) is shown as it loads, before get_settings has
// answered, and the `navigate` fired at it found nobody listening yet.
const opened = window.__SOTTO_OPENED || {};

const mock = {
  hotkey: "ControlRight",
  // A few of hotkey.rs's SUPPORTED_HOTKEYS, so the preview shows key names as the app does.
  hotkeyOptions: [
    { label: "Right Ctrl", name: "ControlRight", risky: false },
    { label: "Left Ctrl", name: "ControlLeft", risky: false },
    { label: "Right Alt", name: "AltGr", risky: false },
    { label: "Caps Lock", name: "CapsLock", risky: false },
    { label: "F8", name: "F8", risky: false },
    { label: "Space", name: "Space", risky: true },
  ],
  activation: "hold",
  polish: "ai",
  threshold: 18,
  launchLogin: true,
  startHidden: true,
  dictionary: [
    { spoken: "gee pee tee", replacement: "GPT", aliases: [], enabled: true, kind: "word" },
    { spoken: "my main email", replacement: "you@example.com", aliases: ["my primary email", "my email"], enabled: true, kind: "snippet" },
    { spoken: "arrow", replacement: "→", aliases: [], enabled: true, kind: "word" },
    { spoken: "الـ build", replacement: "الـ build بتاع الـ release", aliases: [], enabled: true, kind: "snippet" },
  ],
  replacementsEnabled: true,
  formattingCommands: true,
  tone: "",
  appTones: [],
  autoSend: ["Slack", "Terminal"],
  // Settings > Labs (#130). `?variables`, `?calibration`, `?scratchpad` and
  // `?lecture` turn one on, as its config flag does.
  variableRecognition: new URLSearchParams(location.search).has("variables"),
  voiceCorrection: false,
  backtrack: false,
  autoTone: false,
  lazyWindows: false,
  codeEditors: ["VS Code"],
  history: [
    // Arabic-first and code-switched samples keep bidi rendering (#42) checkable in the preview.
    // raw/tier/fallback exist for this session's rows only (#25); the last two stand in for rows reloaded from history.jsonl.
    { time: "2:31 PM", text: "افتح الـ terminal وشغّل الـ build", raw: "يعني افتح ال terminal و شغّل ال build", tier: "rules", fallback: "arabic" },
    { time: "2:20 PM", text: "الاجتماع الساعة اتناشر ونص", raw: "الاجتماع الساعة اتناشر ونص", tier: "rules", fallback: "short" },
    { time: "2:14 PM", text: "Let's ship the overlay states first.", raw: "um so let's let's ship the overlay states first", tier: "ai", fallback: "" },
    { time: "1:58 PM", text: "you@example.com", raw: "my main email", tier: "rules", fallback: "llm-error" },
    { time: "Sep 27", text: "Refactor the polish tier." },
    { time: "Sep 27", text: "Move the standup to Thursday at ten." },
  ],
  models: [
    { id: "parakeet-v3", name: "Parakeet v3", variant: "· English", meta: "NVIDIA", state: "installed", size: "639 MB", selected: true },
    { id: "whisper-turbo", name: "Whisper turbo", variant: "· 99 languages", meta: "OpenAI", state: "download", size: "547 MB", selected: false },
    { id: "egyptian-small", name: "Egyptian Arabic", variant: "· عامية + English", meta: "Whisper small, tuned for Egyptian speech", state: "download", size: "465 MB", selected: false },
  ],
  asrModel: "parakeet-v3",
  asrLanguage: "auto",
  asrLoad: { engine: "parakeet-v3", state: "ready" },
  historyPersist: false,
  // Transforms (#18): items are config::Transform as is, hence keep_words.
  transformsEnabled: false,
  transforms: [
    { name: "Polish", chord: "Ctrl+Alt+Digit1", prompt: "Tighten and clarify it without changing its meaning. Keep the writer's own words wherever you can, and keep I, me and my as they are.", keep_words: true },
    { name: "Prompt engineer", chord: "Ctrl+Alt+Digit2", prompt: "Turn it into a detailed prompt for an AI assistant, as four labelled lines: Goal, Context, Constraints, Output. Use only what the text says.", keep_words: false },
    { name: "بالمصري", chord: "Ctrl+Alt+Shift+KeyE", prompt: "اكتبها بالعامية المصرية وخلي الـ English terms زي ما هي.", keep_words: false },
  ],
  // Trained words, invented. `?calibration` previews the Calibrate card (#22).
  vocabulary: [
    { word: "Claude", heardAs: ["clawed"], recent: [true, false, true, true] },
    { word: "Gemini CLI", heardAs: [], recent: [false, true] },
    { word: "كشري", heardAs: [], recent: [] },
  ],
  calibration: new URLSearchParams(location.search).has("calibration"),
  scratchpad: new URLSearchParams(location.search).has("scratchpad"),
  // Scratchpad (#19), invented rows.
  pad: {
    enabled: new URLSearchParams(location.search).has("scratchpad"),
    chord: "Ctrl+Alt+Space",
    canPark: true,
    targetApp: "Notepad",
    rows: [
      { id: Date.now() - 2 * 86400000, text: "Ask the landlord about the heater before Friday." },
      { id: Date.now() - 3600000, text: "افتكر أجيب عيش وجبنة وأنا راجع" },
      { id: Date.now() - 600000, text: "Launch checklist:\nship the overlay states first\nthen the icon set" },
      { id: Date.now() - 60000, text: "الـ demo بكرة الساعة عشرة، جهز الـ slides" },
    ],
  },
  // Lecture mode (#23): Home's card.
  lectureMode: new URLSearchParams(location.search).has("lecture"),
  lecture: false,
  lecturesDir: "",
  transformDefaults: [
    { name: "Polish", chord: "Ctrl+Alt+Digit1", prompt: "Tighten and clarify it without changing its meaning. Keep the writer's own words wherever you can, and keep I, me and my as they are.", keep_words: true },
    { name: "Prompt engineer", chord: "Ctrl+Alt+Digit2", prompt: "Turn it into a detailed prompt for an AI assistant, as four labelled lines: Goal, Context, Constraints, Output. Use only what the text says.", keep_words: false },
  ],
};

async function invoke(cmd, args) {
  if (hasTauri) return T.core.invoke(cmd, args);
  console.log("[mock invoke]", cmd, args || "");
  if (cmd === "set_asr_model") { mock.models.forEach(m => { m.selected = m.id === args.model; }); mockLoad(args.model); }
  if (cmd === "download_assets") mockDownload();
  if (cmd === "set_transforms") mock.transforms = args.transforms;
  if (cmd === "set_lab_flag") {
    mock[labKey(args.name)] = args.on;
    if (args.name === "scratchpad") mock.pad.enabled = args.on;
  }
  if (cmd === "set_auto_send") mock.autoSend = args.apps;
  if (cmd === "scratchpad_state") return mock.pad;
  if (cmd === "scratchpad_delete") return (mock.pad.rows = mock.pad.rows.filter(r => r.id !== args.id));
  if (cmd === "set_code_editors") mock.codeEditors = args.apps;
  if (cmd === "set_lecture") { mock.lecture = args.on; mock.lecturesDir = "lectures"; }
  // Stands in for Harper's real-word check (#94), enough to preview both notes.
  if (cmd === "add_pronunciation_correction") return args.confirmed ?? !/\b(cloud|clawed|code)\b/i.test(args.heard);
  // `?firstrun` previews a fresh install: nothing on disk yet (#54).
  if (cmd === "assets_status") return MOCK_FIRST_RUN
    ? { ready: false, missing: ["Parakeet v3 (speech-to-text)", "Qwen2.5 1.5B (AI polish)", "llama.cpp runtime"] }
    : { ready: true, missing: [] };
}
const MOCK_FIRST_RUN = !hasTauri && new URLSearchParams(location.search).has("firstrun");
async function getSettings() {
  if (hasTauri) return T.core.invoke("get_settings");
  return mock;
}

let HOTKEY_LABELS = {};
let HOTKEY_RISKY = {};
// Settings > Labs (#130): a flag's get_settings field is its config name, camelCased.
const labKey = (flag) => flag.replace(/_(\w)/g, (_, c) => c.toUpperCase());

const $ = (id) => document.getElementById(id);

// ── settings modal ──
// Settings opens over the app (shell dimmed behind) rather than replacing the
// content area, so you never lose your place in Home/Insights/etc.
function openSettings() {
  $("settings-scrim").hidden = false;
  document.querySelector('.nav-item[data-page="settings"]')?.classList.add("active");
  syncPad();
}
function closeSettings() {
  $("settings-scrim").hidden = true;
  document.querySelector('.nav-item[data-page="settings"]')?.classList.remove("active");
  syncPad();
}
const settingsOpen = () => !$("settings-scrim").hidden;

// ── page routing ──
function navigate(page) {
  if (page === 'settings') { openSettings(); return; }
  closeSettings(); // tray → Insights etc. while the modal is up should land on that page
  document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
  document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
  const pg = $(`page-${page}`);
  if (pg) pg.classList.add('active');
  const nav = document.querySelector(`.nav-item[data-page="${page}"]`);
  if (nav) nav.classList.add('active');
  if (page === 'insights') loadInsights();
  if (page === 'history') loadHistory();
  if (page === 'home') loadHome();
  if (page === 'scratchpad') loadScratchpad();
  if (page === 'pronunciation') loadPronunciation();
  // The trainer is only armed while its page is open (ADR 0001). The backend
  // also drops the arm at the next Start, so this is the belt to its braces.
  else invoke("set_pronunciation_target", { word: null });
  syncPad();
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
  $("win-close").onclick = () => {
    invoke("set_pronunciation_target", { word: null });
    invoke("dismiss_window"); // hidden, or destroyed with lazy windows (#13)
  };
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

// Home's hint and the Settings key row describe the current mode (#58).
function setActivationCopy(mode) {
  const toggle = mode === "toggle";
  $("hint-verb").textContent = toggle ? "Tap" : "Hold";
  $("hint-tail").textContent = toggle ? "to start, tap again to stop." : "and speak.";
  $("activation-sub").textContent = toggle ? "Tap to start — tap again to transcribe" : "Hold to talk — release to transcribe";
}

// ── home stats ──
async function loadHome() {
  try {
    const stats = hasTauri ? await invoke("get_stats") : null;
    if (stats) {
      // "words today" means today (#58): the backend's day buckets use the same local day numbers.
      const today = (stats.daily || []).find(d => d.day === localDayNum());
      $("stat-words").textContent = (today?.words || 0).toLocaleString();
      $("stat-wpm").textContent = stats.avgWpm30d || "0";
      $("stat-streak").innerHTML = (stats.currentStreak || 0) + '<span class="stat-suffix">d</span>';
    }
    const s = await getSettings();
    if (s && s.hotkey) {
      $("keycap-display").textContent = HOTKEY_LABELS[s.hotkey] || s.hotkey;
    }
    renderRecent(s?.history || []);
    updateStatusBar(s);
    renderLecture(s);
  } catch {}
}

// ── lecture capture (#23) ──
// Home's card, shown while `lecture_mode` is on. A click only asks: the audio
// thread answers with "lecture-changed", and that reload is what flips the card.
let lectureOn = false;
function renderLecture(s) {
  lectureOn = !!s?.lecture;
  $("lecture-card").hidden = !s?.lectureMode;
  $("lecture-icon").classList.toggle("on", lectureOn);
  $("lecture-title").textContent = lectureOn ? "Capturing a lecture" : "Lecture capture";
  $("lecture-sub").textContent = lectureOn
    ? "Writing the transcript as it goes. Dictation waits until you stop."
    : "Writes a timestamped transcript to a file. Nothing is typed.";
  $("lecture-toggle").textContent = lectureOn ? "Stop" : "Start";
  // No folder until the first transcript line is written.
  $("lecture-folder").hidden = !s?.lecturesDir;
  $("lecture-folder").onclick = () => invoke("open_url", { url: s.lecturesDir });
}
$("recent-all").onclick = () => navigate("history");
$("lecture-toggle").onclick = async () => {
  await invoke("set_lecture", { on: !lectureOn });
  if (!hasTauri) loadHome();
};
// Why `start_dictation` answered false: paused from the tray (#60), or a lecture has the microphone.
const notStarted = () => lectureOn
  ? "A lecture is being captured. Stop it to dictate."
  : "Dictation is paused. Resume it from the tray menu.";
function updateStatusBar(s) {
  const parts = [];
  // Paused (tray) stops the hotkey and pill from starting takes, same chip
  // as "polish off" for the same reason: nothing else here would show it (#60).
  if (s?.paused) parts.push('<span class="status-warn">paused</span>');
  document.querySelector("#status-bar .status-dot").style.background = s?.paused ? "var(--mm-muted-2)" : "";
  if (s?.models?.length) {
    const sel = s.models.find(m => m.selected);
    if (sel) parts.push(escapeHtml(sel.name));
  }
  // Off silently degrades every dictation (no cleanup, no grammar fixes) and
  // nothing else in the app shows it — this is the state that actually cost
  // Khairy a long stretch of "why does polish feel broken". A warning chip
  // instead of plain text so it can't be skimmed past like the rest of the line.
  // Until its model lands, AI mode runs as rules — say so rather than claim it's on (#54).
  if (s?.polish === "ai") parts.push(aiWaiting ? "AI polish after download" : "AI polish on");
  else if (s?.polish === "rules") parts.push("rules polish");
  else parts.push('<span class="status-warn">polish off</span>');
  parts.push("mic: " + escapeHtml(s?.microphone || "system default"));
  $("status-text").innerHTML = parts.join(" · ");
}

function renderRecent(entries) {
  const host = $("recent-list");
  host.innerHTML = "";
  if (!entries || !entries.length) {
    host.innerHTML = '<div class="recent-empty">Nothing dictated yet this session</div>';
    $("recent-all").hidden = true;
    return;
  }
  $("recent-all").hidden = false;
  // Five, as the design: Home stays one screen and the status line in view.
  entries.slice(0, 5).forEach((e, i) => {
    const row = document.createElement("div");
    row.className = "recent-item";
    row.innerHTML = `
      <span class="recent-time">${e.time}</span>
      <span class="recent-text" dir="auto">${escapeHtml(e.text)}</span>
      <span class="recent-copy" title="Copy">⧉</span>
      <span class="recent-retry" title="Re-polish &amp; copy">↻</span>`;
    row.querySelector(".recent-copy").onclick = (ev) => { ev.stopPropagation(); copyText(e.text); };
    row.querySelector(".recent-retry").onclick = (ev) => { ev.stopPropagation(); invoke("repolish_copy", { text: e.text }); };
    host.appendChild(row);
  });
}

// ── insights ──
async function loadInsights() {
  $("total-period-label").textContent = $("insights-period").dataset.value === "month" ? "this month" : "this week";
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
    // Backend figure: typing time at 40 wpm minus the time spent speaking.
    if ($("total-saved-val")) $("total-saved-val").textContent = (stats.timeSavedMin || 0).toLocaleString();

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

// Which page an entry belongs to. Explicit `kind` from the backend now, not a
// shape guess — a 3-letter snippet used to get silently misfiled onto the
// Dictionary page because it didn't "look like" a snippet. Falls back to the
// old heuristic only for an entry that somehow has no kind at all (shouldn't
// happen post-migration, but a shape guess beats losing the row).
const isSnippet = (e) =>
  e.kind ? e.kind === "snippet" :
    e.replacement.includes(" ") || e.replacement.includes("\n") ||
    e.replacement.includes("@") || e.replacement.length > 15;

// Small chips after the primary phrase so alternate ways of saying it (aliases)
// are visible without opening edit mode — "+N more" once there's more than 2.
function renderAliasChips(aliases) {
  if (!aliases || !aliases.length) return "";
  const shown = aliases.slice(0, 2).map(a => `<span class="alias-chip" dir="auto">${escapeHtml(a)}</span>`).join("");
  const rest = aliases.length - Math.min(2, aliases.length);
  const more = rest > 0 ? `<span class="alias-chip alias-chip-more">+${rest} more</span>` : "";
  return `<span class="alias-list">${shown}${more}</span>`;
}

// set_replacements_enabled is one shared backend flag for both Dictionary and
// Snippets — flipping it on either page's switch has to update BOTH switches
// and both lists' dimming immediately, or the other page shows a stale state
// the next time you look at it.
let replacementsEnabled = true;
function setReplacementsEnabled(on) {
  replacementsEnabled = on;
  invoke("set_replacements_enabled", { enabled: on });
  if ($("dict-master-toggle")) $("dict-master-toggle").setAttribute("aria-checked", String(on));
  if ($("snip-master-toggle")) $("snip-master-toggle").setAttribute("aria-checked", String(on));
  renderDictPage(dictEntries);
  renderSnipPage(snipEntries);
}

// ── dictionary (main page) ──
let dictEntries = [];
function renderDictPage(entries) {
  const host = $("dict-entries");
  host.innerHTML = "";
  host.classList.toggle("list-disabled", !replacementsEnabled);
  const q = ($("dict-search").value || "").toLowerCase();
  const filtered = q ? entries.filter(e => e.spoken.toLowerCase().includes(q) || e.replacement.toLowerCase().includes(q)) : entries;

  if (!filtered.length) {
    host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No dictionary words found</div>';
    return;
  }

  filtered.forEach((e, idx) => {
    const row = document.createElement("div");
    row.className = "dict-row-view";

    // `_draft` marks a row prefilled from an example chip: it opens in edit
    // state even though it has content, because nothing is saved until the
    // user confirms. buildCombinedEntries skips it, so it never reaches Rust.
    let isEditing = (e.spoken === "" || e._draft === true);
    let draftAliases = (e.aliases || []).slice(); // working copy; only committed to e on Save

    // Aliases have their own add/remove flow — re-render just this block so
    // it doesn't clobber whatever's mid-typing in the spoken/replacement inputs.
    const renderAliasEditRow = () => {
      const aliasHost = row.querySelector(".alias-edit-row");
      if (!aliasHost) return;
      aliasHost.innerHTML = draftAliases.map((a) => `
        <div class="alias-edit-item">
          <input class="dict-edit-input alias-input" dir="auto" value="${escapeHtml(a)}" placeholder="another way to say it" />
          <span class="action-btn alias-remove-btn" title="Remove">
            <svg viewBox="0 0 20 20" width="13" height="13"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
          </span>
        </div>
      `).join("") + `<span class="link-change alias-add-btn">+ Add another way to say it</span>`;
      aliasHost.querySelectorAll(".alias-input").forEach((inp, i) => {
        inp.oninput = () => { draftAliases[i] = inp.value; };
      });
      aliasHost.querySelectorAll(".alias-remove-btn").forEach((btn, i) => {
        btn.onclick = (ev) => { ev.stopPropagation(); draftAliases.splice(i, 1); renderAliasEditRow(); };
      });
      aliasHost.querySelector(".alias-add-btn").onclick = (ev) => {
        ev.stopPropagation();
        draftAliases.push("");
        renderAliasEditRow();
        aliasHost.querySelectorAll(".alias-input")[draftAliases.length - 1]?.focus();
      };
    };

    const renderRowContent = () => {
      row.classList.toggle("editing", isEditing);
      if (isEditing) {
        row.classList.remove("entry-off");
        row.innerHTML = `
          <div class="dict-edit-fields">
            <input class="dict-edit-input spoken" dir="auto" value="${escapeHtml(e.spoken)}" placeholder="you say" />
            <span class="dict-edit-arrow">&rarr;</span>
            <input class="dict-edit-input replacement" dir="auto" value="${escapeHtml(e.replacement)}" placeholder="Sotto types" />
            <div class="dict-edit-actions">
              <span class="action-btn save-btn" title="Save">
                <svg viewBox="0 0 20 20" width="15" height="15"><path d="M4 10.5 L8 14.5 L16 5.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </span>
              <span class="action-btn cancel-btn" title="Cancel">
                <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
              </span>
            </div>
          </div>
          <div class="alias-edit-row"></div>
        `;
        renderAliasEditRow();
        row.querySelector(".save-btn").onclick = (ev) => {
          ev.stopPropagation();
          const spoken = row.querySelector(".spoken").value.trim();
          const replacement = row.querySelector(".replacement").value.trim();
          if (spoken) {
            e.spoken = spoken;
            e.replacement = replacement;
            e.aliases = draftAliases.map(a => a.trim()).filter(Boolean);
            delete e._draft; // confirmed — it's a real entry now
            isEditing = false;
            saveDictPage();
            renderRowContent();
          }
        };
        row.querySelector(".cancel-btn").onclick = (ev) => {
          ev.stopPropagation();
          if (e.spoken === "" || e._draft) {
            dictEntries.splice(dictEntries.indexOf(e), 1);
            renderDictPage(dictEntries);
          } else {
            isEditing = false;
            renderRowContent();
          }
        };
        row.querySelector(".spoken").focus();
      } else {
        row.classList.toggle("entry-off", e.enabled === false);
        row.innerHTML = `
          <button class="switch entry-toggle" role="switch" aria-checked="${e.enabled !== false}"><span class="knob"></span></button>
          <span class="term" dir="auto">${escapeHtml(e.spoken)}</span>
          ${renderAliasChips(e.aliases)}
          <span class="arrow">&rarr;</span>
          <span class="replace" dir="auto">${escapeHtml(e.replacement)}</span>
          <div class="actions">
            <span class="action-btn edit-btn" title="Edit">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M13 4 L16 7 L7 16 H4 V13 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn del-btn" title="Remove">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        `;
        initSwitch(row.querySelector(".entry-toggle"), (on) => {
          e.enabled = on;
          saveDictPage();
        });
        row.querySelector(".edit-btn").onclick = (ev) => {
          ev.stopPropagation();
          draftAliases = (e.aliases || []).slice();
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

// Shared by saveDictPage/saveSnipPage — set_dictionary replaces the WHOLE
// list, so every save resends both pages' entries together. Always send all
// 5 fields: dropping any of them here silently wipes it server-side. `kind`
// falls back by which array the entry is sitting in, purely as a last-resort
// safety net — every entry should already carry its own kind from creation.
// Unconfirmed example-chip drafts and blank new rows stay client-side: a save
// from the other page (or a toggle on this one) must not commit them (#57).
function buildCombinedEntries() {
  const saved = (e) => !e._draft && e.spoken.trim() !== "";
  return [
    ...dictEntries.filter(saved).map(e => ({ spoken: e.spoken, replacement: e.replacement, aliases: e.aliases || [], enabled: e.enabled !== false, kind: e.kind || "word" })),
    ...snipEntries.filter(saved).map(e => ({ spoken: e.spoken, replacement: e.replacement, aliases: e.aliases || [], enabled: e.enabled !== false, kind: e.kind || "snippet" })),
  ];
}

function saveDictPage() {
  dictEntries = dictEntries.filter(e => e.spoken.trim() !== "");
  invoke("set_dictionary", { entries: buildCombinedEntries() });
  renderDictPage(dictEntries);
}
$("dict-search").oninput = () => renderDictPage(dictEntries);
$("dict-add").onclick = () => {
  const newEntry = { spoken: "", replacement: "", aliases: [], enabled: true, kind: "word" };
  dictEntries.push(newEntry);
  renderDictPage(dictEntries);
};
// The banner's "+ Add word" chip is a second trigger for the same flow.
if ($("dict-warn-add")) $("dict-warn-add").onclick = () => $("dict-add").click();
// Example chips in the banner: click one to prefill it as a draft row in edit
// state. Nothing is written until the user hits save, so trying an example
// can't quietly add data they didn't choose. Scoped to this page's banner so
// the two pages' chips can't cross-wire.
document.querySelectorAll("#dict-warn-card .dict-warn-tag.example").forEach(chip => {
  chip.onclick = () => {
    dictEntries.push({
      spoken: chip.dataset.spoken || "",
      replacement: chip.dataset.replacement || "",
      aliases: [],
      enabled: true,
      kind: "word",
      _draft: true,
    });
    renderDictPage(dictEntries);
  };
});

// ── snippets ──
let snipEntries = [];
function renderSnipPage(entries) {
  const host = $("snip-entries");
  host.innerHTML = "";
  host.classList.toggle("list-disabled", !replacementsEnabled);
  const q = ($("snip-search").value || "").toLowerCase();
  const filtered = q ? entries.filter(e => e.spoken.toLowerCase().includes(q) || e.replacement.toLowerCase().includes(q)) : entries;

  if (!filtered.length) {
    host.innerHTML = '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No snippets found</div>';
    return;
  }

  filtered.forEach((e, idx) => {
    const row = document.createElement("div");
    row.className = "snip-row-view";

    // `_draft` marks a row prefilled from an example chip: it opens in edit
    // state even though it has content, because nothing is saved until the
    // user confirms. buildCombinedEntries skips it, so it never reaches Rust.
    let isEditing = (e.spoken === "" || e._draft === true);
    let draftAliases = (e.aliases || []).slice(); // working copy; only committed to e on Save

    const renderAliasEditRow = () => {
      const aliasHost = row.querySelector(".alias-edit-row");
      if (!aliasHost) return;
      aliasHost.innerHTML = draftAliases.map((a) => `
        <div class="alias-edit-item">
          <input class="dict-edit-input alias-input" dir="auto" value="${escapeHtml(a)}" placeholder="another way to say it" />
          <span class="action-btn alias-remove-btn" title="Remove">
            <svg viewBox="0 0 20 20" width="13" height="13"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
          </span>
        </div>
      `).join("") + `<span class="link-change alias-add-btn">+ Add another way to say it</span>`;
      aliasHost.querySelectorAll(".alias-input").forEach((inp, i) => {
        inp.oninput = () => { draftAliases[i] = inp.value; };
      });
      aliasHost.querySelectorAll(".alias-remove-btn").forEach((btn, i) => {
        btn.onclick = (ev) => { ev.stopPropagation(); draftAliases.splice(i, 1); renderAliasEditRow(); };
      });
      aliasHost.querySelector(".alias-add-btn").onclick = (ev) => {
        ev.stopPropagation();
        draftAliases.push("");
        renderAliasEditRow();
        aliasHost.querySelectorAll(".alias-input")[draftAliases.length - 1]?.focus();
      };
    };

    const renderRowContent = () => {
      row.classList.toggle("editing", isEditing);
      if (isEditing) {
        row.classList.remove("entry-off");
        row.innerHTML = `
          <div class="dict-edit-fields snip-edit-fields">
            <input class="dict-edit-input spoken" dir="auto" value="${escapeHtml(e.spoken)}" placeholder="you say" />
            <span class="dict-edit-arrow">&rarr;</span>
            <input class="dict-edit-input replacement" dir="auto" value="${escapeHtml(e.replacement)}" placeholder="Sotto types" />
            <div class="dict-edit-actions">
              <span class="action-btn save-btn" title="Save">
                <svg viewBox="0 0 20 20" width="15" height="15"><path d="M4 10.5 L8 14.5 L16 5.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </span>
              <span class="action-btn cancel-btn" title="Cancel">
                <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
              </span>
            </div>
          </div>
          <div class="alias-edit-row"></div>
        `;
        renderAliasEditRow();
        row.querySelector(".save-btn").onclick = (ev) => {
          ev.stopPropagation();
          const spoken = row.querySelector(".spoken").value.trim();
          const replacement = row.querySelector(".replacement").value.trim();
          if (spoken) {
            e.spoken = spoken;
            e.replacement = replacement;
            e.aliases = draftAliases.map(a => a.trim()).filter(Boolean);
            delete e._draft; // confirmed — it's a real entry now
            isEditing = false;
            saveSnipPage();
            renderRowContent();
          }
        };
        row.querySelector(".cancel-btn").onclick = (ev) => {
          ev.stopPropagation();
          if (e.spoken === "" || e._draft) {
            snipEntries.splice(snipEntries.indexOf(e), 1);
            renderSnipPage(snipEntries);
          } else {
            isEditing = false;
            renderRowContent();
          }
        };
        row.querySelector(".spoken").focus();
      } else {
        row.classList.toggle("entry-off", e.enabled === false);
        row.innerHTML = `
          <button class="switch entry-toggle" role="switch" aria-checked="${e.enabled !== false}"><span class="knob"></span></button>
          <span class="trigger" dir="auto">${escapeHtml(e.spoken)}</span>
          ${renderAliasChips(e.aliases)}
          <span class="preview" dir="auto">${escapeHtml(e.replacement)}</span>
          <div class="actions">
            <span class="action-btn edit-btn" title="Edit">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M13 4 L16 7 L7 16 H4 V13 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn del-btn" title="Remove">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        `;
        initSwitch(row.querySelector(".entry-toggle"), (on) => {
          e.enabled = on;
          saveSnipPage();
        });
        row.querySelector(".edit-btn").onclick = (ev) => {
          ev.stopPropagation();
          draftAliases = (e.aliases || []).slice();
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
  invoke("set_dictionary", { entries: buildCombinedEntries() });
  renderSnipPage(snipEntries);
}
$("snip-search").oninput = () => renderSnipPage(snipEntries);
$("snip-add").onclick = () => {
  const newEntry = { spoken: "", replacement: "", aliases: [], enabled: true, kind: "snippet" };
  snipEntries.push(newEntry);
  renderSnipPage(snipEntries);
};
if ($("snip-warn-add")) $("snip-warn-add").onclick = () => $("snip-add").click();
// Example chips in the banner: click one to prefill it as a draft row in edit
// state. Nothing is written until the user hits save, so trying an example
// can't quietly add data they didn't choose. Scoped to this page's banner so
// the two pages' chips can't cross-wire.
document.querySelectorAll("#snip-warn-card .dict-warn-tag.example").forEach(chip => {
  chip.onclick = () => {
    snipEntries.push({
      spoken: chip.dataset.spoken || "",
      replacement: chip.dataset.replacement || "",
      aliases: [],
      enabled: true,
      kind: "snippet",
      _draft: true,
    });
    renderSnipPage(snipEntries);
  };
});

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
      ? "How AI polish sounds, unless an app has its own tone below"
      : "Needs AI polish. Set Cleanup to AI to use tones.";
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
          <input class="replacement" list="tone-preset-datalist" value="${escapeHtml(e.tone)}" placeholder="how it should sound" style="flex:1; margin-right:8px;" />
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

// ── app lists: auto-send (#21), code editors (#27) ──
// Apps that get an Enter after a dictation lands, and apps where spoken
// casing commands run. The Dictionary's row markup with one field: a row is
// an app name and a remove button, and "" is the row being added (it suggests
// from the per-app tones' app datalist). Each list is saved whole by `cmd`.
const autoSend = { listId: "auto-send-list", addId: "auto-send-add", cmd: "set_auto_send", example: "Slack", apps: [] };
const codeEditors = { listId: "code-editors-list", addId: "code-editors-add", cmd: "set_code_editors", example: "Cursor", apps: [] };
function renderAppList(list) {
  const host = $(list.listId);
  if (!host) return;
  host.innerHTML = list.apps.length ? "" : '<div style="padding:14px 16px;font-size:12.5px;color:var(--mm-muted-3)">No apps yet</div>';
  list.apps.forEach((app, i) => {
    const row = document.createElement("div");
    row.className = "dict-row-view";
    if (app === "") {
      row.classList.add("editing");
      row.innerHTML = `
        <div class="dict-edit-fields">
          <input class="dict-edit-input spoken" list="tone-app-datalist" placeholder="app (e.g. ${list.example})" />
          <div class="dict-edit-actions">
            <span class="action-btn save-btn" title="Save">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M4 10.5 L8 14.5 L16 5.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </span>
            <span class="action-btn cancel-btn" title="Cancel">
              <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
            </span>
          </div>
        </div>
      `;
      row.querySelector(".save-btn").onclick = () => { list.apps[i] = row.querySelector("input").value.trim(); saveAppList(list); };
      row.querySelector(".cancel-btn").onclick = () => { list.apps.splice(i, 1); renderAppList(list); };
    } else {
      row.innerHTML = `
        <span class="term">${escapeHtml(app)}</span>
        <div class="actions">
          <span class="action-btn del-btn" title="Remove">
            <svg viewBox="0 0 20 20" width="15" height="15"><path d="M5 5 L15 15 M15 5 L5 15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
          </span>
        </div>
      `;
      row.querySelector(".del-btn").onclick = () => { list.apps.splice(i, 1); saveAppList(list); };
    }
    host.appendChild(row);
  });
  host.querySelector("input")?.focus();
}
function saveAppList(list) {
  list.apps = list.apps.filter(a => a !== "");
  invoke(list.cmd, { apps: list.apps });
  renderAppList(list);
}
for (const list of [autoSend, codeEditors]) {
  if ($(list.addId)) $(list.addId).onclick = () => {
    if (!list.apps.includes("")) list.apps.push("");
    renderAppList(list);
  };
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
    // ± only when this session kept the raw transcript (#25); reloaded rows have none.
    const diffBtn = e.raw ? '<span class="diff-toggle" title="What changed">±</span>' : "";
    row.innerHTML = `<span class="time">${e.time}</span><span class="txt" dir="auto">${escapeHtml(e.text)}</span>${diffBtn}<span class="copy" title="Copy">⧉</span><span class="retry" title="Re-polish &amp; copy">↻</span><span class="flag" title="Flag as wrong">⚑</span>`;
    row.querySelector(".copy").onclick = (ev) => { ev.stopPropagation(); copyText(e.text); };
    row.querySelector(".retry").onclick = (ev) => { ev.stopPropagation(); invoke("repolish_copy", { text: e.text }); };
    const flagEl = row.querySelector(".flag");
    flagEl.onclick = (ev) => {
      ev.stopPropagation();
      if (flagEl.classList.contains("flagged")) return;
      invoke("flag_transcription", { text: e.text });
      flagEl.classList.add("flagged");
      flagEl.title = "Flagged";
    };
    row.onclick = () => copyText(e.text);
    host.appendChild(row);
    if (e.raw) {
      const panel = reviewPanel(e);
      host.appendChild(panel);
      row.querySelector(".diff-toggle").onclick = (ev) => { ev.stopPropagation(); panel.hidden = !panel.hidden; };
    }
  });
}

// `a` → `b` as struck and accent words: History's review (#25), Calibrate (#22).
function diffHtml(a, b) {
  return wordDiff(a, b).map((p) =>
    p.op === "=" ? escapeHtml(p.w) : p.op === "-" ? `<del>${escapeHtml(p.w)}</del>` : `<ins>${escapeHtml(p.w)}</ins>`).join(" ");
}

// Review panel (#25): raw → delivered as struck/accent words, which tier ran,
// and "Use what I said" (raw to the clipboard; see copy_original).
// Why AI mode kept the Rules result: polish.rs's `fallback` codes, in words.
const AI_SKIPPED = {
  arabic: "AI polish is off for Arabic", short: "too short for AI", "no-op": "already clean",
  unavailable: "AI isn't downloaded yet", empty: "AI gave no answer", "llm-error": "AI failed",
  "line-breaks": "AI lost your line breaks", identifiers: "AI changed code names",
  "dropped-words": "AI dropped words", "new-words": "AI added words", "example-leak": "AI added words",
};
function reviewPanel(e) {
  const panel = document.createElement("div");
  panel.className = "hist-review";
  panel.hidden = true;
  const tier = { ai: "AI polish", rules: "Rules", off: "No polish" }[e.tier] || e.tier;
  const note = e.fallback ? `${tier} · ${AI_SKIPPED[e.fallback] || `AI skipped: ${e.fallback}`}` : tier;
  const unchanged = e.raw.trim() === e.text.trim();
  panel.innerHTML = `${unchanged ? "" : `<div class="hist-diff" dir="auto">${diffHtml(e.raw, e.text)}</div>`}
    <div class="hist-review-foot"><span class="hist-tier">${escapeHtml(unchanged ? note + " · no changes" : note)}</span>
    ${unchanged ? "" : '<button class="link-add use-raw">Use what I said</button>'}</div>`;
  panel.querySelector(".use-raw")?.addEventListener("click", () => invoke("copy_original", { raw: e.raw }));
  return panel;
}
function loadHistory() { renderHistoryPage(historyEntries); }

// ── scratchpad (#19) ──
let pad = { enabled: false, chord: "", canPark: false, targetApp: "", rows: [] }; // rows oldest first
async function loadScratchpad() {
  pad = (await invoke("scratchpad_state")) || pad;
  $("nav-scratchpad").hidden = !pad.enabled;
  renderPad();
}
// A take lands in the pad only while it's the page in front: tell the backend.
function syncPad() {
  if (pad.enabled) invoke("scratchpad_page", { open: $("page-scratchpad").classList.contains("active") && !settingsOpen() });
}
function padTime(ms) {
  const d = new Date(ms);
  return d.toDateString() === new Date().toDateString() ? formatTime(d) : d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}
function renderPad() {
  $("pad-chord").innerHTML = pad.chord ? `${chordKeysHtml(pad.chord)} opens and closes it` : "";
  const host = $("pad-list");
  host.innerHTML = "";
  if (!pad.rows.length) {
    host.innerHTML = `<div class="hist-empty">No notes yet. ${escapeHtml($("hint-verb").textContent)} ${escapeHtml($("keycap-display").textContent)} and think out loud.</div>`;
    return;
  }
  const into = escapeHtml(`Type into ${pad.targetApp}`);
  pad.rows.slice().reverse().forEach((r) => {
    const row = document.createElement("div");
    row.className = "hist-row pad-row";
    row.innerHTML = `<span class="time">${padTime(r.id)}</span><span class="txt" dir="auto">${escapeHtml(r.text)}</span>
      <button class="act" data-a="copy" title="Copy" aria-label="Copy">⧉</button>
      <button class="act" data-a="inject" title="${into}" aria-label="${into}"${pad.targetApp ? "" : " hidden"}>↵</button>
      <button class="act" data-a="park" title="Add to your notes file" aria-label="Add to your notes file"${pad.canPark ? "" : " hidden"}>⇲</button>
      <button class="act" data-a="delete" title="Delete" aria-label="Delete">✕</button>`;
    row.onclick = () => copyText(r.text);
    row.querySelectorAll(".act").forEach((b) => b.onclick = async (ev) => {
      ev.stopPropagation();
      const a = b.dataset.a;
      if (a === "copy") copyText(r.text);
      if (a === "inject") invoke("scratchpad_inject", { text: r.text });
      if (a === "delete") { pad.rows = await invoke("scratchpad_delete", { id: r.id }); renderPad(); }
      if (a === "park") {
        try { await invoke("scratchpad_park", { text: r.text }); b.textContent = "✓"; b.title = "Added to your notes file"; }
        catch (err) { b.textContent = "!"; b.title = `Couldn't add it: ${err}`; }
      }
    });
    host.appendChild(row);
  });
}

// ── pronunciation trainer ──
let pronVocabulary = []; // [{word, heardAs: [...], recent: [bool...]}] from settings
let pronState = "idle"; // "idle" | "speaking" (#36) | "listening" | "resolving"
let pronArmedWord = null; // the word the backend is currently armed for -- also what an incoming 'pronunciation-sample' must match

// Strength is the fraction of the last few attempts that matched -- see
// ADR 0001. `recent` is oldest-first booleans, already capped server-side
// (config::PRON_RECENT_CAP), so its length alone tells the window size.
function pronStrength(recent) {
  const total = recent.length;
  const hits = recent.filter(Boolean).length;
  return { hits, total, pct: total ? Math.round((hits / total) * 100) : 0 };
}

function renderTrainedWords() {
  const host = $("pron-trained-list");
  host.innerHTML = "";
  if (!pronVocabulary.length) {
    host.innerHTML = '<div class="hist-empty">No words trained yet</div>';
    return;
  }
  pronVocabulary.forEach(v => {
    const { hits, total, pct } = pronStrength(v.recent || []);
    const row = document.createElement("div");
    row.className = "pron-trained-row";
    row.innerHTML = `
      <span class="pron-trained-word" dir="auto">${escapeHtml(v.word)}</span>
      <span class="pron-trained-heard">${v.heardAs.length ? "heard as: " + v.heardAs.map(h => `<bdi>${escapeHtml(h)}</bdi>`).join(", ") : ((v.recent || []).length ? "no corrections yet" : "no attempts yet")}</span>
      <span class="pron-strength" style="--pct:${pct}"><span class="pron-strength-num">${total ? `${hits}/${total}` : "—"}</span></span>
    `;
    host.appendChild(row);
  });
}

function loadPronunciation() {
  renderTrainedWords();
  if (!$("cal-section").hidden) loadCalibration();
}

// Drives the whole listen/record/resolve flow. `word` only matters when
// entering "listening" -- it's what arms the focus-word display and what
// incoming 'pronunciation-sample' events are matched against.
function pronSetState(state, word) {
  pronState = state;
  if (word !== undefined) pronArmedWord = word;
  const status = $("pron-status");
  const btn = $("pron-listen-btn");
  const focusEl = $("pron-focus-word");
  const input = $("pron-word-input");
  document.querySelector(".pron-listen-card")?.classList.toggle("is-listening", state === "listening");
  focusEl.classList.remove("matched", "missed", "detected");

  if (state === "listening") {
    input.disabled = true;
    btn.disabled = false;
    btn.textContent = "Stop";
    status.textContent = "Listening — say it now";
    status.classList.add("active");
    focusEl.textContent = pronArmedWord;
    focusEl.style.setProperty("--amp", "0");
    focusEl.hidden = false;
    $("pron-samples-list").innerHTML = "";
  } else if (state === "resolving") {
    btn.disabled = true;
    btn.textContent = "Listen for it";
    status.textContent = "Checking…";
    focusEl.style.setProperty("--amp", "0");
  } else {
    input.disabled = false;
    btn.disabled = false;
    btn.textContent = "Listen for it";
    status.textContent = "";
    status.classList.remove("active");
    focusEl.hidden = true;
  }
}

// A trainer take that ends without a sample (too short, no speech,
// cancelled, failed) sends no 'pronunciation-sample'. Without this the page
// sat on "Listening" or "Checking…" with nothing left to come (#49). "done"
// never lands here: the sample handler plays the matched/missed flash first.
// Calibrate's takes (#22) end the same way; only one flow records at a time.
function pronTakeEnded(state) {
  const note = state === "cancelled" ? "" : "Didn't catch that. Try again.";
  // "speaking" (#36) has no take yet: a late event from the last one isn't its end.
  if (pronState === "listening" || pronState === "resolving") { pronSetState("idle"); $("pron-status").textContent = note; }
  if (calState !== "idle") { calSetState("idle"); $("cal-status").textContent = note; }
}

function pronAddSampleRow(word, heard, matched) {
  const row = document.createElement("div");
  row.className = matched ? "pron-sample-row matched" : "pron-sample-row";
  if (matched) {
    row.innerHTML = `<span class="pron-sample-heard">Heard: <b dir="auto">${escapeHtml(heard)}</b></span><span class="pron-sample-match" title="Matched">&#10003;</span>`;
  } else {
    // Nothing heard, nothing to learn: add_pronunciation_correction would save nothing.
    row.innerHTML = `<span class="pron-sample-heard">Heard: <b dir="auto">${escapeHtml(heard || "(nothing)")}</b></span>${heard ? '<button class="btn btn-ghost">Add correction</button>' : ""}`;
    row.querySelector("button")?.addEventListener("click", (ev) => pronAddCorrection(word, heard, ev.currentTarget));
  }
  $("pron-samples-list").prepend(row);
}

// "Add correction" (the trainer's and Calibrate's): the only thing that saves.
// A real-word mishearing is kept as a hint only, never a dictionary entry (#94).
// Harper can't vouch for an Arabic word, so that one asks first (#116): OK
// makes the entry, Cancel keeps the hint only.
async function pronAddCorrection(word, heard, btn) {
  const arabic = /\p{Script=Arabic}/u.test(heard);
  const confirmed = arabic
    ? confirm(`Every “${heard}” you say will be typed as “${word}”, in every app. Add this to your Dictionary?\n\nYou can switch it off there. Choose Cancel to only help Sotto hear “${word}”.`)
    : undefined;
  const exact = await invoke("add_pronunciation_correction", { word, heard, confirmed });
  const s = await getSettings();
  pronVocabulary = (s.vocabulary || []).map(v => ({ word: v.word, heardAs: v.heardAs || [], recent: v.recent || [] }));
  renderTrainedWords();
  document.querySelectorAll("#pron-trained-list .pron-trained-row").forEach(r => {
    if (r.querySelector(".pron-trained-word")?.textContent === word) {
      const ring = r.querySelector(".pron-strength");
      ring?.classList.add("pron-level-up");
      setTimeout(() => ring?.classList.remove("pron-level-up"), 500);
    }
  });
  const note = document.createElement("span");
  note.className = "pron-learned";
  note.textContent = exact ? "Learned" : "Learned as a hint";
  if (!exact && !arabic) note.title = `“${heard}” has a real word in it, so Sotto won't change it everywhere. It's kept as a hint.`;
  if (!exact && arabic) note.title = `Sotto listens for “${word}”, but leaves “${heard}” as you said it.`;
  btn.replaceWith(note);
}

// "Say it first" (#36): the trainer says the word before a take, through
// the WebView's speechSynthesis (Windows' own voices). Off by default; the
// choice lives in localStorage, which can throw.
let pronSayFirst = false;
try { pronSayFirst = localStorage.getItem("pron-say-first") === "true"; } catch {}
// ponytail: fixed guesses. TIMEOUT covers a voice that never fires onend (a
// word takes ~1 s); SETTLE lets the speaker's tail die before the mic opens.
const SAY_TIMEOUT_MS = 3000, SAY_SETTLE_MS = 200;

// Resolves once the word has been said, at once if no voice fits it: an
// English one for Latin script, an Arabic one for Arabic. Local voices
// only: an online one would send the word off the machine. The take starts
// after this, so the spoken word can't leak into it and score a match.
function sayWord(word) {
  const synth = window.speechSynthesis;
  const lang = /\p{Script=Arabic}/u.test(word) ? "ar" : /\p{Script=Latin}/u.test(word) ? "en" : null;
  const voice = lang && synth?.getVoices().find(v => v.localService && v.lang.toLowerCase().startsWith(lang));
  if (!voice) return Promise.resolve();
  return new Promise((done) => {
    const u = new SpeechSynthesisUtterance(word);
    u.voice = voice;
    u.lang = voice.lang;
    const timer = setTimeout(() => { synth.cancel(); done(); }, SAY_TIMEOUT_MS);
    u.onend = u.onerror = () => { clearTimeout(timer); setTimeout(done, SAY_SETTLE_MS); };
    synth.cancel(); // a stuck earlier utterance would hold this one in the queue
    synth.speak(u);
  });
}

if ($("pron-say-first")) {
  $("pron-say-first").setAttribute("aria-checked", String(pronSayFirst));
  initSwitch($("pron-say-first"), (on) => {
    pronSayFirst = on;
    try { localStorage.setItem("pron-say-first", String(on)); } catch {}
  });
  window.speechSynthesis?.getVoices(); // the WebView loads its voice list on first ask
}

if ($("pron-listen-btn")) {
  $("pron-listen-btn").onclick = async () => {
    if (pronState === "listening") {
      invoke("stop_dictation");
      pronSetState("resolving");
      return;
    }
    const word = $("pron-word-input").value.trim();
    if (!word || pronState !== "idle" || calState !== "idle") return;
    if (pronSayFirst) {
      pronState = "speaking"; // holds off a second click and Calibrate
      $("pron-listen-btn").disabled = true;
      await sayWord(word);
      // Left the page while it spoke: the trainer isn't armed elsewhere (ADR 0001).
      if (!$("page-pronunciation").classList.contains("active")) return pronSetState("idle");
    }
    // Armed before Start is sent: Start consumes the arm, so it must land first.
    await invoke("set_pronunciation_target", { word });
    if (await invoke("start_dictation") === false) {
      // Paused from the tray: no take will come, so don't sit on "Listening".
      invoke("set_pronunciation_target", { word: null });
      pronSetState("idle");
      $("pron-status").textContent = notStarted();
      return;
    }
    pronSetState("listening", word);
  };
}

// ── calibrate (#22) ──
// Each sentence goes through the trainer's direct-record path (ADR 0001),
// armed as a calibration target: the take comes back as a
// 'pronunciation-sample' holding the raw transcript, and nothing is scored,
// polished, typed or saved. Only an "Add correction" click saves.
let calSentences = [], calIndex = 0, calState = "idle"; // calState: as pronState, never "speaking"

function loadCalibration() {
  const next = calibrationSentences(pronVocabulary.map(v => v.word));
  if (next.join("\n") !== calSentences.join("\n")) { calSentences = next; calIndex = 0; }
  if (calState === "idle") calShow();
}

function calShow() {
  const sentence = calSentences[calIndex];
  $("cal-progress").textContent = sentence ? `Sentence ${calIndex + 1} of ${calSentences.length}`
    : calSentences.length ? "" : "Train a word above first. Calibrate then gives you a sentence with it to read.";
  $("cal-sentence").textContent = sentence || (calSentences.length ? "That's every trained word." : "");
  $("cal-diff").hidden = true;
  $("cal-pairs").innerHTML = "";
  $("cal-next-btn").textContent = sentence || !calSentences.length ? "Skip" : "Start over";
  calSetState("idle");
}

function calSetState(state) {
  calState = state;
  const status = $("cal-status");
  $("cal-card").classList.toggle("is-listening", state === "listening");
  $("cal-read-btn").textContent = state === "listening" ? "Stop" : "Read it";
  $("cal-read-btn").disabled = state === "resolving" || !calSentences[calIndex];
  $("cal-next-btn").disabled = state !== "idle" || !calSentences.length;
  status.textContent = state === "listening" ? "Listening — read it out loud" : state === "resolving" ? "Checking…" : "";
  status.classList.toggle("active", state === "listening");
}

// The sentence vs what was heard, then one row per trained word heard as
// something else, each with its own "Add correction".
function calResult(heard) {
  const sentence = calSentences[calIndex];
  const pairs = harvest(sentence, heard, pronVocabulary.map(v => v.word));
  const exact = wordDiff(sentence, heard).every(p => p.op === "=");
  calSetState("idle");
  $("cal-diff").innerHTML = diffHtml(sentence, heard);
  $("cal-diff").hidden = exact;
  $("cal-status").textContent = exact ? "Heard it exactly ✓" : pairs.length ? "" : "Nothing to add for your trained words";
  $("cal-next-btn").textContent = "Next";
  $("cal-pairs").innerHTML = "";
  pairs.forEach(([word, as]) => {
    const row = document.createElement("div");
    row.className = "pron-sample-row";
    row.innerHTML = `<span class="pron-sample-heard">Heard <b dir="auto">${escapeHtml(as)}</b> for <b dir="auto">${escapeHtml(word)}</b></span><button class="btn btn-ghost">Add correction</button>`;
    row.querySelector("button").onclick = (ev) => pronAddCorrection(word, as, ev.currentTarget);
    $("cal-pairs").appendChild(row);
  });
}

if ($("cal-read-btn")) {
  $("cal-read-btn").onclick = async () => {
    if (calState === "listening") {
      invoke("stop_dictation");
      calSetState("resolving");
      return;
    }
    const sentence = calSentences[calIndex];
    if (!sentence || pronState !== "idle") return;
    await invoke("set_pronunciation_target", { word: sentence, calibration: true });
    if (await invoke("start_dictation") === false) {
      invoke("set_pronunciation_target", { word: null });
      $("cal-status").textContent = notStarted();
      return;
    }
    calSetState("listening");
  };
  $("cal-next-btn").onclick = () => {
    calIndex = calIndex < calSentences.length ? calIndex + 1 : 0;
    calShow();
  };
}

// ── transforms (#18) ──
// Cards render the list; the editor is the hotkey picker's modal pattern. The
// backend owns the chord matching (hotkey.rs parse_chord); this only builds
// "Ctrl+Alt+Digit1" strings from a key press and keeps them unique.
let transforms = [];
let transformDefaults = [];
let transformEditing = null; // index being edited, or -1 for a new one
let transformDraftChord = "";
const CHORD_MODS = ["Ctrl", "Alt", "Shift", "Win"];

function chordKeysHtml(chord) {
  const parts = (chord || "").split("+").map(p => p.trim()).filter(Boolean);
  const label = (p) => CHORD_MODS.includes(p) ? p : (HOTKEY_LABELS[p] || p.replace(/^(Key|Digit)(?=\w$)/, ""));
  return parts.map(p => `<span class="keycap-mini">${escapeHtml(label(p))}</span>`).join('<span class="key-plus">+</span>');
}

function renderTransforms() {
  const grid = $("transforms-grid");
  grid.innerHTML = "";
  transforms.forEach((t, i) => {
    const card = document.createElement("div");
    card.className = "card-raised transform-card";
    card.innerHTML = `
      <div class="transform-keys">${chordKeysHtml(t.chord)}</div>
      <div class="transform-title" dir="auto">${escapeHtml(t.name)}</div>
      <div class="transform-desc" dir="auto">${escapeHtml(t.prompt)}</div>`;
    card.onclick = () => openTransformModal(i);
    grid.appendChild(card);
  });
  // The dashed "Create your own" card doubles as the empty state.
  const add = document.createElement("div");
  add.className = "transform-card dashed";
  add.innerHTML = `<span class="plus-icon">+</span>
    <div class="transform-title color-accent">Create your own</div>
    <div class="transform-desc">bring your own prompt</div>`;
  add.onclick = () => openTransformModal(-1);
  grid.appendChild(add);
}

function setTransformCapture(chord, note) {
  transformDraftChord = chord;
  $("transform-capture-keys").innerHTML = chord ? chordKeysHtml(chord) : "Press the shortcut";
  $("transform-capture-sub").textContent = note || (chord
    ? "Press another to change it."
    : "Click here, then press Ctrl, Alt or Win plus one key.");
}

function openTransformModal(i) {
  transformEditing = i;
  const t = i >= 0 ? transforms[i] : { name: "", chord: "", prompt: "", keep_words: false };
  $("transform-modal-title").textContent = i >= 0 ? "Edit transform" : "New transform";
  $("transform-name").value = t.name;
  $("transform-prompt").value = t.prompt;
  $("transform-keep-words").setAttribute("aria-checked", String(!!t.keep_words));
  $("transform-remove").hidden = i < 0;
  setTransformCapture(t.chord);
  $("transform-modal").hidden = false;
  setTimeout(() => $("transform-name").focus(), 0);
}
function closeTransformModal() {
  $("transform-modal").hidden = true;
  transformEditing = null;
}
function saveTransforms() {
  invoke("set_transforms", { transforms });
  renderTransforms();
}

$("transform-capture").addEventListener("keydown", (ev) => {
  if (["Control", "Alt", "Shift", "Meta", "AltGraph"].includes(ev.key)) return; // wait for the key
  if (ev.key === "Escape" && !ev.ctrlKey && !ev.altKey && !ev.metaKey) return; // the modal's own close
  ev.preventDefault();
  ev.stopPropagation();
  const name = eventCodeToName(ev.code);
  const mods = [ev.ctrlKey && "Ctrl", ev.altKey && "Alt", ev.shiftKey && "Shift", ev.metaKey && "Win"].filter(Boolean);
  // The regex covers the browser preview, whose mock has no hotkey list.
  const bindable = HOTKEY_LABELS[name] || /^(Key[A-Z]|Digit\d|F([1-9]|1[0-2]))$/.test(name);
  if (!bindable || /^(Mouse|Control|Shift|Alt|Meta|CapsLock)/.test(name)) {
    setTransformCapture(transformDraftChord, "Sotto can't use that key. Try a letter, digit or F-key.");
    return;
  }
  if (!mods.some(m => m !== "Shift")) {
    setTransformCapture(transformDraftChord, "Add Ctrl, Alt or Win, or it would fire while you type.");
    return;
  }
  const chord = [...mods, name].join("+");
  const clash = transforms.find((t, i) => i !== transformEditing && t.chord === chord);
  if (clash) {
    setTransformCapture(transformDraftChord, `Already used by ${clash.name}.`);
    return;
  }
  setTransformCapture(chord);
});

$("transform-save").onclick = () => {
  const name = $("transform-name").value.trim();
  const prompt = $("transform-prompt").value.trim();
  if (!name) { $("transform-name").focus(); return; }
  if (!prompt) { $("transform-prompt").focus(); return; }
  if (!transformDraftChord) {
    setTransformCapture("", "Press a shortcut for it first.");
    $("transform-capture").focus();
    return;
  }
  const t = { name, prompt, chord: transformDraftChord, keep_words: $("transform-keep-words").getAttribute("aria-checked") === "true" };
  if (transformEditing >= 0) transforms[transformEditing] = t; else transforms.push(t);
  closeTransformModal();
  saveTransforms();
};
$("transform-remove").onclick = () => {
  if (transformEditing < 0) return;
  transforms.splice(transformEditing, 1);
  closeTransformModal();
  saveTransforms();
};
initSwitch($("transform-keep-words"), () => {});
$("transform-cancel").onclick = closeTransformModal;
$("transform-modal-close").onclick = closeTransformModal;
$("transform-modal").onclick = (ev) => { if (ev.target.id === "transform-modal") closeTransformModal(); };
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape" && !$("transform-modal").hidden) closeTransformModal();
});
$("transforms-add").onclick = () => openTransformModal(-1);
$("transforms-reset").onclick = () => {
  if (!confirm("Reset transforms to Polish and Prompt engineer?\n\nYour own transforms will be removed.")) return;
  transforms = transformDefaults.map(t => ({ ...t }));
  saveTransforms();
};

function initTransforms(s) {
  transforms = (s.transforms || []).map(t => ({ ...t }));
  transformDefaults = s.transformDefaults || [];
  renderTransforms();
}

// ── labs (#130) ──
// Every Labs switch, and the Transforms page's own, sets its config flag by
// name. Switches for one flag stay in step, and a feature's page, card or
// list shows only while it's on. Turned off on its own page, that page stays
// in front until you leave it.
function showLab(flag, on) {
  document.querySelectorAll(`[data-lab="${flag}"]`).forEach(sw => sw.setAttribute("aria-checked", String(on)));
  if (flag === "transforms_enabled") {
    $("nav-transforms").hidden = !on;
    $("transforms-grid").classList.toggle("list-disabled", !on);
  }
  if (flag === "variable_recognition") $("code-editors-section").hidden = !on;
  if (flag === "calibration") $("cal-section").hidden = !on;
}
function initLabs(s) {
  document.querySelectorAll("[data-lab]").forEach(sw => {
    const flag = sw.dataset.lab;
    showLab(flag, !!s[labKey(flag)]);
    initSwitch(sw, async (on) => {
      await invoke("set_lab_flag", { name: flag, on });
      showLab(flag, on);
      if (flag === "scratchpad") loadScratchpad();
      if (flag === "lecture_mode") loadHome();
      if (flag === "calibration") loadPronunciation();
    });
  });
}

// ── settings page wiring ──
// `selected` (which engine set_asr_model chose) and `state` (installed vs.
// download, i.e. is it actually on disk) are independent — a model can be
// selected but not yet downloaded, mid-download. Only installed + selected
// is truly ACTIVE (it takes over on the next dictation, no restart); installed-but-not-selected offers a
// switch, not-installed always offers Download regardless of selection.
// assets_status()/get_settings() only ever say installed-or-not — the backend
// has no "downloading" state — so the live per-row progress a Download click
// needs is tracked entirely here, fed by the asset-progress/-ready/-error
// events initAssets() already listens for. download_assets() fetches every
// missing asset (runtime, ASR model, LLM…) as one undifferentiated stream
// keyed by asset name, not model id, so it's all mirrored onto whichever row
// the user actually clicked — the same simplification the banner above already makes.
let downloadingModelId = null;
let downloadProgress = null; // { name, pct } | null while downloadingModelId is set
let downloadError = null;
let modelsCache = [];
// What the speech model is doing (#118): { engine, state: "idle" | "loading" |
// "ready" }, from get_settings at boot and the asr-load event after.
let asrLoad = null;
function onAsrLoad(load) {
  asrLoad = load;
  renderModels(modelsCache);
}

// Browser preview only: an installed engine loads as soon as it's picked.
function mockLoad(id) {
  if (mock.models.find(m => m.id === id)?.state !== "installed") return;
  onAsrLoad({ engine: id, state: "loading" });
  setTimeout(() => onAsrLoad({ engine: id, state: "ready" }), 1800);
}

function renderModels(models) {
  modelsCache = models;
  const host = $("model-list");
  host.innerHTML = "";
  models.forEach((m, i) => {
    if (i) host.insertAdjacentHTML("beforeend", '<div class="divider"></div>');
    const row = document.createElement("div");
    row.className = "model-row";

    const downloadingThis = downloadingModelId === m.id;
    let rightStatus = "";
    let metaOverride = null;
    if (downloadingThis && downloadError) {
      rightStatus = `<button class="btn btn-primary model-download-btn" style="font-size:11px; padding:4px 10px; border-radius:6px;">Retry</button>`;
      metaOverride = `<span style="color:var(--mm-coral)" title="${escapeHtml(downloadError)}">Download stopped &middot; Retry picks up where it left off</span>`;
    } else if (downloadingThis) {
      const pct = downloadProgress ? downloadProgress.pct : 0;
      rightStatus = `
        <div class="model-progress">
          <span class="model-progress-label">${pct}%</span>
          <span class="model-progress-bar"><span class="model-progress-fill" style="width:${pct}%"></span></span>
        </div>`;
      metaOverride = downloadProgress ? `Downloading ${escapeHtml(downloadProgress.name)}&hellip;` : "Starting download&hellip;";
    } else if (m.selected && m.state === "installed") {
      rightStatus = `<span class="model-badge">ACTIVE</span>`;
      // Loaded when picked (#118); idle-unloaded, it just says what it is.
      const load = asrLoad && asrLoad.engine === m.id ? asrLoad.state : "idle";
      if (load === "loading") metaOverride = "Loading&hellip;";
      if (load === "ready") metaOverride = `Ready${m.meta ? ` &middot; ${escapeHtml(m.meta)}` : ""}`;
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
          <div class="name" style="font-weight: 500; color: var(--mm-ink);">${escapeHtml(m.name)} <span class="sub" style="color: var(--mm-muted-2); font-size: 11.5px; font-weight: 400;">${escapeHtml(m.variant)}</span>${m.id === "egyptian-small" ? ' <span class="badge-beta">EXPERIMENTAL</span>' : ""}</div>
          <div class="meta" style="font: 400 11.5px 'Hanken Grotesk'; color: var(--mm-muted-3); margin-top: 2px;">${metaOverride || (m.state === "installed" ? m.meta : `${m.meta} &middot; ${m.size}`)}</div>
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
// changes which engine is configured. It loads at once, or once its files
// land (#118), and asr-load tells the row; a Download keeps the old engine
// transcribing until then.
async function selectAsrModel(id, alsoDownload) {
  await invoke("set_asr_model", { model: id });
  if (alsoDownload) {
    downloadingModelId = id;
    downloadProgress = null;
    downloadError = null;
    renderModels(modelsCache); // immediate "starting…" feedback, don't wait for the first progress tick
    invoke("download_assets");
  }
  const s = await getSettings();
  renderModels(s.models || []);
}

// Parakeet is English-only and provably ignores the language setting, and
// egyptian-small always listens for Arabic (#68; asr.rs forces it): grey the
// picker out and say so, rather than let it silently do nothing. The stored
// choice stays, for the next engine picked.
let asrLanguage = "auto";
function updateLanguageDisabled(modelId) {
  const wrapper = $("asr-language-wrapper");
  const select = $("asr-language-select");
  const sub = $("asr-language-sub");
  if (!wrapper || !select) return;
  const fixed = { "parakeet-v3": "Parakeet only hears English.", "egyptian-small": "Egyptian Arabic always listens for Arabic." }[modelId];
  select.value = modelId === "egyptian-small" ? "ar" : asrLanguage;
  select.disabled = !!fixed;
  wrapper.classList.toggle("disabled", !!fixed);
  if (sub) sub.textContent = fixed || "The language you speak, or Auto-detect";
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
  $("capture-sub").textContent = "Click here, then press the key you want.";
}
function pickHotkey(name) {
  if (!HOTKEY_LABELS[name]) {
    $("capture-title").textContent = "Sotto can't use that key";
    $("capture-sub").textContent = "Pick one from the list below.";
    $("hotkey-capture").classList.remove("armed");
    return;
  }
  if (HOTKEY_RISKY[name] && !window.confirm(`Use ${HOTKEY_LABELS[name]} as your dictation key?\n\nYou probably use this key for other things too.`)) return;
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
  const name = eventCodeToName(ev.code);
  $("capture-title").textContent = `Captured: ${HOTKEY_LABELS[name] || name}`;
  pickHotkey(name);
});
$("hotkey-capture").addEventListener("mousedown", (ev) => {
  if (ev.button === 0) return;
  ev.preventDefault();
  const name = MOUSE_BUTTON_TO_KEY[ev.button];
  if (!name) return;
  $("hotkey-capture").classList.add("armed");
  $("capture-title").textContent = `Captured: ${HOTKEY_LABELS[name] || name}`;
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
      $("update-text").innerHTML = `The update didn't install. <a href="#" id="update-manual-link">Download it from GitHub</a>`;
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
// Handlers for the backend's asset-* events, keyed by event name so the
// browser preview's mockDownload() can drive the very same UI.
let assetEvents = {};
// The AI tier's files are still missing, so AI mode polishes with rules for now.
let aiWaiting = false;

// Browser preview only: fakes the selected model's download, dropping at 40%
// on the first try so progress, error, resume (Retry) and done are all visible.
let mockReceived = 0;
let mockDropped = false;
function mockDownload() {
  const m = mock.models.find(x => x.selected);
  const total = parseInt(m.size, 10) * 1048576;
  const tick = setInterval(() => {
    mockReceived = Math.min(total, mockReceived + total / 20);
    assetEvents["asset-progress"]({ name: m.name, received: mockReceived, total });
    if (!mockDropped && mockReceived >= total * 0.4) {
      clearInterval(tick);
      mockDropped = true;
      assetEvents["asset-error"](`downloading ${m.name}: got ${mockReceived} of ${total} bytes — connection dropped`);
    } else if (mockReceived >= total) {
      clearInterval(tick);
      m.state = "installed";
      assetEvents["assets-ready"](true);
      mockLoad(m.id); // the backend loads what landed (#118)
    }
  }, 150);
}

async function initAssets() {
  const banner = $("assets-banner");
  const fill = $("assets-fill");
  const text = $("assets-text");
  const dot = banner.querySelector(".banner-dot");
  banner.hidden = true;
 
  assetEvents = {
    "asset-progress": (p) => {
      p = p || {};
      dot.classList.add("amber");
      const pct = p.total ? Math.round((p.received / p.total) * 100) : 0;
      const mbNow = (p.received / 1048576).toFixed(0);
      const mbAll = p.total ? (p.total / 1048576).toFixed(0) : "?";
      banner.hidden = false;
      fill.style.width = pct + "%";
      text.textContent = `Downloading ${p.name}… ${pct}% (${mbNow} / ${mbAll} MB)`;
      text.title = "";
      // Mirror onto the Models row the user actually clicked Download for —
      // see the comment above renderModels() for why this isn't matched by name.
      if (downloadingModelId) {
        downloadProgress = { name: p.name, pct };
        renderModels(modelsCache);
      }
    },
    "assets-ready": async () => {
      fill.style.width = "100%";
      dot.classList.remove("amber");
      text.textContent = `All set. ${$("hint-verb").textContent} ${$("keycap-display").textContent} ${$("hint-tail").textContent}`;
      // First run: the banner is the only "you're done" signal, so it stays a while.
      setTimeout(() => { banner.hidden = true; }, aiWaiting ? 8000 : 1500);
      if (aiWaiting) { aiWaiting = false; loadHome(); } // AI tier is live now, no restart
      if (downloadingModelId) {
        downloadingModelId = null;
        downloadProgress = null;
        downloadError = null;
        const s = await getSettings();
        renderModels(s.models || []);
      }
    },
    "asset-error": (msg) => {
      banner.hidden = false;
      // Kept `.part` files resume on the next pass (#52), so nothing is lost.
      text.textContent = "Download stopped. Restart Sotto to resume where it left off.";
      text.title = String(msg); // the cause chain, for anyone who hovers
      if (downloadingModelId) {
        downloadError = String(msg);
        renderModels(modelsCache);
      }
    },
  };
  if (hasTauri && T.event) {
    for (const [name, fn] of Object.entries(assetEvents)) T.event.listen(name, (e) => fn(e.payload));
  }
  const status = await invoke("assets_status");
  if (!status || status.ready) return;
  banner.hidden = false;
  const missing = (status.missing || []).join(", ");
  text.textContent = `Downloading ${missing || "voice models"}…`;
  // First run: the progress banner lives in Settings, so open it (#54, README).
  openSettings();
  aiWaiting = (status.missing || []).some(n => /AI polish|llama/.test(n));
  loadHome();
  if (MOCK_FIRST_RUN) { mockDropped = true; mockDownload(); } // the backend auto-starts; the mock must too
  if (status.error) assetEvents["asset-error"](status.error); // it stopped before this window was built (#13)
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

function applyTheme(theme) {
  if (theme === "system") {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.dataset.theme = prefersDark ? "dark" : "light";
  } else {
    document.documentElement.dataset.theme = theme;
  }
}
if (opened.theme) applyTheme(opened.theme); // before the first paint

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
  const initTheme = s.theme || "system";
  applyTheme(initTheme);
  if (hasTauri && T.event) T.event.emit("theme-changed", initTheme);
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    const t = $("theme")?.dataset.value || "system";
    if (t === "system") { applyTheme("system"); if (hasTauri && T.event) T.event.emit("theme-changed", "system"); }
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
  setActivationCopy(s.activation);
  selectSegment($("polish"), s.polish);
  if ($("quote-style")) selectSegment($("quote-style"), s.quoteStyle || "straight");
  // The theme picker has no UI (theme follows the OS per the design doc), but
  // guard rather than assume: an unguarded null here killed the whole rest of
  // boot() — settings, dictionary, snippets, history — in one TypeError.
  if ($("theme")) selectSegment($("theme"), s.theme || "system");
  setThresholdUI(s.threshold);
  initToneUI(s);
  autoSend.apps = [...(s.autoSend || [])];
  renderAppList(autoSend);
  codeEditors.apps = [...(s.codeEditors || [])];
  renderAppList(codeEditors);
  asrLoad = s.asrLoad || null;
  asrLanguage = s.asrLanguage || "auto";
  renderModels(s.models || []);
  if ($("asr-language-select")) {
    $("asr-language-select").onchange = () => {
      asrLanguage = $("asr-language-select").value;
      invoke("set_asr_language", { language: asrLanguage });
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
  if ($("formatting-commands-toggle")) {
    $("formatting-commands-toggle").setAttribute("aria-checked", String(s.formattingCommands !== false));
    initSwitch($("formatting-commands-toggle"), (on) => invoke("set_formatting_commands", { enabled: on }));
  }
  if ($("number-formatting-toggle")) {
    $("number-formatting-toggle").setAttribute("aria-checked", String(s.numberFormatting !== false));
    initSwitch($("number-formatting-toggle"), (on) => invoke("set_number_formatting", { enabled: on }));
  }
  if ($("phonetic-correction-toggle")) {
    $("phonetic-correction-toggle").setAttribute("aria-checked", String(s.phoneticCorrection !== false));
    initSwitch($("phonetic-correction-toggle"), (on) => invoke("set_phonetic_correction", { enabled: on }));
  }
  if ($("overlay-always-visible-toggle")) {
    $("overlay-always-visible-toggle").setAttribute("aria-checked", String(!!s.overlayAlwaysVisible));
    initSwitch($("overlay-always-visible-toggle"), (on) => {
      invoke("set_overlay_always_visible", { enabled: on });
      if (hasTauri && T.event) T.event.emit("overlay-always-visible-changed", on);
    });
  }
  if ($("overlay-position")) {
    selectSegment($("overlay-position"), s.overlayPosition || "bottom-center");
    initSegmented($("overlay-position"), (v) => invoke("set_overlay_position", { position: v }));
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
      const confirmClear = confirm("Clear all Insights data? This can't be undone.");
      if (confirmClear) {
        await invoke("clear_stats");
        loadInsights();
        loadHome();
      }
    };
  }

  // Kept history (N4): consent on the way in, and off deletes the file, so
  // both directions ask first. Cancel puts the switch back.
  if ($("history-persist-toggle")) {
    const t = $("history-persist-toggle");
    t.setAttribute("aria-checked", String(!!s.historyPersist));
    initSwitch(t, (on) => {
      const ok = confirm(on
        ? "Keep History after restarts?\n\nSotto saves your dictated text to history.jsonl in your data folder. It stays on this device and is never uploaded. Turning this off deletes the file."
        : "Stop keeping History?\n\nThis deletes the saved file. This session's list stays until Sotto quits.");
      if (!ok) { t.setAttribute("aria-checked", String(!on)); return; }
      invoke("set_history_persist", { enabled: on });
    });
  }
  if ($("clear-history-btn")) {
    $("clear-history-btn").onclick = async () => {
      if (!confirm("Clear the History list?")) return;
      await invoke("clear_history");
      historyEntries = [];
      renderHistoryPage(historyEntries);
      renderRecent(historyEntries);
    };
  }

  // Recording retention: toggle, size display, folder link, clear button.
  if ($("retention-enabled-toggle")) {
    $("retention-enabled-toggle").setAttribute("aria-checked", String(!!s.retentionEnabled));
    initSwitch($("retention-enabled-toggle"), (on) => invoke("set_retention_enabled", { enabled: on }));
  }
  if ($("recordings-size")) {
    const mb = s.recordingsSizeMb || 0;
    const cap = s.retentionMaxMb || 500;
    $("recordings-size").textContent = `${mb} MB of ${cap} MB used`;
  }
  if ($("recordings-folder-path") && s.recordingsDir) {
    $("recordings-folder-path").textContent = s.recordingsDir;
  }
  if ($("open-recordings-folder")) {
    $("open-recordings-folder").onclick = () => invoke("open_url", { url: s.recordingsDir || "" });
  }
  if ($("clear-recordings-btn")) {
    $("clear-recordings-btn").onclick = async () => {
      const confirmClear = confirm("Delete every kept recording? This can't be undone.");
      if (confirmClear) {
        await invoke("clear_recordings");
        $("recordings-size").textContent = `0 MB of ${s.retentionMaxMb || 500} MB used`;
      }
    };
  }

  initSegmented($("activation"), (v) => { invoke("set_activation", { mode: v }); setActivationCopy(v); });
  if ($("quote-style")) initSegmented($("quote-style"), (v) => invoke("set_quote_style", { style: v }));
  initSegmented($("polish"), (v) => { invoke("set_polish", { mode: v }); updateToneDisabled(v); });
  if ($("theme")) initSegmented($("theme"), (v) => {
    applyTheme(v);
    invoke("set_theme", { theme: v });
    if (hasTauri && T.event) T.event.emit("theme-changed", v);
  });
  initSwitch($("launch-login"), (on) => invoke("set_launch_login", { enabled: on }));
  initSwitch($("start-hidden"), (on) => invoke("set_start_hidden", { enabled: on }));

  // Dictionary & Snippets page data partitioning
  dictEntries = (s.dictionary || []).filter(e => !isSnippet(e)).map(e => ({ ...e, aliases: [...(e.aliases || [])] }));
  snipEntries = (s.dictionary || []).filter(e => isSnippet(e)).map(e => ({ ...e, aliases: [...(e.aliases || [])] }));

  // One shared flag governs both pages — see setReplacementsEnabled.
  replacementsEnabled = s.replacementsEnabled !== false;
  if ($("dict-master-toggle")) {
    $("dict-master-toggle").setAttribute("aria-checked", String(replacementsEnabled));
    initSwitch($("dict-master-toggle"), setReplacementsEnabled);
  }
  if ($("snip-master-toggle")) {
    $("snip-master-toggle").setAttribute("aria-checked", String(replacementsEnabled));
    initSwitch($("snip-master-toggle"), setReplacementsEnabled);
  }

  renderDictPage(dictEntries);
  renderSnipPage(snipEntries);
  initTransforms(s);
  // A rebuilt window (#13) opens on Home: the backend may still think the pad is in front.
  loadScratchpad().then(syncPad);

  // History data
  historyEntries = s.history || [];
  loadHistory();

  // Pronunciation trainer data
  pronVocabulary = (s.vocabulary || []).map(v => ({ word: v.word, heardAs: v.heardAs || [], recent: v.recent || [] }));
  initLabs(s);
  loadPronunciation();

  // A page the tray menu or the Scratchpad chord asks for.
  const goTo = (page) => {
    if (page && document.querySelector(`.nav-item[data-page="${CSS.escape(page)}"]`)) navigate(page);
  };

  // Live event listeners
  if (hasTauri && T.event) {
    T.event.listen("history-updated", (e) => {
      historyEntries = e.payload || [];
      renderHistoryPage(historyEntries);
      renderRecent(historyEntries);
    });
    T.event.listen("pronunciation-sample", async (e) => {
      const { word, heard, matched } = e.payload || {};
      if (calState !== "idle" && word === calSentences[calIndex]) return calResult(heard);
      if (!pronArmedWord || word !== pronArmedWord) return;
      const focusEl = $("pron-focus-word");
      focusEl.classList.add(matched ? "matched" : "missed");
      setTimeout(() => pronSetState("idle"), matched ? 700 : 500);
      pronAddSampleRow(word, heard, matched);
      const s = await getSettings();
      pronVocabulary = (s.vocabulary || []).map(v => ({ word: v.word, heardAs: v.heardAs || [], recent: v.recent || [] }));
      renderTrainedWords();
      document.querySelectorAll("#pron-trained-list .pron-trained-row").forEach(r => {
        if (r.querySelector(".pron-trained-word")?.textContent === word) {
          const ring = r.querySelector(".pron-strength");
          ring?.classList.add("pron-level-up");
          setTimeout(() => ring?.classList.remove("pron-level-up"), 500);
        }
      });
    });
    T.event.listen("overlay-level", (e) => {
      if (pronState !== "listening") return;
      const amp = Math.min(1, (e.payload || 0) * 8);
      $("pron-focus-word")?.style.setProperty("--amp", amp.toFixed(3));
    });
    T.event.listen("word-detected", (e) => {
      // Parakeet heard the trained word mid-take — ignite the glow for real
      // (not just mic level), then auto-stop shortly after: say it, it lights
      // up, done. The pronState guard makes this fire once per listen.
      if (pronState !== "listening" || e.payload !== pronArmedWord) return;
      $("pron-focus-word")?.classList.add("detected");
      const status = $("pron-status");
      if (status) status.textContent = "Got it ✓";
      setTimeout(() => {
        if (pronState === "listening") {
          invoke("stop_dictation");
          pronSetState("resolving");
        }
      }, 700);
    });
    T.event.listen("overlay-state", (e) => {
      if (["idle", "error", "cancelled", "nomodel"].includes(e.payload)) pronTakeEnded(e.payload);
    });
    T.event.listen("paused-changed", () => loadHome());
    T.event.listen("lecture-changed", () => loadHome());
    T.event.listen("scratchpad-updated", (e) => { pad.rows = e.payload || []; renderPad(); });
    T.event.listen("asr-load", (e) => onAsrLoad(e.payload || null));
    T.event.listen("navigate", (e) => goTo(e.payload));
    // Fires on every worker outcome — null once a take is delivered, retried,
    // or dismissed, which is what actually takes the card off the screen.
    T.event.listen("take-changed", (e) => {
      renderTakeAlert(e.payload || null);
      // A take was stashed undelivered. Covers a trainer take whose error
      // state was held back because a newer take was already recording.
      if (e.payload && (pronState === "resolving" || calState === "resolving")) pronTakeEnded("error");
    });
  }

  initUpdates();
  initAssets();
  goTo(opened.page);
}
boot();
