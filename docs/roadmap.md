# Sotto roadmap

Originally written 2026-07-17 as a plan; rewritten 2026-07-18 as a status
report + the next round of thorough plans, after Tracks A–C shipped and D was
overtaken by events. Read bottom-up if you only want what's left to build —
the "Suggested order" section at the end is the live todo list.

---

## Shipped — Tracks A, B, C (2026-07-17/18)

### Track A — Second ASR engine: Whisper (+ a third: Egyptian)

Landed as three commits: `v0.2.0` (engine + Egyptian model conversion),
`271c480`/`d653fdc`/`ce20702` (the AVX2 → Vulkan speed hunt), `d0c26f3`
(Arabic-aware polish). Full detail lives in those commit messages, not
repeated here. Summary of what exists today:

- **Three ASR engines**, switchable in Settings, chosen at `asr::Asr::new()`
  via `config::AsrConfig { model, language }`:
  - `parakeet-v3` — English-only, ONNX, fastest. **Still the default.**
  - `whisper-turbo` — Whisper large-v3-turbo, 99 languages, general.
  - `egyptian-small` — Whisper-small fine-tuned on Egyptian Arabic +
    code-switching (`IbrahimAmin/code-switched-egyptian-arabic-whisper-small`,
    converted to GGML). **Not yet uploaded to the release** — built and wired
    locally only, Khairy holding off on publishing.
- **Vulkan GPU backend** for both Whisper engines, via `whisper-vulkan`
  feature + a custom CMake toolchain file (`scripts/msvc-avx2-toolchain.cmake`)
  that fixes three separate invisible slowdowns cmake-rs/ggml's MSVC path hits
  (optimization silently disabled, SIMD silently disabled, flash_attn
  defaulting to a 6.5x-slower path on this Vulkan backend). Net: 237s → 3.4s on
  a 1.5s clip in the probe harness; ~12.3s in the shipped app on 7.5s of real
  speech (app-vs-probe gap not yet closed — see Track G below).
- **Arabic-aware AI polish**: the Qwen system prompt is now language-neutral
  (keep exact words + language, never translate, punctuate for the right
  script), and the token cap is `words*4` not `*3` so code-switched output
  doesn't truncate.
- **Verified on real dictation, not just synthesized test clips**: Khairy
  dictated live Egyptian Arabic + English code-switching through the
  `egyptian-small` engine. English words came out correctly in Latin script
  inline with Arabic — the thing `whisper-turbo` could not do (it transliterated
  English into Arabic script instead). Remaining gaps are punctuation and some
  dialect spelling, not code-switching itself.

**RTL and injection**: never needed dedicated work — the paste-based injection
path was already Unicode-clean, and no RTL-specific CSS was needed for the
overlay/history (deferred, not found necessary yet; revisit if Arabic history
rows ever look wrong).

### Track B — Tones (Wispr Flow logic)

Shipped in `v0.3.0`. Default tone + per-app overrides, resolved from
`stats::app_name()` at the take's focus target, appended as one clause to the
AI-polish system prompt. Inert outside the AI tier by construction (`tone` is
never read on the `Off`/`Rules` branches of `polish_with_tone`) — the UI greys
out the section and says why rather than silently no-op'ing.

### Track C — Harper grammar in the Rules tier

Shipped alongside Whisper in the same round. `harper-core` 2.5.0, filtered to
mechanical `LintKind`s (Capitalization/Punctuation/Repetition/Spelling/
Typo/BoundaryError) with a hard single-suggestion gate — never guesses among
multiple spelling candidates. Measured 0.5ms warm per call; the one real cost
is binary size (+7MB, `burn` comes along for a POS tagger) and a ~640ms
one-time lint-set build, now paid at worker startup instead of on first
dictation.

## Dead — Track D (DirectML)

Was the fallback if the Vulkan spike failed. It didn't fail — Vulkan runs on
**both** the RTX 3050 and the Radeon 660M with no CUDA dependency, which
DirectML can't offer (NVIDIA-only path historically flakier on AMD). Track D
is formally cancelled; don't resurrect it unless Vulkan itself becomes a
problem on some future user's hardware.

---

## Closed by research — not pursued

### Can we use whatever YouTube uses for Egyptian captions?

Khairy's idea, checked properly (2026-07-18) rather than answered from
memory — this project already got burned once trusting an unverified
model-card WER number, so this got a skeptical pass with real
cross-verification against an independent multi-dialect benchmark
(`elmresearchcenter/open_universal_arabic_asr_leaderboard`, SADA dataset,
arXiv 2412.13788).

**No path exists, for three independent reasons:**
1. YouTube's captioning tech (Google's USM/Chirp lineage) has no open weights
   and no free tier — Google Cloud Speech-to-Text is a paid, internet-required
   API. Closed regardless of quality.
