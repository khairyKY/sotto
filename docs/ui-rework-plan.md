# Sotto UI 2.0 — rework plan

> Feature extraction from the Wispr Flow screenshots (`Screenshots_inspo/`),
> re-imagined for Sotto's constraints: **$0 forever, fully offline, no
> accounts, no telemetry**. This doc is the input for Claude Design — every
> screen is described with layout, behavior, and exact copy.
>
> **Status: PLANNING. Nothing here is committed or pushed until approved.**

---

## 0. Ground rules

Everything below must hold for every feature:

- **Zero cost, zero cloud.** All data lives in `%APPDATA%\sotto` (or `D:\sotto`
  on the dev machine). No server, no account, no quota. The words-remaining
  meter, Upgrade-to-Pro, team sharing, referrals, and "top 2%" percentile
  ranking in Flow all exist because Flow has a backend — **cut them all**.
- **Privacy by default.** Stats store *numbers*, never transcript text.
  Dictation history stays in-memory (as today) unless the user opts into
  on-disk history.
- **Same design system.** Charcoal `#1D1B18`, cyan accent `#4FCFDB`, amber
  `#E3A857`, gold `#F0C982`, rose `#D0959C`, Inter + JetBrains Mono, the
  S-mark, light/dark themes, "calm, quiet, precise" — the existing
  `ui/settings.css` tokens are the palette. Flow's serif display font
  (used in its promo banners) maps to **keeping Inter but larger/lighter** —
  Sotto does not introduce a serif.
- **⚠ License decision needed:** README currently says *"proprietary and
  confidential. All rights reserved."* If Sotto is to "say open source to
  everyone," the repo needs a real OSS license (MIT recommended — permissive,
  zero-friction). **Owner call; not changed yet.**

### Cut list (Flow features we deliberately do NOT port)

| Flow feature | Why cut |
|---|---|
| Account / Plans & Billing / words-remaining / Upgrade to Pro | No backend, no payments — Sotto is free |
| Invite your team / Shared-with-team tabs / Get a free month | No accounts |
| "Top 2%" WPM percentile | Needs cohort data from a server; replaced with *personal best* |
| Download on mobile | Windows-only for now |
| Share button on Insights | Later, if ever (could export a PNG locally) |
| Vibe coding (IDE variable recognition, file tagging) | Server-side model features; the local equivalent is "casing-aware polish", already partly served by the dictionary. Revisit later |

---

## 1. Overlay pill rework — X (cancel) + Retry

### 1.1 Interaction spec

The pill gains **mouse interactivity** for the first time. Buttons are small
circular ghosts (~22 px) at the pill's right edge, matching pill translucency.

| State | Buttons shown | Behavior |
|---|---|---|
| `listening` | **✕** | Click ✕ = cancel (same as Escape). Recording stops, take is stashed, pill → `cancelled`. |
| `transcribing` / `polishing` | **✕** | Click ✕ = cancel at next stage boundary (same as Escape). |
| `cancelled` | **↺ Retry** | Pill lingers **6 s** (up from 1.9 s — needs to be long enough to click). Click ↺ = re-run the pipeline from the stashed take. |
| `error` | **↺ Retry** | Same stash + retry: ASR failed → re-run ASR; polish failed → re-polish; injection failed → re-inject. Lingers 6 s. |
| `done` | none | Unchanged (auto-fades ~0.8 s). |

Copy in the pill:
- `cancelled`: **"Cancelled"** + ↺ button, tooltip **"Retry this dictation"**
- `error`: **"Didn't catch that"** + ↺ button, tooltip **"Try again"**
- ✕ tooltip: **"Cancel (Esc)"**

### 1.2 The "take" — retry's backbone

The worker keeps the **last take** in memory (never on disk):

```
Take {
  samples: Vec<f32>,        // captured audio (16 kHz mono)
  raw_text: Option<String>, // set once ASR succeeded
  focus_target: isize,      // HWND captured at Start
  captured_at: Instant,
}
```

