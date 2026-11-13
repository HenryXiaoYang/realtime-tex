# Live edit

A host sends an edit to a session and receives, in about a millisecond, the re-typeset lines of the unit that was edited, as a display list with the pages and baselines where its rows go.

## Sub-features

- `live-par` re-typesets a plain paragraph.
- `live-routing` reports where an edit went (`fast`, `background`, `preamble`) and why.
- `live-follow-up` delivers a layout after the edit with the changed pages.
- `live-cli` does one edit from the command line.

## How to get to it (user POV)

- Type in an editor whose host drives `rtex serve` (the `edit` command).
- Run `rtex edit --project DIR --find TEXT --text TEXT` in a terminal.
- Call `apply_edit` on a `Session` (Rust) or `rtex_session_apply_edit` (C); covered by `cargo test`.

## Driving it with rtexctl

Preconditions:

- The baseline preconditions hold.
- The text you edit is in the body, inside a paragraph.

- **Edit a paragraph.** Insert text at the start of a paragraph in the middle of the book. Run `.claude/skills/verify-rtex/scripts/serve-edit.py $RUN`. The verdict has `"routed": "fast"`, `"paragraph": {"status": "ok", …}` with `lines` and `glyphs` above 0, and exit code 0.
- **Edit a chosen place.** Run `.claude/skills/verify-rtex/scripts/serve-edit.py $RUN --find 'Baseline export' --text ' (edited)'`. The same fields hold.
- **Follow-up layout.** In the same verdict, `layout_after.compile` is `Ok`, `layout_after.state` is `Converged`, and `pages_changed` lists at least the page of the edit.
- **Routing of a preamble edit.** Run `.claude/skills/verify-rtex/scripts/serve-edit.py $RUN --find '\usepackage{amsmath}' --text '%'`. The verdict has `"routed": "preamble"` and no `paragraph`; the script exits 1 because no live update exists for that edit. That exit is the expected result of this step.
- **CLI entry.** Run `target/release/rtex edit --project $P --find 'Baseline export' --text ' (edited)' > $E/edit-cli.txt 2>&1; cat $E/edit-cli.txt`. The output has a `layout v1 (… Converged)` line, `routed fast`, and a `paragraph … ok in … ms` line with a `fragment page …` line under it.
- **Proof.** `$E/live-edit.json` and the `sent` line with the events after it in `$E/transcript.jsonl`.

## Gotchas

- `serve-edit.py` overwrites `live-edit.json` and `transcript.jsonl` on each call. Copy them aside before a second drive if you need both.
- Each `serve-edit.py` call starts a new session on the unchanged project. The edit is in the session's buffer, never on disk.
- A live edit is followed by a pass on its own only when it changed the unit's height or row count (`pagination_stale: true` in the `ParagraphUpdate`). An edit that keeps the line count gets no layout until one is requested, which is why `serve-edit.py` sends `request_layout`. A script that waits for a layout without asking can wait forever.
- An edit sent before the first layout is routed `background`: the engine has no context for the unit yet.
- `status: ok` does not prove the glyphs are right. Use `fidelity.md` for that.
- The default `--find` is a blank line, so the text lands at the start of a paragraph and begins with a space. That is harmless for a paragraph and wrong for a test of a specific construct: pass `--find`.
