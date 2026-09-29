// Word-level diff of what the ASR heard vs what was delivered — History's
// review panel (#25) and Calibrate's harvest (#22). Pure, so `node ui/diff.js`
// runs the self-check below.
//
// Words match on letters/digits only, ignoring case and punctuation: polish
// re-cases and re-punctuates nearly every word, and striking all of them would
// bury the edits that matter (dropped fillers, rewritten or added words). A
// matched word shows its delivered form.
// ponytail: O(n·m) LCS table; a take is tens of words. Myers if essays show up.

function wordDiff(raw, delivered) {
  const a = raw.split(/\s+/).filter(Boolean), b = delivered.split(/\s+/).filter(Boolean);
  const key = (w) => w.toLowerCase().replace(/[^\p{L}\p{M}\p{N}]/gu, "") || w;
  const ka = a.map(key), kb = b.map(key);
  const n = a.length, m = b.length;
  const L = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      L[i][j] = ka[i] === kb[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out = [];
  let i = 0, j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && ka[i] === kb[j]) { out.push({ op: "=", w: b[j] }); i++; j++; }
    // Deletions before insertions, so a swap reads "old new".
    else if (i < n && (j >= m || L[i + 1][j] >= L[i][j + 1])) out.push({ op: "-", w: a[i++] });
    else out.push({ op: "+", w: b[j++] });
  }
  return out;
}

// Node: calibrate.js reuses it; the self-check runs only for `node ui/diff.js`.
if (typeof window === "undefined") module.exports = { wordDiff };
if (typeof window === "undefined" && require.main === module) {
  const assert = require("node:assert");
  const show = (r, d) => wordDiff(r, d).map((p) => (p.op === "=" ? p.w : p.op + p.w)).join(" ");
  assert.strictEqual(show("um the the plan", "The plan."), "-um The -the plan."); // the stutter's second copy struck
  assert.strictEqual(show("send it to john", "Send it to John."), "Send it to John.");
  assert.strictEqual(show("meet at five no six", "Meet at six."), "Meet at -five -no six.");
  assert.strictEqual(show("gee pee tee is great", "GPT is great"), "-gee -pee -tee +GPT is great");
  assert.strictEqual(show("", "Hello"), "+Hello");
  assert.strictEqual(show("hello", ""), "-hello");
  assert.strictEqual(show("same words", "same words"), "same words");
  // Egyptian Arabic and code-switched: whole words, marks kept.
  assert.strictEqual(show("يعني افتح الـ terminal", "افتح الـ terminal"), "-يعني افتح الـ terminal");
  assert.strictEqual(show("شغّل ال build بتاع ال release", "شغّل الـ build بتاع الـ release"),
    "شغّل -ال +الـ build بتاع -ال +الـ release");
  assert.strictEqual(show("ok — done", "OK, done."), "OK, -— done.");
  console.log("diff.js: all checks passed");
}
