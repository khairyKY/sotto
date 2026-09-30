//! Lecture mode (#23): long-form capture to a transcript file. While it's on,
//! the audio thread cuts the microphone into the same chunks a long dictation
//! is cut into (`next_chunk`), and each chunk's text is appended to
//! `data_dir()/lectures/<date>-<time>.md` as it lands, prefixed by [mm:ss].
//!
//! Nothing is typed, polished, or added to history or stats. A lecture is
//! someone else's speech, and the polish chain (snippets, voice commands,
//! filler removal) is built for the user's own: a line here is what the model
//! heard. The text is the user's all the same: never logged (#48).
//!
//! Every line is flushed to disk as it's written (as `journal.rs`), so after a
//! crash the file is simply what was captured. The audio itself is not kept.
//!
//! Behind `lecture_mode` in config.toml, off by default. Started and stopped
//! from the tray menu and Home, never by a key.

use crate::config;
use crate::hotkey::DictationEvent;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Audio kept behind the last cut. The next chunk re-hears the last second of
/// it (`seam_start`), and `has_speech` reads the rest as the room's recent
/// noise floor. Everything older is dropped: a dictation keeps its whole take
/// for Retry, a two-hour lecture can't (460 MB).
const KEEP: usize = 16_000 * 30;

/// Written where a chunk failed to transcribe, so the gap shows.
const GAP: &str = "(not transcribed)";

pub fn dir() -> PathBuf {
    config::data_dir().join("lectures")
}

// ── mode decisions ─────────────────────────────────────────────────────

/// What the audio thread does with an event that arrives mid-lecture.
#[derive(Debug, PartialEq)]
pub enum Gate {
    Pass,
    Refuse,
    Ignore,
}

/// Lecture capture and dictation don't mix: the microphone is the lecture's.
/// A dictation Start is refused, and the pill says why. The Stop that follows
/// it in hold mode, and an Escape, must not end the lecture: only its own
/// Stop does. Everything else (Retry, a Transform) never touches the recorder.
pub fn gate(lecture: bool, event: &DictationEvent) -> Gate {
    match event {
        _ if !lecture => Gate::Pass,
        DictationEvent::Start => Gate::Refuse,
        DictationEvent::Stop | DictationEvent::Cancel => Gate::Ignore,
        _ => Gate::Pass,
    }
}

/// Whether the transcribe thread may drop the idle speech model (#12). Not
/// mid-take, and not mid-lecture: a quiet stretch (a break, a video) sends no
/// chunks for longer than `idle_unload_secs`, and a reload would hold up the
/// first chunk after it.
pub fn may_unload(listening: bool, lecture: bool) -> bool {
    !listening && !lecture
}

// ── capture side (audio thread) ────────────────────────────────────────

/// One chunk of a lecture on its way to the model.
pub struct Piece {
    file: PathBuf,
    /// Seconds into the lecture its new audio starts: the line's [mm:ss].
    at: u64,
    samples: Vec<f32>,
    /// As `Work::Chunk`'s: the audio starts before the cut, so its text opens
    /// with words the chunk before already wrote.
    seam: bool,
}

/// A lecture being captured: where its transcript goes, and how much audio
/// has been dropped off the front of the audio thread's buffer.
pub struct Capture {
    file: PathBuf,
    dropped: usize,
}

