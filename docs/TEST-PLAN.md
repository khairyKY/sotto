# Sotto test ledger

Everything testable in Sotto, grouped by area. **This file is append-only.** Add rows;
never delete or rewrite one. To change a row's state or record a verification, add a
dated note under "State changes" at the bottom of its section, e.g.
`2026-10-02 · D12 · installed (v0.6.1 swap) · verified by Kai`. To retire a row, mark it
~~struck~~ with a date and a pointer to what replaced it.

Row format: **steps · expected result · covering test · state · last verified**.

- **Covering test** is a Rust test from `cargo test --bin sotto -- --list` (131 tests at
  `ab09d50`, all green on 2026-09-28), or **manual**. `tests::…` are the tests in `src/main.rs`.
- **State**, as of master `ab09d50`, is the furthest tier the behaviour has reached:
  - **merged**: on `master` only.
  - **installed**: in Kai's current build (the 2026-09-24 exe, SHA-256 `760e83af…`, the same
    code as `9823c6c`).
  - **published**: in public **v0.3.0**.
- **Last verified**:
  - `2026-09-28 auto` = its unit tests passed that day;
  - `2026-09-28 mock` = checked in the browser preview (mock mode, no Tauri);
  - `—` = never checked by hand in this ledger.
- **❌ gap A-n** marks an expected result that fails today. The gap is written up in
  `D:\Coding\_claude-cloud\sotto-log\2026-09-28-1704-phase-a-audit.md` and gets a GitHub
  issue number via a dated note.

**Snapshot.** The first version of these rows describes master `ab09d50`; file:line
references are at that commit. Merged after it, and not yet covered by rows:
- `39ff7eb`, the v0.6.1 version bump;
- `10e1251`, #9 persistent history + crash-safe takes (see the note under History).

Add their rows with dated notes.

**2026-09-29 catch-up.** Everything merged in batches 6–9 (`master` = `1ede4e6`) is now covered
by the "Test these first (🆕 since 2026-09-29 catch-up)" section below
and by dated "State changes" notes in the sections each PR touched: #8, #10, #11, #12, #14,
#15, #24, #37 (already covered pre-catch-up), #39, #42, #43, #44, #46, #47, #48, #50, #51, #52,
#53, #55, #56, #57, #58, #59, #60, #61, #62, #63, #64, #65, #66, #67, #70, #71, #72, #73, #74,
#75, #77, #78, #79, #81, #83, #84, #86, #87, #88. See
`D:\Coding\_claude-cloud\sotto-log\2026-09-29-0305-conductor-status.md` for the batch/merge
map and `sotto-log\2026-09-28-*-handoff.md` for the per-issue detail this catch-up drew from.

**2026-09-30 release.** Sotto v0.6.1 was installed on Kai's machine and published on
2026-09-30 (tag `v0.6.1` = master `7ceb023`). The release record is
`D:\Coding\_claude-cloud\sotto-log\2026-09-30-1737-conductor-release-v0.6.1.md`. Full suite
on the released tree, from that record: 236 passed, 2 ignored.

