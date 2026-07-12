# Sotto — Design Brief (prompt for Claude Design)

Copy everything in the box below into Claude Design (or a fresh Claude Design session). It's written as a standalone brief so it doesn't need this repo for context.

---

> **Project:** Design the visual identity and UI for **Sotto**, a local, offline voice-dictation app for Windows. You hold a hotkey, speak, release, and your speech is transcribed and typed into whatever app you're in. The name is from *sotto voce* — "in a quiet voice." The feeling should be **calm, quiet, precise, and unobtrusive** — this is a utility that lives at the edge of attention, not a flashy consumer app. Think: the refined restraint of a good pro audio tool or a native macOS menubar utility, not a colorful startup landing page.
>
> **Critical constraint — this is rendered in Rust (egui, immediate-mode GUI), not the web.** Design only what's achievable with: flat fills, translucency (alpha), simple linear gradients, rounded rectangles, 1–2 embedded fonts, and simple procedural animation (pulsing, moving bars, fades, rotation). **Avoid**: true backdrop blur/glassmorphism, soft multi-layer drop shadows, SVG filters, images/photos, anything that needs a browser. If you show a glass effect, achieve it with flat translucency, not blur. Keep every element buildable from primitives.
>
> **Deliverables (please produce all four):**
>
> **1. The active-state overlay ⭐ (most important).** A small always-on-top pill that appears near the bottom-center of the screen only while Sotto is working, then fades away. It sits over the user's other apps and must read instantly against any background (dark or light desktop). Target size roughly **240 × 56 px**, fully rounded ends. Design each of these states as a distinct frame:
>    - **Listening** (recording, hotkey held): the hero state. A mic glyph + a **live audio waveform** — 5–9 vertical bars that rise/fall with the user's voice level. Label "Listening" or none. This state should feel *alive* and reassuring.
>    - **Transcribing** (key released, model running): the waveform freezes/collapses into an indeterminate progress motion (e.g. a traveling dot or gentle shimmer). Label "Transcribing".
>    - **Polishing** (an AI cleanup step runs afterward): similar to transcribing but visibly distinct (different accent / a small sparkle motif). Label "Polishing".
>    - **Done**: a brief success tick that flashes then fades out (~400 ms).
>    - **Error / timeout**: a muted warning treatment (not alarming red — restrained). Label e.g. "Didn't catch that".
>    Show every state on BOTH a dark and a light desktop background so contrast is proven.
>
> **2. Motion spec** for the listening waveform and the transcribing/polishing indicators: describe the animation (what moves, amplitude, easing, loop duration in ms). Keep it subtle — 60fps-cheap, no frantic motion.
>
> **3. The settings window** (a normal resizable desktop window, opened from the tray). It should respect Windows **light and dark themes — show both**. Organize into clear sections: **Hotkey** (with a "press a key to rebind" capture control; default is Right Ctrl, hold-to-talk), **Activation** (hold vs toggle), **Speech model** (picker, shows download state/size), **Polish** (off / rules-only / AI, with a tier threshold slider), **Dictionary & snippets** (editable list), **History** (recent dictations, click to re-copy), **Startup** (launch on login). Prioritize legibility and a quiet, dense-but-breathable layout on an 8px spacing grid.
>
> **4. The system-tray icon**, in idle and active variants (small, legible at 16–32 px), plus the tray right-click menu (Pause, Polish on/off, Settings, Quit).
>
> **Aesthetic direction:**
> - **Base:** a deep, slightly-warm charcoal (near-black), used translucent for the overlay (~85–92% opacity).
> - **One primary accent** that signals "live/listening" — propose a refined hue that reads as calm-but-alert (a soft cyan or a muted electric blue tends to work; avoid neon). Use a secondary warm tone (amber) only for the processing/polishing states, and a desaturated rose for errors. State = color, so the user reads status peripherally without reading text.
> - **Type:** one clean humanist/geometric sans (e.g. Inter, or similar). Tight, small, confident. No more than two weights.
> - **Corners:** generous radii; the overlay is a full pill. **Grid:** 8px. **Restraint over decoration.**
>
> **Please also output a short implementation spec** alongside the mockups: exact hex values for every color and state, font family + sizes (px), corner radii, the overlay's exact dimensions, and animation durations — so it can be reproduced faithfully in egui.
>
> Do not make it look templated or generic-AI. Make deliberate, quiet, high-craft choices.

---

## Notes for us (not part of the prompt)

- Once Claude Design returns mockups + the hex/size spec, that spec drives the egui implementation of `overlay.rs` (Phase 3) and the settings window.
- The overlay states map 1:1 to Sotto's internal state machine: `Idle(hidden) → Listening → Transcribing → (Polishing) → Done/Error`.
- Keep the accent hex it picks in sync with the tray icon (currently a placeholder violet `#7C3AED` in `tray.rs` — replace with the chosen accent).