impl Capture {
    /// A new lecture, named for its local start time. To the second, so two
    /// started in one minute get a file each.
    pub fn new() -> Self {
        // SAFETY: GetLocalTime just fills a caller-owned SYSTEMTIME.
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let name = format!("{:04}-{:02}-{:02}-{:02}{:02}{:02}.md", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
        Self { file: dir().join(name), dropped: 0 }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Called on every poll with the audio so far (`all`, of which `sent` has
    /// gone to the model): the next chunk if one is due, cut exactly where a
    /// dictation's would be. `None` while it's still growing, and for a chunk
    /// nobody spoke in, which is skipped rather than given to the model to put
    /// words to.
    pub fn step(&mut self, all: &mut Vec<f32>, sent: &mut usize) -> Option<Piece> {
        let (from, end, speech) = crate::next_chunk(all, *sent)?;
        let piece = speech.then(|| self.piece(all, from, *sent, end));
        let cut = end.saturating_sub(KEEP);
        all.drain(..cut);
        self.dropped += cut;
        *sent = end - cut;
        piece
    }

    /// What's left when capture stops: the audio after the last cut, if
    /// anyone spoke in it.
    pub fn tail(&self, all: &[f32], sent: usize) -> Option<Piece> {
        crate::has_speech(all, sent..all.len()).then(|| self.piece(all, crate::seam_start(all, sent), sent, all.len()))
    }

    fn piece(&self, all: &[f32], from: usize, sent: usize, end: usize) -> Piece {
        Piece {
            file: self.file.clone(),
            at: ((self.dropped + sent) / 16_000) as u64,
            samples: all[from..end].to_vec(),
            seam: from < sent,
        }
    }
}

// ── transcript side (transcribe thread) ────────────────────────────────

/// Transcribe one piece and append what it adds to its lecture's file.
/// `prev` is the text of the piece before, kept for the seam.
pub fn write(transcribe: impl FnOnce(&[f32]) -> anyhow::Result<String>, piece: Piece, prev: &mut String) {
    let heard = transcribe(&piece.samples);
    let text = match &heard {
        Ok(text) => fresh(prev, text, piece.seam),
        Err(err) => {
            tracing::warn!(?err, at = piece.at, "lecture chunk failed");
            GAP.to_string()
        }
    };
    *prev = heard.unwrap_or_default();
    if text.is_empty() {
        return;
    }
    if let Err(err) = append(&piece.file, piece.at, &text) {
        tracing::warn!(?err, "couldn't write the lecture transcript");
    }
}

/// What a chunk's `text` adds to the transcript after `prev`: at a seam, the
/// words after the ones its overlap re-heard (the same match `join_text`
/// splices on).
///
/// ponytail: `join_text` also mends the end of `prev` (a word the cut
/// clipped, a full stop mid-sentence). That line is already on disk, so here
/// it stays as written. Rewriting the file's last line is the upgrade.
fn fresh(prev: &str, text: &str, seam: bool) -> String {
    let (aw, bw): (Vec<&str>, Vec<&str>) = (prev.split_whitespace().collect(), text.split_whitespace().collect());
    let from = crate::seam_repeat(&aw, &bw).filter(|_| seam).map_or(0, |(_, j, k)| j + k);
    bw[from..].join(" ")
}

/// One transcript line, "[07:42] text". Minutes run past 59 ("[75:03]")
/// rather than rolling into hours. The blank line after it makes each chunk
/// its own markdown paragraph.
fn line(at: u64, text: &str) -> String {
    format!("[{:02}:{:02}] {text}\n\n", at / 60, at % 60)
}

/// Append one line and flush it to disk: a crash is the case this is written
/// for, so the text can't sit in the OS write cache. A new file gets a title.
fn append(file: &Path, at: u64, text: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let title = match file.exists() {
        true => String::new(),
        false => format!("# Lecture {}\n\n", file.file_stem().unwrap_or_default().to_string_lossy()),
    };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    f.write_all((title + &line(at, text)).as_bytes())?;
    f.sync_data()
}

/// `--lecture <wav>`: a clip through lecture mode's whole path, minus the
/// microphone. Fed in polls as the audio thread gets it, cut by `step` and
/// `tail`, written by `write`. Prints the transcript file, then its contents.
pub fn run_once(wav: &str) -> anyhow::Result<()> {
    let samples = transcribe_rs::audio::read_wav_samples(Path::new(wav))
        .map_err(|e| anyhow::anyhow!("failed to read {wav}: {e}"))?;
    let mut asr = crate::asr::Asr::new();
    let (mut capture, mut all, mut sent, mut prev) = (Capture::new(), Vec::new(), 0, String::new());
    for poll in samples.chunks(crate::CHUNK_POLL.as_millis() as usize * 16) {
        all.extend_from_slice(poll);
        if let Some(piece) = capture.step(&mut all, &mut sent) {
            write(|s| asr.transcribe(s), piece, &mut prev);
        }
    }
    if let Some(piece) = capture.tail(&all, sent) {
        write(|s| asr.transcribe(s), piece, &mut prev);
    }
    println!("{}", capture.file.display());
    print!("{}", std::fs::read_to_string(&capture.file).unwrap_or_default());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private temp file per test, never `data_dir()`.
    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sotto-lecture-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("lectures").join("2026-09-30-140512.md")
    }
    fn cleanup(file: &Path) {
        let _ = std::fs::remove_dir_all(file.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_dictation_start_is_refused_mid_lecture_and_nothing_but_its_own_stop_ends_it() {
        assert_eq!(gate(true, &DictationEvent::Start), Gate::Refuse, "the hotkey, the pill, the trainer");
        assert_eq!(gate(true, &DictationEvent::Stop), Gate::Ignore, "a hold-mode key release");
        assert_eq!(gate(true, &DictationEvent::Cancel), Gate::Ignore, "Escape, pressed for any reason");
        assert_eq!(gate(true, &DictationEvent::Lecture(false)), Gate::Pass);
        assert_eq!(gate(true, &DictationEvent::Retry), Gate::Pass, "no recorder involved");
        for event in [DictationEvent::Start, DictationEvent::Stop, DictationEvent::Cancel] {
            assert_eq!(gate(false, &event), Gate::Pass, "no lecture: dictation as ever");
        }
    }

    #[test]
    fn the_model_stays_loaded_through_a_lecture() {
        assert!(may_unload(false, false), "idle: the RAM goes back (#12)");
        assert!(!may_unload(true, false), "mid-take");
        assert!(!may_unload(false, true), "a lecture's quiet stretch is not idleness");
    }

    #[test]
    fn lines_are_stamped_mm_ss_and_minutes_run_past_the_hour() {
        assert_eq!(line(0, "Good morning."), "[00:00] Good morning.\n\n");
        assert_eq!(line(7 * 60 + 42, "Next slide."), "[07:42] Next slide.\n\n");
        assert_eq!(line(75 * 60 + 3, "Any questions?"), "[75:03] Any questions?\n\n");
    }

    #[test]
    fn each_line_is_on_disk_as_soon_as_it_is_appended() {
        let file = temp_file("append");
        append(&file, 0, "Good morning, everyone.").unwrap();
        // What a crash right here would leave behind.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Lecture 2026-09-30-140512\n\n[00:00] Good morning, everyone.\n\n");
        append(&file, 14, "النهارده هنتكلم عن الـ compilers").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Lecture 2026-09-30-140512\n\n[00:00] Good morning, everyone.\n\n[00:14] النهارده هنتكلم عن الـ compilers\n\n",
            "one title, lines in order, Arabic intact"
        );
        cleanup(&file);
    }

    #[test]
    fn a_seam_chunk_writes_only_its_new_words() {
        // The overlap re-heard the end of the last line.
        assert_eq!(fresh("we should check the numbers again.", "the numbers again. Then send it.", true), "Then send it.");
        // Its first word is the half of one the overlap started in.
        assert_eq!(fresh("check the numbers again.", "ers again. Then send it.", true), "Then send it.");
        // No overlap (a pause at the cut), or nothing repeated: all of it.
        assert_eq!(fresh("I'll fix it.", "It works now.", false), "It works now.");
        assert_eq!(fresh("That was the plan.", "Next we test.", true), "Next we test.");
        // The first line of a lecture, and a line break inside a chunk's text.
        assert_eq!(fresh("", " Good morning.\nLet's start. ", false), "Good morning. Let's start.");
        // The overlap was all this chunk heard: nothing new, nothing written.
        assert_eq!(fresh("see the numbers again.", "the numbers again.", true), "");
    }

    /// Two minutes ten of a speaker who pauses for a second every twelve.
    fn talk() -> Vec<f32> {
        let tone = |i: usize| 0.2 * (i as f32 * 150.0 * std::f32::consts::TAU / 16_000.0).sin();
        (0..16_000 * 130).map(|i| if i % (16_000 * 13) < 16_000 * 12 { tone(i) } else { 0.0 }).collect()
    }

    /// The audio thread's lecture branch, minus the microphone: every piece
    /// `step` and `tail` hand over, and the longest the buffer got.
    fn capture(audio: &[f32], file: &Path) -> (Vec<Piece>, usize) {
        let mut capture = Capture { file: file.to_path_buf(), dropped: 0 };
        let (mut all, mut sent, mut pieces, mut longest) = (Vec::new(), 0, Vec::new(), 0);
        for poll in audio.chunks(4_000) {
            all.extend_from_slice(poll);
            longest = longest.max(all.len());
            pieces.extend(capture.step(&mut all, &mut sent));
        }
        pieces.extend(capture.tail(&all, sent));
        (pieces, longest)
    }

    #[test]
    fn a_long_capture_lands_in_the_file_chunk_by_chunk() {
        let file = temp_file("capture");
        let (pieces, longest) = capture(&talk(), &file);
        assert!(pieces.len() >= 9, "a cut at each pause, got {}", pieces.len());
        assert_eq!(pieces[0].at, 0);
        assert!(!pieces[0].seam, "nothing before the first chunk to re-hear");
        assert!(pieces[1..].iter().all(|p| p.seam), "each later chunk re-hears the second before its cut");
        assert!(pieces.windows(2).all(|w| w[0].at < w[1].at), "stamps only go up");
        assert!(pieces.last().unwrap().at > 110, "and still count from the start once the buffer is trimmed");
        assert!(
            longest <= KEEP + crate::CHUNK_MAX_SAMPLES + 4_000,
            "the buffer held {} s: old audio must be dropped",
            longest / 16_000
        );

        // Through the transcribe thread's half, with a stand-in for the model.
        let mut prev = String::new();
        let stamps: Vec<u64> = pieces.iter().map(|p| p.at).collect();
        for (n, piece) in pieces.into_iter().enumerate() {
            // No word in common, or the seam would take one line for the next's overlap.
            write(|_| Ok(format!("point{n} noted{n}.")), piece, &mut prev);
        }
        let written = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = written.lines().filter(|l| l.starts_with('[')).collect();
        assert_eq!(lines.len(), stamps.len(), "a line per chunk");
        assert_eq!(lines[0], "[00:00] point0 noted0.");
        let last = stamps.len() - 1;
        assert_eq!(lines[last], line(stamps[last], &format!("point{last} noted{last}.")).trim_end());
        cleanup(&file);
    }

    #[test]
    fn silence_writes_nothing() {
        let file = temp_file("silence");
        let (pieces, _) = capture(&vec![0.0; 16_000 * 60], &file);
        assert!(pieces.is_empty(), "a quiet room is never handed to the model");
        assert!(!file.exists(), "and leaves no file");
        cleanup(&file);
    }

    #[test]
    fn a_failed_chunk_leaves_a_marked_gap_and_the_lecture_goes_on() {
        let file = temp_file("gap");
        let piece = |at, seam| Piece { file: file.clone(), at, samples: vec![0.0; 16], seam };
        let mut prev = String::new();
        write(|_| Ok("First point.".into()), piece(0, false), &mut prev);
        write(|_| Err(anyhow::anyhow!("model gone")), piece(12, true), &mut prev);
        assert_eq!(prev, "", "nothing for the next seam to match against");
        write(|_| Ok("Third point.".into()), piece(25, true), &mut prev);
        // The model heard nothing in this one: no empty line.
        write(|_| Ok("  ".into()), piece(38, false), &mut prev);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Lecture 2026-09-30-140512\n\n[00:00] First point.\n\n[00:12] (not transcribed)\n\n[00:25] Third point.\n\n"
        );
        cleanup(&file);
    }
}
