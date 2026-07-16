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

// Initialize state
async function init() {
  if (hasTauri) {
    // Get settings to set initial theme and check history
    try {
      const s = await invoke("get_settings");
      if (s && s.theme) {
        applyTheme(s.theme);
      }
      
      // Check if there is history to enable Retry
      if (s && s.history && s.history.length > 0) {
        const last = s.history[0];
        const retry = document.getElementById("retry-item");
        retry.classList.remove("disabled");
        document.getElementById("retry-meta").textContent = last.time;
      }

      if (s) setPauseUI(!!s.paused);
    } catch(err) {
      console.error("Failed to load Sotto settings:", err);
    }
    
    // Listen for theme changes
    T.event.listen("theme-changed", (e) => applyTheme(e.payload));
    
    // Hide menu window when it loses focus (click away)
    currentWin.onFocusChanged((event) => {
      if (!event.payload) {
        currentWin.hide();
      }
    });
  }
}

function setPauseUI(paused) {
  document.getElementById("pause-item").classList.toggle("checked", paused);
  document.getElementById("pause-label").textContent = paused ? "Resume dictation" : "Pause dictation";
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

init();
