#!/usr/bin/env python
"""Guard against the [hidden] override bug, which has now shipped twice.

An author rule like `.banner { display: flex }` beats the UA stylesheet's
`[hidden] { display: none }` on specificity, so the element renders even with
the attribute set — and every `el.hidden = true` in JS silently does nothing.
That produced both the undismissable update banner and the permanently-stuck
"last dictation wasn't delivered" card, each time presenting as a dead button
rather than as a CSS problem.

Any element that hides via the `hidden` attribute must therefore have an
explicit `.<class>[hidden] { display: none }` guard.

    python scripts/check-hidden.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
html = (ROOT / "ui/index.html").read_text(encoding="utf-8")
css = (ROOT / "ui/index.css").read_text(encoding="utf-8")

bad = []
for eid in re.findall(r'<[^>]*id="([\w-]+)"[^>]*\shidden\b', html):
    tag = re.search(r'<[^>]*id="%s"[^>]*>' % re.escape(eid), html).group(0)
    classes = re.search(r'class="([^"]*)"', tag)
    for cls in (classes.group(1).split() if classes else []):
        sets_display = re.search(
            r"^\.%s\s*\{[^}]*\bdisplay\s*:" % re.escape(cls), css, re.M
        )
        guarded = re.search(
            r"\.%s\[hidden\]\s*\{[^}]*display\s*:\s*none" % re.escape(cls), css
        )
        if sets_display and not guarded:
            bad.append(f"#{eid} (.{cls}) — add `.{cls}[hidden] {{ display: none; }}`")

if bad:
    print("FAIL: author `display` beats [hidden]; these can never hide:")
    print("\n".join("  " + b for b in bad))
    sys.exit(1)
print("PASS: every hideable element's author `display` rule has a [hidden] guard")
