# Design prompt 02: History review diff (#25)

Phase C prompt for Claude Design. v1 shipped plain with existing pieces (a `±` row icon,
an indented panel under the row, `.link-add` for the action, a `copied` overlay note).
This prompt asks for the real design of that panel. Nothing here is final.

## Purpose

The trust spine. After a take, the user can see what polish changed between what they
said (raw ASR) and what was typed for them (delivered), and take back what they said.
It must feel like a quiet receipt, not a correction or a red-pen review.

## Where it lives

The History page (`#page-history`), inside the existing recessed card of `.hist-row`s.
Each row: time (JetBrains Mono) · delivered text · `±` · `⧉` copy · `↻` re-polish · `⚑` flag.
`±` opens the panel under that row; it only exists on rows from this session.

## States

1. Row, no raw (reloaded from history.jsonl, or a previous session): no `±` at all. The page
   footnote says "± shows what polish changed, for lines from this session." No error.
2. Row closed (has raw): `±` in muted, accent-ink on hover.
3. Panel open, changed: the word diff, then a muted meta line (tier, fallback reason) and
   "Use what I said".
4. Panel open, unchanged (raw == delivered, e.g. a short take kept by rules): meta line
   only, ending "· no changes", no action.
5. After "Use what I said": the overlay pill shows the `copied` note, "Copied original"
   with an accent tick, ~3 s, no button (same family as Transforms' "Kept your text").
6. Several panels open at once (each row toggles on its own).

## Sample data (invented)

| Raw (heard) | Delivered | tier | fallback |
|---|---|---|---|
| um so let's let's ship the overlay states first | Let's ship the overlay states first. | ai | |
| my main email | you@example.com | rules | llm-error |
| meet at five no six | Meet at six. | rules | |
| يعني افتح ال terminal و شغّل ال build | افتح الـ terminal وشغّل الـ build | ai | |
| الاجتماع الساعة اتناشر ونص | الاجتماع الساعة اتناشر ونص | rules | short |
| gee pee tee is great بس بطيء شوية | GPT is great بس بطيء شوية. | ai | |

Arabic and code-switched rows are RTL paragraphs (`dir="auto"`); English words inside stay
in place. Deleted and inserted words must read in the right order in both directions.

## Sizes and themes

Main window 980×700 (default) and 760×520 (minimum); light and dark; Kai runs zoom 1.25.
The panel is indented to the text column (84 px) and wraps; it must not push the row icons.

## Tokens and components to reuse

- Tokens only (`ui/theme.css` `--mm-*`). v1: diff body `--mm-text-secondary`; deleted words
  `--mm-muted-2` + line-through; inserted words `--mm-accent-ink`; meta `--mm-muted-3`.
  No red/green. Errors or fallbacks are never alarming (blush at most).
- Type: Hanken Grotesk for text, JetBrains Mono for time. No new families.
- `.hist-row`, `.card-recessed`, `.link-add`, the overlay canvas note states.
- Ask: whether deletions should be hidden by default behind a "show removed words"
  affordance; whether the tier belongs on the row itself.

## DTO (what the UI gets)

`HistoryDto` (Rust `src/main.rs`, event `history-updated` and `get_settings().history`):

| field | type | meaning |
|---|---|---|
| `time` | string | "2:14 PM", or "Sep 27" for an earlier day |
| `text` | string | delivered text |
| `raw` | string | ASR transcript; "" for rows reloaded from disk |
| `tier` | string | tier that ran: "off", "rules", "ai" ("" when no raw) |
| `fallback` | string | why AI mode kept the rules result ("short", "no-op", "unavailable", "empty", "llm-error", "line-breaks", or a rewrite-guard reason); "" otherwise |

The diff itself is computed in the UI (`ui/diff.js`, `wordDiff(raw, delivered)` →
`[{op: "=" | "-" | "+", w}]`). Action: `invoke("copy_original", { raw })`.
