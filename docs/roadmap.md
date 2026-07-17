# Sotto roadmap — planned tracks (not yet started)

Written 2026-07-17, after v0.1.10 shipped. Priorities set by Khairy:
**Egyptian Arabic first**, then non-fluent English, tones, speed/grammar.

The four requests turn out to be **two and a half tracks**, because one
decision serves three of them:

> **Parakeet v3 is English-only.** Arabic, Egyptian dialect, and
> accented/non-fluent English all need a different speech model. The crate we
> already use (`transcribe-rs`) also supports **Whisper** — GGML models, 99
> languages including Arabic, per-request language selection — behind a
> `whisper-cpp` cargo feature. One engine addition unlocks all three, and
> whisper.cpp's Vulkan backend may cover the speed complaint too, on both
> NVIDIA and AMD GPUs, without shipping CUDA.

---

## Track A — Second ASR engine: Whisper (Arabic · Egyptian · accents)

**Why Whisper and not an "Arabic model":** dialectal Egyptian speech is
heavily code-switched (Arabic + English brand names, tech words, numbers).
Whisper is trained on exactly that mix; single-language Arabic models mostly
aren't. It also handles accented English far better than Parakeet — so the
"friends who don't speak fluent English" problem is the same fix, not a
separate one.

### A1 — Engine plumbing (the enabler)

- `Cargo.toml`: add `whisper-cpp` to the `transcribe-rs` features.
- `src/asr.rs`: `enum Engine { Parakeet, Whisper }` chosen from config;
  `transcribe()` passes the configured language (`auto` / `en` / `ar` / …).
- `config.toml`: `[asr] model = "parakeet-v3"` (default, unchanged) and
  `language = "auto"`.
- `assets.rs`: per-model manifest — **only the selected model downloads**.
  Parakeet users see zero change; picking Whisper downloads one GGML file.
- Model: stock `ggml-large-v3-turbo` quantized (q5_0 ≈ 574 MB / q8_0 ≈ 874 MB)
  from the whisper.cpp model repo, rehosted in our `assets-v2` release.

### A2 — Settings UI

- The Models card (currently hardcoded to one entry) becomes a real picker:
  Parakeet v3 (English, fastest) / Whisper turbo (99 languages).
- Language dropdown: Auto-detect, English, Arabic (عربي), + the common ones.
  Auto is the default and handles code-switching by itself.

### A3 — GPU spike (decides the speed story)

- Build the whisper feature with **Vulkan** enabled; measure large-v3-turbo
  on the RTX 3050 and the Radeon 660M.
- Exit criteria: dictation-to-text in ≤ ~1.5× real-time on GPU → ship it and
  **Track D (DirectML) is cancelled**. If Vulkan disappoints, fall back to
  Track D for Parakeet and keep Whisper CPU-only for multilingual.
- One GPU path in the codebase, not two.

### A4 — Egyptian dialect pass (the actual goal)

Stock large-v3-turbo already does passable Egyptian; measure before adding
anything. Candidates if it's not good enough, both verified on HF:

| Model | Base | License | Form | Note |
| :--- | :--- | :--- | :--- | :--- |
| `MAdel121/whisper-small-egyptian-arabic` | whisper-small | MIT | full fine-tune | 242M params — small, may lose to stock turbo overall |
| `AbdelrahmanHassan/whisper-large-v3-egyptian-arabic` | whisper-large-v3 | Apache-2.0 | LoRA | 11.6K downloads, trained on MGB-3 (Egyptian broadcast) |

Both need a one-time conversion to GGML (whisper.cpp's convert script; the
LoRA needs merging first). Plan: convert both, and **Khairy A/B-tests them
against stock turbo by dictating real Egyptian speech** — he's the native
judge. Winner goes into `assets-v2` as "Arabic (Egyptian) — tuned".

### Arabic side-effects to handle in the same track

- **RTL:** history rows, recent list, dictionary values get `dir="auto"`
  (CSS/HTML only, no logic).
- **Injection:** paste path is already Unicode via the clipboard — verify
  with Arabic text early, it should just work.
- **Polish tiers:** the filler list ("um", "uh") and Harper are English-only —
  rules tier for Arabic = dictionary replacements only. The AI tier's prompt
  gets one added line: *reply in the language of the input* (Qwen2.5 handles
  Arabic).

---

## Track B — Tones, general + per-app (Wispr Flow logic)

Flow's model: a default tone, overridable per application — Slack casual,
email professional. **Most of the machinery already exists in our codebase:**
the focused window is captured at dictation start (`Take.focus_target`), and
`stats::app_name()` already turns that into an exe name for Insights.

- **Config:** `tone = ""` (off) and `[[app_tones]] app / tone` pairs.
- **Engine:** in `polish_ai()`, look up `app_tones` by the take's app name,
  fall back to the default tone, append one sentence to the system prompt
  ("Rewrite in a professional, polished tone", etc.). ~15 prompt tokens;
  negligible latency next to the measured 1.7 s AI round-trip.
- **Presets, not framework:** Professional / Friendly / Concise / Casual are
  just strings in the UI; config stores the instruction text itself. A
  "Custom…" option is the same field, free-typed.
- **UI:** a Tone section in Settings — default-tone picker plus per-app rows.
  The app dropdown is populated from apps you've actually dictated into
  (already in stats.jsonl), with free-text as fallback.
- **Honest constraint, shown in the UI:** tone requires the **AI** polish
  tier. Rules mode can strip fillers but cannot re-voice a sentence. If polish
  is Off/Rules, tone settings show as disabled with one line saying why.

Ships independently of Track A. Small, high-visibility.

---

## Track C — Harper: real grammar in the Rules tier

`harper-core` 2.5.0 (Apache/MIT, pure Rust, offline, ~ms latency, verified
maintained). Slots into `tier0()` in `polish.rs` after filler stripping:
lint → apply safe corrections. English-only, which is fine — that's exactly
the tier Arabic bypasses anyway. One dependency, a few dozen lines, one test.

This upgrades the *instant* tier for everyone who finds 1.7 s AI polish too
slow for short phrases — and pairs with Track B's constraint story ("Rules is
instant and now grammatical; AI adds tone").

---

## Track D — DirectML for Parakeet *(conditional — do not start)*

Only runs if the A3 Vulkan spike fails. `transcribe-rs` exposes
`ort-directml`; needs the DirectML `onnxruntime.dll` (~30 MB) swapped into
assets and `OrtAccelerator::DirectMl` at session build. Kept as the fallback
so we never maintain two GPU paths at once.

---

## Suggested order

1. **A1 + A2** — engine + picker + language (unlocks everything else)
2. **A3 spike** — decides speed story, kills or confirms D
3. **A4** — Egyptian A/B with Khairy as judge  ← *the stated top priority*
4. **B** — tones (parallel-friendly; independent of A)
5. **C** — Harper (anytime, smallest)

Explicitly **not** planned: streaming/live transcription, cloud fallbacks,
speaker diarization, translation. None requested; all expensive.
