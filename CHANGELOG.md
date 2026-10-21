# Changelog

## Unreleased

### Fixed

- Live edits no longer stop for good after a paragraph that typesets no rows (only a `\label`,
  `\vspace` or `\noindent`) is typed next to another paragraph typed fresh, before the
  background pass. Its placement read the first of no rows and the engine thread panicked
  ("index out of bounds" in `layout.rs`), so no edit was live again until the engine restarted.

## 0.0.5 (2026-10-19)

### Fixed

- A main file whose name has a space (`my paper.tex`) works. LuaTeX quotes such a job name,
  and the capture wrote its results under the quoted name, so no layout ever arrived (#8).
- A page without glyphs (a blank page left by `\cleardoublepage` with `\pagestyle{empty}`) no
  longer makes the whole layout fail with "invalid type: sequence, expected a map".
- A figure or table with a comment line or `\vspace` right above it, or a comment after its
  `\end`, compiles in the live engine. The engine looked for `\begin{figure}` only at the very
  start of the unit and for `\end{figure}` only at its very end, so it typeset the `\begin` as
  text and failed with "Not in outer par mode".
- A figure, table, list or other block environment that is the last thing in an `\input` file
  is live. Unless the file ended with a blank line, its edits went to the background pass: the
  trailing space rtex keeps for a paragraph at the end of an `\input` file was added after the
  environment too, and it no longer ended with its `\end`.
- A file `\input` in the preamble (a macro file, say) is loaded once by the live engine. It
  was also read again as document setup, so every engine start logged "Command ... already
  defined" for each of its macros.
- `\mathbf` after `\bm` (fontspec documents) is right in the live engine. Its memoized math
  font setup kept the bold math version's alphabets when math returned to the normal version,
  so `\mathbf{M}` came out in the wrong font. After a footnote, TeX could even loop on it until
  the engine stopped, and the paragraph was not live again until the preamble changed.
- The first page of an IEEEtran document is drawn from its display list, not from the
  PDF. Under LuaLaTeX the class writes a `\special{papersize=…}`, which LuaTeX's PDF backend
  ignores, and rtex took it for something a display list cannot show. Only `pdf:` specials,
  the ones the backend draws, mark a page as degraded now.
- A table edited live in IEEEtran keeps the class's `\footnotesize`. The live engine set every
  float box like the LaTeX kernel, in `\normalsize`; it now uses the float box the class sets
  up for tables and figures.
- IEEEtran's `\subsubsection`, a run-in heading like `\paragraph`, waits for the next pass
  instead of going live without the paragraph its title shares a line with. rtex knew run-in
  headings only by name; the capture now records which headings the class sets run-in.
- Math set with newtxmath, or any font whose TFM comes with a `.vf`, is right in the live
  display list. LuaTeX reports such a font as a real font and reads its `.vf` only when it
  writes the PDF, so rtex did not expand it: glyphs named `ntxmi`, `ntxsy`, … with their
  virtual character codes, which have no outline file in `pdftex.map`, and a host drew
  the wrong characters (`+` came out as `Ï`). rtex now reads the `.vf` itself and names the
  real fonts (`NewTXMI`, `txsys`, …).

### New

- IEEEtran's `IEEEkeywords` and `IEEEdescription` (a nomenclature, with `\IEEEsetlabelwidth`
  and `\IEEEusemathlabelsep`) update as you type, like the standard lists. Commands a document
  class provides can be allowed like a package's; these two are allowed with IEEEtran only.

### CI and repository

- The corpus has an IEEEtran journal paper (two columns, a nomenclature, run-in headings,
  floats). CI checks that at least 18 of its units are live and that all of them are exact.
- CI installs newtx (with txfonts and kastrup) for a test that a live edit in newtxmath math
  names the real fonts, not the virtual ones.

## 0.0.4 (2026-10-12)

### Fixed

