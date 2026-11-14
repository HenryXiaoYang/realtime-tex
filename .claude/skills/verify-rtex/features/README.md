# rtex verification map

This directory is the maintained source for verifying the user-facing behaviour of rtex. Read this index before driving, then use the matching feature file as the recipe.

| File | Feature |
|---|---|
| `live-edit.md` | a keystroke re-typesets one unit and returns its display list |
| `background-layout.md` | a full pass delivers pages, placements and convergence |
| `export.md` | a PDF export equal to a clean LuaLaTeX build |
| `fidelity.md` | live output and page lists equal the PDF (`rtex verify`) |
| `doctor.md` | the installation check |

## Baseline preconditions

- `RUN` is set and `.claude/skills/verify-rtex/scripts/rtexctl.sh launch $RUN` printed `ready:`.
- `rtexctl.sh doctor $RUN` exits 0 and `support files:` is this checkout's `tex/`.
- The project is `build/verify-run/$RUN/project`, a generated 10-page book (`book` class, Latin Modern TFM fonts, microtype, amsmath), with `main.tex` as its only source.
- Evidence goes to `build/verify-evidence/$RUN/`.

## Driving conventions

- Run every command from the repository root, exactly as written.
- `P=build/verify-run/$RUN/project` and `E=build/verify-evidence/$RUN` in the recipes.
- Offsets in `edit` commands are bytes into the file as the session holds it, not characters.
- A session needs its first settled `LayoutUpdate` before an edit can be live. Wait for it.
- Never drive a project folder this run did not create.

## Proof and skip reporting

- CLI proof is the command, its stdout and stderr, and its exit code, saved under `$E`.
- Session proof is `transcript.jsonl` plus the verdict file.
- Record the feature file and sub-feature id with each artifact you cite.
- A recipe you could not run is reported with the command tried and the unmet precondition. Do not report it as verified through another path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behaviour, then exactly four H2 sections in this order: `Sub-features`, `How to get to it (user POV)`, `Driving it with rtexctl`, `Gotchas`.
