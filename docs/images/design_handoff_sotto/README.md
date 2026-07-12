# Handoff: Sotto — Visual Identity & UI

## Overview
Sotto is a local, offline, hold-to-talk voice-dictation utility for Windows. Hold a hotkey, speak, release, and speech is transcribed and typed into whatever app has focus. This package specifies the full visual identity and UI: the active-state overlay pill, its motion, the settings window (light + dark), and the system-tray icon + menu.

The feeling to preserve above all: **calm, quiet, precise, unobtrusive** — a utility living at the edge of attention. Restraint over decoration. State is communicated by **color** so status reads peripherally without reading text.

## About the design files
`Sotto.dc.html` is a **design reference created in HTML** — a prototype showing intended look and behavior, not production code to copy. The overlay states, waveform, and tray pulse are rendered on an HTML `<canvas>` specifically to mirror what egui does: paint a desktop, then paint the pill on top at reduced alpha (real translucency, no blur). 

The implementation target is **Rust + egui (immediate-mode GUI)**. Recreate these designs using egui primitives — `Painter::rect_filled`, `rect_stroke`, `circle_filled`, `line_segment`, `add(Shape::Path/CubicBezier)`, and `Color32` with alpha. Everything here is deliberately buildable from: flat fills, translucency (alpha), simple linear gradients, rounded rectangles, 1–2 embedded fonts, and cheap procedural animation. **Do not** reach for backdrop blur, glassmorphism, multi-layer soft shadows, SVG filters, or raster images — none are needed and none fit the aesthetic.

## Fidelity
**High-fidelity.** Colors, type sizes, radii, dimensions, and animation timings are final. Reproduce faithfully. The one open product choice already made with the client: the overlay uses the **"Quiet" direction (1a)** — S mark + bare waveform, no label, state read by color — as the default. Directions 1b (labeled + level meter) and 1c (expressive ring + mirrored wave) are documented in the HTML as alternatives but are **not** the shipping treatment.

---

## The S mark (logomark)
Not a typographic "S" — a drawn, flowing double-arc squiggle (evokes both an "S" and a soft sound wave). Stroke-based, rounded caps, no fill.

- **Canonical path**, normalized to a 16×16 box:
  `M11.3 4.4 C11.3 4.4 5.2 3.6 5.2 6.5 C5.2 9.2 11.4 8 11.4 10.8 C11.4 13.6 5.4 12.8 5.4 12.8`
- Render as a cubic-bezier stroke. **Stroke width ≈ 12% of the mark's box size** (e.g. 1.9px in a 16px box, 2.1px at overlay scale). Round line caps and joins.
- Color = current state accent (see below). In the tray tile it sits centered in a charcoal rounded square.
- In egui: build with `epaint::CubicBezierShape` segments (three curves) or a flattened `Shape::Path` with `PathStroke` rounded caps.

---

## Deliverable 1 — Active-state overlay

### Geometry (exact)
- **Pill size: 200 × 56 px.** Fully rounded ends → **corner radius 28 px** (= height / 2).
- Position: **bottom-center** of the primary display, **22 px above the bottom edge**.
- Always-on-top, click-through, no window chrome.

### Pill background stack (paint in this order)
1. Rounded-rect fill `#1D1B18` at **alpha 0.90** (≈ `Color32::from_rgba_unmultiplied(29,27,24,230)`).
2. A top highlight: linear gradient over the top **55%** of the pill height, from white @ **7% alpha** → transparent. (Fake the "glass" edge with flat translucency, not blur.)
3. 1px inner border stroke: white @ **11% alpha**.

### Layout inside the pill
- S mark anchored at **x = left + 17 px**, vertically centered. Mark occupies ~16px tall.
- Content (waveform / track / label) begins ~**14 px** right of the mark and runs to **right − 18 px**.

### The five states (accent = the whole status signal)
| State | Accent hex | Content |
|---|---|---|
| **Listening** | `#4FCFDB` (cyan) | S mark + live waveform. No label. Hero state. |
| **Transcribing** | `#E3A857` (amber) | Waveform collapses to a dim flat track; a traveling dot sweeps it. Label "Transcribing". |
| **Polishing** | `#F0C982` (gold) + `#FCEBC6` (sparkle) | Same traveling dot, warmer, plus small 4-point sparkles twinkling above/below. Label "Polishing". |
| **Done** | `#4FCFDB` (cyan) | A check strokes on, holds, then the whole pill fades out (~400ms) and stops rendering. |
| **Error / timeout** | `#D0959C` (rose) | Rose "!" glyph + label "Didn't catch that". No red, no shake, no flash. |

