use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use terminal_size::{Width, terminal_size_of};

/// A sleep command with an inline progress bar and countdown display.
///
/// In TTY mode, the progress bar overwrites the same line.
/// In non-TTY mode (piped/redirected), outputs one line per update
/// with no progress bar by default.
#[derive(Parser)]
#[command(name = "sleepx", version, about)]
struct Cli {
    /// Duration to sleep. Examples: 30, 1m30s, 2h5m
    #[arg(value_parser = parse_duration)]
    duration: Duration,

    /// Progress bar style: arrow, dot, block, hash
    #[arg(short = 'S', long, default_value = "arrow", env = "SLEEPX_STYLE")]
    style: BarStyle,

    /// Custom left bracket character (default: [)
    #[arg(long, default_value = "[", env = "SLEEPX_BAR_LEFT", value_parser = parse_single_char)]
    bar_left: String,

    /// Custom fill character (overrides --style)
    #[arg(long, env = "SLEEPX_BAR_FILL", value_parser = parse_single_char)]
    bar_fill: Option<String>,

    /// Custom tip character for progress indicator (e.g., >)
    #[arg(long, env = "SLEEPX_BAR_TIP", value_parser = parse_single_char)]
    bar_tip: Option<String>,

    /// Custom empty character (overrides --style)
    #[arg(long, env = "SLEEPX_BAR_EMPTY", value_parser = parse_single_char)]
    bar_empty: Option<String>,

    /// Custom right bracket character (default: ])
    #[arg(long, default_value = "]", env = "SLEEPX_BAR_RIGHT", value_parser = parse_single_char)]
    bar_right: String,

    /// Progress bar visibility: auto (default), on, off
    #[arg(short = 'b', long, default_value = "auto", env = "SLEEPX_BAR")]
    bar: BarVisibility,

    /// Color output: auto (default, TTY only), on, off
    #[arg(short = 'c', long, default_value = "auto", env = "SLEEPX_COLOR")]
    color: ColorMode,

    /// Use macOS `say` to announce when done
    #[arg(short = 's', long, env = "SLEEPX_SAY")]
    say: bool,

    /// Min output interval in seconds for non-TTY mode
    #[arg(short = 'n', long, default_value = "1", env = "SLEEPX_MIN_INTERVAL", value_parser = parse_interval)]
    min_interval: Duration,

