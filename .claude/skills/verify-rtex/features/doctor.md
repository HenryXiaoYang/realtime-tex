# Doctor

`rtex doctor` tells a user whether rtex can run: where its TeX support files are and which LuaLaTeX it will use.

## Sub-features

- `doctor-ok` prints the version, the support-file directory and the LuaLaTeX banner.
- `doctor-missing-support` exits 1 when the support files are not found.
- `doctor-missing-lualatex` exits 2 when LuaLaTeX is not found.

## How to get to it (user POV)

- Run `rtex doctor`, or `bin/rtex doctor` in an unpacked release archive.

## Driving it with rtexctl

Preconditions:

- `target/release/rtex` exists (`rtexctl.sh launch $RUN`).

- **Healthy.** Run `target/release/rtex doctor > $E/doctor.txt 2>&1; echo "exit $?" >> $E/doctor.txt; cat $E/doctor.txt`. Three lines: `rtex <version>`, `support files: <this checkout>/tex`, `lualatex: <path> (This is LuaHBTeX, …)`. Exit code 0.
- **No LuaLaTeX.** Run `env PATH=/usr/bin:/bin RTEX_TEXLIVE_BIN= target/release/rtex doctor; echo "exit $?"`. Exit code 2 and a message that LuaLaTeX was not found.
- **Proof.** `$E/doctor.txt`.

## Gotchas

- A binary built from this checkout finds `tex/` through its build path, so `doctor-missing-support` cannot be reproduced without moving the binary to a folder that has no `tex/` beside it and unsetting `RTEX_TEXDIR`.
- The no-LuaLaTeX step assumes no `lualatex` in `/usr/bin` or `/bin`.
