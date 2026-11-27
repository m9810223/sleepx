# sleepx

`sleep` with a progress bar. Inline, single-line, no fullscreen TUI.

```
30.0s - 25.0s =  5.0s [=======>                                        ]  16.7%
30.0s -  0.0s = Done! [================================================] 100.0%
```

Each line reads as an equation: **total - remaining = elapsed**, then the bar
and the percentage.

## Install

### From GitHub

```bash
cargo install --git https://github.com/m9810223/sleepx
```

### From source

```bash
git clone https://github.com/m9810223/sleepx
cd sleepx
cargo install --path .
```

## Usage

```bash
sleepx 30          # 30 seconds
sleepx 1.5         # 1.5 seconds
sleepx 1m30s       # 1 minute 30 seconds
sleepx 1m 30s      # several durations are added together, like coreutils sleep
sleepx 2h5m        # 2 hours 5 minutes
```

Units: `s`, `m` (minutes), `h`, `d`, `w`, and other [humantime](https://docs.rs/humantime) units.
An uppercase `M` is rejected because humantime reads it as months; use `m` for minutes.

Progress goes to **stderr**; stdout stays empty, so `x=$(sleepx 30)` captures nothing.

### Display

Times are shown to a tenth of a second. Total and elapsed are truncated
(never rounded up), and remaining is derived from them, so the equation always
holds exactly. The layout is fixed for the whole run and depends on the total:

| Total         | Format    | Example      |
| ------------- | --------- | ------------ |
| under 10s     | `S.ts`    | `5.3s`       |
| under 1 min   | `SS.ts`   | `12.3s`      |
| under 1 hour  | `M:SS.t`  | ` 1:05.3`    |
| 1 hour and up | `H:MM:SS.t` | ` 1:01:01.3` |

Leading zero groups are blanked, so `00:05.3` shows as `    5.3`.

### Options

| Flag                             | Description                                                        |
| -------------------------------- | ------------------------------------------------------------------ |
| `-S`, `--style <STYLE>`          | Bar style: `arrow`, `dot`, `block`, `hash` [default: arrow]        |
| `--bar-left <CHAR>`              | Left bracket character [default: `[`]                              |
| `--bar-fill <CHAR>`              | Fill character (overrides `--style`)                               |
| `--bar-tip <CHAR>`               | Tip character at the head of the fill, e.g. `>`                    |
| `--bar-empty <CHAR>`             | Empty character (overrides `--style`)                              |
| `--bar-right <CHAR>`             | Right bracket character [default: `]`]                             |
| `-b`, `--bar <BAR>`              | `auto`, `on`, `off`; auto shows the bar only when stderr is a terminal |
| `-c`, `--color <COLOR>`          | `auto`, `on`, `off`; auto colors only when stderr is a terminal    |
| `-s`, `--say`                    | Announce "Timer complete" with macOS `say` when done               |
| `-n`, `--min-interval <SECONDS>` | Shortest gap between lines when stderr is not a terminal [default: 1] |
| `-x`, `--max-interval <SECONDS>` | Longest gap between lines when stderr is not a terminal [default: 60] |
| `-h`, `--help`                   | Print help (`--help` shows more detail)                            |
| `-V`, `--version`                | Print version                                                      |

### Styles

```bash
sleepx 30                    # arrow (default)
# 30.0s - 25.0s =  5.0s [=======>                                        ]  16.7%

sleepx 30 --style dot        # dot
# 30.0s - 25.0s =  5.0s [●●●●●●●●○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○]  16.7%

sleepx 30 --style block      # block
# 30.0s - 25.0s =  5.0s [████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]  16.7%

sleepx 30 --style hash       # hash
# 30.0s - 25.0s =  5.0s [########----------------------------------------]  16.7%

sleepx 30 -S dot --bar-fill "★" --bar-empty "☆"  # custom
# 30.0s - 25.0s =  5.0s [★★★★★★★★☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆]  16.7%

sleepx 30 --bar off          # text only
# 30.0s - 25.0s =  5.0s  16.7%
```

The bar stretches to the terminal width (at least 10 cells). The `arrow` style
draws a `>` tip; set `--bar-tip` to give other styles one.

### When stderr is not a terminal

When stderr is redirected or piped, `sleepx` adapts:

- **No progress bar and no color** by default, to keep logs clean
- **One line per update** instead of redrawing, so `>>` and `tee` work
- **Dynamic interval**: the gap between lines steps from 1s (last 10s) through
  5s, 10s and 30s up to 60s (more than an hour left), then is clamped to
  `--min-interval` / `--max-interval`. Lines land on round timestamps.

```bash
# Log progress: one text line per update
sleepx 30 2>&1 | tee timer.log
# 30.0s - 30.0s =  0.0s   0.0%
# 30.0s - 25.0s =  5.0s  16.7%
# 30.0s - 20.0s = 10.0s  33.4%
# ...

# Force the bar even when piped
sleepx 30 --bar on 2>&1 | cat

# Control output frequency
sleepx 1h --min-interval 5 --max-interval 30 2>> build.log
```

If whatever reads stderr goes away (for example `2>&1 | head -1`), `sleepx`
exits quietly with status 141, as if killed by `SIGPIPE`. It notices at its
next line, which can be up to `--max-interval` later.

### Exit status

| Status | Meaning                                         |
| ------ | ----------------------------------------------- |
| 0      | Slept the full duration                         |
| 2      | Invalid arguments or environment variables      |
| 129    | Killed by `SIGHUP` (terminal closed)            |
| 130    | Killed by `SIGINT` (Ctrl+C)                     |
| 141    | The progress reader closed the pipe             |
| 143    | Killed by `SIGTERM`                             |

On a signal, `sleepx` prints a final progress line, restores the terminal and
then dies by that same signal, so wrapper scripts stop on the first Ctrl+C.

## Environment Variables

Every option can be set with an environment variable; a command-line flag
overrides it.

| Variable              | Flag             | Example |
| --------------------- | ---------------- | ------- |
| `SLEEPX_STYLE`        | `--style`        | `block` |
| `SLEEPX_BAR_LEFT`     | `--bar-left`     | `\|`    |
| `SLEEPX_BAR_FILL`     | `--bar-fill`     | `★`     |
| `SLEEPX_BAR_TIP`      | `--bar-tip`      | `>`     |
| `SLEEPX_BAR_EMPTY`    | `--bar-empty`    | `☆`     |
| `SLEEPX_BAR_RIGHT`    | `--bar-right`    | `\|`    |
| `SLEEPX_BAR`          | `--bar`          | `on`    |
| `SLEEPX_COLOR`        | `--color`        | `off`   |
| `SLEEPX_SAY`          | `--say`          | `1`     |
| `SLEEPX_MIN_INTERVAL` | `--min-interval` | `2`     |
| `SLEEPX_MAX_INTERVAL` | `--max-interval` | `10`    |

`SLEEPX_SAY` accepts `1`/`0`, `yes`/`no`, `true`/`false` and `on`/`off` in any
case; an empty value counts as unset.

```bash
# Set defaults in your shell profile
export SLEEPX_STYLE=block
# CLI flags override environment variables
sleepx 30 --style hash  # uses hash, not block
```

## Features

- Single-line inline display: no fullscreen, no alternate screen buffer
- `total - remaining = elapsed` to 0.1s, with percentage
- Bar styles (arrow, dot, block, hash) or fully custom brackets, fill, tip and empty characters
- Bar stretches to the terminal width; colors when stderr is a terminal
- Human-friendly durations (`30`, `1m30s`, `2h5m10s`), summed like coreutils `sleep`
- Progress on stderr, stdout left empty
- Dies by the signal it received (Ctrl+C, `SIGTERM`, `SIGHUP`), after restoring the terminal
- macOS `say` integration (`--say`), checked at startup
- Dynamic output interval when stderr is not a terminal (configurable with `--min-interval` / `--max-interval`)

## License

MIT
