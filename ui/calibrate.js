// Calibration (#22): read a sentence built from your trained words. The
// sentence is known, so anything Sotto heard differently in a trained word's
// place is a mishearing of that word, worth adding as a correction. Pure, so
// `node ui/calibrate.js` runs the self-check below.
//
// Templates, not the local LLM: they work offline and instantly even with AI
// polish off or not downloaded, the same words always give the same
// sentences, and each word sits between plain common words, so the word diff
// isolates it. A model can reword, inflect or drop the very word being tested.
const CAL_TEMPLATES = [
  "Please ask {} to send the notes today.",
  "I think {} is ready for the next step.",
  "Can you open {} before the meeting starts?",
  "We talked about {} for most of the call.",
  "Let me check {} and get back to you.",
  "The new plan puts {} at the top of the list.",
  "Remind me to look at {} after lunch.",
  "Tell the team that {} works well now.",
];

// One sentence per trained word, up to eight (the plan's "read these 8").
// ponytail: vocabulary order, so a ninth word waits. Weakest-first if big
// vocabularies show up.
function calibrationSentences(words) {
  return words.map((w) => w.trim()).filter(Boolean).slice(0, CAL_TEMPLATES.length)
    .map((w, i) => CAL_TEMPLATES[i].replace("{}", () => w));
}

// [word, heard] for each trained word in `sentence` that came back as
// something else. Reads wordDiff(sentence, heard) as change blocks: the
// sentence words struck between two matches and the heard words in their
// place. A block that is just the word gives all its heard words; a block
// with neighbours is split word for word when both sides count the same;
// otherwise which heard words were the trained one is a guess, so it's
// skipped. A word heard as nothing teaches nothing.
function harvest(sentence, heard, words) {
  const key = (w) => w.toLowerCase().replace(/[^\p{L}\p{M}\p{N}]/gu, "");
  const bare = (w) => w.replace(/^[^\p{L}\p{M}\p{N}]+|[^\p{L}\p{M}\p{N}]+$/gu, "");
  const pairs = [];
  let said = [], got = [];
  const block = () => {
    const s = said.map(key);
    for (const word of words) {
      const t = word.split(/\s+/).map(key).filter(Boolean);
      const at = t.length ? s.findIndex((_, i) => t.every((k, j) => s[i + j] === k)) : -1;
      const h = at < 0 ? null : s.length === t.length ? got : s.length === got.length ? got.slice(at, at + t.length) : null;
      const as = (h || []).map(bare).filter(Boolean).join(" ");
      if (as) pairs.push([word, as]);
    }
    said = []; got = [];
  };
  for (const p of wordDiff(sentence, heard)) {
    if (p.op === "-") said.push(p.w);
    else if (p.op === "+") got.push(p.w);
    else block();
  }
  block();
  return pairs;
}

if (typeof window === "undefined") {
  const assert = require("node:assert");
  globalThis.wordDiff = require("./diff.js").wordDiff;

  // Sentences: one per word, in order, blanks skipped, capped at eight.
  assert.deepStrictEqual(calibrationSentences(["Claude", " ", "Gemini CLI"]),
    ["Please ask Claude to send the notes today.", "I think Gemini CLI is ready for the next step."]);
  assert.strictEqual(calibrationSentences(Array.from({ length: 11 }, (_, i) => `w${i}`)).length, 8);
  assert.deepStrictEqual(calibrationSentences([]), []);
  assert.strictEqual(calibrationSentences(["$& co"])[0], "Please ask $& co to send the notes today."); // no replace() patterns
  // Every generated sentence harvests its own word back.
  calibrationSentences(["Sotto", "Kai's Flow", "كشري"]).forEach((s, i) =>
    assert.strictEqual(harvest(s, s.replace(["Sotto", "Kai's Flow", "كشري"][i], "zzz"), ["Sotto", "Kai's Flow", "كشري"]).length, 1));

  const S = "Please ask Claude to send the notes today.";
  // The word misheard alone: its heard form, edge punctuation off.
  assert.deepStrictEqual(harvest(S, "Please ask clawed to send the notes today", ["Claude"]), [["Claude", "clawed"]]);
  assert.deepStrictEqual(harvest("Remind me to look at Sotto.", "Remind me to look at soto.", ["Sotto"]), [["Sotto", "soto"]]);
  // Heard right, whatever the case and punctuation: nothing to learn.
  assert.deepStrictEqual(harvest(S, "please ask claude, to send the notes today", ["Claude"]), []);
  // One word heard as two, and a multi-word target heard as two others.
  assert.deepStrictEqual(harvest("I think Sotto is ready.", "I think so too is ready.", ["Sotto"]), [["Sotto", "so too"]]);
  assert.deepStrictEqual(harvest("Can you open Gemini CLI before the meeting starts?",
    "Can you open German ICLI before the meeting starts?", ["Gemini CLI"]), [["Gemini CLI", "German ICLI"]]);
  // A neighbour misheard too: split word for word when the counts match...
  assert.deepStrictEqual(harvest(S, "Please asked clawed to send the notes today.", ["Claude"]), [["Claude", "clawed"]]);
  // ...and skipped when they don't, or when the word was dropped.
  assert.deepStrictEqual(harvest(S, "Please cloudy to send the notes today.", ["Claude"]), []);
  assert.deepStrictEqual(harvest(S, "Please ask to send the notes today.", ["Claude"]), []);
  // Other words misheard aren't harvested; only trained words in the sentence are.
  assert.deepStrictEqual(harvest(S, "Please ask Claude to sand the notes today.", ["Claude"]), []);
  assert.deepStrictEqual(harvest(S, "Please ask clawed to send the notes today.", ["Sotto"]), []);
  // Egyptian Arabic inside the English frame.
  assert.deepStrictEqual(harvest("Please ask كشري to send the notes today.", "Please ask كوشري to send the notes today.", ["كشري"]),
    [["كشري", "كوشري"]]);
  console.log("calibrate.js: all checks passed");
}
