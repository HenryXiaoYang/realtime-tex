---
name: verify-rtex
description: Drive the real rtex (realtime-tex) command line and its `rtex serve` JSON-lines session against a generated LaTeX project, and capture proof that a change works. Use after changing anything under crates/ or tex/, before saying a live edit, a background layout, a PDF export or display-list output works.
---

# Verify rtex

rtex is a library for editors. Its user-facing surface in this repo is the `rtex` command (`crates/rtex-cli`), above all `rtex serve`, the JSON-lines session an editor host drives. There is no GUI here; the VS Code extension lives in another repository and talks to `rtex serve`. The Rust crate and the C library (`include/rtex.h`) expose the same session and are covered by `cargo test`, not by this skill.

Every command below runs from the repository root. A run has an id you choose (`RUN=fix-footnotes`). Its scratch state lives in `build/verify-run/$RUN/` and its proof in `build/verify-evidence/$RUN/`. `build/` is gitignored.

## Launch

rtex has no server to keep alive. Launch builds the binary and generates a disposable 10-page book.

```sh
RUN=my-change
.claude/skills/verify-rtex/scripts/rtexctl.sh launch $RUN
```

Ready is the line `ready: project …, evidence …`. The first build takes about a minute. Launch again after any change to a `.rs` file. Files under `tex/` are read at run time and need no rebuild.

You need LuaLaTeX on `PATH`, or `RTEX_TEXLIVE_BIN` set to a TeX Live `bin` directory (`scripts/install-texlive.sh` installs a minimal one into `build/texlive`).

## Doctor

```sh
.claude/skills/verify-rtex/scripts/rtexctl.sh doctor $RUN
```

It prints the support-file directory and LuaLaTeX that rtex found, the binary's version, the revision at launch against `HEAD`, and the project. It fails when a `.rs` file under `crates/` is newer than the binary, which means you are about to drive an old build. Run it first whenever a result looks wrong. `support files:` must point at this checkout's `tex/`; another path means `RTEX_TEXDIR` is set and you are testing other Lua files.

## Drive

Read `features/README.md`, then the feature file that matches what you changed. The main recipe is a live edit through a session:

```sh
.claude/skills/verify-rtex/scripts/serve-edit.py $RUN
.claude/skills/verify-rtex/scripts/serve-edit.py $RUN --find 'Baseline export' --text ' and more'
```

It starts `rtex serve` on the run's project, waits for the first settled layout, sends one `edit` command, waits for the `ParagraphUpdate`, sends `request_layout` and waits for that `LayoutUpdate`. It prints a one-line JSON verdict and exits 0 only when the paragraph came back `ok` with glyphs and a layout with at least one changed page arrived.

To drive a session by hand, write one JSON object per line to `rtex serve --project build/verify-run/$RUN/project --build build/verify-run/$RUN/session`: `{"cmd":"edit","path":"main.tex","start":N,"end":N,"text":"…"}` (byte offsets), `request_layout`, `status`, `spans`, `units`, `set_document`, `export_pdf`, `quit`. Events arrive on stdout with an `event` field (`EngineState`, `LayoutUpdate`, `ParagraphUpdate`, `BackgroundScheduled`, `Diagnostics`, `PdfExported`), replies with a `reply` field. `docs/embedding.md` describes each.

To verify against your own document instead of the generated book, replace `build/verify-run/$RUN/project` with a copy of it before driving. Never point a run at a project folder someone is editing: a session writes `build/rtex/` inside the project.

## Evidence

Everything goes to `build/verify-evidence/$RUN/`:

- `live-edit.json`: the verdict of `serve-edit.py` (routing, paragraph status, lines, glyphs, pages changed).
- `transcript.jsonl`: every command sent and every event received, timestamped, display lists reduced to counts.
- `serve.stderr.txt`: the session's stderr.
- whatever a feature recipe adds (`export.pdf`, `export.txt`, `verify.txt`, `verify.json`).

Proof standards:

- Drive `rtex serve` or the `rtex` command, as a host does. A passing `cargo test` is not a proof of user-visible behaviour.
- Show the action and the result: the `sent` line and the events after it in `transcript.jsonl`.
- For anything that changes what is typeset, also run the fidelity check in `features/fidelity.md`. A `ParagraphUpdate` with status `ok` says the engine answered, not that the glyphs are right.
- An edit that is routed `background` or `preamble` produces no `ParagraphUpdate`. That is a different feature (`features/background-layout.md`), not a pass of the live path.
- A layout that ends `PassLimitReached` is not converged. Report it; do not treat it as success.

## Cleanup

```sh
.claude/skills/verify-rtex/scripts/rtexctl.sh cleanup $RUN
```

It stops the session this run started, if one is still alive (by the pid in `build/verify-run/$RUN/serve.pid`), and removes `build/verify-run/$RUN/`. It keeps `build/verify-evidence/$RUN/`. Do not kill `rtex` or `lualatex` by name: the user's editor runs its own sessions on this machine.

## Changing `tex/*.lua`

Lua has no compiler, so a local that a move or a rename left behind becomes a global and nothing complains until that line runs. Two helpers cover it:

```sh
# before and after the change: the global names of each file must be the same set
.claude/skills/verify-rtex/scripts/lua-globals.sh tex/rtex-dl.lua tex/rtex-dl-walk.lua

# for a change that must not alter output: old support files (a git revision) against the
# working tree, byte for byte, on as many projects as you can afford
.claude/skills/verify-rtex/scripts/lua-identity.sh $RUN HEAD build/verify-run/$RUN/project fixtures/corpus/*
```

`lua-identity.sh` exits 1 on any difference and writes `lua-identity.txt` to the evidence folder. It compares page lists and live display lists exactly, and the units file with the build folder's name masked, because that file holds absolute source paths. Also run `texlua tex/tests/run.lua`. A local that is reassigned (not just read) cannot be handed to another file by value: check for that before moving code between Lua files.

## Helpers

- `scripts/rtexctl.sh launch|doctor|cleanup RUN`: build, check and tear down one run.
- `scripts/serve-edit.py RUN [--find TEXT] [--text TEXT]`: one live edit through `rtex serve`, with transcript and verdict.
- `scripts/lua-globals.sh FILE...`: the global names each Lua file touches.
- `scripts/lua-identity.sh RUN BASE_REV PROJECT...`: output with the old and the new support files, compared byte for byte.

## Isolation

Runs with different ids do not share state: each has its own project and build directory, and `rtex serve` uses no ports. Two sessions on the same project folder share `build/rtex/` and corrupt each other.
