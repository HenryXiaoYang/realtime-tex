# PDF export

A session builds the document without the capture package, as many passes as it needs, and writes a PDF that equals what plain LuaLaTeX produces.

## Sub-features

- `export-cli` exports from the command line.
- `export-check` compares the export with an independent clean build.
- `export-session` exports the session's edited buffer, not the file on disk.

## How to get to it (user POV)

- Run `rtex export --project DIR --out FILE [--check]`.
- Send `{"cmd":"export_pdf","out":"FILE"}` to `rtex serve` and wait for `PdfExported`.

## Driving it with rtexctl

Preconditions:

- The baseline preconditions hold.

- **Export and check.** Run `target/release/rtex export --project $P --out $E/export.pdf --check > $E/export.txt 2>&1; echo "exit $?" >> $E/export.txt; cat $E/export.txt`. The output has `export: status Ok converged true passes 2` and `independent clean build: 2 passes, equal = true []`, and the exit code is 0.
- **The file exists.** Run `ls -l $E/export.pdf`. It is a PDF of roughly 100 KB for the 10-page book.
- **Compare two PDFs yourself.** Run `target/release/rtex pdf-compare A.pdf B.pdf`. It compares content streams, fonts and images and ignores metadata.
- **Proof.** `$E/export.txt` and `$E/export.pdf`.

## Gotchas

- `rtex export` has no `--build` option: it builds in `$P/build/rtex`, inside the run's project. Do not export a project a session is running on.
- `equal = true` with an empty list is the pass. A non-empty list names what differs.
- `export-session` has no recipe here yet: driving `export_pdf` after an `edit` in one session needs a longer script than `serve-edit.py`. `crates/rtex-core/tests/export.rs` covers it.