- **Cancel** no longer discards — it *stashes*. Accidental Escape is always
  recoverable until the next dictation starts (next Start overwrites the stash).
- **Retry** resumes from the furthest completed stage: if `raw_text` exists,
  skip ASR (instant re-polish + inject); else re-run ASR from `samples`.
- Retry re-targets the stashed `focus_target` (the existing
  `IsWindow`-checked `restore_focus` handles a since-closed window gracefully).

### 1.3 Retry expansion — everywhere the concept applies

Per "export the expansion of that feature to different aspects":

1. **Error pill → Retry** (above) — errors stop being dead ends.
2. **Home / History rows** — each recent dictation gets, on hover:
   **⧉ Copy again** (exists) and **↺ Re-polish** (re-runs the *current*
   polish tier + dictionary over that entry's text and copies the result —
   useful after fixing a dictionary entry or switching tiers).
3. **Last take slot on Home** — if the last take was cancelled/errored and
   never injected, Home shows a one-row card: *"Your last dictation wasn't
   delivered"* + **Retry** / **Dismiss**. Covers the case where the pill faded
   before the user could click.
4. **Tray menu** — add **"Retry last dictation"** item (greyed when no stash).

### 1.4 Click-through mechanics (implementation note)

The overlay is currently `set_ignore_cursor_events(true)` globally. New model:
a lightweight cursor poll (~30 ms, Rust side, `GetCursorPos` vs pill rect)
toggles ignore off only while the cursor is inside the pill bounds and a
button-bearing state is active. Everywhere else stays click-through, so the
pill never blocks the app underneath.

---

## 2. New app shell — from one settings window to a small app

Flow's model (sidebar + pages) fits Sotto's grown feature set. One window
(default ~940 × 640, min 760 × 520), custom title bar as today.

**Sidebar** (icons + labels, collapsible to icons):

```
S  Sotto                    ← S-mark tile + wordmark

   Home
   Insights
   Dictionary
   Snippets
   History
   ─────────
   Settings                 ← pinned to bottom
```

- No Transforms/Scratchpad in v2.0 nav (phase 2 — see §6; design them now,
  ship later).
- The old single settings window's sections get redistributed:
  Hotkey/Activation/Polish/Model → **Settings**; Dictionary → **Dictionary**
  page; History → **History** page (and a slice on Home).
- Tray "Settings…" opens this window on whatever page was last open.

---

## 3. Insights — the stats dashboard ($0 architecture)

### 3.1 What we record

Append one JSON line per completed dictation to `data_dir()/stats.jsonl`
(numbers only — **no transcript text**):

```json
{"ts":"2026-07-15T14:03:22Z","words":34,"audio_ms":11840,"app":"Code.exe",
 "tier":"ai","ai_changed_words":6,"dict_hits":1,"outcome":"injected"}
```

`outcome` ∈ `injected | cancelled | error`. Cancelled/errored takes still log
(words=0 unless transcribed) so the funnel is visible.

- **WPM** = `words / (audio_ms / 60000)` — speaking rate per dictation.
- **Fixes** = `ai_changed_words` (word-level diff raw→polished, computed at
  polish time with a simple split-token LCS) + `dict_hits` (counted in
  `apply_dictionary`).
- **App name** = process image name from the already-captured focus HWND
  (`GetWindowThreadProcessId` → `QueryFullProcessImageNameW`), e.g. `Code.exe`
  → display "VS Code" via a tiny built-in prettifier map, else the exe stem.
- **Time saved** = `words / 40 wpm` (typing baseline) − actual dictation time,
  floored at 0. Honest label: "vs. typing at 40 wpm".
- Aggregations (day buckets, streaks, totals) computed on page load — a few
  thousand JSONL lines parse in milliseconds; no database. `stats.jsonl` is
  excluded from the uninstaller's "keep data" default wipe question (it's
  inside the data dir, so the existing prompt already covers it).

### 3.2 Metrics shown (mapping Flow → Sotto)

