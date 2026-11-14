# Benchmarks

Everything on this page was measured on **2026-11-14** with rtex 0.0.7 plus the changes in
CHANGELOG "Unreleased" (main 18bee72), on a 4-vCPU cloud VM (Intel Xeon @ 2.10 GHz) with Linux
and TeX Live 2026. A typical laptop is faster. Raw results are in
[`bench/results/`](../bench/results/).

Earlier runs used other VMs of the same type, and the VM matters. This one is about as fast at
line breaking alone as the 2026-11-06 one (hardware factor *h* 1.12 against 1.04, see
[below](#compared-with-the-paper)), but its round trips came out about 1.5× slower. The code of
the 2026-11-06 run measures the same on this VM (0.50–0.58 ms for one line), so the difference
is the machine, not a regression. Compare figures within one run, not across runs.

## Typing latency

This is the time from `apply_edit` to the `ParagraphUpdate` in the host's hands, with the
display list decoded. That is everything except drawing. Each figure is the median of 300
separate single-character edits, 2 ms apart, in a warm session. The documents are generated
books in the `book` class with Latin Modern, microtype and amsmath.

| Paragraph | 10-page book | 100-page book | 300-page book |
|---|---|---|---|
| 1 line | 0.50 ms | 0.54 ms | 0.50 ms |
| 4 lines | 0.91 ms | 1.12 ms | 1.11 ms |
| 10–11 lines | — | 1.78 ms | 1.78 ms |
| 3 lines with inline math | 0.90 ms | 0.82 ms | 1.07 ms |

The cost depends on the paragraph, not on the document. A 300-page book is about as fast as a
10-page one; the cells differ by up to ±30 %. That is partly noise on a shared VM, and partly
because each book's paragraphs are different text. The 300-page "inline math" paragraph is
heavier on fractions than the 10-page one, and its TeX stage alone is 0.49 ms against 0.35 ms.
The 4-line paragraphs differ too (263 glyphs on the 100-page book, 215 on the 10-page one).

When edits come back to back (the paper's method: the mean of a burst of 100), threads stay warm
and the cost drops to 0.18–0.22 ms for one line and 0.62–0.86 ms for four lines.

### By kind of unit

Same method, on the books with every kind of unit (`--variant units`):

| Unit | 10-page book | 100-page book | Rows | TeX stage (100 p) |
|---|---|---|---|---|
| heading (`\section`) | 1.21 ms | 0.90 ms | 1 | 0.46 ms |
| paragraph with display math | 1.24 ms | 1.59 ms | 5 | 0.84 ms |
| paragraph with a footnote | 1.79 ms | 1.68 ms | 3–5 | 1.03 ms |
| list (`itemize`, 3 items) | 1.55 ms | 1.64 ms | 3 | 1.05 ms |
| figure (image and caption) | 1.67 ms | 1.87 ms | 3 | 1.27 ms |
| table (`booktabs`, caption) | 1.49 ms | 1.71 ms | 2 | 0.98 ms |

Environments spend most of their time in LaTeX's own machinery: `\list`, `\halign`,
`\@startsection`, `\includegraphics`, counters. The P95 is within 1.5× of the median for every
kind, except the heading on the 10-page book (2.1 ms) and the list and the table on the
100-page book (4.1 and 4.9 ms).

### Fonts matter most

The same 300-page book in three font setups:

| Font setup | 1 line | 4 lines | 11 lines | inline math |
|---|---|---|---|---|
| `[T1]{fontenc}` + `lmodern` (TFM) | 0.55 ms | 1.18 ms | 2.01 ms | 1.01 ms |
| fontspec, TeX Gyre Pagella, `Renderer=Basic` | 0.57 ms | 1.18 ms | 2.21 ms | 1.09 ms |
| fontspec, TeX Gyre Pagella, default (node mode) | 0.81 ms | 3.63 ms | 8.76 ms | 2.93 ms |

With fontspec's default node mode, luaotfload shapes the text in Lua on every compile. For the
11-line paragraph, the TeX stage is 7.5 ms against 1.0 ms with TFM fonts. `Renderer=Basic` uses
the engine's own ligature and kerning tables and is as fast as TFM. HarfBuzz mode
(`Renderer=HarfBuzz`) is slower still than node mode. If you want the fastest updates for Latin
text, use `Renderer=Basic` or TFM fonts ([live-editing.md](live-editing.md#making-it-faster)).

### Where the time goes

For a one-line paragraph on the 300-page book (0.50 ms in total):

| Stage | Time |
|---|---|
| TeX: restoring the context and typesetting the box | 0.15 ms |
| walking the box and encoding the display list | 0.06 ms |
| everything else: two process hand-offs, thread wake-ups, decoding, the session | 0.29 ms |

For four lines (1.11 ms) it is 0.54, 0.23 and 0.34 ms. The walk costs about 1 µs per glyph. The
fixed cost of 0.3–0.35 ms is mostly waking processes and threads that slept between keystrokes. That
is why bursts of edits are two to three times faster than isolated ones. On a laptop with
faster context switches this part is smaller.

On macOS and Windows the host reads the engine's responses on a dedicated thread, which fixed
lost results on macOS, where polling missed frames
([engine-protocol.md](engine-protocol.md#channels)); the hand-off to that thread costs about
0.1 ms. On Linux, where polling is reliable, the thread that waits for the result reads it
itself. Measured on the 2026-11-06 VM against 0.0.6, that and cheaper per-keystroke bookkeeping
brought one line on the 300-page book from 0.56 to 0.38 ms.

## Background passes

The time until the next layout arrives. A *pass* is one LuaLaTeX run with the capture package. A
layout is one or more passes, repeated until cross-references settle.

| Document | Pages | Live engine ready | First layout | Each later layout | Clean export |
|---|---|---|---|---|---|
| book, TFM fonts | 10 | 0.5 s | 1.1 s (2 passes) | 0.3 s | 1.4 s |
| book, TFM fonts | 100 | 0.6 s | 3.3 s (2 passes) | 1.3 s | 2.5 s |
| book, TFM fonts | 300 | 0.6 s | 7.8 s (2 passes) | 3.6 s | 4.7 s |
| book, OpenType (node mode) | 100 | 1.2 s | 10.3 s (2 passes) | 4.7 s | 10.3 s |
| mixed: TOC, biblatex/biber, floats, footnotes | 10 | 1.8 s | 6.1 s (3 passes) | 1.7 s | 7.7 s |
| mixed | 100 | 1.8 s | 15.0 s (3 passes) | 4.9 s | 16.0 s |

"Each later layout" is a pass after a one-character edit: the median of five, each needing one
pass. It runs in the standby engine, which already has the preamble loaded. "Clean export" is a
full PDF build without the capture package, from an empty build folder, with as many passes as
the document needs (2 or 3 here). Each figure is the median of two runs (seven for the first
layout of the 10-page mixed book, where one run of 12.5 s was an outlier).

**What makes passes slow.** For long documents, the capture package used to be the main cost.
It wrote a display list for every page as JSON, about 120 KB per page, and the host parsed the
pages whose JSON changed. It now writes each page in the binary format as the page ships.
Measured on this VM with the same method against the code of the 2026-11-06 run, a later layout
of the 300-page book takes 3.6 s instead of 5.5 s, and the first one 7.8 s instead of 11.6 s.
Documents where other work dominates gain less: a later layout of the 100-page mixed book takes
4.9 s instead of 5.4 s, and the 10-page book does not change. Clean exports, which never ran
the capture, take the same time in both.

On an Apple M5 Pro (2026-11-13), a pass over the 300-page TFM book spends 1.17 s in LuaLaTeX
itself and 0.25 s on the page lists, where the JSON took 0.98 s; a later layout takes 1.6 s
instead of 2.4 s. What is left above plain LuaLaTeX is the page walk, the units and paragraphs
file (0.14 s) and the per-paragraph bookkeeping.

For short documents the standby engine helps most. Starting LuaLaTeX and loading the preamble
is most of a short run, so a later layout of the 10-page book takes 0.3 s, against 1.1 s for
the first.

**Pictures.** TikZ and pgfplots documents are dominated by drawing. The picture cache reuses
unchanged pictures from the previous pass. On a 111-page document with 120 pictures, that
shortened a pass from 45 s to 18 s.

## Compared with Typst and Overleaf

This times the same text in all three: the document the paper's Typst comparison uses. It is
Lorem ipsum paragraphs on A4 at 11 pt, about five lines each, with the middle paragraph edited
30 times. `bench/compare/compare.py` builds a LaTeX version of it and runs everything on the same
machine:

| Pages | rtex, per edit (P95) | Typst 0.15.1, `typst watch` | Typst, one page exported | Full pdfLaTeX run | Full LuaLaTeX run |
|---|---|---|---|---|---|
| 10 | 1.5 ms (1.8) | 19 ms | 8 ms | 0.11 s | 0.52 s |
| 100 | 1.4 ms (2.0) | 140 ms | 49 ms | 0.21 s | 0.87 s |
| 300 | 1.5 ms (1.9) | 455 ms | 125 ms | 0.42 s | 1.70 s |

The pages column is the LaTeX page count, rounded; the files record the exact counts. How each
tool was measured:

- **rtex:** the session's round trip for each edit, from dispatching it to the live engine to
  having the decoded result. This is the median over 29 edits, after the first.
- **Typst:** the paper's comparison script (`bench/upstream/luatex-benchmark/typst-comparison/
  bench.mjs`), unmodified. In watch mode Typst writes the whole PDF after every edit. With
  `--pages 1` it writes one page, which is close to its compile time alone. Typst's editor
  previews (the web app, tinymist) skip PDF export, so they behave more like the second column.
- **Full LaTeX run:** one pdfLaTeX or LuaLaTeX run over the whole document, best of three. This
  is the least an Overleaf recompile does, since Overleaf compiles the whole document every
  time. The hosted service adds its own queueing, sandbox, PDF download and rendering, so this
  is a lower bound, not a measurement of Overleaf.

The comparison is not quite like for like:
- rtex's figure is the paragraph alone. The pages, page breaks and references settle with the
  next background pass, seconds later (see above).
- Typst and a LaTeX run produce the whole consistent document each time, which is why their
  time grows with its length.

What the numbers do show is what you wait for after a keystroke before the text you typed
appears typeset:
- rtex: a millisecond or two, whatever the length;
- Typst: tens to hundreds of milliseconds, growing with the document;
- full recompiles: a second or more.

Beyond speed, the tools make different trade-offs:

| | rtex | Typst | Overleaf |
|---|---|---|---|
| Language | LaTeX (LuaLaTeX only), existing documents and packages | its own markup; LaTeX documents must be rewritten | LaTeX (pdfLaTeX, XeLaTeX, LuaLaTeX) |
| Output | identical to LuaLaTeX | Typst's own typesetting | real LaTeX output |
| While you type | the paragraph is exact at once; the layout catches up in seconds | everything consistent on every compile | everything consistent after each recompile |
| Collaboration | none, local | web app with real-time collaboration | real-time collaboration, comments, history, templates |
| Setup | TeX Live and Rust locally (the VS Code extension installs both) | one binary, or the browser | the browser |

To reproduce the comparison:

```sh
source build/texlive.env
cargo build --release -p rtex-cli
# Typst: a release binary on PATH (bench/upstream/luatex-benchmark/typst-comparison/fetch-typst.sh), and Node.js
python3 bench/compare/compare.py --pages 10 100 300
```

Without `typst` on PATH, the script measures only rtex and the full LaTeX runs.

## Compared with the paper

The design follows Clemens Lode, *Real-Time LuaTeX: Recompiling Large Documents in 1 ms*
(TUGboat 2026). Its benchmark ships with rtex in `bench/upstream/` and runs unmodified as part of
`rtex bench`. It times LuaTeX's line breaking alone, inside one process:

| Paragraph | Paper (i7-1355U laptop) | This VM |
|---|---|---|
| short | 0.17 ms | 0.05 ms |
| medium | 0.70 ms | 0.78 ms |
| long | 1.99 ms | 2.21 ms |
| inline math | 0.20 ms | 0.22 ms |
| display math | 0.11 ms | 0.12 ms |

So this VM is about as fast as the paper's laptop at typesetting: `rtex bench` calls this ratio
the hardware factor *h*, 1.12 here (1.04 on the 2026-11-06 VM, 1.3 on the 2026-10-10 one). It
varied between 1.12 and 1.26 across runs on this VM. The paper's full round trip, editor to
engine and back excluding drawing, was 0.79 ms for a short paragraph and 6.11 ms for a medium
one. rtex measures 0.50–0.54 ms and 0.91–1.12 ms here. Most of the difference is in the work around TeX.
For a medium paragraph, the paper reports 4.99 ms for line breaking plus walking the result. rtex
walks the boxes and encodes a compact binary display list in about 0.2 ms.

## Running the benchmarks

```sh
source build/texlive.env
cargo build --release -p rtex-cli
for n in 10 100 300; do target/release/rtex gen-book --pages $n --fonts lm-tfm --out fixtures/book-$n-pure-lmtfm; done
for n in 10 100;     do target/release/rtex gen-book --pages $n --variant units --fonts lm-tfm --out fixtures/book-$n-units-lmtfm; done
target/release/rtex bench \
  --project fixtures/book-10-pure-lmtfm fixtures/book-100-pure-lmtfm fixtures/book-300-pure-lmtfm \
            fixtures/book-10-units-lmtfm fixtures/book-100-units-lmtfm \
  --categories short,medium,long,inline-math,display-math,footnote,list,figure,table,heading
```

This takes about five minutes. It writes a Markdown report, JSON and CSV to `bench/results/`.
Add `--quick` for a smoke run. Other font setups: `--fonts pagella-base` (`Renderer=Basic`),
`pagella` (node mode), `pagella-harf` (HarfBuzz).

`rtex bench` also checks a set of limits, all scaled by the hardware factor *h*:
- one line ≤ 1.5 ms × h (≤ 1.0 ms × h for bursts);
- four lines ≤ 6.11 ms × h (the paper's figure);
- every unit kind ≤ 5 ms × h;
- the walk and encoding of a 4-line paragraph ≤ 0.5 ms;
- 100- and 300-page books within 1.25× of the 10-page book.

In the 2026-11-14 run every limit passed except two size ratios, both for units on 100 vs 10
pages: display math at 1.29, and the heading at 0.74 (the check also flags ratios below 0.75,
here because the 10-page heading was slow). As explained above, they compare different
paragraphs. CI runs a quick version on GitHub's
shared runners and only warns when a limit is missed.

`rtex probe --project DIR` breaks one paragraph's latency into stages and times every unit
kind directly against the engine. Use it to find where a slow document spends its time.