2. The two most-discussed open multilingual alternatives — **Meta MMS** and
   **SeamlessM4T v2** — are both **CC-BY-NC** (non-commercial), which blocks
   redistribution in a free public app outright, independent of quality.
3. They also fail architecturally: neither is Whisper-family, so neither
   converts to GGML/whisper.cpp — adopting either means building an entirely
   separate PyTorch/fairseq2 inference stack, and on the one independent
   benchmark that actually breaks results out by Egyptian dialect, **both
   score worse than plain Whisper-large-v3 anyway** (73–82% WER vs 59%).

The one model that measurably beats Whisper on Egyptian (NVIDIA's Arabic
FastConformer, ~41% WER) has the same architectural wall — NeMo/CUDA, no GGML
path. Adopting it would mean a second inference engine, not a model swap.

**Verdict:** our current `egyptian-small` fine-tune is realistically near the
ceiling of what's swappable into this pipeline today. The only lever left
in-family is sizing up (a large-v3 Egyptian LoRA instead of small) for a
latency/VRAM cost — folded into Track J below, not a separate effort.

**Worth remembering when this comes up again:** "match YouTube" isn't a fair
bar for a live tool regardless of model — YouTube captions are non-causal
(whole video available, server-scale compute, often refined after the fact);
Sotto is causal streaming ASR on a laptop iGPU with zero future context. Even
Google's own live products (Meet captions) look worse than YouTube's batch
captions for exactly that reason.

### Accented / non-fluent English — researched, no action taken

Checked 2026-07-18 with the same rigor bar as the YouTube research. The gap
itself is real and large — confirmed across four independent, peer-reviewed
sources, not one:

| Source | What it measured | Verified degradation |
| :--- | :--- | :--- |
| EdAcc (arXiv:2303.18110) | Whisper-large, 40+ accents, 51 L1s | 19.7% WER vs 2.7% on LibriSpeech clean — ~7x |
| AfriSpeech-200 (TACL 2024, arXiv:2310.00274) | Whisper-large/medium, 120 African accents | 30.6–37.5% WER vs Whisper's usual 2–4% — ~10x |
| CORAAL re-analysis (arXiv:2407.13982) | Whisper on African American English | 13.7–33.2% WER, avg 34.35% |
| Koenecke et al. 2020 (PNAS, 617 citations) | 5 commercial ASR systems, matched speakers | 0.35 WER (Black speakers) vs 0.19 (white) — pre-Whisper, establishes this as ASR-wide, not Whisper-specific |

**But no candidate fine-tune clears the bar `egyptian-small` was held to.** Two
turned up on HF; both fail on independent grounds:
- `intronhealth/afrispeech-whisper-medium-all` — real, peer-reviewed
  improvement (33–36% → 22–24% WER), but scoped to African accents only, and
  its training data (`afrispeech-200`) is **CC-BY-NC-SA** — the exact
  non-commercial licensing wall that killed MMS/SeamlessM4T in the YouTube
  research above.
- `Tejveer12/Indian-Accent-English-Whisper-Finetuned` — MIT-licensed, same
  `large-v3-turbo` base we already ship, but **unverified**: a single-author
  self-reported number with no baseline comparison, 4 likes, no peer review.

**And there's a structural reason not to chase this even if a clean candidate
existed**: "accented English" isn't one target, it's dozens of distinct,
sometimes-conflicting ones (Indian, Nigerian, Egyptian-accented, Jamaican,
Scottish...). The CORAAL paper states plainly that fine-tuned ASR models
don't generalize outside their fine-tuning data — a narrow accent fine-tune
trades a broad, even limitation for a narrower, riskier one, with no
guaranteed win and a documented regression risk elsewhere. Independent
evidence (full-size vs distilled Whisper on identical accented benchmarks)
shows the gap narrows with more/larger general pretraining — which is
exactly what `large-v3-turbo` already is, and exactly what a narrow fine-tune
doesn't replicate.

**Verdict: no action at the model layer.** We already ship the best-positioned
model on this axis. What a strong general model actually produces on accented
speech is mostly individual misrecognized words, not systemic failure — a
text-cleanup problem the AI-polish tier already exists for, not an
acoustic-model problem. If this ever gets revisited: the only legitimate path
is the `egyptian-small` pattern — one accent group, concrete user need, a
verified fine-tune, shipped as an optional engine, never a blanket default.

---

## Live backlog — thorough plans

Ordered by the priority already set in Akiflow (🔊 Sotto project).

### 1. Transforms — rewrite anywhere *(HIGH — next up)*