- **Two-column documents** (IEEEtran, `twocolumn`) are live again. When a column break fell
  inside a paragraph, the page's two-column box was recorded as a row of that paragraph, and
  the lines inside it lost their placements: in a 12-page IEEEtran paper almost no body text
  was live (#2). The capture now leaves everything the output routine builds out of units.
- Comment lines inside or right above a paragraph no longer send its edits to the background
  pass. rtex read two `%` lines in a row as a blank line (a paragraph break); like TeX, it now
  drops the end of a commented line too (#3).
- A paragraph with a `\marginpar` is live in probe mode, with the note in the margin following
  after the next pass, like footnote text. The live engine reports the note as an insert.
- Diagnostics keep the whole text of LaTeX, class and package messages, including the
  `(LaTeX)`/`(<package>)` continuation lines. Before, only the first line came through, so
  an error such as "Mismatched LaTeX support files detected" lost the lines that say which
  versions do not match (#1).

### CI and repository

- `scripts/install-texlive.sh` installs only missing packages and contacts the TeX Live
  repository only then, updating tlmgr first. A newer tlmgr on CTAN stopped re-runs over an
  existing installation with "tlmgr itself needs to be updated".

## 0.0.3 (2026-10-11)

### Fixed

- **Live typing on Windows with MiKTeX.** The live engine exited at startup with
  "exit code: 1" on every MiKTeX installation (realtime-tex-vsc-plugin#1): MiKTeX refuses to
  let LuaTeX write outside the output directory, which includes the engine's response pipe.
  rtex now sets `MIKTEX_ALLOWUNSAFEOUTPUTFILES=1` for the LuaLaTeX processes it starts, the
  MiKTeX equivalent of the `openout_any=a` it already sets for TeX Live.
- When the live engine cannot start, the error quotes the first TeX or Lua error from its log
  instead of only the exit code.

### New

- **Prebuilt binaries** for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and Windows
  (x86_64), attached to each release by CI. Each holds `rtex`, the shared C library, `rtex.h`
  and the support files.
- `rtex` finds its support files next to its executable (`share/rtex/tex`), so an unpacked
  archive works from anywhere. `rtex doctor` checks the installation.

### CI and repository

- A job runs a live-editing session on Windows with MiKTeX, in paths with spaces and non-ASCII
  characters and on a CRLF document.
- GitHub counts the project as Rust: fixtures, benchmarks and the TeX/Lua files are marked as
  vendored for language statistics.

## 0.0.2 (2026-10-10)

### New

- **Windows support.** rtex runs on Linux, macOS and Windows. CI tests all three against a real
  TeX Live.
- **Much more goes live.** Not just plain paragraphs any more, but:
  - paragraphs with display math, footnote marks, references and citations (natbib and
    biblatex);
  - lists, quotes, theorems and proofs, figures and tables with images and captions, verbatim
    and listings;
  - headings, the title block, manual bibliographies;
  - your own macros and environments, and text in `\input`/`\include`d files.

  Counters, `\the<counter>` formats and macros the body redefines are restored per unit, so
  numbers come out right.
- **Probe mode** (the default). A paragraph that uses commands rtex does not know is no longer
  sent to the background automatically. rtex compiles it once as the last pass saw it and
  compares the result with the page. If it matches exactly, edits go live. Definitions that
  leak out of a paragraph are detected.
- **New paragraphs and splits stay live.** A paragraph you split or type fresh borrows its
  neighbour's settings until the next pass.
- **Faster background passes.**
  - A standby LuaLaTeX with the preamble already loaded typesets only the body (3–4× faster for
    short documents).
  - Unchanged TikZ/pgfplots pictures are reused from the previous pass (on a 111-page document
    with 120 pictures, 45 s → 18 s per pass).
  - Slow multi-pass runs show each finished pass right away.
- **Bibliographies and indexes run automatically.** biber, bibtex, makeindex, and imakeidx's
  own program and options.
- **Configurable time budget** per live compile (`fast_budget_ms`, default 50 ms).
- **Diagnostics for engine failures.** Set `debug_dir` and every failure leaves a bundle: what
  was sent, the TeX log, and a stage-by-stage trace.
- **New `rtex` subcommands.** `rtex probe` shows where a document's live latency goes, and
  `rtex verify --permissive` / `--pic-cache` check probe mode and the picture cache.

### Faster

- **About 0.5 ms per keystroke for a one-line paragraph, 1.1 ms for four lines**, independent of
  document length. That is down from 0.8 and 1.5 ms in 0.0.1. The engine is driven by raw
  frames, the editor's thread writes straight to an idle engine, and LaTeX work that cannot be
  seen in a paragraph (running-head marks, repeated math font setup, file probes) is skipped in
  the live engine.

### Fixed

Most of these were found by running real documents (a 111-page TikZ-heavy set of notes and a
58-page document with luatexja, biblatex, imakeidx, glossaries and tcolorbox):

- The response channel is read by a dedicated thread. On macOS a large result could block the
  engine until the watchdog killed it.
- A Lua error inside the live engine is reported at once instead of hanging until the watchdog.
- Projects with subfolders, and figures that are PDFs, now work in background passes.
- Layouts stopped converging when bookmarks recorded the pass directory, or the index was
  missing. Both are fixed.
- `\begin{document}` inside comments or verbatim no longer confuses rtex.
- biblatex citations are live in a session opened without an earlier build.
- glossaries' first-use forms are right in live updates.
- Running heads no longer count as part of the paragraph at the top of a page, and appendix
  numbering is right.
- Paragraphs that start inside a group (`{\em …}`) now have their lines.
- Fixed a deadlock when an edit arrived while a layout was being installed.
- A paragraph that crashes the engine is kept off the live path instead of crashing it on
  every keystroke.
- PDFs of earlier layouts stay valid while the host reads them. Two engines could write the
  same file, which produced blank pages.

### Changed

- The library, C ABI, display-list format and engine protocol are versioned together as 0.0.x.
  Nothing is frozen before 0.1.
- The C ABI takes more configuration keys: `fast_budget_ms`, `eligibility`, `picture_cache`,
  `debug_dir`, `unit_envs` and `warm_background`.

## 0.0.1

First version:
- a persistent LuaTeX engine that re-typesets single paragraphs;
- the capture package;
- the display-list format (binary and JSON);
- the Rust library with a C ABI;
- background passes with honest convergence reporting;
- PDF export;
- the fidelity checks;
- fixtures and benchmarks.