Every state must read on **both** a dark and a light desktop — the pill's own charcoal + alpha guarantees this; do not tint the pill by desktop.

### Listening waveform (the hero)
- A continuous wave across the content area (equivalent of 5–9 samples; drawn as a smooth polyline of ~56 points).
- Wave value per point `u∈[0,1]`: `sin(u·7π − φ·2)·0.58 + sin(u·2.6π + φ)·0.42`, multiplied by a **sin-window taper** `sin(u·π)` so it's zero at both ends (never clips the pill), times amplitude × the live level envelope.
- **Peak amplitude ±12 px.**
- **Level envelope**: retarget to a new random value in `[0.22, 0.97]` every **110 ms**; each frame lerp current→target by **k = 0.12**. (In production, drive the target from real mic RMS instead of random.)
- **Phase** `φ = t · 0.005` (t in ms) → drifts sideways.
- Draw two passes for a soft bloom: a **5px** underlay at **22% alpha**, then the **2.4px** solid accent line on top.

---

## Deliverable 2 — Motion spec
All motion is 60fps-cheap and never frantic. Nothing scales, bounces, or overshoots.

- **Pill enter**: opacity 0→1 + translateY 4→0 px, **180 ms**, ease-out.
- **Pill exit**: opacity 1→0 + translateY 0→4 px, **260 ms**, ease-out.
- **Listening wave**: phase loop ≈ **1260 ms** (seamless); envelope retarget every 110 ms, lerp k=0.12/frame (see above).
- **Transcribing**: a dot + a ~22px leading gradient trail travels left→right across the track. **1150 ms per pass**, ease-in-out on position, repeats. Track is a 2px line in amber @ 30% alpha. Dot radius 3px.
- **Polishing**: same traveling dot at **1250 ms/pass**, gold. Plus sparkles: spawn one every **~210 ms** at a random point along the track, y-offset ±11px; each sparkle lives **760 ms**, scale follows `sin(age·π)` (0→1→0), size 4–6.5px, drawn as a 4-point twinkle (+ and × strokes), color `#FCEBC6`.
- **Done**: check path draws 0→full over **260 ms** ease-out; hold **140 ms**; then fade opacity+track to 0 over **400 ms**.
- **Error**: fade in **160 ms**; dwell **~2400 ms** (readable); fade out **300 ms**.
- **Tray active pulse**: an outline ring, scale 1→1.7 + opacity 0.55→0, over **1800 ms**, ease-out, looping.

**Easing reference:** ease-out = `1−(1−t)³`; ease-in-out = `t<.5 ? 2t² : 1−(−2t+2)²/2`.

---

## Deliverable 3 — Settings window
A normal resizable desktop window opened from the tray. Honors Windows light + dark themes. Natural minimum width **548 px**. 8px spacing grid; dense but breathable. Sections top-to-bottom: **Hotkey, Activation, Speech model, Polish, Dictionary & snippets, History, Startup.**

### Layout
- Title bar height **44 px**: left = 18px S-mark tile + "Sotto — Settings"; right = min/max/close (each 44×34 hit area).
- Body padding **20px vertical / 22px horizontal**; **22px** gap between sections.
- Each section = an uppercase 11px header (letter-spacing .07em, muted) above a card.
- Cards: radius **10 px**, 1px border, 13–16px internal padding. Rows split by 1px dividers.
- Controls: inputs/buttons/segmented-toggles radius **8 px**; keycap radius **6 px**.

### Section specifics
- **Hotkey**: label "Dictation key", subtitle "Hold to talk — release to transcribe". Right side shows the current key as a keycap (default **Right Ctrl**, monospace) + a "Rebind" button. The **capture state** (shown in the light mock) swaps the keycap for a dashed-outline pill "Press any key…" with a blinking accent dot + a "Cancel" affordance.
- **Activation**: segmented control **Hold / Toggle** (default Hold).
- **Speech model**: radio-list of models. Each row: radio, name + sublabel, a state pill (Installed / Download / downloading), and size on the right. Example rows: *Parakeet v3 · English* (Installed, 670 MB, selected), *Parakeet v3 · multilingual* (Download, 720 MB), *Whisper large-v3* (downloading with a 62% progress bar).
- **Polish**: segmented **Off / Rules / AI** (default AI). Below it a **tier-threshold slider** "Use AI past — 18 words" (clips shorter than the threshold use instant rules, skipping the LLM).
- **Dictionary & snippets**: editable rows of `spoken → replacement` (e.g. "gee pee tee → GPT", "my email → dev@sotto.app", "arrow → →"), each with a delete ✕, plus a "+ Add entry" affordance.
- **History**: rows of `time · text`, muted "Click an entry to copy it again" hint; clicking re-copies. Show 3 recent examples.
- **Startup**: two toggle rows — "Launch Sotto at login" and "Start hidden in the tray" (both on).