    /// Max output interval in seconds for non-TTY mode
    #[arg(short = 'x', long, default_value = "60", env = "SLEEPX_MAX_INTERVAL", value_parser = parse_interval)]
    max_interval: Duration,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum BarVisibility {
    Auto,
    On,
    Off,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ColorMode {
    Auto,
    On,
    Off,
}

struct Theme {
    bar_fill: &'static str,
    bar_empty: &'static str,
    bracket: &'static str,
    remaining: &'static str,
    elapsed: &'static str,
    pct: &'static str,
    done: &'static str,
    reset: &'static str,
}

const COLOR_THEME: Theme = Theme {
    bar_fill: "\x1b[32m",       // green
    bar_empty: "\x1b[2m",       // dim
    bracket: "\x1b[2m",         // dim
    remaining: "\x1b[33m",      // yellow (countdown)
    elapsed: "\x1b[36m",        // cyan
    pct: "\x1b[1m",             // bold
    done: "\x1b[1;32m",         // bold green
    reset: "\x1b[0m",
};

const NO_COLOR: Theme = Theme {
    bar_fill: "",
    bar_empty: "",
    bracket: "",
    remaining: "",
    elapsed: "",
    pct: "",
    done: "",
    reset: "",
};

#[derive(Clone, Debug, clap::ValueEnum)]
enum BarStyle {
    Arrow,
    Dot,
    Block,
    Hash,
}

impl BarStyle {
    fn fill_char(&self) -> &str {
        match self {
            BarStyle::Arrow => "=",
            BarStyle::Dot => "●",
            BarStyle::Block => "█",
            BarStyle::Hash => "#",
        }
    }

    fn empty_char(&self) -> &str {
        match self {
            BarStyle::Arrow => " ",
            BarStyle::Dot => "○",
            BarStyle::Block => "░",
            BarStyle::Hash => "-",
        }
    }

    fn tip_char(&self) -> Option<&str> {
        match self {
            BarStyle::Arrow => Some(">"),
            _ => None,
        }
    }
}

struct BarChars {
    fill: String,
    empty: String,
    tip: Option<String>,
    bar_left: String,
    bar_right: String,
}

/// Converts seconds to a `Duration`, rejecting values `Duration` cannot hold.
fn secs_to_duration(secs: f64) -> Result<Duration, String> {
    if !secs.is_finite() {
        return Err("must be a finite number".to_string());
    }
    if secs < 0.0 {
        return Err("must not be negative".to_string());
    }
    Duration::try_from_secs_f64(secs).map_err(|_| "too large".to_string())
}

fn parse_duration(input: &str) -> Result<Duration, String> {
    // Try parsing as plain number (seconds)
    if let Ok(secs) = input.parse::<f64>() {
        return secs_to_duration(secs);
    }

    // Try parsing as humantime format (e.g. 1m30s, 2h5m)
    humantime::parse_duration(input).map_err(|_| {
        "invalid duration (expected seconds like 30 or 1.5, or units like 1m30s, 2h)".to_string()
    })
}

fn parse_interval(input: &str) -> Result<Duration, String> {
    let secs: f64 = input
        .parse()
        .map_err(|_| "expected a number of seconds".to_string())?;
    if secs.is_finite() && secs <= 0.0 {
        return Err("must be positive".to_string());
    }
    // A positive value below 1ns still converts to a zero Duration, which
    // would leave no gap between output lines.
    let interval = secs_to_duration(secs)?;
    if interval.is_zero() {
        return Err("must be positive".to_string());
    }
    Ok(interval)
}

fn parse_single_char(input: &str) -> Result<String, String> {
    if input.chars().count() != 1 {
        return Err("must be a single character".to_string());
    }
    Ok(input.to_string())
}

fn hour_digits(total: Duration) -> usize {
    let h = total.as_secs() / 3600;
    if h == 0 { 2 } else { (h.ilog10() as usize + 1).max(2) }
}

/// Width of the ".t" tenths suffix every format carries.
const TENTHS_WIDTH: usize = 2;

/// Drops everything below a tenth of a second.
fn truncate_to_tenth(d: Duration) -> Duration {
    let tenth_nanos = d.subsec_nanos() / 100_000_000 * 100_000_000;
    Duration::new(d.as_secs(), tenth_nanos)
}

/// Formats `d` in the layout chosen by `total`, to a tenth of a second
/// (truncated, so the display never runs ahead of real time).
fn format_duration_fixed(d: Duration, total: Duration) -> String {
    let total_secs = total.as_secs();
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let tenths = d.subsec_millis() / 100;

    if total_secs >= 3600 {
        let w = hour_digits(total);
        let raw = format!("{:0>w$}:{:02}:{:02}", h, m, s);
        format!("{}.{}", prettify_duration(&raw), tenths)
    } else if total_secs >= 60 {
        let raw = format!("{:02}:{:02}", m, s);
        format!("{}.{}", prettify_duration(&raw), tenths)
    } else if total_secs >= 10 {
        format!("{:2}.{}s", s, tenths)
    } else {
        format!("{}.{}s", s, tenths)
    }
}

/// Suppress leading "00:" groups, then replace single leading zero with space.
fn prettify_duration(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i + 2 < bytes.len()
        && bytes[i] == b'0'
        && bytes[i + 1] == b'0'
        && bytes[i + 2] == b':'
    {
        result.push_str("   ");
        i += 3;
    }
    if i < bytes.len() && bytes[i] == b'0' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()
    {
        result.push(' ');
        i += 1;
    }
    result.push_str(&s[i..]);
    result
}

fn duration_display_width(total: Duration) -> usize {
    let secs = total.as_secs();
    let whole = if secs >= 3600 {
        hour_digits(total) + 6 // H..H:MM:SS
    } else if secs >= 60 { 5 } // MM:SS
    else if secs >= 10 { 3 }   // NNs
    else { 2 };                 // Ns
    whole + TENTHS_WIDTH
}

fn non_tty_interval(remaining: Duration, min: Duration, max: Duration) -> Duration {
    let base = if remaining > Duration::from_secs(3600) {
        Duration::from_secs(60)
    } else if remaining > Duration::from_secs(600) {
        Duration::from_secs(30)
    } else if remaining > Duration::from_secs(60) {
        Duration::from_secs(10)
    } else if remaining > Duration::from_secs(10) {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(1)
    };

    base.clamp(min, max)
}

/// Usable width of the terminal behind stderr, where progress is drawn.
fn get_terminal_width() -> usize {
    terminal_size_of(io::stderr())
        .map(|(Width(w), _)| (w as usize).saturating_sub(1))
        .unwrap_or(79)
}

/// Escape sequence that returns to the start of the previous output, which
/// may have wrapped across several rows, and clears from there to the end.
fn clear_previous_output(prev_output_len: usize, term_width: usize) -> String {
    let mut seq = String::new();
    if prev_output_len > 0 && term_width > 0 {
        let lines = prev_output_len.div_ceil(term_width);
        if lines > 1 {
            // Move cursor up to the first wrapped line
            seq.push_str(&format!("\x1b[{}A", lines - 1));
        }
    }
    // Return to column 1 and clear from cursor to end of display
    seq.push_str("\r\x1b[J");
    seq
}

/// Exit status when the progress reader has gone away; this is what a shell
/// reports for a process killed by SIGPIPE (128 + 13).
const EXIT_BROKEN_PIPE: i32 = 141;

/// Exit status for any other failure to write progress.
const EXIT_WRITE_FAILED: i32 = 1;

fn write_error_exit_code(err: &io::Error) -> i32 {
    if err.kind() == io::ErrorKind::BrokenPipe {
        EXIT_BROKEN_PIPE
    } else {
        EXIT_WRITE_FAILED
    }
}

/// Writes progress text, exiting without a message if the stream is unusable:
/// progress is the only output, so there is nowhere left to report the error.
fn write_progress(out: &mut impl Write, text: &str) {
    if let Err(err) = out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        std::process::exit(write_error_exit_code(&err));
    }
}

fn render_progress(
    elapsed: Duration,
    total: Duration,
    chars: &BarChars,
    no_bar: bool,
    theme: &Theme,
) -> String {
    let progress = if total.as_secs_f64() > 0.0 {
        (elapsed.as_secs_f64() / total.as_secs_f64()).min(1.0)
    } else {
        1.0
    };

    // Truncate to tenths and derive remaining so the equation always holds
    let total_shown = truncate_to_tenth(total);
    let elapsed_shown = truncate_to_tenth(elapsed).min(total_shown);
    let remaining_shown = total_shown - elapsed_shown;

    let total_str = format_duration_fixed(total_shown, total);
    let elapsed_str = format_duration_fixed(elapsed_shown, total);
    let remaining_str = format_duration_fixed(remaining_shown, total);
    let pct = progress * 100.0;
    let pct_str = format!("{:5.1}%", pct);

    if no_bar {
        return format!(
            "{} - {}{}{} = {}{}{} {}{}{}",
            total_str,
            theme.remaining, remaining_str, theme.reset,
            theme.elapsed, elapsed_str, theme.reset,
            theme.pct, pct_str, theme.reset,
        );
    }

    // Layout: "{total} - {remaining} = {elapsed} {lb}{bar}{rb} {pct}"
    let lb = &chars.bar_left;
    let rb = &chars.bar_right;
    let dur_w = duration_display_width(total);
    let overhead = dur_w + 3              // total + " - "
        + dur_w + 3                      // remaining + " = "
        + dur_w + 1                      // elapsed + space
        + lb.chars().count() + rb.chars().count()
        + 1 + 6;                         // space + pct
    let bar_width = get_terminal_width()
        .saturating_sub(overhead)
        .max(10);

    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_content = if let Some(ref tip) = chars.tip {
        if filled > 0 && empty > 0 {
            format!(
                "{}{}{}{}{}{}{}",
                theme.bar_fill,
                chars.fill.repeat(filled - 1),
                tip,
                theme.reset,
                theme.bar_empty,
                chars.empty.repeat(empty),
                theme.reset,
            )
        } else {
            format!(
                "{}{}{}{}{}{}",
                theme.bar_fill,
                chars.fill.repeat(filled),
                theme.reset,
                theme.bar_empty,
                chars.empty.repeat(empty),
                theme.reset,
            )
        }
    } else {
        format!(
            "{}{}{}{}{}{}",
            theme.bar_fill,
            chars.fill.repeat(filled),
            theme.reset,
            theme.bar_empty,
            chars.empty.repeat(empty),
            theme.reset,
        )
    };

    format!(
        "{} - {}{}{} = {}{}{} {}{}{}{}{}{}{} {}{}{}",
        total_str,
        theme.remaining, remaining_str, theme.reset,
        theme.elapsed, elapsed_str, theme.reset,
        theme.bracket, lb, theme.reset,
        bar_content,
        theme.bracket, rb, theme.reset,
        theme.pct, pct_str, theme.reset,
    )
}

fn render_done(total: Duration, chars: &BarChars, no_bar: bool, theme: &Theme) -> String {
    let total_str = format_duration_fixed(total, total);
    let zero_str = format_duration_fixed(Duration::ZERO, total);
    let dur_w = duration_display_width(total);
    let done_w = dur_w.max(5);
    let done_padded = format!("{:<width$}", "Done!", width = done_w);
    let done_label = format!("{}{}{}", theme.done, done_padded, theme.reset);
    let pct_str = "100.0%";

    if no_bar {
        return format!(
            "{} - {}{}{} = {} {}{}{}",
            total_str,
            theme.remaining, zero_str, theme.reset,
            done_label,
            theme.pct, pct_str, theme.reset,
        );
    }

    let lb = &chars.bar_left;
    let rb = &chars.bar_right;
    let overhead = dur_w + 3              // total + " - "
        + dur_w + 3                      // remaining + " = "
        + done_w + 1                     // done_label + space
        + lb.chars().count() + rb.chars().count()
        + 1 + 6;                         // space + pct
    let bar_width = get_terminal_width()
        .saturating_sub(overhead)
        .max(10);

    format!(
        "{} - {}{}{} = {} {}{}{}{}{}{}{}{}{} {}{}{}",
        total_str,
        theme.remaining, zero_str, theme.reset,
        done_label,
        theme.bracket, lb, theme.reset,
        theme.bar_fill, chars.fill.repeat(bar_width), theme.reset,
        theme.bracket, rb, theme.reset,
        theme.pct, pct_str, theme.reset,
    )
}

/// Signals that end the sleep early.
const STOP_SIGNALS: [i32; 3] = [SIGINT, SIGTERM, SIGHUP];

/// Value of the stop flag while no stop signal has arrived.
const NO_SIGNAL: usize = 0;

/// Returns a flag that holds the number of the last stop signal received.
fn install_stop_handlers() -> Arc<AtomicUsize> {
    let received = Arc::new(AtomicUsize::new(NO_SIGNAL));
    for signal in STOP_SIGNALS {
        let value = usize::try_from(signal).expect("signal numbers are positive");
        signal_hook::flag::register_usize(signal, Arc::clone(&received), value)
            .unwrap_or_else(|err| panic!("failed to install handler for signal {signal}: {err}"));
    }
    received
}

/// Ends the process by the signal that stopped it, as if no handler had been
/// installed. The parent shell then sees a death by that signal (status
/// 128 + n), and wrapper scripts stop on the first Ctrl+C.
fn die_by_signal(signal: usize) -> ! {
    let signal = i32::try_from(signal).expect("signal numbers fit in i32");
    if let Err(err) = signal_hook::low_level::emulate_default_handler(signal) {
        eprintln!("warning: could not re-raise signal {signal}: {err}");
    }
    // Only reached if re-raising failed: fall back to the shell's status code.
    std::process::exit(128 + signal);
}

/// Disable `ECHOCTL` so the terminal does not echo `^C` on Ctrl+C.
/// Returns the original termios settings for later restoration.
fn disable_echoctl() -> Option<libc::termios> {
    unsafe {
        let mut orig: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut orig) != 0 {
            return None;
        }
        let mut modified = orig;
        modified.c_lflag &= !libc::ECHOCTL;
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &modified) != 0 {
            return None;
        }
        Some(orig)
    }
}