State changes:
- 2026-09-30 · every row marked **merged** as of `7ceb023` (including P24 and P25, still
  marked `branch (#8)`, which merged) · now **installed** (Kai's machine, 2026-09-30) and
  **published** (v0.6.1). Rows marked **installed** (the 2026-09-24 exe) are published too:
  v0.6.1 is built from `7ceb023`, which contains that code. Read every "merged" or "branch"
  State cell above with this note. The "Test these first" sections' "not yet installed"
  wording is history.
- 2026-09-30 · Last verified · **not changed by the above.** Only 🆕 D24 (§1) is verified
  in this note. Every other row keeps its "Last verified" value, and the manual ones stay
  manual, not yet verified on the installed build.

Test data is always invented. Never use real snippets, contacts or dictated text here.
Real-voice runs are manual rows for Kai.

How to run the automated part:
```powershell
pwsh -NoProfile -File D:\Coding\_claude-cloud\sotto-tools\cargo.ps1 -Dir <worktree> test --bin sotto
node --check ui/index.js; node --check ui/overlay.js; node --check ui/menu.js
python scripts/check-hidden.py
bash scripts/check-uninstall-delete.sh          # #53: uninstaller only touches Sotto's own files
sotto.exe --replay-flags                        # #8: flag-corpus regression, see §3's how-to
```

---

## Test these first (🆕 since 2026-09-28)

Rows marked **merged** need the v0.6.1 build installed first (the installer is built and
waiting for Kai's swap "go"). On the current Sep-24 build they still show the old
behaviour.

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 F1 | Settings → Mode **Toggle**, **Always show the pill** on. Focus Notepad. Click the small pill, say a sentence, press the dictation key **once**. | Pill goes Listening → Transcribing → Done. The whole sentence lands in Notepad. One press was enough. | `hotkey::tests::a_take_started_by_click_is_stopped_by_the_first_toggle_press` + manual | merged (#37) | 2026-09-28 auto |
| 🆕 F2 | Same setup. Click the pill, speak, then click the **body** of the listening pill (not the ✕). | The take stops and is typed into Notepad. | manual (Conductor mock check of `overlay.js`, PR #38) | merged (#37) | 2026-09-28 mock |
| 🆕 F3 | Same setup. Click the pill, speak, click the **✕**. | "Cancelled" toast with ↻. Nothing typed. ↻ types it. | manual | merged (#37) | 2026-09-28 mock |
| 🆕 F4 | Pronunciation page (Toggle mode): type a word, **Listen for it**, say it, then press the dictation key once. | The take ends and a "Heard: …" sample row appears. No second recording starts. | `hotkey::tests::a_take_started_by_click_is_stopped_by_the_first_toggle_press` + manual | merged (#37) | 2026-09-28 auto |
| 🆕 F5 | Mode **Hold**, always-show on. Click the pill, say "first half", then press **and hold** the key, say "second half", release. | Both halves land. Before #37, the key press restarted the recorder and wiped the first half. | manual (audio-thread guard `src/main.rs:1652-1661`, no unit test) | merged (#37) | — |
| 🆕 F6 | Mode **Hold**. Hold the key, speak, click the pill body to stop **while still holding**, then release. | The Transcribing pill stays until Done. The text lands once. | manual (stray-Stop guard `src/main.rs:1711-1719`) | merged (#37 review) | — |
| 🆕 F7 | Mode **Hold**. Hold the key, speak, press **Escape** while still holding, then release. | The "Cancelled" toast stays its full ~6 s with its countdown bar. ↻ still works. | manual | merged (#37 review) | — |
| 🆕 F8 | Pronunciation: type a word you've trained, **Listen for it**, say it once. | While you are still recording, the big word ignites and the status shows "Got it ✓". About 0.7 s later the take stops by itself and a sample row appears. Saying a *different* word does not ignite. | `tests::heard_word_matches_whole_words_and_multiword_targets` + manual | installed (`9823c6c`) | 2026-09-28 auto |

**Found by today's audit. Check these right after F8** (they are in the installed build):

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 F9 | Do F8. Then go to Notepad and dictate normally with the hotkey. | Text lands in Notepad and shows in History. **❌ gap A-1**: the take is swallowed (pill says Done, nothing typed, a "Heard: …" row appears on the trainer page) until Sotto restarts. Workaround: Quit from the tray and relaunch after training. | manual | installed | — |
| 🆕 F10 | Pronunciation: type a word, **Listen for it**, then click **Stop** right away without speaking. | The trainer returns to idle and the button works again. **❌ gap A-3**: it stays on "Checking…" with the button disabled until the window reloads. | manual | installed | 2026-09-28 mock |
| 🆕 F11 | Pronunciation: say the word so the engine adds a full stop (e.g. a clear one-word sentence). | Glow and sample agree. **❌ gap A-3**: the glow can say "Got it ✓" while the sample is recorded as a miss, because the sample needs an exact whole-transcript match. | manual | installed | — |

---

## Test these first (🆕 since 2026-09-29 catch-up)

Everything below merged on 2026-09-28 in batches 6–9 (`master` = `1ede4e6`). **State: merged
only** — none of it is in Kai's installed build yet (the 2026-09-24 exe) or published; v0.6.1
is built and waiting on Kai's swap "go". Check these on the v0.6.1 build once it's installed.
Full suite on `1ede4e6` (this catch-up, via the cargo wrapper): **206 passed, 0 failed**.

**Dictation & takes**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G1 | Dictate a long take (25 s+) at a quiet, low volume from the first word. | The opening sentences are typed — before this, a quiet take under the old whole-chunk-mean silence check could be sent empty and lost. | `tests::quiet_speech_is_not_silence`, `tests::silence_is_recognised_as_silence` + manual | merged (#64) | 2026-09-29 auto |
| 🆕 G2 | Dictate a long take (25 s+) with natural pauses, ending near a chunk boundary. | No doubled or dropped words at the ~10–24 s seams, and no stray "." or capital left mid-sentence. | `tests::seam_join_drops_the_words_both_sides_heard`, `tests::seam_join_leaves_coincidences_alone`, `tests::chunks_overlap_the_cut_only_when_there_is_speech_to_re_hear` + manual | merged (#64) | 2026-09-29 auto |
| 🆕 G3 | Hold the hotkey 2–3 s in a silent room and release. | Nothing is typed, no "Didn't catch any speech" alert; the take ends idle at once. This is a stricter version of D19 (❌ gap A-5) — a genuinely silent short take is now dropped before ASR runs at all. | manual (`src/main.rs` Stop handler, #51) | merged (#51) | — |
| 🆕 G4 | Say a repeated filler run on purpose, e.g. "to to to to to the store". | Collapses to one copy ("to the store"), the way P3's stutter pass already does for 2–3 word runs; digit runs ("5 5 5 5 5") are left alone. | `tests::repetition_loops_collapse_to_one_copy`, `tests::ordinary_repeats_are_speech_not_loops`, `tests::same_word_ignores_case_and_edge_punctuation` | merged (#64) | 2026-09-29 auto |
| 🆕 G5 | Use the Pronunciation trainer (arm a word, Listen, say it, get a sample), then immediately dictate normally with the hotkey into Notepad. | The text is typed and shows in History; the trainer page gets no new "Heard:" row. **This fixes ❌ gap A-1 / 🆕 F9** — before #46/#47/#71, the very next take after training was silently swallowed until restart. | `tests::a_start_always_consumes_the_trainer_arm`, `tests::stale_peek_never_fires`, `tests::trainer_takes_are_not_journaled` + manual | merged (#46, #47, #71) | 2026-09-29 auto |
| 🆕 G6 | Pronunciation: Listen for it, then Stop immediately without speaking. | The page returns to an enabled "Listen for it" at once. **Fixes ❌ gap A-3 / 🆕 F10's "stuck on Checking…" half.** | manual (`ui/index.js` overlay-state listener, #71) | merged (#71) | — |
| 🆕 G7 | Arm a word, Listen, speak, kill Sotto from Task Manager mid-take, relaunch. | No recovered take appears on Home or as a tray "Retry" — a trainer take is never journaled, so there's nothing to recover into a real dictation. | `tests::trainer_takes_are_not_journaled` + manual | merged (#44, #46) | 2026-09-29 auto |

**Engines**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G8 | Settings → Speech model → **Use this** on a different engine (e.g. Whisper turbo), then dictate right away with **no restart**. | The row shows ACTIVE at once, no "Restart Sotto…" note. `sotto.log` shows `ASR engine switched from=… to=…` then `ASR model loaded engine=…`, and that take's `transcribed engine=…` names the new one. **This supersedes E3, E5's "restart" wording and SE1's "Speech model needs a restart" note** — see the dated notes below. | manual (`asr.rs::sync`, #10) | merged (#10, #87) | — |
| 🆕 G9 | In toggle mode, start a take longer than 15 s, switch engine in Settings while it's still recording, then stop. Dictate again. | Every line of the first take's transcript names the old engine; the second take names the new one — the swap never happens mid-take. | `asr::tests::a_settings_change_without_an_engine_switch_keeps_the_model_then_idle_frees_it` + manual | merged (#10, #87) | 2026-09-29 auto |
| 🆕 G10 | Dictate once, then leave Sotto idle 5+ minutes. Then say one word. | The log shows `ASR model unloaded (idle)`, `sotto.exe` memory drops (~650 MB Parakeet, or ~1.1 GB VRAM on Whisper). The next word still delivers, at most ~2 s slower than usual (the reload happens while you speak). | `asr::tests::idle_unload_waits_only_while_a_model_is_loaded` + manual | merged (#12, #87) | 2026-09-29 auto |
| 🆕 G11 | On a Whisper engine, dictate a sentence naming a word you trained in Vocabulary. | Comes out spelled the trained way more often — the trained words are now passed as Whisper's `initial_prompt`. `sotto.log` shows `ASR warmed up prompt_bytes=` > 0. Off-switch: `[asr] vocabulary_prompt = false`. Parakeet is unaffected (it can't take a prompt). | `asr::tests::no_words_means_no_prompt`, `asr::tests::misheard_words_lead_and_ties_keep_list_order`, `asr::tests::caps_the_prompt_keeping_the_most_relevant_words`, `asr::tests::an_overlong_word_is_skipped_not_cut`, `config::dir_tests::vocabulary_prompt_defaults_on_in_an_older_asr_section_and_can_be_switched_off` + manual | merged (#15) | 2026-09-29 auto |
| 🆕 G12 | Set engine to whisper-turbo, restart, wait ~10 s, then dictate one sentence. | `sotto.log` shows `ASR warmed up warmup_ms=…` right after `ASR model loaded` — the first take is no longer slow (a cold GPU driver cache used to add 44–80 s to the very first dictation). | manual (`asr.rs`, #14) | merged (#14) | — |

**Polish**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G13 | Train "Claude" and "Sotto". Dictate "I'm not sure but I said we should study it soon, sorry." | Unchanged, no flyout — before this, the loose single-word phonetic rule fired on ordinary words like "said" and "sure". | `polish::tests::soundex_codes_known_pairs`, `polish::tests::split_affixes_keeps_a_multibyte_last_char_whole`, `polish::tests::phonetic_never_replaces_common_words_or_code`, `polish::tests::phonetic_pulls_mishearings_to_trained_words` + manual | merged (#63) | 2026-09-29 auto |
| 🆕 G14 | Say an Arabic or code-switched sentence with any vocabulary trained. | No crash. **Fixes a real panic**: the old code sliced inside a multi-byte character and crashed on any Arabic word going through number formatting or the phonetic pass. | `polish::tests::split_affixes_keeps_a_multibyte_last_char_whole` + manual | merged (#63) | 2026-09-29 auto |
| 🆕 G15 | Train a multi-word phrase (e.g. an invented product name of 2 words). Dictate a sentence where ASR mishears the word boundary (e.g. "hugging phase" for a trained "Hugging Face"). | The trained phrase comes out correctly, and the smart flyout shows the heard → corrected pair. Ordinary two-word English phrases are left alone. | `polish::tests::phonetic_pulls_multiword_mishearings_to_trained_phrases`, `polish::tests::phonetic_multiword_leaves_ordinary_english_alone` + manual | merged (#24) | 2026-09-29 auto |
| 🆕 G16 | Cleanup **AI**. Dictate a longer sentence and watch for dropped clauses or invented content. | If the AI output drops 2+ content words, adds invented ones, or leaks a worked-example word, Sotto now falls back to the Rules result instead of typing the damaged AI text. The pill still shows Polishing; only the final text differs. `sotto.log` records the fallback reason (counts only, never the text). | `polish::tests::rewrite_guard_accepts_a_cleanup`, `polish::tests::rewrite_guard_rejects_a_dropped_clause`, `polish::tests::rewrite_guard_rejects_invented_words`, `polish::tests::rewrite_guard_catches_the_worked_example_leaking` + manual | merged (#65, #67) | 2026-09-29 auto |
| 🆕 G17 | Cleanup **AI** with a fallback-worthy take (see G16), then check History/Recordings for that take. | The recordings index gains a `fallback` field naming why the AI result was rejected (blank for a normal AI take). Old index lines without it still load. | `recordings::tests::index_lines_carry_the_fallback_reason_and_old_lines_still_load` | merged (#65, #67) | 2026-09-29 auto |
| 🆕 G18 | Say "hello there new line see you soon" in **AI** mode (or Rules). | A real line break appears — this was P19's known-fixed case for Rules, but AI mode's number/phonetic passes used to silently drop the break by rebuilding text with a plain word-join. | `polish::tests::new_line_command_survives_the_rules_pipeline` + manual | merged (#85) | 2026-09-29 auto |
| 🆕 G19 | Say "just give me one option" (or any sentence using "one" as a pronoun, not a count). | "one" stays a word; it isn't swallowed by the numbers pass the way a bare cardinal would be. | `polish::tests::numbers_leave_the_pronoun_one_alone` | merged (#66) | 2026-09-29 auto |
| 🆕 G20 | Cleanup **AI**, dictate a short clip (under the AI threshold) or trigger the rules fallback, with a Harper-fixable slip (e.g. a doubled word). | Harper now runs on AI mode's rules path too, same as Rules mode (P4/P5). **This fixes ❌ gap A-10** (P5). | `polish::tests::ai_mode_fallback_gets_the_same_harper_fixes_as_rules` | merged (#55) | 2026-09-29 auto |
| 🆕 G21 | Dictate anything, then open `%APPDATA%\sotto\logs\sotto.log`. | The log holds character/word counts only — never the dictated text or dictionary/vocabulary/snippet entries. **Fixes ❌ gap A-2** (R2). Full transcripts still go to the `debug` level only, never `info`. README's log-privacy claim now matches the code. | manual (`config_for_log`, #48, #70) | merged (#48, #70) | — |

**UI**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G22 | Dictate an Egyptian Arabic or code-switched sentence, then open Home and History. | The line reads right-to-left in the correct word order; the time column stays left and the buttons stay right. **Fixes ❌ gap A-6** (H5, DI8). Same for Dictionary/Snippets cells, alias chips, edit inputs, and the Pronunciation trainer's word/sample text. | manual (`dir="auto"` / `<bdi>`, #42) | merged (#42) | — |
| 🆕 G23 | Dictate a code-switched line into the overlay flyout ("heard → corrected" toast) with Arabic-first text. | The Arabic segment draws right-to-left and the English words inside it stay in place; the pill's own layout doesn't move. | manual (`ui/overlay.js` canvas direction, #42) | merged (#42) | — |
| 🆕 G24 | Toggle Windows between light/dark with Sotto's theme on System, and open the served UI preview specifically. | No `Cannot read properties of undefined (reading 'event')` console error in the preview (mock mode had no `T.event` guard on the theme-change listener). | manual (browser-mock check, #43) | merged (#43) | 2026-09-29 mock |
| 🆕 G25 | Always-show pill on. Settings → change the pill position (e.g. bottom-center → top-right) with **no restart**. | The window and the tucked pill both move to the new edge on the next frame; hover, click, and ✕ all line up with the new spot. **Fixes ❌ gap A-4** (O6) — before this, the canvas kept the old edge and the hit-test was misaligned until restart. | manual (`overlay-position` listener, #50) | merged (#50) | — |
| 🆕 G26 | Open the tray menu → Settings. | The window opens with the Settings modal showing. **Fixes ❌ gap A-11** (T5). | manual (#56) | merged (#56) | — |
| 🆕 G27 | Click an example chip on Dictionary or Snippets (leave the draft open), then toggle any other entry's switch. | The open draft is **not** saved. **Fixes ❌ gap A-12** (DI6, S2). | manual (#57) | merged (#57) | 2026-09-29 mock |
| 🆕 G28 | Dictate today, then check Home's "words today" card. Also try Insights Week/Month and "min saved". | "words today" shows today's count, not the week's; the Insights period label follows Week/Month; "min saved" uses the backend's real figure. **Fixes ❌ gap A-13** (H2, H3, N3, N4). | manual (`localDayNum`, `timeSavedMin`, #58) | merged (#58) | — |
| 🆕 G29 | Tray → Pause dictation. | Home's status bar shows a muted "paused" chip; the tray menu dot and tooltip go muted too ("Sotto (paused)"). Clicking the always-visible idle pill, or opening Pronunciation, while paused now also does nothing / says paused — **fixes ❌ gap A-15** (D6). | manual (#60) | merged (#60) | — |
| 🆕 G30 | Windows Settings → Personalization → Colors → Windows mode Dark (apps Light), then launch Sotto. | The tray tile is the dark-taskbar variant from launch, not light. **Fixes ❌ gap A-9's tray half** (T9). | `tray::tests::theme_picks_tile_and_system_reads_the_taskbar` + manual | merged (#60) | 2026-09-29 auto |
| 🆕 G31 | With Sotto already running (hidden or minimized), launch it again from the Start menu or a shortcut. | The running window comes to the front; no second tray icon, no silent second-instance exit. **Fixes ❌ gap A-16** (A14). | `single_instance::tests::a_signal_wakes_the_listener_and_an_unheard_one_is_harmless` + manual | merged (#61) | 2026-09-29 auto |

**Install / uninstall / downloads**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G32 | Cut the network mid-download of a large model, restore it, click Download or relaunch. | Resumes from the partial `.part` file instead of restarting at 0%. **Fixes ❌ gap A-7** (A4). | `assets::tests::dropped_stream_keeps_part_then_next_run_resumes`, `assets::tests::empty_part_is_a_plain_download`, `assets::tests::oversized_part_restarts_clean_after_416` + manual | merged (#52) | 2026-09-29 auto |
| 🆕 G33 | Settings → Speech model: an engine mid-download shows its row and the first-run banner; interrupt it, then Retry. | Row: "Download stopped · Retry picks up where it left off". Banner names the model + cause. Retry resumes, not restarts. A downloaded-but-not-running engine shows "RESTART TO USE" instead of a false ACTIVE badge. | manual (#11) | merged (#11, #52) | — |
| 🆕 G34 | Set `assets_dir = 'E:\some-folder'` (any non-default drive/folder) with a `models\` subfolder inside, launch once, then uninstall → **Yes**. | Both `%APPDATA%\sotto` and that custom assets folder are removed. **Fixes ❌ gap A-8** (A11) in both directions: a real `D:\sotto` example folder is no longer force-deleted, and a real custom `assets_dir` elsewhere now is. | `bash scripts/check-uninstall-delete.sh` (scripted, deletes only two throwaway test folders — never real data) + manual | merged (#53) | 2026-09-29 auto |
| 🆕 G35 | Point `assets_dir` at a folder with neither `models\` nor `onnxruntime.dll`, or at a bare drive root / the user's profile folder, then uninstall → Yes. | That folder is left untouched — the uninstaller's safety checks refuse to touch anything that doesn't look like a Sotto assets folder. | `bash scripts/check-uninstall-delete.sh` + manual | merged (#53) | 2026-09-29 auto |

**Privacy**

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| 🆕 G36 | Flag a History row (⚑), then check `bug-reports.jsonl`. | The line carries `raw` (what the engine actually heard) next to `text` (what was typed) — `raw` lives in memory only and is never written to `history.jsonl`. Restart, then flag an old row: `raw` comes back empty rather than erroring. | `bug_reports::tests::a_line_from_before_raw_existed_still_parses`, `history::tests::raw_is_found_by_row_text_but_never_written_to_disk` (see P24) | merged (#8) | 2026-09-28 auto |
| 🆕 G37 | `sotto.exe --replay-flags` (see P25 and the §3 how-to). | Replays every ⚑ flag's raw transcript through the live polish chain, diffs against the last run, and exits 1 on any change — the trust spine for every polish-chain change from here on. Nothing it prints or logs is dictated text beyond what was already flagged. | `bug_reports::tests::replay_diffs_against_the_last_run` (see P25) | merged (#8) | 2026-09-28 auto |
| 🆕 G38 | Read the README/RUNNING claims about the pill, asset tags, the update banner location, GPU support, and log privacy. | All five now match `master`'s real behaviour (four were fixed, one confirmed already accurate); two genuinely master-only opt-ins (Keep history, always-visible pill) are called out by name in a note. | manual (docs review, #59) | merged (#59) | — |

---

## 1. Dictation

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| D1 | Mode Hold. Hold the key in Notepad, say a sentence, release. | Listening (lilac) → Transcribing (amber) → Done ✓. Text typed where you started. | manual | published | — |
| D2 | Mode Toggle. Tap, speak, tap. | Same as D1. | `hotkey::tests::a_clean_press_release_toggles_once` + manual | published | 2026-09-28 auto |
| D3 | Toggle mode. Hold the key down past the keyboard auto-repeat delay, then release. | Exactly one Start. The next tap stops. | `hotkey::tests::os_auto_repeat_while_held_does_not_re_toggle`, `hotkey::tests::release_with_no_prior_press_is_a_no_op` | installed | 2026-09-28 auto |
| D4 | Tray → Pause dictation. Press the hotkey. Then Resume and press again. | Paused: nothing starts. Resumed: works. | `hotkey::tests::paused_blocks_starting_but_still_tracks_the_key_as_down` + manual | published | 2026-09-28 auto |
| D5 | Start a take, then Pause from the tray, then stop with the key. | The take stops and delivers (pause only blocks *starting*). | `hotkey::tests::paused_does_not_block_stopping_an_already_active_dictation` | published | 2026-09-28 auto |
| D6 | Paused, with always-show on: click the idle pill. | Nothing starts. **❌ gap A-15**: the pill still starts a take while paused. | manual | installed | — |
| D7 | Settings → Change… hotkey → pick F8 (or press a key in the capture box). Use F8 without restarting. | F8 dictates at once. The Home hint and the Settings keycap show "F8". | manual | published | — |
| D8 | Pick a "risky" key (e.g. Space) in the hotkey list. | Asks "Use … as your dictation hotkey?". Cancel keeps the old key. | manual | published | — |
| D9 | Bind "Mouse: Middle click". Hold middle-click, speak, release. | Dictates like a key. | manual | published | — |
| D10 | Tap the hotkey for less than 0.5 s. | The pill disappears. Nothing typed, no error toast, nothing in the Home card. | manual | published | — |
| D11 | Press Escape while Listening. | "Cancelled" toast with ↻ and a 6 s countdown. Home card: "Last dictation wasn't delivered · Cancelled · Ns of audio". | manual | published | — |
| D12 | Press Escape while Transcribing/Polishing. | Cancelled. Nothing typed. The take is stashed (↻ / tray Retry work). | manual | published | — |
| D13 | After D11: click ↻ on the pill. | The take is transcribed and typed without speaking again. The Home card disappears. | manual | published | — |
| D14 | After D11: Home card **Retry**, and separately **Dismiss**. | Retry: delivered, card gone. Dismiss: card gone, tray "Retry last dictation" greys out ("none yet"). | `tests::take_info_counts_words_once_transcribed`, `tests::take_info_has_no_word_count_before_transcription` + manual | published | 2026-09-28 auto |
| D15 | Dictate one long take (40 s+) with natural pauses. | The log shows `chunk sent` lines during recording. After release the wait is only the last few seconds. The text is complete with no dropped or doubled words at the seams. | `tests::cuts_inside_the_pause`, `tests::no_cut_while_still_mid_sentence`, `tests::unbroken_speech_still_cuts_at_the_ceiling`, `tests::short_takes_are_never_chunked`, `tests::join_text_handles_empty_sides`, `audio::tests::piecewise_drain_matches_one_shot`, `audio::tests::resample_prefix_never_over_consumes` + manual | installed | 2026-09-28 auto |
| D16 | In a long take, stay silent 12 s in the middle. | No invented words in the silent stretch. | `tests::silence_is_recognised_as_silence` + manual | installed | 2026-09-28 auto |
| D17 | Long take: release the key right after a pause. | No stray word appended at the end. | manual (`src/main.rs:1998-2005`) | installed | — |
| D18 | Start a take; while it's Transcribing, start a second one. | The second starts recording immediately and the pill stays Listening. Both texts land in order. | manual | installed | — |
| D19 | In a quiet room, hold the key 2–3 s without speaking. | Nothing typed. **❌ gap A-5**: a short silent take still goes to the model and anything it invents is typed. | manual | published | — |
| D20 | Start in Notepad, Alt-Tab to another window mid-take, release. | Text lands in Notepad (the window focused at start). | manual | published | — |
| D21 | Settings → Microphone: pick a device, dictate. Then unplug it and dictate again. | The log `opening input stream device=` shows the pick. Unplugged: it falls back to the default mic, no failure. | manual | published | — |
| D22 | Settings → Dictation sounds on/off. | On: a soft tick at start and a tock at stop/cancel. Off: silent. | `sounds::tests::wav_header_is_valid_and_sized` + manual | published | 2026-09-28 auto |
| D23 | Always-show pill on. Click the idle tucked pill. | It dips, then morphs into Listening. | manual | installed | — |
| 🆕 D24 | **Hotkey dictation lands in the focused window.** Focus a text field in another app (a chat box or Notepad), press the dictation key, say one sentence, and finish the take (release in Hold, tap again in Toggle). | The sentence is typed once, into the window you started in. `sotto.log` shows `transcript raw_chars=N chars=N`, then `injected chars=N`, with N the length of the sentence that arrived. | manual (a real take on the installed v0.6.1 build) | installed, published (v0.6.1) | **VERIFIED 2026-09-30** by a real take (Kai): 4.69 s of audio, transcribed in 641 ms, `injected chars=42`, matching the 42-character sentence that arrived in the chat window he had focused. The key mode was not recorded. |

State changes:
- 2026-09-29 · D19 · ❌ gap A-5 narrowed by #51 (merged, not yet installed): a genuinely
  silent short take is now dropped before it reaches ASR. See 🆕 G3. D19's hallucination case
  (audio present but still garbage) is unchanged. Re-verify D19 once #51 is installed.
- 2026-09-29 · D15 · chunk seam handling and quiet-speech detection rewritten by #64 (merged).
  See 🆕 G1, G2. D15's covering tests are for the pre-#64 pipeline; new tests are
  `tests::quiet_speech_is_not_silence`, `tests::seam_join_drops_the_words_both_sides_heard`.
  Re-verify D15/D16 once #64 is installed.

## 2. ASR engines + engine switch

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| E1 | With a temp `SOTTO_DATA_DIR` holding the models: `sotto.exe --transcribe sample.wav` (an invented English clip). | Prints `(N samples, M ms) => "…"` and exits. Works while Kai's copy runs. | manual | published | — |
| E2 | Default engine: dictate English. | Log `ASR model loaded … engine=parakeet-v3`. Correct text. | manual | published | — |
| E3 | Settings → Speech model → **Use this** on Whisper turbo (installed). | The "Restart Sotto for this change to take effect" note appears. After restart the log shows `engine=whisper-turbo`, and `load_ms` is logged. | manual | published | — |
| E4 | Whisper turbo: dictate. | `transcribe_ms` about real-time or better on the GPU (Vulkan, flash_attn off). | manual (speed gap tracked in #14) | published | — |
| E5 | Pick a not-installed engine → **Download**. | The row shows %, then ACTIVE after restart. The Egyptian engine fails today (asset 404, #4). | manual | published (Egyptian: installed) | — |
| E6 | With Parakeet selected, open the Language dropdown. | Disabled, with "Parakeet is English-only and ignores this." | manual | published | 2026-09-28 mock |
| E7 | Whisper: set Language to Arabic, restart, dictate Egyptian Arabic. | Arabic script out. (Accuracy: Phase B, #7.) | `asr::tests::auto_language_maps_to_none`, `asr::tests::explicit_language_passes_through` + manual | published | 2026-09-28 auto |
| E8 | Parakeet with `asr.language = "ar"` in config.toml, restart. | The log warns that `asr.language` is inert under Parakeet. | manual | installed | — |
| E9 | Launch, then dictate within ~10 s. | The first take isn't delayed by the model load (`ASR model loaded` appears before the first `listening`). | manual | published | — |
| E10 | Remove the speech model files (temp data dir) and dictate. | The pill shows "Model downloading…" with ↻. The take is stashed and ↻ works once the model lands. | manual | published | — |

State changes:
- 2026-09-29 · E3, E5 · **superseded by #10/#87 (merged, not yet installed).** "Restart Sotto
  for this change to take effect" no longer appears for the speech-model row: an engine switch
  applies on the very next take. See 🆕 G8, G9. Re-verify E3/E5 once #10/#87 are installed;
  the "Download" flow in E5 (Egyptian 404) is unaffected.
- 2026-09-29 · E7 · **superseded by #10/#87.** "restart, dictate" → no restart needed; language
  applies on the next take too. See 🆕 G8.
- 2026-09-29 · E4 · warm-up added by #14 (merged): the first dictation after a restart is no
  longer slow on a cold GPU driver cache. See 🆕 G12.
- 2026-09-30 · E5 · the "Egyptian engine fails today (asset 404, #4)" text no longer holds:
  the file was uploaded to `assets-v2` on 2026-09-23T20:58Z and returns 200 (see the dated
  notes in `docs/updating.md` §0). The engine's remaining known issue is #68 (English speech
  read as Arabic on Auto language). E5's "Last verified" is unchanged.

## 3. Polish chain

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| P1 | Cleanup **Off**. Dictate "um so the the plan". | Raw text, only trimmed. The Home status bar shows the "polish off" warning chip. | manual | installed (chip) / published | — |
| P2 | Cleanup **Rules**: fillers and spacing. | Fillers (um/uh…) gone, spaces collapsed, first letter capitalized. "like"/"so" kept. | `polish::tests::removes_fillers_and_tidies`, `polish::tests::collapses_whitespace`, `polish::tests::keeps_ambiguous_words`, `polish::tests::strips_filler_with_trailing_punctuation`, `polish::tests::idempotent`, `polish::tests::empty_stays_empty` | published | 2026-09-28 auto |
| P3 | Rules: stutters. | "the the store" → "The store". "very very", "had had", "no no" kept. Digits and new sentences are never collapsed. | `polish::tests::collapses_repeated_words`, `polish::tests::collapses_the_exact_ten_uh_report_case`, `polish::tests::stutter_collapse_preserves_first_occurrence_casing`, `polish::tests::stutter_collapse_respects_sentence_boundaries`, `polish::tests::stutter_collapse_keeps_legitimate_doubles`, `polish::tests::stutter_collapse_skips_numeric_tokens`, `polish::tests::stutter_collapse_does_not_eat_a_quote_glyph_glued_to_a_repeated_word` | installed | 2026-09-28 auto |
| P4 | Rules: Harper. | Mechanical single-suggestion fixes only. Ambiguous spellings untouched. | `polish::tests::harper_removes_doubled_word`, `polish::tests::harper_leaves_ambiguous_spelling_alone`, `polish::tests::harper_runs_after_filler_stripping` | published | 2026-09-28 auto |
| P5 | Cleanup **AI**, a short clip (under the threshold) with a Harper-fixable slip. | Same text as Rules. **❌ gap A-10**: AI mode skips Harper on short clips and on LLM fallback. | manual | published | — |
| P6 | AI: dictate 25+ words with fillers. | The pill shows Polishing (gold). The log shows `spawning llama-server` (first time) and `AI polish applied llm_ms=…`. Clean text, same language, not translated. | manual | published | — |
| P7 | AI: move the "Use AI past" slider to 60, dictate 20 words. | No Polishing state; the rules result is typed. | manual | published | — |
| P8 | AI: kill `llama-server.exe` mid-polish, or rename the model file (temp data dir). | Rules result typed. Log `AI polish failed — using rules`. The take is never lost. | manual | published | — |
| P9 | AI: leave Sotto idle 5+ min after a polish. | Log `llm sidecar idle-killed (VRAM freed)`. The next AI take respawns it. Quitting Sotto kills the sidecar too. | manual | published | — |
| P10 | `sotto.exe --polish "um so I think the the plan works"` with a temp `SOTTO_DATA_DIR`. | Prints raw → polished and timing. Runs alongside Kai's copy. | manual | published | — |
| P11 | AI on an Egyptian/code-switched sample. | Each word stays in its spoken language. Not truncated. | manual (Phase B, #7) | installed | — |
| P12 | Dictionary word entry "gee pee tee" → GPT. Dictate it. | "GPT" typed, whole phrase, any casing. | `polish::tests::dictionary_replaces_whole_phrases_case_insensitively`, `polish::tests::word_kind_entries_run_before_tier0_so_a_filler_word_inside_the_phrase_survives` | published | 2026-09-28 auto |
| P13 | An entry with aliases, plus an overlapping shorter entry. | Every alias gives the one replacement. The longest phrase wins. | `polish::tests::aliases_all_map_to_one_replacement`, `polish::tests::longer_phrase_wins_over_shorter_overlapping_one` | installed | 2026-09-28 auto |
| P14 | Disable one entry; separately, flip the master "All replacements" off. | The disabled entry doesn't fire. Master off: nothing fires, and the entries are kept. | `polish::tests::disabled_entries_are_excluded_from_the_live_dictionary` + manual | installed | 2026-09-28 auto |
| P15 | A snippet whose expansion is a sentence (invented "sign off" → "Thanks, and talk soon."). | The expansion is typed verbatim, even in AI mode (snippets run after polish). | manual | published | — |
| P16 | An old config.toml with entries that have no `kind`. | The first launch backfills the kind with the old shape rule and saves once. An explicit kind is never overridden. | `config::entry_kind_tests::migration_matches_the_old_ui_heuristic_word_vs_snippet`, `config::entry_kind_tests::migration_is_a_noop_once_kind_is_already_set`, `config::entry_kind_tests::short_snippet_is_the_bug_this_fixes` | installed | 2026-09-28 auto |
| P17 | Train a 4+ letter word (e.g. "Sotto"). Dictate a new mishearing of it. | Corrected to the trained word. The pill shows the flyout "heard → Sotto". Correct and unrelated words untouched. Toggle "Fix trained words by sound" off → no correction. | `polish::tests::soundex_codes_known_pairs`, `polish::tests::phonetic_pulls_mishearings_to_trained_words`, `polish::tests::phonetic_leaves_correct_and_unrelated_words_alone` + manual | installed | 2026-09-28 auto |
| P18 | Say "twenty three", "seven fifteen a.m.", "the twenty first", "nineteen ninety nine", "three and four". | 23 · 7:15 a.m. · 21st · 1999 · "3 and 4". "seven fifteen" alone is never 22. Toggle off → words kept. | `polish::tests::numbers_the_three_reported_examples`, `polish::tests::numbers_cardinals_and_scales`, `polish::tests::numbers_ordinals_take_the_right_suffix`, `polish::tests::numbers_seven_fifteen_is_not_twenty_two`, `polish::tests::numbers_year_pairs_only_for_19xx_20xx`, `polish::tests::numbers_preserve_surrounding_words_and_punctuation`, `polish::tests::numbers_connector_and_is_not_swallowed` | installed | 2026-09-28 auto |
| P19 | Say "hello new line world new paragraph done". | Real line and paragraph breaks, no stray spaces. Works in Off/Rules/AI. Toggle off → the words are kept. A snippet containing "new line" in its expansion survives. | `polish::tests::formatting_commands_produce_the_right_breaks`, `polish::tests::formatting_commands_absorb_surrounding_whitespace`, `polish::tests::formatting_commands_are_case_insensitive`, `polish::tests::formatting_commands_inert_when_toggle_off`, `polish::tests::formatting_commands_run_before_the_dictionary_so_a_snippet_saying_new_line_survives` | installed | 2026-09-28 auto |
| P20 | Say "open quote hello close quote", "quote hello there unquote", "and I quote, this works." and "a quote from the paper". | Quotes hug their content. The comma form closes at the full stop. The noun "quote" is left alone. Curly style gives “ ”. Formatting toggle off → no quotes. | `polish::tests::open_close_quote_tokens_hug_their_content`, `polish::tests::open_close_quote_tokens_are_case_insensitive`, `polish::tests::paired_quote_unquote_wraps_the_span_between`, `polish::tests::quote_on_quote_is_an_alias_for_the_paired_opener`, `polish::tests::a_lone_quote_with_no_closer_is_left_alone`, `polish::tests::a_comma_pause_after_quote_infers_the_close_at_the_next_period`, `polish::tests::a_comma_pause_after_quote_with_no_terminal_punctuation_closes_at_end_of_text`, `polish::tests::explicit_unquote_still_trims_a_comma_on_either_side`, `polish::tests::two_separate_pairs_in_one_take_both_wrap`, `polish::tests::curly_style_uses_curly_glyphs`, `polish::tests::quote_commands_are_inert_when_formatting_toggle_is_off`, `polish::tests::curly_quote_style_setting_reaches_the_full_pipeline`, `polish::tests::quote_commands_survive_rules_mode_end_to_end` | installed | 2026-09-28 auto |
| P21 | Tone: set Default tone to Casual and a per-app override "Slack → Professional". AI mode. Dictate into Slack and into Notepad. | Slack gets the override (matched case-insensitively), Notepad the default. In Rules/Off the tone has no effect, and the Tone card is greyed out with "Needs AI polish". | `polish::tests::resolve_tone_prefers_exact_app_match_case_insensitively`, `polish::tests::resolve_tone_falls_back_to_default_when_no_app_match`, `polish::tests::resolve_tone_empty_default_yields_no_tone`, `polish::tests::tone_has_no_effect_outside_ai_mode`, `llm::tests::empty_tone_leaves_system_prompt_unchanged`, `llm::tests::tone_appends_as_one_extra_clause` + manual | published | 2026-09-28 auto |
| P22 | Vocabulary hints: train a word with a logged mishearing. AI mode, dictate the mishearing in a long sentence. | The model fixes it back. "I need to write some code" stays "code". | `polish::tests::vocabulary_clause_names_heard_as_variants`, `polish::tests::vocabulary_clause_skips_blank_entries_and_empty_list_is_empty_string`, `llm::tests::empty_vocabulary_leaves_system_prompt_unchanged`, `llm::tests::vocabulary_appends_after_tone`, `llm::tests::vocabulary_alone_appends_without_a_tone_clause`, `llm::tests::vocab_correction_example_is_two_user_assistant_pairs` + manual | installed | 2026-09-28 auto |
| P23 | Insights "Fixes made by Sotto" after a take with fillers and one dictionary hit. | Corrected words and dictionary fixes are counted separately, with no double count. | `polish::tests::changed_words_counts_edits_not_reorderings_of_identical_text` + manual | published | 2026-09-28 auto |
| 🆕 P24 | Dictate "um so the the plan works", then ⚑ flag its History row. | The new line in `bug-reports.jsonl` has `"raw"` (what the engine heard) beside `"text"` (what was typed). Raw lives in memory only: history.jsonl never holds it, so a row reloaded after a restart flags with an empty `raw`. Older lines without `raw` still load. | `bug_reports::tests::a_line_from_before_raw_existed_still_parses`, `history::tests::raw_is_found_by_row_text_but_never_written_to_disk` + manual | branch (#8) | 2026-09-28 auto |
| 🆕 P25 | `sotto.exe --replay-flags` twice. Change a polish rule, run it again, then once more. | Each flag with a raw prints raw / delivered / now; older flags count as "skipped (no raw)". `bug-reports.snapshot.json` appears next to the flags. Run 2: "0 changed", exit 0. After the change: the moved flags show `CHANGED  last run …`, exit 1. Run 4: exit 0 (run 3 became the baseline). No dictated text in `sotto.log`. | `bug_reports::tests::replay_diffs_against_the_last_run` + manual | branch (#8) | 2026-09-28 auto + debug-build CLI run on invented flags |

**Flag replay (#8).** Run it after any polish-layer change. It replays every ⚑ flag's raw
transcript through the current chain and your live config (dictionary, vocabulary,
toggles). Flags use the Rules tier, except a flag taken in AI mode, which replays through AI.
```powershell
sotto.exe --replay-flags | Out-Host; $LASTEXITCODE              # the data dir's bug-reports.jsonl
sotto.exe --replay-flags D:\x\flags.jsonl | Out-Host; $LASTEXITCODE  # any flags file; snapshot beside it
```
- The pipe matters on a release build: it has no console, and the pipe makes PowerShell wait for the exit code.
- Exit 1 means some flag's output moved since the last run. Read the `CHANGED` lines. That run is
  already the new baseline, so the next run passes.
- Like `--polish`, it starts a fresh `sotto.log` in its data dir. A Sotto running on the same
  data dir keeps logging into what is now `sotto.log.1`.

State changes:
- 2026-09-29 · P5 · ❌ gap A-10 **fixed by #55 (merged, not yet installed)**: AI mode's rules
  fallback now runs Harper too. See 🆕 G20.
- 2026-09-29 · P19 · the "new line" drop found separately in AI mode (P19's own Rules-mode
  tests were never wrong) is **fixed by #85 (merged)**. See 🆕 G18.
- 2026-09-29 · P17, P22 · the single-word phonetic rule these rows depend on was tightened by
  #63 (merged): it no longer fires on real English words. Both rows' own behaviour and
  covering tests are unaffected (they test trained-word mishearings, which still fire); the
  false-positive fix is new row 🆕 G13.

## 4. Injection + clipboard safety net

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| I1 | Default (paste). Dictate into Notepad, VS Code, a browser field, the Claude Code terminal and a chat app. | Text lands once in each. | manual | published | — |
| I2 | Hotkey Right Ctrl, paste mode: dictate. | No self-triggered extra dictation from the injected Ctrl+V (log shows one `listening`). | manual | published | — |
| I3 | After a delivered take, press Ctrl+V somewhere else. | Pastes the dictated text (safety net). | manual | published | — |
| I4 | Dictate into a window that refuses input (e.g. an elevated admin app while Sotto is not elevated). | Error toast with ↻. Home card: "Couldn't paste into that app". The text is on the clipboard. | manual | published | — |
| I5 | Set `injection_mode = "unicode"` in config.toml, restart, dictate text with an emoji and Arabic. | Typed character by character, emoji intact, no stuck repeated letters. | manual | published | — |
| I6 | Dictate Arabic / code-switched text into Notepad and VS Code. | The right script and order in the target app. | manual (Phase B, #7) | installed | — |

State changes:
- (none yet)

## 5. Overlay pill

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| O1 | Run D1 on a dark wallpaper and on a light one. | The pill is readable on both. Listening lilac bars, Transcribing amber dots, Polishing gold shimmer, Done checkmark, then fade. | manual | published | — |
| O2 | Trigger an error (e.g. I4). | Blush toast "Didn't catch that" with ↻ and a 6 s countdown, then fades. | manual | published | — |
| O3 | ✕ while Listening / Transcribing / Polishing. | Cancels (see D11/D12). | manual | published | — |
| O4 | A take where a phonetic correction fires (P17). | After Done: "✳ heard → corrected (+N)" for about 3 s. Not clickable. | manual | installed | — |
| O5 | Settings → Pill position: try all 9 anchors (with always-show on). | The pill sits against that edge/corner of the work area, 8 px in. Toasts grow inward. | `tests::anchor_xy_top_left`, `tests::anchor_xy_top_center`, `tests::anchor_xy_top_right`, `tests::anchor_xy_middle_left`, `tests::anchor_xy_middle_center`, `tests::anchor_xy_middle_right`, `tests::anchor_xy_bottom_left`, `tests::anchor_xy_bottom_center`, `tests::anchor_xy_bottom_right`, `tests::anchor_xy_unknown_falls_back_to_bottom_center`, `tests::anchor_xy_respects_monitor_origin`, `tests::tucked_pill_hugs_the_top_edge_not_the_canvas_centre`, `tests::tucked_pill_hugs_the_bottom_edge`, `tests::tucked_pill_hugs_left_and_right_edges`, `tests::centre_anchors_still_centre_on_that_axis`, `tests::expanded_pill_grows_inward_from_the_anchored_edge` + manual | installed | 2026-09-28 auto |
| O6 | Change the pill position while Sotto runs (no restart), then hover and click the idle pill, and ✕ during a take. | The pill is drawn at the new anchor; hover, click and ✕ all work. **❌ gap A-4**: the canvas keeps the old edge until restart, and the click targets are misaligned. | manual | installed | — |
| O7 | Always-show on, idle: hover the tucked pill. | It grows 46×16 → 62×20 without flicker. The mouse passes through anywhere outside the pill. | manual | installed | — |
| O8 | Auto-hide taskbar, bottom anchor: dictate, then move the mouse to the bottom edge so the taskbar slides in. | The pill lifts above the taskbar and eases back down when it hides. Done/flyout also clear it. | manual | installed | — |
| O9 | Click the always-show pill, then press Win+Up. Also Alt-Tab. | The pill never maximizes and never appears in Alt-Tab or the taskbar. | manual | installed | — |
| O10 | While dictating into Notepad, keep typing after the pill appears. | Focus stays in Notepad (the pill never takes focus). | manual | published | — |
| O11 | Start take B while take A is still transcribing. | The pill stays on Listening; A's Done doesn't flash over it. | manual | installed | — |

State changes:
- 2026-09-29 · O6 · ❌ gap A-4 **fixed by #50 (merged, not yet installed)**: the pill re-anchors
  live, with no restart, and the hit-test lines up with the new position. See 🆕 G25.

## 6. Tray + tray-menu window

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| T1 | Left-click the tray icon. | The app window opens and is focused. | manual | published | — |
| T2 | Right-click the tray icon, then click elsewhere. | The menu opens next to the cursor, fully visible, and hides on click-away. The status reads "Ready" (or "Paused"). | manual | published | — |
| T3 | Menu → Pause dictation, then reopen the menu. | The label flips to "Resume dictation" and the status to "Paused". See D4. | manual | published | — |
| T4 | Menu → Insights / Dictionary / History. | The window opens on that page. | manual | published | — |
| T5 | Menu → Settings. | The window opens with Settings showing. **❌ gap A-11**: it opens on Home. | manual | published | — |
| T6 | Menu → Retry last dictation, with and without a stashed take. | Enabled ("ready") only with a stash, and it delivers it. Otherwise greyed "none yet". | manual | published | — |
| T7 | Menu → Quit. | Sotto exits. `llama-server.exe` is gone too. | manual | published | — |
| T8 | Dictate and watch the tray icon. | The icon turns active (lilac mark) while listening and back to idle after. | `tray::tests::tile_has_transparent_corner_opaque_body_and_visible_mark` + manual | published | 2026-09-28 auto |
| T9 | Theme Dark (or System on a dark taskbar), then relaunch. | The dark tray icon from launch. **❌ gap A-15**: light at launch, and light for System. | manual | published | — |
| T10 | Switch the app theme, then open the menu. | The menu follows the theme. | manual | published | — |

State changes:
- 2026-09-29 · T5 · ❌ gap A-11 **fixed by #56 (merged, not yet installed)**: tray → Settings
  now opens the Settings modal. See 🆕 G26.
- 2026-09-29 · T9 · ❌ gap A-15 (tray-icon half) **fixed by #60 (merged)**: the tray tile
  follows the taskbar's theme from launch, including "system". See 🆕 G30. The other half of
  A-15 (D6, idle pill starting a take while paused) is also fixed by #60 — see 🆕 G29.
- 2026-09-29 · T3, T6 · extended by #60 (merged): a paused Sotto now also shows a muted chip on
  Home and a muted tray dot/tooltip, and a paused pill-click/trainer no longer silently tries
  to start. See 🆕 G29.

## 7. App pages

### Home
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| H1 | Open the app morning, afternoon and evening. | "Good morning." / "Good afternoon." / "Good evening." | manual | published | 2026-09-28 mock |
| H2 | Look at the three stat cards after dictating today. | The numbers match their labels. **❌ gap A-13**: "words today" shows this week's words. | manual | published | — |
| H3 | Toggle mode, then look at the Home hint. | Says "Tap <key> …". **❌ gap A-13**: always "Hold <key> and speak." | manual | published | — |
| H4 | Recent list: ⧉ and ↻ on a row. | ⧉ copies the text. ↻ re-polishes with the current tier onto the clipboard and flashes a Done pill. | manual | published | — |
| H5 | The status bar with each engine / cleanup / mic. | "<engine> · AI polish on / rules polish / ⚠ polish off · mic: <name or system default>". | manual | installed | — |
| H6 | Alert card after a cancelled take. | See D14. | manual | published | — |

### Insights
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| N1 | Dictate several takes over days, then open Insights. | Speed gauge = 30-day average WPM (takes of 10+ words), personal best, total words, top 5 apps over 30 days with %, current and longest streak. | `stats::tests::aggregate_totals_streaks_and_wpm`, `stats::tests::streak_tolerates_no_dictation_yet_today`, `stats::tests::days_from_civil_matches_known_dates` + manual | published | 2026-09-28 auto |
| N2 | Streak calendar. | 19 week-columns Sun→Sat, today in the last column on its weekday, future days blank, hover title "YYYY-MM-DD · N words". | manual | published | 2026-09-28 mock |
| N3 | Week / Month toggle. | The period row shows that period's words **and** says "this week" / "this month". **❌ gap A-13**: the label stays "this week". | manual | published | 2026-09-28 mock |
| N4 | "≈ N min saved vs. typing". | Uses the backend's per-take time saved (typing at 40 wpm minus speaking time). **❌ gap A-13**: the JS uses words × 0.02. | manual | published | — |

### Dictionary
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| DI1 | + Add word → type phrase and replacement → ✓. Restart Sotto. | The entry persists and fires (P12). | manual | published | — |
| DI2 | Edit an entry, add two aliases (+ Add another way to say it), save. | The chips show up to 2, then "+N more". The aliases fire (P13). | manual | installed | — |
| DI3 | Per-entry switch off; master "All replacements" off. | The row dims / the whole list dims. Behaviour as P14. Snippets' master switch mirrors it. | manual | installed | 2026-09-28 mock |
| DI4 | Search box. | Filters by phrase or replacement. | manual | published | 2026-09-28 mock |
| DI5 | Click an example chip, then ✕ (cancel) without saving. | Nothing is saved. | manual | installed | — |
| DI6 | Click an example chip (leave it open), then toggle any other entry. | The open draft is **not** saved. **❌ gap A-12**: it is sent in the same save. | manual | installed | 2026-09-28 mock |
| DI7 | Close the "Sotto spells the way you do" banner, reload. | It stays closed. | manual | installed | 2026-09-28 mock |
| DI8 | An entry whose phrase or replacement is Arabic (invented). | Renders in the right reading order. **❌ gap A-6**: no `dir="auto"`, LTR order. | manual | installed | 2026-09-28 mock |

Home/Insights/Dictionary state changes:
- 2026-09-29 · H2, H3, N3, N4 · ❌ gap A-13 **fixed by #58 (merged, not yet installed)**. See
  🆕 G28.
- 2026-09-29 · DI6 · ❌ gap A-12 **fixed by #57 (merged)**. See 🆕 G27 (applies to S2 too).
- 2026-09-29 · DI8 · ❌ gap A-6 **fixed by #42 (merged)**: `dir="auto"` now covers Dictionary's
  spoken/replacement cells, alias chips, and edit inputs. See 🆕 G22.

### Snippets
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| S1 | + Add snippet → trigger + expansion → ✓, restart. | Persists. Appears on Snippets, not Dictionary, even when short. | `config::entry_kind_tests::short_snippet_is_the_bug_this_fixes` + manual | installed | 2026-09-28 auto |
| S2 | Aliases, toggles, search, example chips, banner close. | Same as DI2–DI7 on this page (DI6's gap applies here too). | manual | installed | 2026-09-28 mock |

### History
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| HI1 | Dictate 25 takes. | Newest first, capped at 20, times like "2:14 PM". | `history::tests::newest_first_and_capped`, `history::tests::clock_label_is_12_hour_with_am_pm` | published | 2026-09-28 auto |
| HI2 | Click a row, ⧉, and ↻. | Click and ⧉ copy. ↻ re-polishes to the clipboard with a Done pill. | manual | published | — |
| HI3 | ⚑ Flag a row. | The icon shows flagged. One line appended to `%APPDATA%\sotto\bug-reports.jsonl` with engine, language, polish mode, counts and version. Nothing sent anywhere. | `bug_reports::tests::record_writes_a_readable_jsonl_line`, `bug_reports::tests::multiple_flags_append_rather_than_overwrite` + manual | installed | 2026-09-28 auto |
| HI4 | Quit and relaunch. | History is empty (in memory only, as the footnote says). #9 changes this. | manual | published | — |
| HI5 | A code-switched Arabic take in History and Home Recent. | The right reading order. **❌ gap A-6** (measured: LTR order). | manual | installed | 2026-09-28 mock |

State changes:
- 2026-09-28 · HI4 · master `10e1251` (#9) adds an opt-in "Keep history" (off by default). HI4
  still holds with it off. Rows for Keep history, Clear history and "Recovered after a
  restart" takes are still to be added.
- 2026-09-29 · HI5 · ❌ gap A-6 **fixed by #42 (merged, not yet installed)**: History rows now
  resolve `dir="auto"` per row. See 🆕 G22. HI3 also gains a `raw` field alongside `text` in
  `bug-reports.jsonl` (#8) — see P24, 🆕 G36.

### Pronunciation
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| PR1 | Type a word, **Listen for it**, say it, **Stop**. | The focus view shows the big word reacting to mic level. A sample row "Heard: …" appears with ✓ (match) or "Add correction". The page returns to idle. | manual | installed | 2026-09-28 mock (listening state only) |
| PR2 | A missed sample → **Add correction**. | " — added". The trained-words list shows "heard as: …". A matching Word dictionary entry is added once (no duplicates on retry). | manual | installed | — |
| PR3 | Five attempts at one word, mixing hits and misses. | The ring shows hits/total over the last 5 (a 6th drops the oldest). | `config::pronunciation_tests::first_attempt_creates_the_entry`, `config::pronunciation_tests::matches_the_existing_entry_case_insensitively`, `config::pronunciation_tests::caps_the_window_dropping_the_oldest` | installed | 2026-09-28 auto |
| PR4 | Glow on real detection. | See 🆕 F8. | — | installed | — |
| PR5 | Re-listen quickly on the same word right after a take. | The glow waits for the new take. Stale peek: #39. | manual | installed | — |
| PR6 | Leave the page mid-take, or after a take, and dictate normally. | Normal dictation. **❌ gap A-1** (see 🆕 F9). | manual | installed | — |

Pronunciation state changes:
- 2026-09-29 · PR5, PR6 · ❌ gap A-1 and the #39 stale-peek issue PR5 names are both **fixed by
  #39, #44, #46, #47, #71 (merged, not yet installed)**: a trainer take can no longer swallow
  the next real dictation, and a stale peek can no longer ignite the wrong take's glow. See
  🆕 G5, G6, G7. Re-verify PR5/PR6 (and 🆕 F9) once these are installed.
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| SE1 | Change every control, restart Sotto, reopen Settings. The controls: Mode, Microphone, Cleanup, Use-AI-past slider, Default tone (+ Custom), per-app tones, Language, the three dictation toggles, Quote style, Always show the pill, Theme, Pill position, Zoom, Sounds, Launch at login, Start hidden, Usage stats, Keep recordings. | Every value survives the restart. Each one took effect at once without a restart (except Speech model and Language, which show the restart note). | manual | published (newer toggles: installed) | — |
| SE2 | Escape with Settings open, and with the hotkey picker open. | Closes the topmost layer only. | manual | installed | 2026-09-28 mock |
| SE3 | Zoom: −/+/Reset, and Ctrl +/−/0 (incl. numpad). | Steps along 50%…200%. Layout stays intact at 760×520. Persists across restart. | manual | published | — |
| SE4 | Theme Light / Dark / System (then flip the Windows theme). | The app, overlay and menu follow. System tracks the OS live. | manual | published | — |
| SE5 | Settings → Check for updates (offline, then online). | Offline: "You're on the latest version" within ~8 s, no hang. Online with a newer release: the banner appears. | manual | published | — |

Settings state changes:
- 2026-09-29 · SE1 · **superseded by #10/#87 (merged, not yet installed)**: "except Speech
  model and Language, which show the restart note" is no longer true — both apply on the next
  take, no restart, no note. See 🆕 G8.

### Data & privacy
| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| DP1 | Usage stats **off**, dictate, then check `stats.jsonl`. | No new line. Back on: lines resume. The switch shows the real state on open. | manual | installed (toggle fix) / published | — |
| DP2 | Clear stats → confirm. | `stats.jsonl` removed. Insights and Home refresh to zero. | manual | published | — |
| DP3 | Data folder → Open folder. With `assets_dir` set elsewhere, also Models folder. | Explorer opens `%APPDATA%\sotto`. The Models folder row appears only when relocated and opens it. | manual | published | — |
| DP4 | Keep recordings **on**, dictate 3 takes, open the Recordings folder. | 3 μ-law `.wav` files that play in any player, plus 3 `index.jsonl` lines (raw + polished + app + engine + tier). The size line updates on reopen. | `recordings::tests::wav_header_is_mulaw_and_correctly_sized`, `recordings::tests::mulaw_round_trip_stays_within_companding_tolerance` + manual | installed | 2026-09-28 auto |
| DP5 | Set `[retention] max_mb = 1`, dictate until over the cap. | The oldest recordings are deleted first, and the index is rewritten to match. | `recordings::tests::enforce_cap_evicts_oldest_first_and_rewrites_the_index`, `config::dir_tests::retention_defaults_off_when_the_whole_section_is_missing`, `config::dir_tests::retention_round_trips_through_toml` | installed | 2026-09-28 auto |
| DP6 | Clear recordings → confirm. | The folder is removed. The size shows 0 MB. | manual | installed | — |
| DP7 | Fresh install. | Keep recordings is **off** by default. | `config::dir_tests::retention_defaults_off_when_the_whole_section_is_missing` | installed | 2026-09-28 auto |

State changes:
- (none yet)

## 8. Stats, logs, privacy

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| R1 | Dictate, then open `stats.jsonl`. | Counts, timings, app and tier only. No text. | manual | published | — |
| R2 | Open `%APPDATA%\sotto\logs\sotto.log` after a launch and a dictation. | Version, dirs, `listening id=`, `transcribe_ms`, the delivery line, no ERROR. **❌ gap A-2**: it also holds the full dictionary (snippets included) and each take's text, while the README says it never holds dictated text. | manual | published | — |
| R3 | Relaunch Sotto. | The previous run moves to `sotto.log.1`; a new `sotto.log` starts. | manual | published | — |
| R4 | Watch the network (Resource Monitor) with the models present, while dictating in AI mode. | Only loopback `127.0.0.1:8177`, plus the GitHub update check at launch. | manual | published | — |

State changes:
- 2026-09-29 · R2 · ❌ gap A-2 **fixed by #48, #70 (merged, not yet installed)**: `sotto.log`
  no longer holds dictionary/vocabulary/snippet text or full transcripts at `info`; those move
  to `debug` only, and even then only as part of the transcript trail, never the dictionary. See
  🆕 G21.

## 9. First-run assets, updater, installer, startup, single instance

| ID | Steps | Expected | Covered by | State | Last verified |
|---|---|---|---|---|---|
| A1 | Launch with an empty temp `SOTTO_DATA_DIR`. | Downloads onnxruntime, the configured speech model, Qwen and the llama runtime. Log `asset check … present=false` lines, then `all assets provisioned`. Only the configured engine's model is fetched. | `assets::tests::manifest_is_well_formed`, `assets::tests::markers_match_config_consumers` + manual | published | 2026-09-28 auto |
| A2 | First run as a new user (default config). | The download progress is visible without hunting. **❌ gap A-9**: the window starts hidden and the progress banner is inside the closed Settings modal. | manual | published | — |
| A3 | After A1 finishes (same session), switch Cleanup to AI and dictate 25 words. | AI polish runs. **❌ gap A-9**: it stays on rules until restart, while the status bar says "AI polish on". | manual | published | — |
| A4 | Cut the network mid-download of the 1 GB model, restore it, click Download / relaunch. | Resumes from the partial file. **❌ gap A-7**: the `.part` is deleted and the download restarts at 0. | `assets::tests::resume_appends_on_matching_206`, `assets::tests::resume_restarts_on_200`, `assets::tests::resume_restarts_on_content_range_start_mismatch`, `assets::tests::resume_restarts_when_total_shrank_below_part_len`, `assets::tests::resume_restarts_on_unparseable_content_range`, `assets::tests::resume_restarts_on_missing_content_range_for_206`, `assets::tests::fresh_download_when_no_part_exists`, `assets::tests::parses_content_range` (the decision only; the caller never reaches it) | installed | 2026-09-28 auto |
| A5 | A truncated download (the connection drops without an error). | Not renamed to the final name. The next run retries. | manual | installed | — |
| A6 | `curl -sI` every manifest URL (docs/updating.md §3.5). | All 200. **Known:** `assets-v2/ggml-egyptian-codeswitch-small.bin` is 404 (#4). | manual | — | — |
| A7 | Set `assets_dir = 'E:\models'` (any other drive) in config.toml, move the models, restart. | Models load from there; config and stats stay in `%APPDATA%\sotto`. Settings shows both folders. | `config::dir_tests::assets_dir_is_configurable_and_trimmed`, `config::dir_tests::config_lives_in_data_dir_not_assets_dir`, `config::dir_tests::default_does_not_pin_assets_to_a_drive`, `config::dir_tests::no_drive_letter_is_hardcoded_in_path_resolution` + manual | published | 2026-09-28 auto |
| A8 | Install an older version, launch with a newer release published. | The update banner (Settings) shows "Sotto vX is available". Install & restart downloads with % progress, verifies the signature and relaunches. Models untouched. On failure, a manual GitHub link. | manual | published | — |
| A9 | `Sotto_x_x64-setup.exe /S` over an existing install. | Installs silently to the user dir, and launches fine. | manual | published | — |
| A10 | Uninstall while Sotto and llama-server run. Answer **No** to the data prompt. | Both processes are killed, the app is removed, `%APPDATA%\sotto` and the models are kept, and the Run key is gone. | manual | published | — |
| A11 | Uninstall and answer **Yes**. | `%APPDATA%\sotto` and the configured `assets_dir` are removed, nothing else. **❌ gap A-8**: it deletes a hardcoded `D:\sotto` and misses any other `assets_dir`. Only on a throwaway user/VM. | manual | published | — |
| A12 | Launch at login on, sign out and back in. Then off. | On: Sotto starts (hidden if Start hidden is on), and `reg query HKCU\…\Run /v Sotto` exists. Off: the value is gone. | manual | published | — |
| A13 | Start hidden on/off, then relaunch. | On: tray only. Off: the window shows at launch. | manual | published | — |
| A14 | Launch Sotto a second time while it runs. | The running window comes to the front. **❌ gap A-16**: the second copy just exits (log "another Sotto instance is already running"), with no window. | manual | published | — |
| A15 | While Sotto runs: `sotto.exe --transcribe x.wav` and `--polish "…"` with a temp `SOTTO_DATA_DIR`. | Both run and exit. The running copy is unaffected. | manual | published | — |

State changes:
- 2026-09-29 · A4 · ❌ gap A-7 **fixed by #52 (merged, not yet installed)**: the cleanup loop
  that deleted every `.part` before provisioning is gone, so resume now actually runs. See
  🆕 G32.
- 2026-09-29 · A11 · ❌ gap A-8 **fixed by #53 (merged)**, in both directions: a real
  `D:\sotto`/example `assets_dir` is no longer force-deleted, and a real custom `assets_dir`
  elsewhere is now correctly removed, gated by the four safety checks. See 🆕 G34, G35 and the
  scripted `scripts/check-uninstall-delete.sh`.
- 2026-09-29 · A14 · ❌ gap A-16 **fixed by #61 (merged)**: a second launch now brings the
  running window forward instead of silently exiting. See 🆕 G31.
- 2026-09-29 · A2, A3 · ❌ gap A-9 is unchanged by this batch (still open, no fixing PR in
  #46–#88). Not to be confused with #10/#87's restart removal (🆕 G8), which is about the
  Speech-model/Language controls, not first-run visibility.
- 2026-09-30 · A6 · "Known: `assets-v2/ggml-egyptian-codeswitch-small.bin` is 404 (#4)" no
  longer holds: all six manifest URLs return 200 (release gate before and after the v0.6.1
  publish; see `docs/updating.md` §0). A6's "Last verified" is unchanged.

## 10. M-track (Android)

Planned only (`docs/roadmap.md`), no code. Rows are added when M1 is picked.
