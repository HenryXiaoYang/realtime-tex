# Fidelity

rtex promises that what it shows equals what LuaLaTeX produces. `rtex verify` checks that promise on a whole project: every eligible unit typeset live against the capture of a full run, and every page list against the PDF's content stream.

## Sub-features

- `fidelity-pdf` the instrumented run's PDF equals a clean build's (T3).
- `fidelity-live` live output of each eligible unit equals the full run's rows (layer 1).
- `fidelity-pages` each page list places glyphs, rules and images where the PDF does (layer 2).

## How to get to it (user POV)

- Run `rtex verify --project DIR`.

## Driving it with rtexctl

Preconditions:

- The baseline preconditions hold.

- **Verify.** Run `target/release/rtex verify --project $P --build build/verify-run/$RUN/verify --json-out $E/verify.json > $E/verify.txt 2>&1; echo "exit $?" >> $E/verify.txt; cat $E/verify.txt`. Exit code 0.
- **PDF identity.** The output has `T3 (instrumented PDF == clean PDF): true`.
- **Live units.** The `layer 1` line reads `N eligible of M units; N compiled; G/G glyphs identical; 0 units with differences`, with the two glyph counts equal.
- **Pages.** The `layer 2` line reads `10 pages, 10 exact, 0 degraded (PDF fallback), pass=true`.
- **Proof.** `$E/verify.txt` and `$E/verify.json`.

## Gotchas

- `--max-paragraphs N` shortens layer 1 to N units. A run with it does not prove the live path.
- A page counted `degraded` is drawn from the PDF by hosts. It is allowed for content a display list cannot describe (TikZ, PDF literals) and is a regression on the generated book, which has none.
- `warning: aux family not stable after 5 passes` means the document did not converge; the comparison that follows is of an unsettled document.
- Use `--variant units` or `--variant mixed` books, and the documents in `fixtures/corpus/`, for lists, floats, tables, footnotes and citations. CI runs them all.