### Theme tokens
**Dark:** window `#201F22`, card `#26252A`, input/keycap `#17161A`, text `#E8E5DF`, secondary text `#928E85`, section header `#7C786F`, divider white@6%, title bar `#26252A`. Accent `#4FCFDB`.
**Light:** window `#FBFAF8`, card `#FFFFFF`, input `#FBFAF8`, text `#23221F`, secondary text `#6F6C64`, section header `#8A857B`, divider black@6%, title bar `#EFEDE8`. Accent `#1B93A1` (darker cyan for AA contrast on light).

> Note the accent shift: use `#4FCFDB` on dark surfaces, `#1B93A1` on light surfaces. Same hue family, tuned for contrast.

---

## Deliverable 4 — System tray
- **Icon**: the S mark, stroke = accent, centered in a charcoal rounded-square tile `#1D1B18` (radius scales: 4px @16, 6px @24, 8px @32). The dark tile makes it read on any taskbar color.
  - **Idle**: mark stroke `#9A968C` (muted stone).
  - **Active (listening)**: mark stroke `#4FCFDB` + the pulsing outline ring (see motion). Replaces the current placeholder violet `#7C3AED` in `tray.rs`.
- Provide the icon at **16 / 24 / 32 px**.
- **Right-click menu** items in order: header row (S tile + "Sotto" + status "Ready" in accent), divider, **Pause dictation** (⏸), **Polish** (checkmark + current tier "AI"), divider, **Settings…**, divider, **Quit Sotto**. Menu radius 9px; item radius 5px; selected/active item gets a faint accent-tinted background (cyan @ ~13% on dark, @ ~12% on light).

---

## Design tokens (complete)

### Colors
- Live / Listening / Done accent (on dark): `#4FCFDB`
- Accent on light surfaces: `#1B93A1` (hover `#157e8a`)
- Transcribing: `#E3A857`
- Polishing: `#F0C982`; sparkle `#FCEBC6`
- Error: `#D0959C`; error text `#CBA0A5`
- Overlay pill fill: `#1D1B18` @ **alpha 0.90**; border white @ 11%; top highlight white @ 7%
- Overlay label: `#ECE8E1`; overlay muted label: `#9E998E`
- Tray tile: `#1D1B18`; idle mark: `#9A968C`; light-idle mark: `#B7B2A8`
- Settings dark: `#201F22` / `#26252A` / `#17161A` / text `#E8E5DF`
- Settings light: `#FBFAF8` / `#FFFFFF` / text `#23221F`

### Spacing
8px grid. Common steps used: 8, 12, 14, 16, 20, 22px.

### Typography
- **Inter** — weights **400, 500, 600** only. (Humanist geometric sans; embed the .ttf/.otf in the egui font stack.)
- **JetBrains Mono** — weights 400, 500 — for keycaps, sizes, times, technical labels.
- Sizes (px): overlay label 12–13 / 500; overlay mark ~16 tall; settings body 13 / 500; settings description 12 / 400; section header 11 / 600 uppercase (letter-spacing .07em); menu item 12.5 / 400; mono labels 10.5–12.

### Radii
- Overlay pill: 28 (full). Cards: 10. Inputs/buttons/toggles: 8. Keycap: 6. Menu: 9; menu item: 5. Tray tile: 4/6/8 (by size).

---

## Assets
No image or icon-font assets. The only mark is the S logomark (bezier path above). Icons in menus (⏸ pause bars, ✓ check, ! error, ⧉ copy) are simple glyphs — reproduce the pause/check/copy/error as small painted primitives in egui rather than shipping a font of them. Embed **Inter** and **JetBrains Mono** font files.

## Files
- `Sotto.dc.html` — the full interactive design reference (all four deliverables + motion demos + this spec, navigable from the left rail). Open it in a browser to see the live animation.

## Existing code touchpoints
The repo already has the state machine this maps onto — `Idle → Listening → Transcribing → Polishing → Done/Error`. `tray.rs` holds the placeholder icon to replace; `config.rs` holds hotkey/activation/model/polish settings that the Settings window edits; `polish.rs` implements the Off/Rules/AI tiers and the word-count threshold the Polish slider controls.