From Khairy's Wispr Flow screenshots: select text in any app, press a chord,
Sotto's local Qwen rewrites it in place. The biggest remaining felt feature —
it makes Sotto useful even when you're not actively dictating.

**Why this is mostly wiring, not new capability** — every piece already
exists in the codebase for a different purpose:

| Need | Already exists as |
| :--- | :--- |
| Global hotkey listening | `hotkey.rs` — rdev listener, `Input::Key`/`Input::Button` matching, already handles chord-like combos for the dictation hotkey |
| LLM rewrite | `llm.rs`'s `Llm::polish()` — just needs a different system prompt per Transform, not a new sidecar |
| Paste text into the focused app | `inject.rs`'s paste-based injection, already Unicode-clean |
| Clipboard safety | `arboard`, already a dependency |

**What's actually new:**

1. **Selection grab.** No existing code reads arbitrary selected text — dictation only ever *writes*. Plan: on the Transform chord, simulate Ctrl+C via the existing key-injection path, read the clipboard, then **restore the clipboard's prior contents** after the rewrite is injected (a Transform must not clobber whatever the user had copied before triggering it — this is the one correctness trap in an otherwise simple feature).
2. **A second chord listener**, or extend the existing one to recognize multiple bindings (dictation hotkey + N transform chords) without them fighting each other. Must not fire a Transform while a dictation is actively recording.
3. **Config model** — `[[transforms]] name / chord / prompt`, mirroring the existing `AppTone` pattern in `config.rs` exactly (small struct, `Vec` in `Config`, `#[serde(default)]`). Default seed: **Polish** ("tighten and clarify without changing meaning"), **Prompt engineer** ("restructure into a strong AI prompt"), **Create your own** (free-typed prompt + chord picker, reusing the existing hotkey-picker modal UI).
4. **New Settings page** — card grid, each card = chord chips + name + description, "Create new" opens the same modal pattern as the hotkey picker. Marshmallow-styled, matching the existing card/chip visual language from the intro banners.
5. **Windows shortcut collision**: Win+Alt combos collide with Windows 11 system shortcuts (already noted in the original Wispr-Flow extraction) — default new Transforms to **Ctrl+Alt**, not Win+Alt.

**Not planned in v1**: transforms running on dictated audio (voice-triggered transforms) — text-selection-triggered only, matching the original ask.

**Open question to resolve before starting:** does simulated Ctrl+C reliably work in every app the way paste-injection already does, or does it need the same per-app fallback logic `inject.rs` has for paste? Spike this first — it's the one piece with no existing analog to copy.

### 2. Root-cause the Whisper app-vs-probe ~3× speed gap *(MEDIUM)*

The app takes ~12.3s to transcribe 7.5s of audio; the standalone probe
harness (`examples/whisper_probe.rs`) does the identical file in ~4.1s with
what should be the same settings. `src/asr.rs` now logs `transcribe_ms` +
`audio_ms` per dictation (commit `271c480`) — the tool to actually measure
this exists; the investigation itself doesn't.

**Hypotheses to test, in order of suspicion:**

1. **Thread count.** The probe explicitly calls `set_n_threads(8)` (or 4).
   `src/asr.rs`'s `TranscribeOptions` doesn't set thread count at all — check
   whether transcribe-rs's `WhisperEngine::transcribe_raw` actually forwards a
   thread count from `TranscribeOptions`, or silently falls back to
   whisper.cpp's own default (`min(4, cores)`), which could be running on
   fewer threads than the probe's explicit setting. **Cheapest hypothesis to
   test — do this first.**
2. **`no_speech_thold`.** transcribe-rs may set a different default than
   whisper.cpp's own (0.2 vs whisper's 0.6) — a lower threshold makes the
   model more reluctant to skip segments as silence, which is more decode
   work, not less. Read transcribe-rs's `WhisperInferenceParams::default()`
   against what the probe explicitly passes.
3. **`no_context` forced true.** transcribe-rs forces this on every call
   (confirmed from the vendored source read during the engine work).
   whisper.cpp normally keeps `prompt_past` across chunks of the same
   utterance to speed continuous decoding — forcing a context reset every
   time may be doing extra work the probe's manual params don't.
4. **Padding path** — already ruled out. `SpeechModel::transcribe()`'s default
   leading/trailing silence for Whisper is 0/0, which hits the fast path
   straight into `transcribe_raw` with no buffer copy. Not the cause; don't
   re-investigate.

**Method:** reuse `whisper_probe.rs` — it's already a working A/B harness.
Change one parameter at a time (thread count first), re-measure on the same
`spoken.wav`, and compare against the app's `transcribe_ms` log line for the
same file. Whichever hypothesis closes the gap, apply the fix in `asr.rs`; if
none of them do, the gap may live inside transcribe-rs's own call path and is
worth a GitHub issue upstream rather than a local patch.

