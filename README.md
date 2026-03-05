# sleepx

`sleep` with a progress bar. Inline, single-line, no fullscreen TUI.

```
[●●●●●●●●●●●●●●●○○○○○○○○○○○○○○○] 12s elapsed, 18s remaining ( 40%)
[●●●●●●●●●●●●●●●●●●●●●●●●●●●●●●] 30s elapsed, Done!
```

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
sleepx 1m30s       # 1 minute 30 seconds
sleepx 2h5m        # 2 hours 5 minutes
```

### Options

| Flag                              | Description                                              |
|-----------------------------------|----------------------------------------------------------|
| `-S`, `--style <NAME>`            | Progress bar style: dot, block, hash, arrow [default: dot] |
| `--fill <CHAR>`                   | Custom fill character (overrides --style)                |
| `--empty <CHAR>`                  | Custom empty character (overrides --style)               |
| `-b`, `--bar`                     | Force show progress bar (even in non-TTY)                |
| `--no-bar`                        | Don't show progress bar (auto-enabled in non-TTY)        |
| `-s`, `--say`                     | Use macOS `say` to announce when done                    |
| `-n`, `--min-interval <SECONDS>`  | Min output interval in non-TTY mode [default: 1]         |
| `-x`, `--max-interval <SECONDS>`  | Max output interval in non-TTY mode [default: 60]        |
| `-h`                              | Print help                                               |
| `-V`                              | Print version                                            |

### Styles

```bash
sleepx 30                    # dot (default)
# [●●●●●●●●●●○○○○○○○○○○○○○○○○○○○○] 12s elapsed, 18s remaining ( 40%)

sleepx 30 --style block      # block
# [██████████░░░░░░░░░░░░░░░░░░░░] 12s elapsed, 18s remaining ( 40%)

sleepx 30 --style hash       # hash
# [##########--------------------] 12s elapsed, 18s remaining ( 40%)

sleepx 30 --style arrow      # arrow
# [=========>--------------------] 12s elapsed, 18s remaining ( 40%)

sleepx 30 --fill "★" --empty "☆"  # custom
# [★★★★★★★★★★☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆☆] 12s elapsed, 18s remaining ( 40%)

sleepx 30 --no-bar           # text only
# 12s elapsed, 18s remaining ( 40%)
```

### Non-TTY Mode

When piped or redirected (non-TTY), `sleepx` automatically adapts:

- **No progress bar** by default (text only) — prevents noisy output in logs
- **One line per update** instead of overwriting — safe for `tee`, `>>`, etc.
- **Dynamic interval** — updates less frequently as remaining time increases (1s for short timers, up to 60s for long ones)

```bash
# Piped output: text only, one line per update
sleepx 30 | tee timer.log
# 1s elapsed, 29s remaining (  3%)
# 6s elapsed, 24s remaining ( 20%)
# ...

# Force progress bar even when piped
sleepx 30 --bar | cat
# [●●●●●●●●●●○○○○○○○○○○○○○○○○○○○○] 12s elapsed, 18s remaining ( 40%)

# Control output frequency
sleepx 1h --min-interval 5 --max-interval 30 2>&1 | tee -a build.log
```

### Examples

```bash
sleepx 5m --say            # 5 min timer, speak when done
sleepx 5m --min-interval 2 --max-interval 10  # output every 2-10s in non-TTY
```

## Environment Variables

All flags can be configured via environment variables. CLI flags take priority.

| Variable              | Flag               | Example              |
|-----------------------|--------------------|----------------------|
| `SLEEPX_STYLE`        | `--style`          | `block`              |
| `SLEEPX_FILL`         | `--fill`           | `★`                  |
| `SLEEPX_EMPTY`        | `--empty`          | `☆`                  |
| `SLEEPX_BAR`          | `--bar`            | `true`               |
| `SLEEPX_NO_BAR`       | `--no-bar`         | `true`               |
| `SLEEPX_SAY`          | `--say`            | `true`               |
| `SLEEPX_MIN_INTERVAL` | `--min-interval`   | `2`                  |
| `SLEEPX_MAX_INTERVAL` | `--max-interval`   | `10`                 |

```bash
# Set defaults in your shell profile
export SLEEPX_STYLE=block
# CLI flags override environment variables
sleepx 30 --style hash  # uses hash, not block
```

## Features

- Single-line inline display — no fullscreen, no alternate screen buffer
- Shows elapsed and remaining time with percentage
- Multiple progress bar styles (dot, block, hash, arrow) or fully custom characters
- `--no-bar` for text-only output
- Human-friendly duration parsing (`30`, `1m30s`, `2h5m10s`)
- Graceful Ctrl+C handling
- macOS `say` integration (`--say`)
- TTY-aware output: `\r` overwrite in terminal, line-per-update when piped
- Dynamic output interval in non-TTY mode (adjusts based on remaining time, configurable with `--min-interval` / `--max-interval`)

## License

MIT