| Flow | Sotto version |
|---|---|
| 107 WPM + "Top 2%" gauge | **Average speaking speed** (30-day) + **personal best** badge instead of percentile |
| 431 fixes made by Flow (words corrected / dictionary fixes) | **Fixes made by Sotto**: AI-corrected words + dictionary fixes, same two-row breakdown |
| 7,700 total words dictated + per-device | **Total words dictated** (lifetime) + this week/this month rows (no devices) |
| Desktop usage by app category (emails/messages/docs, % bars) | **Where you dictate**: top 5 apps by words, % bars (real app names — no category guessing, that was Flow server magic) |
| Streak + GitHub-style calendar + longest streak | Identical, locally computed. Cyan intensity ramp instead of Flow teal |
| Time saved | (Flow shows this on mobile) **Time saved** stat tile — nice free win |

---

## 4. Screen-by-screen specs for Claude Design

> Format: purpose → layout → exact copy. Shared chrome: existing custom title
> bar (S-mark tile, "Sotto", min/max/close), sidebar from §2, both themes.
> All type/dimension tokens from `ui/settings.css`.

### Screen 1 — Home

**Purpose:** land somewhere warm; see status + last dictations at a glance.

**Layout:** Greeting header. Row of 3 stat chips. "Recent dictations" list
(last 8, from in-memory history) with hover actions. If the last take was
undelivered, a recovery card sits above the list. Small footer status line.

**Copy:**
- Header: **"Good morning."** / "Good afternoon." / "Good evening." (time-based;
  no name — no accounts) with sub: **"Hold {hotkey} and speak — Sotto types
  it where your cursor is."** ({hotkey} = live binding, e.g. "Right Ctrl")
- Stat chips: **"{n} words dictated"** · **"{n} wpm average"** ·
  **"{n}-day streak"**
- Recovery card (conditional): title **"Your last dictation wasn't
  delivered"**, sub **"Cancelled 2 min ago · 34 words"**, buttons **"Retry"**
  (primary) / **"Dismiss"** (ghost)
- Recent list header: **"Recent dictations"**, hint **"Click to copy again"**
- Row hover actions: ⧉ **"Copy"** · ↺ **"Re-polish"**
- Empty state: **"Nothing dictated yet this session"** + sub **"Hold
  {hotkey}, speak, release. That's it."**
- Footer status: **"Parakeet v3 · AI polish on · mic: {device name}"**

### Screen 2 — Insights

**Purpose:** the dashboard. Personal, honest numbers; no gamified pressure.

**Layout:** 2 rows. Row 1: three stat cards — *Speaking speed* (big number +
semicircular gauge to personal best, cyan), *Fixes made by Sotto* (big number,
two sub-rows), *Total words* (big number, two sub-rows). Row 2: *Where you
dictate* (horizontal % bars, top 5 apps) and *Streak* (GitHub-style calendar,
~26 weeks visible, arrows to scroll).

**Copy:**
- Page title: **"Insights"**, sub **"All numbers live on this device. Nothing
  is uploaded — ever."**
- Card 1: **"{n}"** big, label **"words per minute"**, gauge caption
  **"personal best {n}"**, tooltip ⓘ **"Average speaking speed over your last
  30 days of dictation."**
- Card 2: **"{n}"** big, label **"fixes made by Sotto"**, rows:
  **"{n} words corrected"** ⓘ "Words the AI polish changed — fillers removed,
  self-corrections resolved, punctuation added." / **"{n} dictionary fixes"**
  ⓘ "Replacements from your dictionary and snippets."
- Card 3: **"{n}"** big, label **"total words dictated"**, rows: **"{n} this
  week"** / **"≈ {n} min saved vs. typing"** ⓘ "Compared with typing the same
  words at 40 wpm."
- Card 4: **"Where you dictate"**, right-aligned meta **"top apps · 30 days"**,
  bars: app name + **"{pct}%"** in-bar + **"{n} words"** after.
- Card 5: **"{n}-day streak"**, right meta **"longest {n} days"**, legend
  **"More / Less"**, empty-cell tooltip: date, filled: **"{date} · {n} words"**.