### 3. Download indicator on model switch *(MEDIUM)*

Picking an undownloaded engine in Settings currently gives no visible
progress — it can read as the app freezing. The download pipeline itself
already exists and works (`assets.rs`'s manifest + the existing
`#assets-banner`/`asset-progress`/`assets-ready` event flow used for first-run
downloads) — this is wiring an existing signal into a second trigger point,
not building a new downloader.

**Plan:** when `set_asr_model` picks a model whose file isn't present yet
(`config::asr_model_present()` already answers this per-engine), immediately
invoke the same `download_assets` path the first-run flow uses, and surface
the existing assets-banner rather than leaving the Settings row looking
inert. Disable the model row (or show a spinner state on it) while its own
download is in flight, and clear it on `assets-ready`. Handle the failure
case explicitly — a network hiccup mid-switch shouldn't leave the row stuck
in a permanent "downloading" state with no retry.

### 4. Quantize the Egyptian model *(LOW)*

Currently ships unquantized fp16 (487MB) — the earlier conversion agent
skipped quantization because cmake wasn't installed on this machine yet at
that point. It is now (Track A's toolchain work installed cmake + Ninja +
Vulkan SDK for the main build) — this is a same-toolchain follow-up, not new
infrastructure.

**Plan:** whisper.cpp ships a standalone `quantize` CLI target in its own
CMake project (separate from the vendored `whisper-rs-sys` build — clone
whisper.cpp fresh at the same `v1.8.3` tag used for the GGML conversion, the
way the conversion agent pulled `convert-h5-to-ggml.py`, and build just the
`quantize` target with the existing cmake+ninja+Vulkan setup). Run:

```
quantize ggml-egyptian-codeswitch-small.bin ggml-egyptian-codeswitch-small-q5_0.bin q5_0
```

Then A/B fp16 vs q5_0 on real Egyptian audio (reuse whatever test clips Track
5 below produces) for output quality, and measure size/load/decode with
`whisper_probe.exe`. Stock `large-v3-turbo` already ships at q5_0 in our
`assets-v2` release — matching that treatment for the Egyptian model roughly
halves its download size if quality holds up. Ship only if Khairy's ear
confirms no regression; this is a size/speed win, not worth trading dialect
accuracy for.

### 5. Rigorous Egyptian model A/B *(LOW)*

Khairy's live test clearly favored `egyptian-small` on code-switching — but
that was one dictation, not a systematic comparison, and we have no
repeatable Arabic test set (no Arabic TTS voice on this machine, so every
prior Arabic test has been Khairy speaking live and un-recorded).

**Plan:** ask Khairy to record ~10–15 short reusable clips once — mix of
plain numbers, code-switched sentences (the kind that broke `whisper-turbo`),
and a few dialect words known to be phonetically ambiguous (ثانية vs سانية
being the discovered example). Save as WAV files under something like
`docs/test-audio/egyptian/` (git-ignored or LFS if size matters — decide at
implementation time). Once these exist, comparing `egyptian-small` against
stock `whisper-turbo` and the two other candidates from the original
research (`MAdel121/whisper-small-egyptian-arabic` full fine-tune,
`dev-ahmedhany/whisper-large-v3-turbo-arabic-ft-lora`) becomes a fast, cheap,
repeatable check — run all candidates through `whisper_probe.exe` on the same
clips, present transcripts side by side, Khairy judges. This is also the test
set Track 4's quantization A/B should reuse rather than inventing a second
one.

**Not pursuing:** a formal WER metric. Without a ground-truth transcript
service for Egyptian dialect, manual side-by-side judging by a native speaker
is the honest measurement — which is exactly what's already been happening;
this just makes it repeatable instead of one-off.

---

## Suggested order

1. **Transforms** — the agreed next feature; start with the Ctrl+C-selection
   spike since it's the one part with no existing code to copy.
2. **Speed gap** (thread count first) — cheap to test, meaningfully improves
   every Whisper/Egyptian dictation if it pans out.
3. **Download indicator** — small, mostly wiring an existing signal.
4. **Record the Egyptian test clips** (Track 5) — unblocks both the
   quantization A/B (Track 4) and any future model candidate check cheaply.
5. **Quantize** — do after test clips exist, so the A/B has something real to
   judge against.
6. **Publish the Egyptian model** whenever Khairy says go — independent of the
   above; could happen at any point once he's satisfied with quality.

Explicitly **not** planned: streaming/live transcription, cloud fallbacks,
speaker diarization, translation, a formal WER pipeline. None requested, or
ruled out above as not worth the cost.
