# Sotto — Marshmallow Theme — Extracted Design Spec

Source: `D:\Coding\sotto\Sotto Marshmallow.dc.html` (584 lines, Claude-Design "dc.html" handoff).
Extracted literally — every hex value, shadow recipe, font spec, copy string, SVG path, and layout number below was read verbatim from the source. Anchors covered: `#tokens #home #insights #dictionary #snippets #history #settings #transforms #scratchpad #pill #icons` — **all 11 anchors exist and are covered**, nothing missing.

Global fonts loaded: `Newsreader` (opsz,wght 400/500/600, serif display), `Hanken Grotesk` (400/500/600/700, body sans), `JetBrains Mono` (400/500, mono/labels).

Global keyframes (defined once, used across screens — see per-section notes for which element uses which):

```css
@keyframes mm-breathe{0%,100%{transform:scale(.8);opacity:.4}50%{transform:scale(1.05);opacity:.82}}
@keyframes mm-bar{0%,100%{transform:scaleY(.28)}50%{transform:scaleY(1)}}
@keyframes mm-rise{0%{transform:translateY(9px) scale(.4);opacity:0}25%{opacity:.95}72%{opacity:.55}100%{transform:translateY(-15px) scale(1.05);opacity:0}}
@keyframes mm-blob{0%,100%{transform:translate(0,0) scale(1)}33%{transform:translate(3px,-2px) scale(1.16)}66%{transform:translate(-3px,1px) scale(.88)}}
@keyframes mm-bounce{0%,100%{transform:translateY(4px)}50%{transform:translateY(-4px)}}
@keyframes mm-shimmer{0%{transform:translateX(-28px);opacity:0}50%{opacity:1}100%{transform:translateX(28px);opacity:0}}
@keyframes mm-draw{0%{stroke-dashoffset:26}22%,100%{stroke-dashoffset:0}}
@keyframes mm-twinkle{0%,100%{transform:scale(0);opacity:0}50%{transform:scale(1);opacity:1}}
```

`mm-bounce` is **defined but not used anywhere** in this doc — no screen references it. Flag as unused/reserved.

Base page chrome (doc-level, not app): `body{background:#DED5C7}`, link color `#6E58A8` / hover `#5c4894`, selection `rgba(169,143,224,.4)`.

---

## Tokens

### Light palette (box bg `#F0E9DF`, radius 18px, padding 22px 24px, box-shadow `0 20px 44px rgba(150,128,104,.24), 0 5px 12px rgba(150,128,104,.12)`)

| Token name | Hex | Swatch box | Notes |
|---|---|---|---|
| Window | `#DED5C7` | inset outline `0 0 0 1px rgba(150,128,104,.12)` | outermost app bg |
| Base | `#E6DFD4` | same inset outline | recessed-well / list bg |
| Surface | `#F0E9DF` | same inset outline | card / main-content bg |
| Tint | `#ECE5F4` | same inset outline | lavender highlight bg (active nav, chips, callouts) |
| Accent ink | `#6E58A8` | no outline | darker purple — text-on-light accent |
| Accent | `#8E74D0` | no outline | primary purple — buttons, active fills |
| Blush | `#F0BFCF` | no outline | pink — warning/error badge bg |
| Ink | `#332E3A` | no outline | primary text color |

### Dark palette (box bg `#2C2634`, radius 18px, padding 22px 24px, box-shadow `0 20px 44px rgba(20,14,26,.4), 0 5px 12px rgba(0,0,0,.28)`)

| Token name | Hex | Swatch box | Notes |
|---|---|---|---|
| Window | `#1E1A24` | inset outline `0 0 0 1px rgba(255,255,255,.05)` | |
| Base | `#241F2A` | same | |
| Surface | `#2C2634` | same | |
| Tint | `#3A3340` | same | |
| Accent ink | `#CBB9F0` | no outline | |
| Accent | `#C9B8EE` | no outline | |
| Blush | `#5B3F4E` | no outline | |
| Ink | `#EFE9F1` | no outline | |

### Canonical shadow recipes (as illustrated in the Tokens box itself)

| Name | Light | Dark |
|---|---|---|
| RAISED (illustrative) | `5px 5px 11px rgba(196,183,165,.34), -5px -5px 11px rgba(255,254,250,.55)` | `5px 5px 11px rgba(0,0,0,.5), -5px -5px 11px rgba(255,255,255,.035)` |
| INSET (illustrative) | `inset 3px 3px 7px rgba(196,183,165,.32), inset -3px -3px 7px rgba(255,254,250,.52)` | `inset 3px 3px 7px rgba(0,0,0,.5), inset -3px -3px 7px rgba(255,255,255,.035)` |

**Important:** the INSET example in the tokens box (`.32`/`.52`, 7px blur) is a rounded illustrative value. Actual usage across the app diverges slightly by context — see the full shadow catalog below. Always use the *actual per-element* recipe, not the tokens-box illustration, for pixel accuracy.

### Full shadow-recipe catalog (every distinct recipe found, with usage sites)

