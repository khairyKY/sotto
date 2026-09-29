# Design prompt 04: Scratchpad (#19)

Phase C prompt for Claude Design. v1 shipped plain with existing pieces: History's rows
(`.hist-row` in a `.card-recessed` list) with four icon buttons, the BETA badge, the chord
as `keycap-mini` caps, and the History lock footnote. This prompt asks for its real design.
Nothing here is final.

## Purpose

A private place to think out loud. The user presses a chord from any app, the pad comes
up, they dictate, and each take becomes a row there instead of being typed somewhere.
Later a row is copied, typed into the app they came from, parked to a markdown file, or
deleted. It should feel like a quiet notebook that listens, not a chat log or an inbox.

## How it differs from the Marshmallow mock

`Sotto Marshmallow.html#scratchpad` (see `docs/marshmallow-spec-extracted.md`
"Scratchpad") draws a notes app: a recents list on the left, one editable note on the
right, "+ New note". v1 is rows, not notes: one row per take, newest first, no titles, no
editing. Reconcile the two. Ask: should consecutive takes group into one note (by time
gap?), and does a row need a title at all?

## Where it lives

A page in the main window (`#page-scratchpad`, nav item under Transforms), shown only when
the `scratchpad` config flag is on. Not a new window: the RAM work (#13) wants fewer
WebView2 windows. The chord (default Ctrl+Alt+Space) shows the main window on this page
and hides it again, returning focus to the app the user was in.

A take lands in the pad only when it was spoken into the pad (this page in front). A take
spoken into any other app is typed there as usual, even while the pad window is visible.

## States

1. Flag off: no nav item, no page.
2. Empty: "No notes yet. Hold Right Ctrl and think out loud." (verb and key follow the
   hotkey settings).
3. Rows: time (JetBrains Mono, "2:14 PM" today, "Sep 27" earlier) · text · actions.
   Multi-line takes keep their line breaks.
4. A row just landed (the pill shows its normal "done"): does the new row need an arrival
   cue?
5. Actions per row, v1 glyphs:
   - `⧉` copy (a row click copies too);
   - `↵` "Type into Notepad", hidden when no app was captured;
   - `⇲` "Park to file", hidden when no park file is set; turns `✓` once parked, `!` with
     the error in its title if the append failed;
   - `✕` delete, no confirm.
6. After "Type into …": the pad hides, focus returns to that app, the text is typed, the
   pill shows "done" (or "Kept your text" if the app is gone or the typing failed).
7. Save failure: a take that couldn't be written stays undelivered; the pill shows the
   normal error with ↻, and Home's alert card offers the retry.

## Sample data (invented)

| Time | Text |
|---|---|
| 4:45 PM | الـ demo بكرة الساعة عشرة، جهز الـ slides |
| 4:36 PM | Launch checklist:<br>ship the overlay states first<br>then the icon set |
| 3:46 PM | افتكر أجيب عيش وجبنة وأنا راجع |
| Sep 27 | Ask the landlord about the heater before Friday. |

Arabic and code-switched rows are RTL paragraphs (`dir="auto"`); English words inside stay
in place. Actions stay on the right in every row.

## Sizes and themes

Main window 980×700 (default) and 760×520 (minimum); light and dark; Kai runs zoom 1.25.
At 760×520 × 1.25 the text column is narrow (about 150 px next to four actions): consider
actions on hover plus keyboard focus, or a row menu.

## Tokens and components to reuse

- Tokens only (`ui/theme.css` `--mm-*`). v1: row text `--mm-text`, time `--mm-muted-2`,
  actions `--mm-muted-2` → `--mm-accent-ink` on hover and focus.
- Type: Newsreader (title), Hanken Grotesk (text), JetBrains Mono (time, keycaps).
- `.hist-row`, `.card-recessed`, `.history-footnote`, `.badge-beta`, `.keycap-mini`, the
  pill's existing states. No new pill state is needed.
- Ask: whether "Park" should also remove the row (v1 keeps it).

## DTO (what the UI gets)

`scratchpad_state()` (Rust `src/scratchpad.rs`) → `PadState`:

| field | type | meaning |
|---|---|---|
| `enabled` | bool | the `scratchpad` config flag |
| `chord` | string | "Ctrl+Alt+Space", as configured |
| `canPark` | bool | `scratchpad_park_file` is set |
| `targetApp` | string | the app "Type into" types into; "" when none |
| `rows` | `[{id, text}]` | oldest first; `id` is the Unix ms it landed |

Event `scratchpad-updated` carries `rows` after a take lands. Actions:
`scratchpad_page({open})`, `copy_text({text})`, `scratchpad_inject({text})`,
`scratchpad_park({text})` (rejects with a message), `scratchpad_delete({id})` → rows.
