# sleepp

`sleep` with a progress bar. Inline, single-line, no fullscreen TUI.

```
[###############---------------] 12s remaining ( 51%)
[##############################] Done!
```

## Install

### From GitHub

```bash
cargo install --git https://github.com/m9810223/sleepp
```

### From source

```bash
git clone https://github.com/m9810223/sleepp
cd sleepp
cargo install --path .
```

## Usage

```bash
sleepp 30          # 30 seconds
sleepp 1m30s       # 1 minute 30 seconds
sleepp 2h5m        # 2 hours 5 minutes
```

### Options

| Flag        | Description                           |
|-------------|---------------------------------------|
| `--say`     | Use macOS `say` to announce when done |
| `--no-bell` | Don't ring terminal bell at the end   |
| `-h`        | Print help                            |
| `-V`        | Print version                         |

### Examples

```bash
sleepp 5m --say            # 5 min timer, speak when done
sleepp 30 --no-bell        # 30 sec, silent
sleepp 1h --say --no-bell  # 1 hour, say only, no bell
```

## Features

- Single-line inline display — no fullscreen, no alternate screen buffer
- Progress bar with remaining time and percentage
- Human-friendly duration parsing (`30`, `1m30s`, `2h5m10s`)
- Graceful Ctrl+C handling
- Terminal bell on completion (disable with `--no-bell`)
- macOS `say` integration (`--say`)

## License

MIT