| # | Recipe (light) | Recipe (dark) | Used on |
|---|---|---|---|
| 1 | `0 30px 60px rgba(150,128,104,.30), 0 8px 18px rgba(150,128,104,.14)` | `0 30px 60px rgba(20,14,26,.5), 0 8px 18px rgba(0,0,0,.3)` | Outer app-window drop shadow (all app-shell mockups: Home/Insights/Dictionary/Snippets/History/Settings/Transforms/Scratchpad) |
| 2 | `0 20px 44px rgba(150,128,104,.24), 0 5px 12px rgba(150,128,104,.12)` | `0 20px 44px rgba(20,14,26,.4), 0 5px 12px rgba(0,0,0,.28)` | Tokens reference-card container only (doc chrome, not app UI) |
| 3 | `2px 2px 4px rgba(196,183,165,.40), -2px -2px 4px rgba(255,254,250,.55)` | `2px 2px 4px rgba(0,0,0,.5), -2px -2px 4px rgba(255,255,255,.04)` | Header logo mark (26×26 box, all screens) |
| 4 | `inset 2px 2px 4px rgba(188,172,206,.30), inset -2px -2px 4px rgba(255,254,250,.55)` | `inset 2px 2px 4px rgba(0,0,0,.5), inset -2px -2px 4px rgba(255,255,255,.045)` | Sidebar **active** nav item (note: light uses a lilac-tinted shadow base `rgba(188,172,206,x)`, not the warm `rgba(196,183,165,x)` used elsewhere) |
| 5 | `inset 3px 3px 7px rgba(196,183,165,.30), inset -3px -3px 7px rgba(255,254,250,.50)` | `inset 3px 3px 7px rgba(0,0,0,.5), inset -3px -3px 7px rgba(255,255,255,.035)` | Main content well (`<main>` bg, all screens); also Dictionary/Snippets/History list wells (bg Base) |
| 6 | `5px 5px 11px rgba(196,183,165,.34), -5px -5px 11px rgba(255,254,250,.55)` | `5px 5px 11px rgba(0,0,0,.5), -5px -5px 11px rgba(255,255,255,.035)` | All raised stat/card tiles: Home stat cards + alert banner, Insights 5 cards, Settings section-group cards, Transforms cards |
| 7 | `inset 3px 3px 6px rgba(196,183,165,.34), inset -3px -3px 6px rgba(255,254,250,.50)` | `inset 3px 3px 6px rgba(0,0,0,.55), inset -3px -3px 6px rgba(255,255,255,.03)` | Home "Recent" list well only (6px blur, distinct from recipe 5's 7px blur; dark variant also bumps shadow opacity to `.55` and highlight down to `.03`) |
| 8 | `inset 2px 2px 4px rgba(196,183,165,.34), inset -2px -2px 4px rgba(255,254,250,.5)` | `inset 2px 2px 4px rgba(0,0,0,.5), inset -2px -2px 4px rgba(255,255,255,.03)` | Small inset wells: search bars (Dictionary/Snippets), Insights progress-bar tracks, Settings toggle/slider tracks & segmented-control backgrounds, keycap chips |
| 9 | `4px 4px 10px rgba(196,183,165,.40), -4px -4px 10px rgba(255,254,250,.60)` | `4px 4px 10px rgba(0,0,0,.5), -4px -4px 10px rgba(255,255,255,.035)` | Overlay pill body (raised) — same opacities as recipe 3/6 family but 4/10 offset-blur instead of 2/4 or 5/11 |
| 10 | `inset 2px 2px 5px rgba(196,183,165,.2), inset -2px -2px 5px rgba(255,254,250,.4)` | *(no dark variant shown)* | Pill "Motion spec" dashed-border callout box (light only in doc) |
| 11 | `0 8px 20px rgba(120,100,150,.26)` (cream bg) / `0 8px 20px rgba(100,74,168,.32)` (lilac bg) / `0 8px 20px rgba(0,0,0,.48)` (dark variant) | — | App-icon squircle drop shadow (96px); scales down to `0 4px 10px rgba(120,100,150,.24)` at 48px and `0 3px 7px rgba(120,100,150,.22)` at 32px — **not neumorphic, plain single drop-shadow** |
| 12 | `0 18px 44px rgba(150,128,104,.28)` | `0 18px 44px rgba(0,0,0,.55)` | Tray right-click context menu panel |
| 13 | `0 1px 3px rgba(150,128,104,.4)` | *(not shown dark)* | Settings slider-handle knob shadow |

---

## Home

`id="home"`. Section eyebrow: badge `tokens`-style pill (`font:600 12px 'JetBrains Mono'`, bg Tint, color Accent ink, `padding:4px 9px`, `border-radius:6px`) reading **"home"**; title `App shell · Home` (`font:500 18px Newsreader`, Ink); caption `labeled sidebar · recessed content well · raised cards` (`font:400 13px 'Hanken Grotesk'`, color `#8E847A` — undeclared doc-chrome grey, repeats on every section header).

### Window & shared app-shell chrome (applies to Home, Insights, Dictionary, Snippets, History, Settings, Transforms, Scratchpad unless noted)

- Outer window: `border-radius:20px`, `overflow:hidden`, `display:flex;flex-direction:column`. Home size **700×500**. Shadow = recipe 1. bg = Base token (`#E6DFD4` light / `#241F2A` dark).
- **Header bar**: `height:44px`, `padding:0 16px`, flex row `justify-content:space-between`.
  - Left cluster (gap 10px): logo mark — `26×26` box, `border-radius:8px`, bg `#EBE3F5` light / `#3A3340` dark (Tint dark, but light value `#EBE3F5` is a **near-duplicate of Tint `#ECE5F4`, NOT identical** — flag), centered wave-mark SVG (see Icons catalog, "wave · small"), icon color Accent ink light / Accent dark. Then wordmark "**Sotto**" — `font:500 15px Newsreader`, color `#332E3A` light (= Ink token, exact match) / `#EEE7F2` dark (**does NOT exactly match Ink dark token `#EFE9F1`** — off by one hex — flag as inconsistency to reconcile at implementation time).
  - Right cluster (gap 13px, color `#AEA394` light / `#6E6676` dark): minimize = flat bar `11×2px`, `border-radius:2px`, `background:currentColor`; maximize = `10×10px` square, `border:1.5px solid currentColor`, `border-radius:3px`; close = text glyph `✕` (`&#10005;`, U+2715), `font:400 13px 'Hanken Grotesk'`. These are custom Windows-style titlebar controls, not native OS chrome.
- **Body row**: `flex:1;display:flex;padding:0 14px 14px;gap:12px`.
  - **Sidebar** `<aside>`: `width:152px`, `flex-direction:column`, `padding-top:4px`. Items, in order: **Home, Insights, Dictionary, Snippets, History**, then **Settings** pinned to bottom via `margin-top:auto` with a `border-top:1px solid rgba(150,128,104,.16)` (light) / `rgba(255,255,255,.07)` (dark) separator and `padding-top:13px`.
    - Each item: flex row, `gap:11px`, `padding:9px 12px`, `border-radius:11px`, `margin-bottom:5px`. Icon `17×17` (viewBox `0 0 20 20`). Label `font:500 12.5px 'Hanken Grotesk'`.
    - **Inactive**: color `#7C7268` light / `#948AA0` dark, label weight 500.
    - **Active**: color Accent ink, bg Tint, shadow = recipe 4, label weight 600 (bumped `font:600 12.5px`).
  - **Main** `<main>`: `flex:1;min-width:0`, bg Surface, `border-radius:16px`, shadow = recipe 5, `padding:24px 26px` (Home) or `22px 24px` (all other screens), `overflow:hidden` (or `hidden auto` for Settings, which scrolls).

### Home main-content, top → bottom

1. **H1** "Good morning." — `font:500 27px Newsreader;letter-spacing:-.01em`, color Ink. *(Note: Home's H1 is 27px; every other screen's H1 is 25px — genuine, intentional size difference, not a typo.)*
2. Subtext "Hold **Right Ctrl** and speak." — `font:400 13px 'Hanken Grotesk'`, color `#8B8177` light / `#9A8FA0` dark (undeclared muted tones). Inline keycap chip "Right Ctrl": `font:500 11.5px 'JetBrains Mono'`, bg Base, `border-radius:5px`, `padding:2px 7px`, color `#645d68` light / `#C6BCC9` dark.
3. Row of 3 raised stat cards, `flex:1` each, `gap:12px`, `padding:14px 16px`, `border-radius:13px`, shadow = recipe 6:
   - **"1,240"** (`font:500 23px Newsreader`, Ink) / "words today" (`font:400 11px 'Hanken Grotesk'`, `#948C86` light / `#8F859A` dark)
   - **"112"** / "wpm avg"
   - **"5" + "d"** (the "d" suffix at `font:400 13px 'Hanken Grotesk'`, muted) / "streak"
4. **Alert/banner card** (raised, same recipe 6, `padding:13px 15px`, flex row gap 13px):
   - Circular badge `27×27px`, `border-radius:50%`, bg Blush, color `#A85874` (light, undeclared danger-rose) / bg Blush-dark `#5B3F4E`, color `#F0C3D2` (dark) — glyph "**!**" `font:600 13px 'Hanken Grotesk'`.
   - Title "**Last dictation wasn't delivered**" — `font:600 13px 'Hanken Grotesk'`, Ink, truncated (`white-space:nowrap;overflow:hidden;text-overflow:ellipsis`).
   - Subtitle "**Cancelled · 34 words**" — `font:400 11px 'Hanken Grotesk'`, `#948C86`/`#8F859A`.
   - Buttons (gap 6px): "**Dismiss**" ghost text button — `font:600 12.5px 'Hanken Grotesk'`, color `#8B8177` light / `#9A8FA0` dark, `padding:8px 12px`, `border-radius:10px`, no fill. "**Retry**" filled button — `font:600 12.5px 'Hanken Grotesk'`, `padding:8px 16px`, `border-radius:10px`, bg Accent, color `#fff` light / bg Accent-dark `#C9B8EE`, **color `#241F2A` (Window-dark token, not white)** in dark mode — deliberate: white-on-light-lilac would look wrong in dark theme, so dark-mode Retry text uses the dark Window color instead.
5. Eyebrow "**Recent**" — `font:600 10px 'JetBrains Mono';letter-spacing:.1em;text-transform:uppercase`, color `#A79E95` light / `#7C7386` dark, `margin:20px 0 8px` (or `16px 0 8px` dark — same value family).
6. Recessed list well — bg Base, `border-radius:12px`, `padding:4px 14px`, shadow = recipe 7. Three rows, each `padding:10px 0`, divided by `border-bottom:1px solid rgba(150,128,104,.13)` (light) / `rgba(255,255,255,.055)` (dark) except the last row:
   - Timestamp column, `width:52px`, `font:500 10px 'JetBrains Mono'`, `#A79E95`/`#7C7386`.
   - Text, `flex:1`, truncated single line, `font:400 12.5px 'Hanken Grotesk'`, `#544F5A` light / `#CDC5D2` dark.
   - Copy-glyph "⧉" (`&#10697;`, U+29C9, text not SVG) — `font:600 13px 'Hanken Grotesk'`, `#A79E95`/`#7C7386`.
   - Retry-glyph "↻" (`&#8635;`, U+21BB, text not SVG) — `font:600 13px 'Hanken Grotesk'`, Accent / Accent-dark.
   - Rows (verbatim): `2:14 PM` "Ship the overlay states first." · `1:58 PM` *(Cloudflare-obfuscated `[email protected]` placeholder — represents an email-address dictation)* · `11:02 AM` "Let's push the review to Thursday."
7. Footer status line — `font:400 10.5px 'Hanken Grotesk'`, `#A79E95` light / `#7C7386` dark, flex row gap 7px, leading 6×6px dot `border-radius:50%` bg `#8FBF9F` (light "ready" green) / `#6FA080` (dark "ready" green) — **two distinct undeclared semantic-success greens, light ≠ dark, not simply a theme-swap of one value**. Copy: **"Parakeet v3 · AI polish on · mic: MacBook Mic"**.

### Home — one-off colors introduced here (not in Tokens)
`#8E847A`(doc chrome grey) `#8B8177` `#9A8FA0` `#645d68` `#C6BCC9` `#948C86` `#8F859A` `#A79E95` `#7C7386` `#544F5A` `#CDC5D2` `#A85874` `#F0C3D2` `#8FBF9F` `#6FA080` `#EBE3F5` `#EEE7F2` `#AEA394` `#6E6676` `rgba(150,128,104,.13/.16)` `rgba(255,255,255,.055/.07)`.

### Home interaction/animation notes
No live animation in this static screen (it's the idle app-shell state). Sidebar active item is a static state (no hover spec given for inactive items on this screen — see Pill section for the one place hover IS specified). Dark and light are otherwise pixel-mirrors of each other via the token substitution above.

---

## Insights

`id="insights"`. Badge **"insights"**; title `App shell · Insights`; caption "speaking speed · fixes · total words · where you dictate · streak". Shares the exact app-shell chrome from Home (sidebar now highlights **Insights**). Window size **720×600** (taller/wider than Home's 700×500). Main padding `22px 24px`.

### Layout, top → bottom
1. Header row: **H1 "Insights"** (`font:500 25px Newsreader;letter-spacing:-.01em`, Ink) + segmented control (bg Base, `border-radius:9px`, `padding:3px`, shadow = recipe 8): "**Week**" active pill (`font:600 11px 'Hanken Grotesk'`, `#fff`/`#241F2A`(dark text on Accent-dark), bg Accent, `padding:5px 13px`, `border-radius:7px`) + "**Month**" inactive (`font:500 11px 'Hanken Grotesk'`, `#948C86`/`#8F859A`, `padding:5px 13px`, no bg).
2. Privacy note — `font:400 11.5px 'Hanken Grotesk'`, `#948C86`/`#8F859A`: **"All numbers live on this device. Nothing is uploaded — ever."**
3. Row of 3 raised cards (`display:grid;grid-template-columns:repeat(3,1fr);gap:12px`, each `padding:14px 16px;border-radius:14px`, shadow = recipe 6):
   - **Speaking speed**: eyebrow `font:600 9.5px 'JetBrains Mono';letter-spacing:.1em;text-transform:uppercase`, `#A79E95`/`#7C7386`. Half-donut gauge SVG `130×74` viewBox `0 0 130 74`: track path `M11 66 A54 54 0 0 1 119 66` stroke `#DED2C0` light / `#201B26` dark, `stroke-width:11`; value path same `d`, stroke Accent light / Accent-dark, `stroke-width:11`, `stroke-dasharray:169.6`, `stroke-dashoffset:21.2` (both themes — same offset, i.e. same 112-wpm fill ratio). Center number "**112**" `font:500 31px Newsreader`, Ink. Sub-label "words per minute" `font:400 11px 'Hanken Grotesk'`, `#948C86`/`#8F859A`. Below: "**◆ personal best 128**" (`&#9670;` U+25C6 diamond glyph) `font:500 10.5px 'Hanken Grotesk'`, color Accent ink.
   - **Fixes made by Sotto**: eyebrow same style. Big number "**431**" `font:500 34px Newsreader`, Ink. Two detail rows, each `padding:6px 0;border-top:1px solid rgba(150,128,104,.13)`/`rgba(255,255,255,.055)`, `font:400 11.5px 'Hanken Grotesk'` `#645d68`/`#B7ADBE` with an inline bold span `font:600 11.5px 'Hanken Grotesk'` Ink: "**387** words corrected", "**44** dictionary fixes".
   - **Total words**: same pattern. Big number "**48,290**". Detail rows: "**6,180** this week", "**≈ 142** min saved vs. typing" (`&asymp;`).
4. Row of 2 cards (`grid-template-columns:1fr 1.4fr;gap:12px;margin-top:12px`, `padding:15px 18px`, shadow = recipe 6):
   - **"Where you dictate"**: header row (eyebrow + "**30 days**" tag, `font:500 9px 'JetBrains Mono'`, `#A79E95`/`#7C7386`). 5 bar rows (`gap:9px;margin-bottom:8px`): label `font:500 11px 'Hanken Grotesk'` `#544F5A`/`#CDC5D2`, `width:52px`; track `flex:1;height:9px`, bg Base, `border-radius:5px`, shadow = recipe 8; fill bg Accent/Accent-dark, `border-radius:5px`; value `font:500 9.5px 'JetBrains Mono'`, `#A79E95`/`#7C7386`, `width:38px;text-align:right`. Rows verbatim: **Slack** 82% `5.1k` · **Mail** 61% `3.8k` · **Notion** 44% `2.7k` · **VS Code** 28% `1.7k` · **Docs** 18% `1.1k`.
   - **Streak calendar**: header "**12-day streak**" `font:500 15px Newsreader`, Ink + "**longest 21 days**" `font:500 9.5px 'JetBrains Mono'`, `#A79E95`/`#7C7386`. Heatmap grid: `grid-template-rows:repeat(7,9px);grid-auto-flow:column;grid-auto-columns:9px;gap:3px`, 133 cells (7×19), each `9×9px` `border-radius:2px`, color driven by a per-cell intensity level 0–4 (see script below). Legend: "**Less**" … 5 swatches (8×8px, `border-radius:2px`) … "**More**", `font:400 9.5px 'JetBrains Mono'`, `#A79E95`/`#7C7386`.

### Streak-heatmap 5-level color ramps (exact, from the embedded generator script)
| Level | Light | Dark |
|---|---|---|
| 0 (least) | `#E6DFD4` (= Base) | `#241F2A` (= Base dark) |
| 1 | `#DED0EC` | `#3A3340` (= Tint dark) |
| 2 | `#C3ABE6` | `#5C4E86` |
| 3 | `#A98FE0` | `#8E74D0` (= Accent light, reused as dark level-3!) |
| 4 (most) | `#8E74D0` (= Accent) | `#C9B8EE` (= Accent dark) |

Generator logic (deterministic pseudo-random via sine/cosine, 133 cells, last ~14 cells biased upward to guarantee a visible recent streak) — reproduce exactly if you want the same-looking heatmap, or replace with real usage data:
```js
const n = 133, ints = [];
for (let i = 0; i < n; i++) {
  let r = Math.abs(Math.sin(i * 2.3 + 1.1)) * Math.abs(Math.cos(i * 0.7));
  let v = Math.min(4, Math.floor(r * 6));
  if (i > n - 14 && (i % 7) < 6) v = Math.max(v, 2 + (i % 2 ? 2 : 1));
  if (v > 4) v = 4;
  ints.push(v);
}
```

### Insights — one-off colors introduced here
`#DED2C0` `#201B26` `#645d68` `#B7ADBE` (dark detail-row text, distinct from `#8F859A`) — all undeclared.

### Insights interaction notes
Static screen, no animation. The gauge/bars are pre-rendered SVG/CSS-width, not live-animated in this doc (no `mm-*` keyframe reference in this section).

---

## Dictionary

`id="dictionary"`. Badge **"dictionary"**; title `App shell · Dictionary`; caption "words Sotto should spell your way". **Light theme only shown** (dark "follows the tokens" per the intro paragraph, no explicit dark mockup in the doc). Window **700×540**, same chrome as Home (sidebar highlights Dictionary).

### Layout, top → bottom
1. Header row: **H1 "Dictionary"** (25px Newsreader) + primary button "**+ Add word**" (`font:600 12px 'Hanken Grotesk'`, `#fff`, bg Accent, `padding:8px 15px`, `border-radius:10px`).
2. Callout box — bg `#EBE3F5` (near-dupe of Tint, see Home note), `border-radius:13px`, `padding:15px 17px`, `position:relative`. Dismiss "✕" top-right, `color:#A897C8`, `font:400 13px 'Hanken Grotesk'`. Title "**Sotto spells the way you do.**" `font:500 14px Newsreader`, color `#4A3A78` (undeclared plum-ink, distinct from Ink and Accent ink). Body `font:400 11.5px/1.55 'Hanken Grotesk'`, color `#6E5F94` (undeclared muted plum), `max-width:400px`: **"Teach it names, jargon, and terms it keeps getting wrong — say the word, get your spelling. Everything stays on this device."** Example chips (`gap:7px`, wrap): `font:500 10.5px 'JetBrains Mono'`, bg Tint, color Accent ink, `padding:4px 9px`, `border-radius:7px`, `white-space:nowrap`:
   - `"gee pee tee" → GPT`
   - `"khairy" → Khairy`
   - `"arrow" → →`
3. Search bar — flex row gap 9px, bg Base, `border-radius:10px`, `padding:9px 13px`, shadow = recipe 8. Magnifying-glass icon (see Icons catalog) + placeholder "**Search your words…**" `font:400 12.5px 'Hanken Grotesk'`, `#A79E95`.
4. List well — bg Base, `border-radius:12px`, `padding:2px 14px`, shadow = recipe 5. 5 rows, `padding:11px 4px`, divided by `border-bottom:1px solid rgba(150,128,104,.13)` (all but last):
   - Term (mono) `font:500 12px 'JetBrains Mono'`, `#544F5A`, `flex:1`.
   - Arrow glyph "→" (`&rarr;`, text not SVG) — `font:600 13px 'Hanken Grotesk'`, color **`#B7A1E4`** (undeclared light-purple, distinct from both Accent `#8E74D0` and Accent-ink `#6E58A8`).
   - Result `font:600 12.5px 'Hanken Grotesk'`, Ink, `flex:1`.
   - Action icons (gap 12px, color `#A79E95`): edit-pencil SVG + delete/remove-X SVG (see Icons catalog).
   - Rows verbatim: `gee pee tee → GPT` · `khairy → Khairy` · `sotto → Sotto` · `parakeet → Parakeet` · `arrow → →`.

### Dictionary — one-off colors introduced here
`#EBE3F5` `#A897C8` `#4A3A78` `#6E5F94` `#B7A1E4` `#544F5A` `#A79E95`.

---

## Snippets

`id="snippets"`. Badge **"snippets"**; title `App shell · Snippets`; caption "say a phrase, drop a paragraph". Light only. Window **700×540**, identical chrome pattern to Dictionary (sidebar highlights Snippets).

### Layout, top → bottom
1. Header: **H1 "Snippets"** + "**+ Add snippet**" button (same primary-button spec as Dictionary's Add button).
2. Callout box (identical visual spec to Dictionary's): title "**The stuff you shouldn't have to re-type.**"; body: **"Save text you use often — your email, an intro, a prompt — then say the trigger phrase to drop it in instantly."**; chips: `"my email" → khairy@…`, `"intro line" → Hey, great…`.
3. Search bar, placeholder "**Search your snippets…**" (same visual spec as Dictionary).
4. List well (same container spec). 4 rows, `padding:12px 4px`:
   - Trigger label — fixed `width:150px` column, `font:600 12.5px 'Hanken Grotesk'`, Ink.
   - Preview text — `flex:1`, truncated single line, `font:400 12px 'Hanken Grotesk'`, color `#8B8177`.
   - Same edit/delete action icons as Dictionary.
   - Rows verbatim:
     - **my email** → *(Cloudflare-obfuscated email placeholder)*
     - **intro line** → "Hey, great to meet you — thanks for making the time today."
     - **sign off** → "Best, Khairy"
     - **standup** → "Yesterday I — . Today I'll — . No blockers."

### Snippets — one-off colors introduced here
`#8B8177` (reused from Home).

---

## History

`id="history"`. Badge **"history"**; title `App shell · History`; caption "session log · copy or re-polish any line". Light only. Window **700×540**, same chrome (sidebar highlights History).

### Layout, top → bottom
1. Header: **H1 "History"** + hint text "**Click any entry to copy it again**" (`font:400 11.5px 'Hanken Grotesk'`, `#948C86`).
2. List well — bg Base, `border-radius:12px`, `padding:2px 14px`, shadow = recipe 5, `flex:1;min-height:0;overflow:hidden`. 8 rows, `padding:11px 0`, divided (all but last) by `border-bottom:1px solid rgba(150,128,104,.13)`:
   - Timestamp `width:56px` (note: wider than Home's `52px`), `font:500 10px 'JetBrains Mono'`, `#A79E95`.
   - Text `flex:1`, truncated, `font:400 12.5px 'Hanken Grotesk'`, `#544F5A`.
   - Copy glyph "⧉" (`#A79E95`) + retry glyph "↻" (Accent) — both `font:600 13px 'Hanken Grotesk'`, gap 11px.
   - Rows verbatim (all 8):
     1. `2:14 PM` — "Ship the overlay states first, then the icon set."
     2. `1:58 PM` — *(obfuscated email placeholder)*
     3. `1:32 PM` — "Let's push the review to Thursday afternoon."
     4. `12:47 PM` — "Can you send over the latest Marshmallow tokens?"
     5. `11:20 AM` — "Refactor the take-stash logic before the retry work."
     6. `10:55 AM` — "Reminder: cancel stashes, it never discards."
     7. `10:12 AM` — "The pill lingers six seconds on error now."
     8. `9:40 AM` — "Good morning — starting with the insights screen."
3. Footer note — flex row gap 7px, `font:400 10.5px 'Hanken Grotesk'`, `#A79E95`. Lock-icon SVG (see Icons catalog) + **"History lives in memory and clears when Sotto quits. Nothing is written to disk."**

---

## Settings

`id="settings"`. Badge **"settings"**; title `App shell · Settings`; caption "one scrolling page · dictation, polish, model, privacy". Light only. Window **700×560** (tallest of the fixed-height mockups). Main content `overflow:hidden auto;display:block` — this is the one screen that scrolls.

### Layout — one scrolling column, section-eyebrow + raised-card-group pattern repeated 7×

Section eyebrow style (repeats for every group): `font:600 10px 'JetBrains Mono';letter-spacing:.1em;text-transform:uppercase`, color `#A79E95`, `margin:0 0 8px` (or `20px 0 8px` mid-page). Each card group: bg Surface, `border-radius:13px`, shadow = recipe 6, `overflow:hidden;margin-bottom:18px`. Each row inside: flex `justify-content:space-between`, `padding:13px 16px`. Rows within a group separated by `height:1px;background:rgba(150,128,104,.1);margin:0 16px`.

Row title style: `font:500 13px 'Hanken Grotesk'`, Ink. Row description style: `font:400 11.5px 'Hanken Grotesk'`, `#948C86`.

1. **Dictation**
   - "Dictation key" / "Hold to talk — release to transcribe" → key chip **"Right Ctrl"** (`font:500 12px 'JetBrains Mono'`, bg Base, `border-radius:6px`, `padding:5px 10px`, color `#544F5A`, shadow = recipe 8) + "**Change…**" link (`font:500 12px 'Hanken Grotesk'`, Accent ink).
   - "Mode" / "Hold the key while speaking, or tap to start & stop" → segmented control (bg Base, `border-radius:9px`, `padding:3px`, shadow = recipe 8): "**Hold**" active (filled Accent pill, white text, `font:600 11px`) / "**Toggle**" inactive (`font:500 11px`, `#8B8177`).
   - "Microphone" / "Which mic Sotto listens to" → dropdown pill (bg Base, `border-radius:8px`, `padding:7px 12px`, shadow = recipe 8): "**MacBook Mic**" (`font:500 12px 'Hanken Grotesk'`, `#544F5A`) + "▼" (`&#9660;`, `#A79E95`, `font-size:9px`).
2. **Polish**
   - "Cleanup" / "Off · Rules (instant, local) · AI (local LLM rewrite)" → 3-way segmented: "**Off**" / "**Rules**" (both inactive, `#8B8177`) / "**AI**" (active, filled Accent).
   - "Use AI past" / "Shorter clips stay on instant rules" → slider: track `width:210px;height:6px`, bg Base, `border-radius:3px`, shadow = recipe 8; fill `34%` bg Accent; round handle `16×16px`, `border-radius:50%`, `#fff`, shadow = recipe 13 (`0 1px 3px rgba(150,128,104,.4)`), positioned at `left:34%`; value label "**12 words**" `font:500 11px 'JetBrains Mono'`, `#544F5A`, `width:60px;text-align:right`.
3. **Speech model**
   - Icon badge `34×34px`, `border-radius:9px`, bg Tint, centered plug/note-shaped SVG stroke Accent ink (see Icons catalog "speech-model icon"). Name "**Parakeet v3**" (`font:500 13px`, Ink) + "Installed · 1.2 GB · on-device" (`font:400 11.5px`, `#948C86`). Right-aligned badge "**ACTIVE**" — `font:600 10px 'JetBrains Mono'`, color `#3E8E6E` (undeclared third semantic-green, distinct from the two "ready" dot greens `#8FBF9F`/`#6FA080`), bg `rgba(62,142,110,.12)`, `padding:4px 9px`, `border-radius:6px`.
4. **Sound** — "Dictation sounds" / "A soft tick when recording starts and stops" → toggle **ON**.
5. **Startup** — "Launch Sotto at login" → toggle **ON**; "Start hidden in the tray" → toggle **OFF**.
6. **Updates** — "Sotto 2.0.0" / "You're up to date" → outline button "**Check for updates**" (`font:500 12px 'Hanken Grotesk'`, color Accent ink, `border:1px solid rgba(110,88,168,.3)`, `padding:6px 13px`, `border-radius:9px`, transparent bg).
7. **Data & privacy**
   - "Usage stats" / "Counts words and timing on this device to power Insights. Never your words' content, never uploaded." → toggle **ON**.
   - "Clear stats" / "Delete all Insights data" → destructive outline button "**Clear stats**" — `font:500 12px 'Hanken Grotesk'`, color **`#A85874`** (danger-rose, matches the Home alert-badge icon color — reused semantically), `border:1px solid rgba(168,88,116,.32)`, `padding:6px 13px`, `border-radius:9px`.
   - "Data folder" / "**~/Library/Application Support/sotto**" *(macOS-style path — flag: Sotto is a Windows app per the brief, so this literal path string needs correcting to a Windows equivalent, e.g. `%APPDATA%\sotto`, at implementation time — it's almost certainly a copy-paste leftover in the mock)* → link "**Open folder**" (Accent ink, no border).

### Toggle switch spec (reused 4× on this screen)
- **ON**: `width:38px;height:22px;border-radius:11px`, bg Accent, `padding:0 2px`, `justify-content:flex-end` (knob pushed right); knob `18×18px`, `border-radius:50%`, `#fff`.
- **OFF**: bg **`#DED2C0`** (undeclared, close to but not equal to Window `#DED5C7`), knob pushed left, knob color **`#F6F0E6`** (undeclared, lighter than Surface `#F0E9DF` — also reused as the App-icon gradient's light stop, see Icons).

### Settings — one-off colors introduced here
`#948C86` `#544F5A` `#8B8177` `#A79E95` `#3E8E6E` `#A85874` `#DED2C0` `#F6F0E6`.

---

## Transforms

`id="transforms"`. Badge **"transforms"**; title `App shell · Transforms`; caption "phase 2 · rewrite selected text in place". Light only. Window **700×540**, same chrome (sidebar highlights nothing new — this is a phase-2 screen not in the main nav list; presumably reached another way).

### Layout, top → bottom
1. Header row: **H1 "Transforms"** (25px Newsreader) + **"BETA"** badge — `font:600 9.5px 'JetBrains Mono';letter-spacing:.05em`, color **`#B07A3A`** (undeclared amber/brown "beta" semantic color), bg `rgba(212,160,106,.16)`, `padding:3px 8px`, `border-radius:6px`. Right-aligned: "Enabled" label (`font:400 11.5px`, `#948C86`) + toggle **ON** (same spec as Settings toggle-on).
2. Subhead "**Rewrite anywhere you write.**" — `font:500 15px Newsreader`, color **`#4A4550`** (undeclared, softened-Ink tone, distinct from pure Ink `#332E3A`). Body — `font:400 12px/1.55 'Hanken Grotesk'`, `#8B8177`, `max-width:440px`: **"Select text in any app, press the shortcut, and Sotto's local AI rewrites it in place. Your text never leaves this device."**
3. Grid of 3 (`grid-template-columns:1fr 1fr;gap:12px` — third item wraps/spans):
   - **Polish** card (raised, recipe 6, `padding:16px;border-radius:14px`): keycap row "**Ctrl**" + "**Alt**" + "**1**" (each: `font:500 10.5px 'JetBrains Mono'`, bg Base, `border-radius:5px`, `padding:3px 7px`, color `#544F5A`, shadow = recipe 8; separated by "+" `#B5ADA0` `font-size:10px`). Title "**Polish**" (`font:500 15px Newsreader`, Ink). Body "**Tightens and clarifies without changing your meaning.**" (`font:400 11.5px/1.5`, `#8B8177`).
   - **Prompt engineer** card (identical spec): keycaps "**Ctrl**"+"**Alt**"+"**2**". Title "**Prompt engineer**". Body "**Restructures your idea into a strong AI prompt.**"
   - **"Create your own"** dashed card — bg Base, `border:1.5px dashed rgba(150,128,104,.3)`, `border-radius:14px`, `min-height:120px`, centered column: "+" circle (`34×34px`, `border-radius:50%`, bg Tint, color Accent ink, `font:400 20px 'Hanken Grotesk'`), label "**Create your own**" (`font:500 12.5px`, Accent ink), sublabel "**bring your own prompt**" (`font:400 10.5px`, `#948C86`).
4. Footer row (`margin-top:auto;padding-top:18px`): "**Reset to defaults**" ghost text (`font:500 12px 'Hanken Grotesk'`, `#8B8177`) left; "**+ Create new**" primary button (same spec as Dictionary's Add button) right, pushed apart by a `flex:1` spacer.

### Transforms — one-off colors introduced here
`#B07A3A` `#4A4550` `#B5ADA0` `#948C86` `#8B8177`.

---

## Scratchpad

`id="scratchpad"`. Badge **"scratchpad"**; title `App shell · Scratchpad`; caption "phase 2 · dictate into a private local pad". Light only. Window **700×540**, same chrome.

### Layout
- Header (note: NOT the standard `22px 24px` main padding — this screen's `<main>` has `padding:0`, and the header row itself carries `padding:20px 22px 14px`): **H1 "Scratchpad"** + "**+ New note**" button (standard primary button spec).
- Body: `flex:1;min-height:0;display:flex;gap:0` — two-pane split, **no gap**, no divider line drawn between panes (visual separation comes purely from the left pane's transparent bg vs the right pane's recessed Base bg).
  - **Left list** `width:190px`, `padding:0 12px 16px`. Items `padding:10px 12px;border-radius:10px;margin-bottom:4px`:
    - **Active** item: bg Tint, shadow = recipe 4 (same inset-lilac recipe as sidebar active nav), title `font:600 12px 'Hanken Grotesk'`, color `#4A3A78` (same plum-ink as Dictionary/Snippets callout titles).
    - **Inactive** items: no bg, title `font:600 12px 'Hanken Grotesk'`, color `#544F5A`.
    - All items: timestamp below, `font:400 10px 'Hanken Grotesk'`, `#A79E95`.
    - Rows verbatim: **"Launch checklist"** / 2:14 PM *(active)* · **"Marshmallow notes"** / Yesterday · **"Call w/ Sam — ideas"** / Mon · **"Random thought"** / Jul 9.
  - **Right editor pane** — bg Base, `border-radius:14px`, shadow = recipe 5, `margin:0 16px 16px 4px`, `padding:22px 24px`:
    - Note title "**Launch checklist**" — `font:500 21px Newsreader`, Ink.
    - Meta "**Edited 2:14 PM · stored on this device**" — `font:400 10px 'JetBrains Mono'`, `#A79E95`.
    - Body — `font:400 13.5px/1.7 'Hanken Grotesk'`, `#544F5A`: **"Ship the overlay states first, then the icon set. Double-check the retry stash survives an accidental Escape. Reskin the tray to the flat marshmallow mark before the build call"** followed inline by a blinking-cursor caret: `display:inline-block;width:2px;height:16px`, bg Accent, `vertical-align:-3px;margin-left:1px` (implies a text-cursor / dictation-in-progress affordance; no animation keyframe attached to it in this doc, but candidate for a blink animation in implementation).

---

## Pill (overlay)

`id="pill"`. Badge **"pill"**; title `Overlay pill — Marshmallow`; caption "neumorphic cushion · lilac/amber/blush accents · ✕ cancel · ↻ retry". Intro paragraph (verbatim): *"The pill is a soft raised cushion — cream in light, plum in dark. Waveform colour carries the state: **lilac** while listening, **amber** while transcribing, **gold** while polishing. Toasts (done/cancelled/error) expand to fit their label and add a 6 s countdown bar. The ✕ cancel and ↻ retry buttons are 22 px ghost discs at the pill's right edge."*

Every state is shown twice, side by side: against a dark "desktop" backdrop swatch `280×68px`, `border-radius:14px`, bg **`#1A161F`** (a one-off "wallpaper" placeholder, darker than Window-dark `#1E1A24` — not a real token, purely presentational context) and against a light backdrop bg **`#D0C8BA`** (also a one-off wallpaper placeholder, not a token). The pill itself sits centered in each backdrop.

### Live states — pill body `148×40px`, `border-radius:20px`, `padding:0 8px 0 14px`, shadow = recipe 9 (bg `#2C2634` dark / `#F0E9DF` light — i.e. the pill uses Surface-dark/Surface-light regardless of which backdrop it's shown against)

| State | Row label color | Waveform / icon | Cancel disc | Caption |
|---|---|---|---|---|
| **Idle** | `#8E847A` | Single breathing dot `22×22px`, `border-radius:50%`, `radial-gradient(circle at 40% 35%, #E6DAF7, #C9B8EE)` dark / `radial-gradient(circle at 40% 35%, #B7A1E4, #8E74D0)` light, `box-shadow:0 0 10px #C9B8EE66` dark / `#8E74D066` light, `animation:mm-breathe 4.2s ease-in-out infinite` | 22×22px circle, bg `rgba(255,255,255,.06)` dark / `rgba(0,0,0,.05)` light, glyph "✕" `font:400 10px 'Hanken Grotesk'`, color `#8F859A` dark / `#948C86` light | "breathing waveform · ~4 s cycle" |
| **Listening** | `#6E58A8` | 5 vertical bars, `3.5px` wide, `20px` tall, `border-radius:3px`, color `#C9B8EE` dark / `#8E74D0` light, `animation:mm-bar` alternating `0.9s`/`1.15s` durations, staggered delays `0, -0.18, -0.36, -0.54, -0.72s` | same as Idle | "live mic-tracking waveform · lilac" |
| **Transcribing** | `#D4A06A` | 4 rising bubbles inside a `66×26px` box, sizes 9/7/11/8px diameter, positions left `8/24/38/52px`, color `#E8C48E` dark / `#D4A06A` light, `box-shadow:0 0 6px <color>55`, `animation:mm-rise 2.3s ease-in-out infinite`, delays `0, -0.5, -1, -1.5s` | same as Idle | "rising bubbles · warm amber" |
| **Polishing** | `#E8C78A` | 3 overlapping blurred blobs (`filter:blur(.4px)` wrapper `54×24px`) at 16/20/15px diameter, color `#F0D9A4` dark / `#E8C78A` light, `opacity:.85`, `animation:mm-blob 2.6s ease-in-out infinite`, delays `0,-0.5,-1s`; plus a shimmer bar `22×3px` `linear-gradient(90deg,transparent,#FAF0D6,transparent)` dark / `#F5E2B0` light, `animation:mm-shimmer 2s`; plus a 4-point twinkle star SVG (`viewBox 0 0 12 12`, path `M6 0 l1 4 4 1 -4 1 -1 4 -1 -4 -4 -1 4 -1z`), color `#FAF0D6` dark / `#F5E2B0` light, `animation:mm-twinkle 1.8s` | same as Idle | "drifting cloud + shimmer · soft gold" |
| **Done** *(auto-fade, not a toast)* | `#6E58A8` | Pill collapses to just a centered checkmark, no side padding (`justify-content:center` only, no cancel disc). SVG `viewBox 0 0 24 24`, path `M5 12 L10 17 L19 7`, stroke `#C9B8EE` dark / `#8E74D0` light, `stroke-width:2.6`, `stroke-linecap/linejoin:round`, `stroke-dasharray:26`, `animation:mm-draw 2.6s ease-out infinite` (draw-on stroke reveal) | *(none — no cancel/retry on Done)* | "check strokes on · auto-fades ~0.8 s" |

### Toast states — auto-width × 40px, 6s countdown bar, shadow = recipe 9 (same pill-body raised recipe)

| State | Row label color | Width | Left icon | Label text | Right control | Countdown bar |
|---|---|---|---|---|---|---|
| **Cancelled** | `#B5ADA0` | `220px` | `20×20px` circle bg `#3A3340` dark / `#E6DFD4` light, X-glyph SVG `viewBox 0 0 16 16`, path `M4 4 L12 12 M12 4 L4 12`, stroke `#8F859A` dark / `#B5ADA0` light, `stroke-width:2` | "**Cancelled**" `font:500 12px 'Hanken Grotesk'`, `#CDC5D2` dark / `#544F5A` light | **↻ retry disc** `22×22px`, bg `rgba(201,184,238,.12)` dark / `rgba(110,88,168,.10)` light, glyph "↻" `font:600 13px 'Hanken Grotesk'`, color `#C9B8EE` dark / `#6E58A8` light | track `height:3px`, bg `#241F2A` dark / `#E6DFD4` light; fill `62%` wide, bg `#8F859A` dark / `#B5ADA0` light |
| **Error** | `#C88A94` | `236px` (widest — longest label) | `20×20px` circle bg `#5B3F4E` dark (= Blush dark) / `#F0BFCF` light (= Blush light), glyph "**!**" `font:700 11px 'Hanken Grotesk'`, color `#F0C3D2` dark / `#A85874` light | "**Didn't catch that**" (same text style as Cancelled) | same **↻ retry disc** spec as Cancelled | fill `45%` wide, bg `#7A5060` dark / `#C88A94` light |

Both toast rows share: container `border-radius:20px`, `padding:0 8px 0 12px`, `gap:8px`, `position:relative;overflow:hidden` (so the countdown bar's bottom corners clip to match the pill's rounded bottom).

**Retry-button visual spec (explicit answer to "find the exact spec"):** the ↻ retry control is a **22×22px circular ghost disc** at the pill's right edge, background is the accent color at ~10–14% opacity (`rgba(201,184,238,.12)` dark / `rgba(110,88,168,.10)` light — i.e. a translucent accent tint, not a solid fill), containing only the `↻` Unicode glyph (U+21BB) rendered as text (`font:600 13px 'Hanken Grotesk'`) in solid Accent/Accent-ink color — **no SVG path**, it's a font glyph. The ✕ cancel control (shown only on the 4 "live" — working — states, never on toasts) is the same 22×22px disc shape but with a *neutral* translucent bg (`rgba(255,255,255,.06)` dark / `rgba(0,0,0,.05)` light) and a muted-grey `✕` glyph, signaling "this doesn't commit to anything, it's a working-state escape hatch" vs. retry's accent-colored "this is an action."

### Motion spec (from the doc's own callout box, verbatim)

| Property | Value |
|---|---|
| enter | `opacity 0→1 · translateY 4→0 · 180ms ease-out` |
| exit | `opacity 1→0 · translateY 0→4 · 260ms ease-out` |
| controls | one button · ✕ cancel while working · ↻ retry on toasts |
| timeout | toast bar depletes over 6s · empty → pill unmounts |
| hover | control disc fades 0 → 8% white · 120ms |
| rule | never scale, never bounce, never overshoot |

This callout box itself: bg Surface, `border:1px dashed rgba(150,128,104,.22)`, `border-radius:14px`, `padding:16px 20px`, shadow = recipe 10, title `font:600 13px 'Hanken Grotesk'` Ink, rows `font:500 11.5px 'JetBrains Mono'` `#544F5A` with row-label color `#A79E95`.

### Pill — one-off colors introduced here
`#1A161F` `#D0C8BA` `#E6DAF7` `#8E847A` `#D4A06A` `#E8C48E` `#E8C78A` `#F0D9A4` `#FAF0D6` `#F5E2B0` `#B5ADA0` `#C88A94` `#7A5060` `#F0C3D2` (also seen on Home banner) `#A85874` (also seen on Home banner/Settings).

---

## Icons

`id="icons"`. Badge **"icons"**; title `Big Sur icon & tray`; caption "squircle mark · idle/active tray states". *(Note: caption says "Big Sur" — macOS terminology — again a likely cross-platform-template leftover; for a Windows app these should be reframed as the `.ico`/app-icon and Windows system-tray equivalents, but the visual spec — squircle mark, gradient, tray glyph — still applies 1:1.)*

### App icon (squircle), 3 color treatments at 96px + 2 smaller sizes

All squircles: `border-radius:22%` (a true squircle super-ellipse approximation via CSS radius-percentage, not a fixed px radius), centered wave-mark SVG.

| Treatment | Size | Background | Wave stroke | Shadow |
|---|---|---|---|---|
| A — "cream / lilac" | 96px | `linear-gradient(145deg, #F6F0E6, #E6DFD4)` | `#8E74D0`, `stroke-width:4.5` | `0 8px 20px rgba(120,100,150,.26)` |
| B — "lilac / cream" | 96px | `linear-gradient(145deg, #A98FE0, #7B5FC8)` | `#F6F0E6`, `stroke-width:4.5`, `opacity:.9` | `0 8px 20px rgba(100,74,168,.32)` |
| Dark-mode — "plum / lilac" | 96px | `linear-gradient(145deg, #322A3E, #241F2A)` | `#C9B8EE`, `stroke-width:4.5` | `0 8px 20px rgba(0,0,0,.48)` |
| A at 48px | 48px | same gradient A | `#8E74D0`, `stroke-width:5.5` | `0 4px 10px rgba(120,100,150,.24)` |
| A at 32px | 32px | same gradient A | `#8E74D0`, `stroke-width:7` | `0 3px 7px rgba(120,100,150,.22)` |

Every squircle's SVG: `viewBox="0 0 56 24"`, path `M4 12 Q10 3 16 12 T28 12 T40 12 T52 12` (the **"big" 4-hump wave** — distinct from the 2-hump "small" wave used in-app headers, see below), rendered at width/height scaled to ~46%/40% of the box (e.g. 96px box → 44×19 svg render size).

**One-off colors introduced here:** `#F6F0E6` (reused from Settings toggle-off knob), `#A98FE0` (reused from streak-heatmap light level-3), `#7B5FC8` (new — deep lilac gradient stop), `#322A3E` (new — warm dark-plum gradient stop, distinct from both Base-dark `#241F2A` and Surface-dark `#2C2634`).

### System tray icons (16/24/32px), idle vs active, dark vs light taskbar

Two taskbar swatches (`border-radius:10px`, `padding:12px 16px`, flex row gap 14px), each containing an idle icon + active icon at 24px:

| Taskbar | State | Icon box bg | Wave stroke | Caption color |
|---|---|---|---|---|
| Dark taskbar (bg `#1E1A24`) | idle | `#2C2634` | `#8F859A` | `#7C7386` |
| Dark taskbar | active | `#2C2634` | `#C9B8EE` | `#C9B8EE` |
| Light taskbar (bg `#E6DFD4`) | idle | `#F0E9DF` | `#B5ADA0` | `#A79E95` |
| Light taskbar | active | `#EBE3F5` | `#6E58A8` | `#6E58A8` |

Icon box: `24×24px`, `border-radius:6px`. SVG in all tray icons: same `viewBox="0 0 56 24"`, path `M4 12 Q10 3 16 12 T28 12 T40 12 T52 12` (big 4-hump wave), rendered `14×6px`, `stroke-width:7`. Caption label: `font:500 8px 'JetBrains Mono'`, "idle"/"active"; taskbar caption below: `font:500 10px 'JetBrains Mono'`, `#8E847A`, "dark taskbar"/"light taskbar".

### Tray right-click context menu

Two variants (dark, light), each `230×~auto`, outer padding wrapper `26px 22px` (simulating desktop margin) containing the actual menu panel:

**Panel**: `border-radius:12px`, `padding:6px`, shadow = recipe 12. Dark: bg `#2C2634`, `border:1px solid rgba(255,255,255,.07)`. Light: bg `#F4EEE4` (a **new undeclared near-Surface tone**, close to but not equal to Surface `#F0E9DF`), `border:1px solid rgba(150,128,104,.14)`.

**Header row** (`padding:8px 10px 10px`, gap 9px): icon badge `22×22px`, `border-radius:6px`, bg `#3A3340` dark / `#EBE3F5` light, color Accent-dark / Accent-ink, containing the **small 2-hump wave SVG** (`viewBox 0 0 20 20`, path `M2.5 10 Q5 4.6 7.5 10 T12.5 10 T17.5 10`, `stroke-width:2.6`, rendered 12×12) — same small-wave variant as the in-app header logo, NOT the big 4-hump tray/app-icon wave. Title "**Sotto**" `font:500 13px Newsreader`, `#EEE7F2` dark / `#332E3A` light. Status "**Ready**" — 6px dot bg `#6FA080` dark / `#8FBF9F` light + text `font:400 10px 'Hanken Grotesk'`, `#8F859A` dark / `#948C86` light.

Divider: `height:1px`, `background:rgba(255,255,255,.06)` dark / `rgba(150,128,104,.12)` light, `margin:2px 4px 4px` (first) or `margin:4px` (subsequent).

**Menu items** (`padding:8px 10px`, `border-radius:8px`, `gap:11px`, icon `15×15px` viewBox `0 0 20 20`, label `font:400 12.5px 'Hanken Grotesk'`):
1. **Insights** (bar-chart icon)
2. **Dictionary** (open-book icon)
3. **History** (clock icon)
— divider —
4. **Retry last dictation** (↻ glyph, not svg, `width:15px;text-align:center;font:600 14px`) + trailing muted "**none yet**" tag (`font:400 10px`)
— divider —
5. **Settings…** (gear icon, `&hellip;`)
6. **Quit Sotto** (logout-door icon: `viewBox 0 0 20 20`, path `M8 4 H5 A1 1 0 0 0 4 5 V15 A1 1 0 0 0 5 16 H8 M12 7 L15.5 10 L12 13 M15.5 10 H7.5`, `stroke-width:1.6`)

**Item color states:**
| Variant shown | Default item color | Hover-highlighted item | Disabled/muted item |
|---|---|---|---|
| Dark (caption: "dark · Insights hover") | `#CDC5D2` | **Insights** row: bg `rgba(201,184,238,.14)`, text `#E6DAF7` | **Retry last dictation**: text `#6E6478`, "none yet" `#5C5568` |
| Light (caption: "light · Settings hover") | `#4A4550` | **Settings…** row: bg `#ECE5F4` (= Tint exactly), text `#5A4894` | **Retry last dictation**: text `#B7AD9E`, "none yet" `#C4BAAB` |

Caption labels below each menu mockup: `font:500 10px 'JetBrains Mono'`, `#8E847A`.

### Icons — one-off colors introduced here
`#F4EEE4` `#7B5FC8` `#322A3E` `#E6DAF7` `#6E6478` `#5C5568` `#B7AD9E` `#C4BAAB` `#5A4894` (plus reuses of `#8F859A #948C86 #6FA080 #8FBF9F #EEE7F2 #332E3A #A79E95 #B5ADA0 #6E58A8 #C9B8EE #A98FE0` etc. noted above).

---

## SVG icon catalog (every distinct icon, full path data)

All nav/UI icons share `viewBox="0 0 20 20"` unless noted. Sizes quoted are as rendered in the sidebar (17×17) — the same paths are reused at 15×15 in the tray menu and other contexts; only the render `width`/`height` changes, never the `d` data.

### Navigation icons (sidebar + tray menu)

| Icon | stroke-width | Path `d` |
|---|---|---|
| **Home** | 1.7 | `M3.6 9.3 L10 4 L16.4 9.3 M5.4 8.3 V16 H14.6 V8.3` |
| **Insights** | 1.7 | `M3.6 16.4 H16.4 M5.7 16 V11 M10 16 V5.6 M14.3 16 V8.6` |
| **Dictionary** | 1.5 | `M10 5.3 C8.4 4.3 6 4.1 4.2 4.5 V15.4 C6 15 8.4 15.2 10 16.2 C11.6 15.2 14 15 15.8 15.4 V4.5 C14 4.1 11.6 4.3 10 5.3 Z M10 5.3 V16.2` |
| **Snippets** | 1.7 | `M5 6.5 H15 M5 10 H15 M5 13.5 H11` |
| **History** | 1.7 | `<circle cx="10" cy="10" r="6.2"/>` + `M10 6.6 V10 L12.5 11.7` |
| **Settings** (gear) | 1.7 | `<circle cx="10" cy="10" r="3.1"/>` + `M10 2.6v2.3M10 15.1v2.3M2.6 10h2.3M15.1 10h2.3M4.8 4.8l1.6 1.6M13.6 13.6l1.6 1.6M15.2 4.8l-1.6 1.6M6.4 13.6l-1.6 1.6` |

All above: `fill="none"`, `stroke="currentColor"`, `stroke-linecap="round"` (Home/Insights/Snippets/History/Settings) or `stroke-linejoin="round"` added where curves meet (Home, Dictionary).

### Wordmark / wave logo (2 distinct variants — do not conflate)

| Variant | viewBox | Path `d` | stroke-width | Used where |
|---|---|---|---|---|
| **Small (2-hump)** | `0 0 20 20` | `M2.5 10 Q5 4.6 7.5 10 T12.5 10 T17.5 10` | 2.6 | In-app header logo mark (all app-shell screens), tray-menu header icon |
| **Big (4-hump)** | `0 0 56 24` | `M4 12 Q10 3 16 12 T28 12 T40 12 T52 12` | 4.5 (96px icon) / 5.5 (48px) / 7 (32px + all tray-taskbar icons) | App icon squircle (all 3 treatments, all sizes), taskbar tray icon (16/24/32) |

Both: `fill="none"`, `stroke-linecap="round"`, `stroke-linejoin="round"`.

### Utility / action icons

| Icon | viewBox | stroke-width | Path `d` / shape | Used where |
|---|---|---|---|---|
| **Search (magnifier)** | `0 0 20 20` | 1.7 | `<circle cx="9" cy="9" r="5.4"/>` + `M13 13 L17 17` | Dictionary/Snippets search bars |
| **Edit (pencil)** | `0 0 20 20` | 1.6 | `M13 4 L16 7 L7 16 H4 V13 Z` (`stroke-linejoin="round"`) | Dictionary/Snippets row actions |
| **Remove/Delete (X)** | `0 0 20 20` | 1.6 | `M5 5 L15 15 M15 5 L5 15` (`stroke-linecap="round"`) | Dictionary/Snippets row actions |
| **Lock (padlock)** | `0 0 20 20` | 1.5 | `<rect x="4.5" y="8.5" width="11" height="7.5" rx="1.5"/>` + `M7 8.5 V6.3 A3 3 0 0 1 13 6.3 V8.5` | History footer note |
| **Speech-model glyph** (musical-note-like) | `0 0 20 20` | 1.6 | `M10 3 V13 M10 13 A2.4 2.4 0 1 0 7.6 15.4 A2.4 2.4 0 0 0 10 13 M10 3 L15 4.6 V8` (`stroke-linecap/linejoin="round"`) | Settings → Speech model row icon |
| **Logout/Quit (door)** | `0 0 20 20` | 1.6 | `M8 4 H5 A1 1 0 0 0 4 5 V15 A1 1 0 0 0 5 16 H8 M12 7 L15.5 10 L12 13 M15.5 10 H7.5` (`stroke-linecap/linejoin="round"`) | Tray menu → "Quit Sotto" |
| **Cancel X (pill, small)** | `0 0 16 16` | 2 | `M4 4 L12 12 M12 4 L4 12` (`stroke-linecap="round"`) | Pill Cancelled toast left icon |
| **Checkmark (done)** | `0 0 24 24` | 2.6 | `M5 12 L10 17 L19 7` (`stroke-linecap/linejoin="round"`, `stroke-dasharray:26`, `animation:mm-draw`) | Pill "Done" state |
| **Twinkle star (4-point)** | `0 0 12 12` | fill only | `M6 0 l1 4 4 1 -4 1 -1 4 -1 -4 -4 -1 4 -1z` (`fill="currentColor"`) | Pill "Polishing" state sparkle |

### Glyph-based "icons" (Unicode text characters, NOT SVG — flag: these need real icon assets or font-glyph rendering at implementation time, they have no path data to trace)

| Glyph | Codepoint | Meaning | Used where |
|---|---|---|---|
| ✕ | `&#10005;` U+2715 | window close | Header bar close control |
| ⧉ | `&#10697;` U+29C9 | copy | Home/History recent-entry rows |
| ↻ | `&#8635;` U+21BB | retry/refresh | Home/History rows, tray "Retry last dictation", **pill retry button** |
| ◆ | `&#9670;` U+25C6 | "personal best" marker | Insights speaking-speed card |
| ▼ | `&#9660;` U+25BC | dropdown caret | Settings microphone selector |
| → | `&rarr;` | maps-to arrow | Dictionary row arrows (also has a literal svg-drawn equivalent used elsewhere for the wordmark — not the same glyph) |
| ! | plain text | warning/error badge | Home alert badge, Pill error toast, Blush-badge circle |

---

## Cross-cutting flags (things to reconcile before/while implementing)

1. **Dark wordmark color drift**: header "Sotto" + tray-menu title use `#EEE7F2` in dark mode, but the Ink-dark token is `#EFE9F1` and every other dark-mode heading uses the token exactly. Decide whether to standardize on the token (recommended) or preserve the drift intentionally.
2. **`#EBE3F5` vs Tint `#ECE5F4`**: the header-logo-mark bg and Dictionary/Snippets callout-box bg both use `#EBE3F5`, one hex-step off Tint. Likely meant to be Tint; verify before assuming it's deliberate.
3. **macOS-isms in a Windows app**: Settings → "Data folder" shows `~/Library/Application Support/sotto`; Icons section is captioned "Big Sur icon & tray". Both need Windows-appropriate substitutes (path + terminology) at implementation time.
4. **Three distinct "success/ready" greens**: `#8FBF9F` (light status dot), `#6FA080` (dark status dot), `#3E8E6E` (Settings "ACTIVE" badge text). Confirm whether these should consolidate to one semantic-success token or intentionally stay distinct (dot vs. badge).
5. **Danger/error color reused consistently**: `#A85874` (light) / blush-pair `#F0C3D2` on `#5B3F4E` (dark) appears in 3 places — Home alert badge, Settings "Clear stats" button, and (via the Blush token pairing) Pill error toast — good candidate for a formal "danger" token.
6. **`mm-bounce` keyframe is defined but never referenced** anywhere in the doc — safe to omit from a first implementation pass, or reserve for a future micro-interaction.
7. **Two wave-logo variants must not be conflated**: small 2-hump (`viewBox 0 0 20 20`) for in-app chrome vs. big 4-hump (`viewBox 0 0 56 24`) for the actual app/tray icon assets — using the wrong one at the wrong size will look off-brand.
8. **Dictionary/Snippets/History/Transforms/Scratchpad/Settings have no explicit dark-mode mockup** in this doc (only Home, Insights, and the Pill states show both themes). Per the doc's own intro line, dark for these "follows the tokens" — apply the standard light→dark token substitution table above, but there is no ground-truth pixel reference to check against for these screens' dark mode.