- Global empty state (no stats yet): **"Dictate once and your stats start
  here."**

### Screen 3 — Dictionary

**Purpose:** words Sotto should spell your way (short corrections).

**Layout:** Page header + **"Add word"** button (top right). Intro card
(dismissible, shown until first entry). Search field. Single-column list:
each row `spoken → replacement`, hover reveals ✎ edit / ✕ delete. Inline
add-row (same pattern as current settings). No tabs — no team.

**Copy:**
- Title: **"Dictionary"**, button **"Add word"**
- Intro card: heading **"Sotto spells the way you do."**, body **"Teach it
  names, jargon, and terms it keeps getting wrong — say the word, get your
  spelling. Everything stays on this device."**, example chips:
  `"gee pee tee" → GPT` · `"khairy" → Khairy` · `"arrow" → →`
- Search placeholder: **"Search your words…"**
- Row placeholders: **"spoken"** / **"replacement"**
- Empty state: **"No words yet — add the first one Sotto keeps mishearing."**

### Screen 4 — Snippets

**Purpose:** longer expansions — say a phrase, drop a paragraph. Same storage
as dictionary (one list, `kind: correction | snippet`, split by UI): a
snippet is just an entry whose replacement is long. Migration: existing
entries with replacement > 40 chars or containing a newline present as
snippets.

**Layout:** identical skeleton to Dictionary; rows show the trigger phrase
bold + first line of expansion truncated; ✎ opens a two-field editor
(trigger + multiline expansion).

**Copy:**
- Title: **"Snippets"**, button **"Add snippet"**
- Intro card: heading **"The stuff you shouldn't have to re-type."**, body
  **"Save text you use often — your email, an intro, a prompt — then say the
  trigger phrase to drop it in instantly."**, example chips:
  `"my email" → khairy@…` · `"intro line" → Hey, great to meet you —…`
- Editor labels: **"When I say"** / **"Sotto types"**
- Empty state: **"No snippets yet. What do you type every day?"**

### Screen 5 — History

**Purpose:** full recent-dictation list (in-memory session history today;
same data as Home's slice, longer).

**Layout:** list rows: time (mono, muted) + text (one line, ellipsis) +
hover ⧉ / ↺. Header hint. Privacy note pinned at bottom.

**Copy:**
- Title: **"History"**, hint **"Click any entry to copy it again"**
- Hover: **"Copy"** / **"Re-polish"**
- Footer note: **"History lives in memory and clears when Sotto quits.
  Nothing is written to disk."**
- Empty: **"Nothing dictated yet this session"**

### Screen 6 — Settings

**Purpose:** everything configurable, grouped like Flow's General/System but
in one scrolling page with section headers (Sotto has fewer knobs; no nested
settings-window-within-window).

**Sections & copy:**
- **Dictation**
  - "Dictation key" + keycap + **"Change…"** (opens existing press-any-key
    modal) · sub **"Hold to talk — release to transcribe"**
  - "Mode" segmented **Hold / Toggle** · sub **"Hold the key while speaking,
    or tap to start & stop"**
  - **"Microphone"** row + dropdown of input devices + **"System default"**
    first option · sub **"Which mic Sotto listens to"**  *(new feature: cpal
    device enumeration — free)*
- **Polish**
  - "Cleanup" segmented **Off / Rules / AI** · sub **"Off · Rules (instant,
    local) · AI (local LLM rewrite)"**
  - "Use AI past" slider · **"{n} words"** · sub **"shorter clips stay on
    instant rules"**
- **Speech model** — existing model list card (Parakeet v3 · installed ·
  size)
- **Sound** *(new, small + free)*
  - **"Dictation sounds"** toggle · sub **"A soft tick when recording starts
    and stops"** (two tiny bundled .wav, played via rodio)
- **Startup**
  - **"Launch Sotto at login"** toggle
  - **"Start hidden in the tray"** toggle
- **Updates** — existing About/updates card (version, check button,
  releases-page fallback)