fn restore_termios(original: &libc::termios) {
    unsafe {
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
    }
}

fn main() {
    let cli = Cli::parse();

    let total = cli.duration;
    let min_interval = cli.min_interval;
    let max_interval = cli.max_interval;
    if min_interval > max_interval {
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                format!(
                    "--min-interval ({}s) must not exceed --max-interval ({}s)",
                    min_interval.as_secs_f64(),
                    max_interval.as_secs_f64()
                ),
            )
            .exit();
    }

    let chars = BarChars {
        fill: cli
            .bar_fill
            .unwrap_or_else(|| cli.style.fill_char().to_string()),
        empty: cli
            .bar_empty
            .unwrap_or_else(|| cli.style.empty_char().to_string()),
        tip: cli
            .bar_tip
            .or_else(|| cli.style.tip_char().map(|s| s.to_string())),
        bar_left: cli.bar_left,
        bar_right: cli.bar_right,
    };

    let stop_signal = install_stop_handlers();

    let start = Instant::now();
    // Progress goes to stderr so stdout stays empty for callers that capture it.
    let mut out = io::stderr();
    let is_tty = out.is_terminal();
    let orig_termios = if is_tty { disable_echoctl() } else { None };
    let no_bar = match cli.bar {
        BarVisibility::On => false,
        BarVisibility::Off => true,
        BarVisibility::Auto => !is_tty,
    };
    let theme = match cli.color {
        ColorMode::On => &COLOR_THEME,
        ColorMode::Off => &NO_COLOR,
        ColorMode::Auto => {
            if is_tty {
                &COLOR_THEME
            } else {
                &NO_COLOR
            }
        }
    };
    let mut last_print = Instant::now() - Duration::from_secs(9999); // force first print
    let mut prev_display_width: usize = 0;

    loop {
        let signal = stop_signal.load(Ordering::SeqCst);
        if signal != NO_SIGNAL {
            let elapsed = start.elapsed().min(total);
            let output = render_progress(elapsed, total, &chars, no_bar, theme);
            let line = if is_tty {
                let clear = clear_previous_output(prev_display_width, get_terminal_width());
                format!("{clear}{output}\x1b[K\n")
            } else {
                format!("{output}\n")
            };
            write_progress(&mut out, &line);
            if let Some(ref orig) = orig_termios {
                restore_termios(orig);
            }
            die_by_signal(signal);
        }

        let elapsed = start.elapsed();

        if elapsed >= total {
            break;
        }

        if is_tty {
            let output = render_progress(elapsed, total, &chars, no_bar, theme);
            let clear = clear_previous_output(prev_display_width, get_terminal_width());
            write_progress(&mut out, &format!("{clear}{output}\x1b[K"));
            prev_display_width = if no_bar {
                output.len()
            } else {
                get_terminal_width()
            };
            thread::sleep(Duration::from_millis(100));
        } else {
            let remaining = total.saturating_sub(elapsed);
            let interval = non_tty_interval(remaining, min_interval, max_interval);

            if last_print.elapsed() >= interval {
                let output = render_progress(elapsed, total, &chars, no_bar, theme);
                write_progress(&mut out, &format!("{output}\n"));
                last_print = Instant::now();
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    if let Some(ref orig) = orig_termios {
        restore_termios(orig);
    }

    // Final state: 100%
    let done_msg = render_done(total, &chars, no_bar, theme);

    if is_tty {
        let clear = clear_previous_output(prev_display_width, get_terminal_width());
        write_progress(&mut out, &format!("{clear}{done_msg}\x1b[K\n"));
    } else {
        write_progress(&mut out, &format!("{done_msg}\n"));
    }

    // macOS say
    if cli.say
        && let Err(e) = Command::new("say").arg("Timer complete").spawn()
    {
        eprintln!("Warning: failed to run `say`: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // === output helpers ===

    /// Boundary: output that fits one row needs no cursor-up.
    ///
    /// Scenario: redrawing a progress line in an 80-column terminal.
    /// Purpose: moving up when nothing wrapped would erase the line above.
    /// Means: previous lengths of 0 and exactly the terminal width.
    /// Checks: both give only "return and clear".
    #[test]
    fn clear_previous_output_without_wrapping() {
        assert_eq!(clear_previous_output(0, 80), "\r\x1b[J");
        assert_eq!(clear_previous_output(80, 80), "\r\x1b[J");
    }

    /// Boundary: one column past the width means one wrapped row.
    ///
    /// Scenario: the window was narrowed below the previous line's length.
    /// Purpose: the cursor must climb to the first row of the old output.
    /// Means: previous lengths of 81 and 160 at width 80, and 161.
    /// Checks: 81 and 160 move up 1 row; 161 moves up 2.
    #[test]
    fn clear_previous_output_climbs_wrapped_rows() {
        assert_eq!(clear_previous_output(81, 80), "\x1b[1A\r\x1b[J");
        assert_eq!(clear_previous_output(160, 80), "\x1b[1A\r\x1b[J");
        assert_eq!(clear_previous_output(161, 80), "\x1b[2A\r\x1b[J");
    }

    /// Sad: a write error maps to an exit status by its kind.
    ///
    /// Scenario: the progress reader closed the pipe, or the log disk is full.
    /// Purpose: a closed pipe should look like a SIGPIPE death (141); other
    /// failures exit 1.
    /// Means: classify a BrokenPipe and a PermissionDenied error.
    /// Checks: 141 and 1.
    #[test]
    fn write_error_exit_code_distinguishes_broken_pipe() {
        let broken = io::Error::from(io::ErrorKind::BrokenPipe);
        let other = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(write_error_exit_code(&broken), 141);
        assert_eq!(write_error_exit_code(&other), 1);
    }

    // === parse_duration ===

    #[test]
    fn parse_duration_plain_seconds() {
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn parse_duration_fractional_seconds() {
        assert_eq!(parse_duration("1.5").unwrap(), Duration::from_secs_f64(1.5));
    }

    #[test]
    fn parse_duration_zero() {
        assert_eq!(parse_duration("0").unwrap(), Duration::from_secs(0));
    }

    #[test]
    fn parse_duration_minutes_and_seconds() {
        assert_eq!(parse_duration("1m30s").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn parse_duration_hours_and_minutes() {
        assert_eq!(parse_duration("2h5m").unwrap(), Duration::from_secs(7500));
    }

    #[test]
    fn parse_duration_hours_minutes_seconds() {
        assert_eq!(parse_duration("1h2m3s").unwrap(), Duration::from_secs(3723));
    }

    #[test]
    fn parse_duration_negative() {
        let err = parse_duration("-5").unwrap_err();
        assert!(err.contains("negative"), "expected 'negative' in: {err}");
    }

    /// Sad: text that is not a duration explains what is expected.
    ///
    /// Scenario: a user mistypes `sleepx abc`.
    /// Purpose: clap prints this after "invalid value 'abc'", so it starts
    /// lowercase and names valid forms instead of humantime internals.
    /// Means: parse `abc`.
    /// Checks: the error mentions "invalid duration" and an example form.
    #[test]
    fn parse_duration_invalid() {
        let err = parse_duration("abc").unwrap_err();
        assert!(err.contains("invalid duration"), "got: {err}");
        assert!(err.contains("1m30s"), "got: {err}");
    }

    /// Sad: non-finite seconds are rejected instead of panicking.
    ///
    /// Scenario: `sleepx inf` or `sleepx nan`.
    /// Purpose: `Duration::from_secs_f64` panics on these values.
    /// Means: parse `inf`, `-inf` and `nan`.
    /// Checks: each error is "must be a finite number".
    #[test]
    fn parse_duration_rejects_non_finite() {
        for input in ["inf", "-inf", "nan"] {
            assert_eq!(
                parse_duration(input),
                Err("must be a finite number".to_string()),
                "input: {input}"
            );
        }
    }

    /// Boundary: values beyond `Duration::MAX` are "too large".
    ///
    /// Scenario: `sleepx 1e30`.
    /// Purpose: a finite but huge value must fail cleanly at the edge of the type.
    /// Means: convert just below and above `Duration::MAX` (about 1.8447e19s).
    /// Checks: 1.8e19 converts; 1.9e19 is "too large".
    #[test]
    fn secs_to_duration_rejects_values_beyond_duration_max() {
        assert!(secs_to_duration(1.8e19).is_ok());
        assert_eq!(secs_to_duration(1.9e19), Err("too large".to_string()));
    }

    /// Boundary: an interval must be at least one nanosecond.
    ///
    /// Scenario: `-n 1e-10 -x 1e-10`.
    /// Purpose: positive values below 1ns become a zero Duration.
    /// Means: parse values below, at and above 1ns, plus zero and negative.
    /// Checks: 1e-10, 0 and -0.5 are "must be positive"; 1e-9 and 2e-9 convert.
    #[test]
    fn parse_interval_requires_at_least_one_nanosecond() {
        for input in ["1e-10", "0", "-0.5"] {
            assert_eq!(
                parse_interval(input),
                Err("must be positive".to_string()),
                "input: {input}"
            );
        }
        assert_eq!(parse_interval("1e-9"), Ok(Duration::from_nanos(1)));
        assert_eq!(parse_interval("2e-9"), Ok(Duration::from_nanos(2)));
    }

    /// Sad: intervals take plain seconds, not humantime units.
    ///
    /// Scenario: `-n 1m`.
    /// Purpose: the flag is documented in seconds; say so instead of guessing.
    /// Means: parse `1m`.
    /// Checks: "expected a number of seconds".
    #[test]
    fn parse_interval_rejects_non_numbers() {
        assert_eq!(
            parse_interval("1m"),
            Err("expected a number of seconds".to_string())
        );
    }

    /// Boundary: bar characters count Unicode scalars, not bytes.
    ///
    /// Scenario: `--bar-fill ★` versus `--bar-fill ab` or an empty value.
    /// Purpose: multi-byte characters must be accepted as one character.
    /// Means: parse an empty string, one star, and two letters.
    /// Checks: only the star is accepted.
    #[test]
    fn parse_single_char_counts_unicode_scalars() {
        assert!(parse_single_char("").is_err());
        assert_eq!(parse_single_char("★"), Ok("★".to_string()));
        assert!(parse_single_char("ab").is_err());
    }

    // === hour_digits ===

    #[test]
    fn hour_digits_zero() {
        assert_eq!(hour_digits(Duration::from_secs(0)), 2);
    }

    #[test]
    fn hour_digits_under_one_hour() {
        assert_eq!(hour_digits(Duration::from_secs(3599)), 2);
    }

    #[test]
    fn hour_digits_exactly_one_hour() {
        assert_eq!(hour_digits(Duration::from_secs(3600)), 2);
    }

    #[test]
    fn hour_digits_ten_hours() {
        assert_eq!(hour_digits(Duration::from_secs(36000)), 2);
    }

    #[test]
    fn hour_digits_hundred_hours() {
        assert_eq!(hour_digits(Duration::from_secs(360000)), 3);
    }

    #[test]
    fn hour_digits_thousand_hours() {
        assert_eq!(hour_digits(Duration::from_secs(3600000)), 4);
    }

    // === format_duration_fixed ===

    /// Asserts `format_duration_fixed(d, total)` for each `(d, expected)` row.
    fn assert_formats(total: Duration, cases: &[(Duration, &str)]) {
        for &(d, expected) in cases {
            assert_eq!(
                format_duration_fixed(d, total),
                expected,
                "d: {d:?}, total: {total:?}"
            );
        }
    }

    /// Happy: totals under 10s show seconds with one decimal.
    ///
    /// Scenario: a short `sleepx 9` countdown.
    /// Purpose: the shortest format carries tenths without padding.
    /// Means: format several values against a 9s total.
    /// Checks: `0.0s`, `5.3s`, `9.0s`.
    #[test]
    fn format_duration_fixed_single_digit_seconds_shows_tenths() {
        assert_formats(
            Duration::from_secs(9),
            &[
                (Duration::ZERO, "0.0s"),
                (Duration::from_millis(5_300), "5.3s"),
                (Duration::from_secs(9), "9.0s"),
            ],
        );
    }

    /// Happy: totals of 10s to 59s pad seconds to two digits, plus tenths.
    ///
    /// Scenario: `sleepx 30`, where values cross from one to two digits.
    /// Purpose: fixed width keeps the line from shifting at 9.9s -> 10.0s.
    /// Means: format several values against a 30s total.
    /// Checks: ` 5.3s`, `12.0s`, `30.0s`.
    #[test]
    fn format_duration_fixed_two_digit_seconds_shows_tenths() {
        assert_formats(
            Duration::from_secs(30),
            &[
                (Duration::from_millis(5_300), " 5.3s"),
                (Duration::from_secs(12), "12.0s"),
                (Duration::from_secs(30), "30.0s"),
            ],
        );
    }

    /// Happy: minute-range totals use M:SS.t with leading zeros blanked.
    ///
    /// Scenario: `sleepx 2m`.
    /// Purpose: tenths follow the seconds field; prettified zeros stay blank.
    /// Means: format zero, 65.3s and 120s against a 120s total.
    /// Checks: `    0.0`, ` 1:05.3`, ` 2:00.0`.
    #[test]
    fn format_duration_fixed_minutes_shows_tenths() {
        assert_formats(
            Duration::from_secs(120),
            &[
                (Duration::ZERO, "    0.0"),
                (Duration::from_millis(65_300), " 1:05.3"),
                (Duration::from_secs(120), " 2:00.0"),
            ],
        );
    }

    /// Happy: hour-range totals use H:MM:SS.t with leading zeros blanked.
    ///
    /// Scenario: `sleepx 2h`.
    /// Purpose: the longest format also carries tenths.
    /// Means: format zero and 3661.3s against a 7200s total.
    /// Checks: `       0.0`, ` 1:01:01.3`.
    #[test]
    fn format_duration_fixed_hours_shows_tenths() {
        assert_formats(
            Duration::from_secs(7200),
            &[
                (Duration::ZERO, "       0.0"),
                (Duration::from_millis(3_661_300), " 1:01:01.3"),
            ],
        );
    }

    /// Boundary: three-digit hours widen the hour field.
    ///
    /// Scenario: a 100-hour sleep.
    /// Purpose: the hour field grows with the total instead of overflowing.
    /// Means: format 100h against a 100h total.
    /// Checks: `100:00:00.0`.
    #[test]
    fn format_duration_fixed_hundred_hours() {
        assert_formats(
            Duration::from_secs(360_000),
            &[(Duration::from_secs(360_000), "100:00:00.0")],
        );
    }

    /// Boundary: tenths are truncated, never rounded up.
    ///
    /// Scenario: 12.39s have passed.
    /// Purpose: the display must not run ahead of real time.
    /// Means: format values just below, at and above a tenth boundary.
    /// Checks: 12.299999999s -> `12.2s`, 12.3s -> `12.3s`, 12.399s -> `12.3s`.
    #[test]
    fn format_duration_fixed_truncates_tenths() {
        assert_formats(
            Duration::from_secs(30),
            &[
                (Duration::new(12, 299_999_999), "12.2s"),
                (Duration::from_millis(12_300), "12.3s"),
                (Duration::from_millis(12_399), "12.3s"),
            ],
        );
    }

    // === prettify_duration ===

    #[test]
    fn prettify_duration_no_leading_zeros() {
        assert_eq!(prettify_duration("12:34"), "12:34");
    }

    #[test]
    fn prettify_duration_single_leading_zero() {
        assert_eq!(prettify_duration("01:05"), " 1:05");
    }

    #[test]
    fn prettify_duration_double_zero_minutes() {
        assert_eq!(prettify_duration("00:05"), "    5");
    }

    #[test]
    fn prettify_duration_triple_zero_groups() {
        assert_eq!(prettify_duration("00:00:05"), "       5");
    }

    #[test]
    fn prettify_duration_all_zeros() {
        assert_eq!(prettify_duration("00:00:00"), "       0");
    }

    #[test]
    fn prettify_duration_hours_with_leading_zero() {
        assert_eq!(prettify_duration("01:02:03"), " 1:02:03");
    }

    #[test]
    fn prettify_duration_large_hours() {
        assert_eq!(prettify_duration("99:59:59"), "99:59:59");
    }

    // === duration_display_width ===

    /// Boundary: display width steps up at 10s, 60s, 3600s and 100h totals.
    ///
    /// Scenario: the bar width is computed from how wide the times print.
    /// Purpose: width must match `format_duration_fixed`, including ".t".
    /// Means: check totals on both sides of each format boundary.
    /// Checks: 9s->4, 10s->5, 59s->5, 60s->7, 3599s->7, 3600s->10, 100h->11.
    #[test]
    fn duration_display_width_matches_formatted_width() {
        let cases = [
            (9, 4),
            (10, 5),
            (59, 5),
            (60, 7),
            (3_599, 7),
            (3_600, 10),
            (360_000, 11),
        ];
        for (secs, expected) in cases {
            let total = Duration::from_secs(secs);
            assert_eq!(duration_display_width(total), expected, "total: {secs}s");
            assert_eq!(
                format_duration_fixed(total, total).len(),
                expected,
                "formatted width for total: {secs}s"
            );
        }
    }

    // === render_progress ===

    /// Happy: total minus remaining equals elapsed, to the tenth.
    ///
    /// Scenario: 12.34s into `sleepx 30` with the bar off.
    /// Purpose: the equation layout must stay arithmetically true with tenths.
    /// Means: render without bar or color at 12.34s of 30s.
    /// Checks: `30.0s - 17.7s = 12.3s  41.1%`.
    #[test]
    fn render_progress_equation_holds_in_tenths() {
        let chars = BarChars {
            fill: "#".to_string(),
            empty: "-".to_string(),
            tip: None,
            bar_left: "[".to_string(),
            bar_right: "]".to_string(),
        };
        let line = render_progress(
            Duration::from_millis(12_340),
            Duration::from_secs(30),
            &chars,
            true,
            &NO_COLOR,
        );
        assert_eq!(line, "30.0s - 17.7s = 12.3s  41.1%");
    }

    /// Boundary: a total that is not a whole number of tenths still balances.
    ///
    /// Scenario: `sleepx 0.35`, 0.34s in.
    /// Purpose: total is truncated like elapsed, so remaining never goes negative.
    /// Means: render without bar or color at 0.34s of 0.35s.
    /// Checks: `0.3s - 0.0s = 0.3s`.
    #[test]
    fn render_progress_truncates_total_like_elapsed() {
        let chars = BarChars {
            fill: "#".to_string(),
            empty: "-".to_string(),
            tip: None,
            bar_left: "[".to_string(),
            bar_right: "]".to_string(),
        };
        let line = render_progress(
            Duration::from_millis(340),
            Duration::from_millis(350),
            &chars,
            true,
            &NO_COLOR,
        );
        assert!(line.starts_with("0.3s - 0.0s = 0.3s"), "line: {line}");
    }

    // === non_tty_interval ===

    #[test]
    fn non_tty_interval_large_remaining() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(7200),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn non_tty_interval_fifteen_minutes() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(900),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn non_tty_interval_two_minutes() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(120),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn non_tty_interval_thirty_seconds() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(30),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn non_tty_interval_five_seconds() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(5),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn non_tty_interval_min_clamp() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(5),
                Duration::from_secs(3),
                Duration::from_secs(60)
            ),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn non_tty_interval_max_clamp() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(7200),
                Duration::from_secs(1),
                Duration::from_secs(10)
            ),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn non_tty_interval_boundary_3600() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(3600),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn non_tty_interval_boundary_60() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(60),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn non_tty_interval_boundary_10() {
        assert_eq!(
            non_tty_interval(
                Duration::from_secs(10),
                Duration::from_secs(1),
                Duration::from_secs(60)
            ),
            Duration::from_secs(1)
        );
    }
}
