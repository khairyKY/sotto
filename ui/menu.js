const hasTauri = !!window.__TAURI__;
const T = hasTauri ? window.__TAURI__ : null;
const invoke = hasTauri ? T.core.invoke : () => null;
const currentWin = hasTauri && T.window ? T.window.getCurrentWindow() : null;

// Sync Theme
function applyTheme(theme) {
  if (theme === "system") {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.dataset.theme = prefersDark ? "dark" : "light";
  } else {
    document.documentElement.dataset.theme = theme;
  }
}

// A lecture being captured (#23) outranks "Paused" in the header: the
// microphone is open either way.
let lectureOn = false;
function setPauseUI(paused) {
  document.getElementById("pause-item").classList.toggle("checked", paused);
  document.getElementById("pause-label").textContent = paused ? "Resume dictation" : "Pause dictation";
  document.getElementById("status-text").textContent = lectureOn ? "Capturing" : paused ? "Paused" : "Ready";
  // Green reads as "live"; muted while paused, same as Home's status dot.
  document.querySelector(".status-dot").style.background = paused && !lectureOn ? "var(--mm-muted-2)" : "";
}

// Pull live state (theme, paused, retry availability). The menu window
// persists across opens — it's hidden, not destroyed — so this must run on
// every show, not just once at load.
async function refreshState() {
  if (!hasTauri) return;
  try {
    const s = await invoke("get_settings");
    if (!s) return;
    if (s.theme) applyTheme(s.theme);
    // Enable Retry only when the worker actually has a stashed take —
    // history alone doesn't mean there's anything to retry.
    const retry = document.getElementById("retry-item");
    retry.classList.toggle("disabled", !s.hasTake);
    document.getElementById("retry-meta").textContent = s.hasTake ? "ready" : "none yet";
    lectureOn = !!s.lecture;
    const lecture = document.getElementById("lecture-item");
    lecture.hidden = !s.lectureMode;
    lecture.classList.toggle("checked", lectureOn);
    document.getElementById("lecture-label").textContent = lectureOn ? "Stop lecture capture" : "Start lecture capture";
    setPauseUI(!!s.paused);
  } catch (err) {
    console.error("Failed to load Sotto settings:", err);
  }
}

if (hasTauri) {
  T.event.listen("theme-changed", (e) => applyTheme(e.payload));
  // Refresh on every show; hide when clicking away.
  currentWin.onFocusChanged((event) => {
    if (event.payload) refreshState();
    else currentWin.hide();
  });
}

// Click Handlers
document.querySelectorAll(".menu-item").forEach(item => {
  item.onclick = async (e) => {
    if (item.classList.contains("disabled")) return;
    const action = item.dataset.action;
    if (!action) return;

    if (hasTauri) {
      await invoke("menu_action", { action });
      // Pause toggles in place rather than closing the menu, so a user can
      // glance at the new state before deciding what else to do.
      if (action === "pause") {
        const nowPaused = !document.getElementById("pause-item").classList.contains("checked");
        setPauseUI(nowPaused);
      } else {
        currentWin.hide();
      }
    } else {
      console.log("Mock Action:", action);
    }
  };
});

refreshState();
