use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use terminal_size::{Width, terminal_size};

/// A sleep command with an inline progress bar and countdown display.
///
/// In TTY mode, the progress bar overwrites the same line.
/// In non-TTY mode (piped/redirected), outputs one line per update
/// with no progress bar by default.
#[derive(Parser)]
#[command(name = "sleepx", version, about)]
struct Cli {
    /// Duration to sleep. Examples: 30, 1m30s, 2h5m
    duration: String,

    /// Progress bar style: arrow, dot, block, hash
    #[arg(short = 'S', long, default_value = "arrow", env = "SLEEPX_STYLE")]
    style: BarStyle,

    /// Custom left bracket character (default: [)
    #[arg(long, default_value = "[", env = "SLEEPX_BAR_LEFT")]
    bar_left: String,

    /// Custom fill character (overrides --style)
    #[arg(long, env = "SLEEPX_BAR_FILL")]
    bar_fill: Option<String>,

    /// Custom tip character for progress indicator (e.g., >)
    #[arg(long, env = "SLEEPX_BAR_TIP")]
    bar_tip: Option<String>,

    /// Custom empty character (overrides --style)
    #[arg(long, env = "SLEEPX_BAR_EMPTY")]
    bar_empty: Option<String>,

    /// Custom right bracket character (default: ])
    #[arg(long, default_value = "]", env = "SLEEPX_BAR_RIGHT")]
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
    #[arg(short = 'n', long, default_value = "1", env = "SLEEPX_MIN_INTERVAL")]
    min_interval: f64,

    /// Max output interval in seconds for non-TTY mode
    #[arg(short = 'x', long, default_value = "60", env = "SLEEPX_MAX_INTERVAL")]
    max_interval: f64,
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

fn parse_duration(input: &str) -> Result<Duration, String> {
    // Try parsing as plain number (seconds)
    if let Ok(secs) = input.parse::<f64>() {
        if secs < 0.0 {
            return Err("Duration must not be negative".to_string());
        }
        return Ok(Duration::from_secs_f64(secs));
    }

    // Try parsing as humantime format (e.g. 1m30s, 2h5m)
    humantime::parse_duration(input).map_err(|e| format!("Invalid duration '{}': {}", input, e))
}

fn hour_digits(total: Duration) -> usize {
    let h = total.as_secs() / 3600;
    if h == 0 { 2 } else { (h.ilog10() as usize + 1).max(2) }
}

fn format_duration_fixed(d: Duration, total: Duration) -> String {
    let total_secs = total.as_secs();
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;

    if total_secs >= 3600 {
        let w = hour_digits(total);
        let raw = format!("{:0>w$}:{:02}:{:02}", h, m, s);
        prettify_duration(&raw)
    } else if total_secs >= 60 {
        let raw = format!("{:02}:{:02}", m, s);
        prettify_duration(&raw)
    } else if total_secs >= 10 {
        format!("{:2}s", s)
    } else {
        format!("{}s", s)
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
    if secs >= 3600 {
        hour_digits(total) + 6 // H..H:MM:SS
    } else if secs >= 60 { 5 } // MM:SS
    else if secs >= 10 { 3 }   // NNs
    else { 2 }                  // Ns
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

fn get_terminal_width() -> usize {
    terminal_size()
        .map(|(Width(w), _)| (w as usize).saturating_sub(1))
        .unwrap_or(79)
}

/// Clear previously rendered output that may have wrapped across multiple lines.
fn clear_wrapped_lines(prev_output_len: usize) {
    let term_width = get_terminal_width();
    if prev_output_len > 0 && term_width > 0 {
        let lines = (prev_output_len + term_width - 1) / term_width;
        if lines > 1 {
            // Move cursor up to the first wrapped line
            print!("\x1b[{}A", lines - 1);
        }
    }
    // Return to column 1 and clear from cursor to end of display
    print!("\r\x1b[J");
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

    // Truncate to whole seconds and derive remaining so the equation always holds
    let total_secs = total.as_secs();
    let elapsed_secs = elapsed.as_secs().min(total_secs);
    let remaining_secs = total_secs - elapsed_secs;

    let total_str = format_duration_fixed(total, total);
    let elapsed_str = format_duration_fixed(Duration::from_secs(elapsed_secs), total);
    let remaining_str = format_duration_fixed(Duration::from_secs(remaining_secs), total);
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

fn main() {
    let cli = Cli::parse();

    let total = match parse_duration(&cli.duration) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if cli.min_interval <= 0.0 {
        eprintln!("Error: --min-interval must be positive");
        std::process::exit(1);
    }
    if cli.max_interval <= 0.0 {
        eprintln!("Error: --max-interval must be positive");
        std::process::exit(1);
    }
    if cli.min_interval > cli.max_interval {
        eprintln!(
            "Error: --min-interval ({}) must not exceed --max-interval ({})",
            cli.min_interval, cli.max_interval
        );
        std::process::exit(1);
    }

    let min_interval = Duration::from_secs_f64(cli.min_interval);
    let max_interval = Duration::from_secs_f64(cli.max_interval);

    if let Some(ref f) = cli.bar_fill
        && f.chars().count() != 1
    {
        eprintln!("Error: --bar-fill must be a single character");
        std::process::exit(1);
    }
    if let Some(ref e) = cli.bar_empty
        && e.chars().count() != 1
    {
        eprintln!("Error: --bar-empty must be a single character");
        std::process::exit(1);
    }
    if cli.bar_left.chars().count() != 1 {
        eprintln!("Error: --bar-left must be a single character");
        std::process::exit(1);
    }
    if cli.bar_right.chars().count() != 1 {
        eprintln!("Error: --bar-right must be a single character");
        std::process::exit(1);
    }
    if let Some(ref t) = cli.bar_tip
        && t.chars().count() != 1
    {
        eprintln!("Error: --bar-tip must be a single character");
        std::process::exit(1);
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

    // Handle Ctrl+C gracefully
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    })
    .expect("Failed to set Ctrl+C handler");

    let start = Instant::now();
    let is_tty = io::stdout().is_terminal();
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
        if interrupted.load(Ordering::SeqCst) {
            let elapsed = start.elapsed().min(total);
            let output = render_progress(elapsed, total, &chars, no_bar, theme);
            if is_tty {
                clear_wrapped_lines(prev_display_width);
                print!("{}\x1b[K\n", output);
            } else {
                println!("{}", output);
            }
            io::stdout().flush().ok();
            // Re-raise SIGINT so the parent shell sees a signal death,
            // not just a non-zero exit (avoids double Ctrl+C in wrappers).
            unsafe {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::raise(libc::SIGINT);
            }
        }

        let elapsed = start.elapsed();

        if elapsed >= total {
            break;
        }

        if is_tty {
            let output = render_progress(elapsed, total, &chars, no_bar, theme);
            clear_wrapped_lines(prev_display_width);
            print!("{}\x1b[K", output);
            io::stdout().flush().ok();
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
                println!("{}", output);
                last_print = Instant::now();
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    // Final state: 100%
    let done_msg = render_done(total, &chars, no_bar, theme);

    if is_tty {
        clear_wrapped_lines(prev_display_width);
        print!("{}\x1b[K\n", done_msg);
    } else {
        println!("{}", done_msg);
    }
    io::stdout().flush().ok();

    // macOS say
    if cli.say
        && let Err(e) = Command::new("say").arg("Timer complete").spawn()
    {
        eprintln!("Warning: failed to run `say`: {}", e);
    }
}