- **Data & privacy**
  - **"Usage stats"** toggle · sub **"Counts words and timing on this device
    to power Insights. Never your words' content, never uploaded."**
  - **"Clear stats"** button (confirm: **"Delete all Insights data? This
    can't be undone."**)
  - Data folder row: path + **"Open folder"**

### Screen 7 — Overlay pill states (redesign of existing + new)

All existing states unchanged visually except sizing for buttons:
- `listening`: S-mark cyan + waveform + **✕** ghost button right.
- `transcribing`/`polishing`: unchanged + **✕**.
- `cancelled` *(new visual)*: rose ✕-glyph + **"Cancelled"** + **↺** button,
  6 s linger.
- `error`: rose "!" + **"Didn't catch that"** + **↺** button, 6 s linger.
- `done`: unchanged.

### Phase-2 screens (design now, build later)

**Screen 8 — Transforms.** Local-LLM text rewriting anywhere: select text in
any app, press a chord, Sotto rewrites in place (same clipboard-paste
machinery as dictation; the local Qwen does the rewrite — $0). Layout: opt-in
toggle top-right; card grid, each card = chord chips + name + one-line
description; **"Create new"** card opens editor (name, chord picker reusing
the hotkey modal, prompt textarea).
Copy: title **"Transforms"** + Beta pill; header **"Rewrite anywhere you
write."** body **"Select text in any app, press the shortcut, and Sotto's
local AI rewrites it in place. Your text never leaves this device."**
Default cards: **"Polish — tightens and clarifies without changing your
meaning"** (Ctrl+Alt+1) · **"Prompt engineer — restructures your idea into a
strong AI prompt"** (Ctrl+Alt+2) · **"Create your own — bring your own
prompt"**. Buttons: **"Reset to defaults"** / **"Create new"**.
*(Note: Win+Alt chords collide with Windows 11 system shortcuts — default to
Ctrl+Alt.)*

**Screen 9 — Scratchpad.** Dictate into a private local pad instead of an
app. Layout: header + **"New note"**; two-pane (recents list / editor);
notes = plain .md files in `data_dir()/scratchpad`. Copy: title
**"Scratchpad"**; header **"For thoughts you want to come back to."** body
**"Brain-dump an idea, draft a message, park a to-do — dictate here when the
words don't have a home yet."**; empty state **"No notes yet. Hold {hotkey}
and think out loud."**

---

## 5. Backend work implied (for later implementation phases)

| Piece | Size | Notes |
|---|---|---|
| Take stash + retry + cancel-stash | S | worker refactor in `main.rs` §1.2 |
| Overlay buttons + cursor-poll click-through | M | §1.4; overlay JS + Rust poll thread |
| `stats.rs` (jsonl append, aggregate, word-diff, app-name) | M | §3.1; +`Win32_System_ProcessStatus` or `QueryFullProcessImageNameW` feature |
| Commands: `get_stats`, `clear_stats`, `retry_last`, `set_history/stats toggles`, `get/set_microphone`, `list_microphones` | S | |
| Mic picker in `audio.rs` (device by name instead of default) | S | cpal enumerate |
| Dictation sounds (rodio + 2 wavs) | S | |
| Multi-page shell (sidebar SPA — plain JS routing, no framework) | M | keep vanilla JS, hash-routing |
| Transforms engine (selection-grab via Ctrl+C, LLM rewrite, paste-back, chord listener) | L | phase 2 |
| Scratchpad (md files CRUD) | S–M | phase 2 |

Phases: **2.0** = shell + Home + Insights + Dictionary/Snippets/History +
Settings + pill X/Retry. **2.1** = Transforms. **2.2** = Scratchpad.

---

## 6. Open questions for the owner

1. **License** — switch README/repo to MIT (or another OSS license)? Required
   before "open source" is true.
2. Home greeting uses no name (no accounts). OK, or add an optional local
   display-name field in Settings?
3. Streak framing: Flow gamifies ("0 day streak" guilt). Keep the calendar
   but drop the guilt-flavored zero state? Proposed zero copy: **"No
   dictation today — yet."**
