# CONTEXT

## Glossary
- **Listen for it** — the button on the Pronunciation page (`ui/index.html:302`, `#pron-listen-btn`). Now drives recording directly: one click arms the target *and* starts the mic; the same button (relabeled "Stop") ends the take. No hotkey involved in this flow. See [ADR 0001](docs/adr/0001-pronunciation-trainer-direct-record-and-success-rate.md).
- **Sample** — one attempt at saying the trained word while armed: what Parakeet heard, and whether it matched the target (`pron-sample-row` in the UI, `PronunciationSampleDto` in Rust). Every resolved sample now feeds the rolling success-rate history, whether or not a correction is logged for it.
- **Focus view** — the big, centered word display shown in place of the small input while a take is recording. Reacts live to mic input level (`overlay-level`) as visible proof it's hearing you. No live recognition/confidence signal exists mid-utterance, so this is an amplitude reaction, not a partial-transcript one.
- **Strength** — how "recognized" a word is, shown as a glow/fill on the big word and as the ring in "Trained words." Now defined as *recent success rate* (matches ÷ recent attempts, rolling window), not the old "number of distinct mishearings caught" count. `heard_as` (the correction list) still exists and still feeds the AI vocabulary-hint prompt — it's just no longer what's rendered as strength.
- **Trained word** — an entry in `polish.vocabulary` (`VocabEntry { word, heard_as }`) — a word Sotto has at least one recorded attempt for.

## Decisions (lightweight)
- The white bar Kai saw after picking a suggestion in the word input is the browser's native autofill styling (`input:-webkit-autofill` forces a light background/dark text, overriding the app's dark theme) — root-caused from code, not a design question. Fix: CSS override + `autocomplete="off"` on `#pron-word-input`.
- The "Listen for it" button already used the same shared `.btn.btn-primary` class as every other primary action in the app — no CSS mismatch existed. Read as "make this page's primary action feel alive," which the button-starts-recording decision below now does structurally (it has real state to animate, not just a label).
- Recording start/stop, focus view, and the strength-metric change are recorded as [ADR 0001](docs/adr/0001-pronunciation-trainer-direct-record-and-success-rate.md) — a real architecture/data-model change, not a routine tweak.
- The existing "Trained words" list ring switches to the same success-rate metric as the focus view, so the app only shows one definition of "strength" (not two competing ones side by side).
- Rolling-history window size and exact persistence shape (`config.toml` next to `VocabEntry`, most likely) are left as implementation details — not worth a question, no competing product directions.

## Open questions
None outstanding — ready to spec/build against ADR 0001.
