# 0001. Pronunciation trainer: button-driven recording + recent-success-rate strength

- Status: accepted
- Date: 2026-09-05

## Context

The Pronunciation trainer's "Listen for it" button only armed a target word
(`set_pronunciation_target`); the user still had to press their real
dictation hotkey to actually record, in whatever activation mode (Hold/Toggle)
they'd configured globally. Live testing showed this read as "nothing
happens" — the two-step flow wasn't discoverable, and the status text had to
explain hotkey-mode-specific instructions just to compensate.

Separately, the trainer's only feedback signal was a "strength" ring counting
`heard_as.len()` — the number of distinct mishearings ever corrected for a
word, capped at 5. This number never moves when a word is heard *correctly*,
and only grows when it's heard wrong AND the user clicks "Add correction." So
a word that Sotto now recognizes reliably looks identical to a word never
attempted. Kai's request was for a fingerprint-enrollment-style effect that
lights up more "the more it's sure of it getting the majority... correctly"
— i.e. it should track getting it *right*, not cataloging ways to get it
wrong.

## Decision

1. **The button drives recording directly.** Clicking "Listen for it" sends
   a `DictationEvent::Start` immediately (bypassing the global hotkey
   listener entirely for this flow); clicking it again (now labeled "Stop")
   sends `Stop`. This is a second entry point into the same dictation state
   machine that `hotkey.rs` currently owns exclusively — the pipeline itself
   doesn't care who sent the event.
2. **A big focus view replaces the small input while listening.** The word
   being trained is shown large and centered for the duration of the take
   ("zoom in on the word"), reacting live to mic input level (the existing
   `overlay-level` signal) so it's visibly alive while recording — there is
   no live ASR confidence signal available mid-utterance (chunked
   transcription doesn't stream partial text to the frontend), so the live
   reaction is amplitude-driven, not recognition-driven.
3. **Strength becomes a recent-success-rate metric**, not a correction
   count. Every resolved take (matched or not — logging a correction is a
   separate, optional action) is appended to a capped rolling history per
   word; the glow/fill is `matches / len(recent)`. This is a new persisted
   data shape alongside `VocabEntry.heard_as` — a `recent: Vec<bool>` (or
   equivalent), capped at a small fixed window.
4. The existing "Trained words" list ring is updated to show this same
   success-rate metric instead of the old correction count, so the app only
   ever shows one definition of "strength" — the old metric doesn't disappear
   in spirit, `heard_as` still exists and still feeds the AI vocabulary hint
   prompt, it's just no longer what's rendered as "strength."

## Consequences

- Makes: the trainer feel immediately interactive (one click, visible
  recording, visible payoff) and makes "strength" mean what a user intuitively
  expects ("Sotto gets this right now") instead of an internal implementation
  detail (mishearing coverage for the LLM prompt).
- Makes harder: `hotkey.rs`'s global listener is no longer the sole source of
  `DictationEvent`s — any future reasoning about "what can start a
  recording" has to account for this second path. Kept narrow (only this one
  button, only while the Pronunciation page is open) to limit the blast
  radius.
- Rules out: showing live per-syllable recognition confidence during a take
  (the fingerprint-scan metaphor taken completely literally) without adding
  streaming partial ASR results to the pipeline — out of scope here, called
  out explicitly rather than silently approximated.
- New state to design: where the rolling per-word attempt history lives
  (`config.toml` alongside `VocabEntry`, most likely) and its window size —
  left as an implementation detail, not re-litigated here.
