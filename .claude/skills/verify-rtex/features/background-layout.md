# Background layout

After an edit, or on request, a session runs LuaLaTeX over the whole document and delivers a layout: the pages that changed as display lists, where every unit sits, and whether the document's references have settled.

## Sub-features

- `layout-first` delivers the first layout after a session opens.
- `layout-after-edit` delivers only the changed pages after an edit.
- `layout-convergence` reports `Converged`, `Converging` or `PassLimitReached` with the pass count.
- `layout-diagnostics` reports LaTeX errors and warnings of the pass.

## How to get to it (user POV)

- Open a document in a host that drives `rtex serve`; the first layout arrives without a request.
- Make an edit that is routed to the background or that changes a unit's height; a pass starts after the debounce time (300 ms).
- Send `{"cmd":"request_layout"}`.
- Send `{"cmd":"status"}` to read the current versions and convergence.

## Driving it with rtexctl

Preconditions:

- The baseline preconditions hold.

- **First layout.** Run `.claude/skills/verify-rtex/scripts/serve-edit.py $RUN`. In the verdict, `first_layout.pages_total` is 10 and `first_layout.passes` is 2 for the generated book.
- **Changed pages only.** In the same verdict, `layout_after.pages_changed` is a subset of the pages, starting at the page of the edit, not all ten unless the edit moved every page break.
- **Convergence.** `layout_after.state` is `Converged`. Open `$E/transcript.jsonl` and read the `LayoutUpdate` events: each carries `passes`, `compile.state` and `convergence`.
- **Diagnostics.** Run `.claude/skills/verify-rtex/scripts/serve-edit.py $RUN --text ' \undefinedmacro'`. The transcript has a `Diagnostics` event with `source` `background` whose items name `\undefinedmacro`, and the following `LayoutUpdate` has `compile.state` `CompiledWithErrors` and `convergence.state` `Converging` with the reason `1 compile errors`. The script exits 1 because the layout is not `Ok`; that is the expected result of this step.
- **Proof.** The `LayoutUpdate` and `Diagnostics` lines of `$E/transcript.jsonl`.

## Gotchas

- A layout whose compile has errors reports `Converging` and no further pass follows until the source changes. Do not wait for `Converged` after it.
- A pass starts 300 ms after the last edit (`SessionConfig::debounce`). That wait is not pass time; `wall_ms` in the event is.
- A document whose bibliography program fails still converges, with biblatex's warning in `Diagnostics`. On a machine without a working `biber` the bibliography is simply missing.
- Pass directories alternate between `session/bg/pass-0` and `pass-1`. Page lists are `main.rtex-page<n>.bin` there; `target/release/rtex dl2json FILE` prints one as JSON.
- The generated book has no citations, floats or table of contents. To verify those, generate `--variant mixed` or `--variant units` into the run's project folder before driving: `target/release/rtex gen-book --pages 10 --variant mixed --out $P`.
