use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

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

    /// Progress bar style: dot, block, hash, arrow
    #[arg(short = 'S', long, default_value = "dot", env = "SLEEPX_STYLE")]
    style: BarStyle,

    /// Custom fill character (overrides --style)
    #[arg(long, env = "SLEEPX_FILL")]
    fill: Option<String>,

    /// Custom empty character (overrides --style)
    #[arg(long, env = "SLEEPX_EMPTY")]
    empty: Option<String>,

    /// Force show progress bar (even in non-TTY)
    #[arg(short = 'b', long, conflicts_with = "no_bar", env = "SLEEPX_BAR")]
    bar: bool,

    /// Don't show progress bar, only text (auto-enabled in non-TTY)
    #[arg(long, env = "SLEEPX_NO_BAR")]
    no_bar: bool,

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
enum BarStyle {
    Dot,
    Block,
    Hash,
    Arrow,
}

impl BarStyle {
    fn fill_char(&self) -> &str {
        match self {
            BarStyle::Dot => "●",
            BarStyle::Block => "█",
            BarStyle::Hash => "#",
            BarStyle::Arrow => "=",
        }
    }

    fn empty_char(&self) -> &str {
        match self {
            BarStyle::Dot => "○",
            BarStyle::Block => "░",
            BarStyle::Hash => "-",
            BarStyle::Arrow => "-",
        }
    }

    fn is_arrow(&self) -> bool {
        matches!(self, BarStyle::Arrow)
    }
}

struct BarChars {
    fill: String,
    empty: String,
    is_arrow: bool,
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

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else if mins > 0 {
        format!("{:02}:{:02}", mins, secs)
    } else {
        format!("{}s", secs)
    }
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

fn render_progress(
    elapsed: Duration,
    total: Duration,
    bar_width: usize,
    chars: &BarChars,
    no_bar: bool,
) -> String {
    let progress = if total.as_secs_f64() > 0.0 {
        (elapsed.as_secs_f64() / total.as_secs_f64()).min(1.0)
    } else {
        1.0
    };

    let remaining = if total > elapsed {
        total - elapsed
    } else {
        Duration::ZERO
    };

    let elapsed_str = format_duration(elapsed);
    let remaining_str = format_duration(remaining);
    let pct = (progress * 100.0) as u32;
    let time_info = format!(
        "{} elapsed, {} remaining ({:3}%)",
        elapsed_str, remaining_str, pct
    );

    if no_bar {
        return time_info;
    }

    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_content = if chars.is_arrow && filled > 0 && empty > 0 {
        format!(
            "{}>{}",
            chars.fill.repeat(filled - 1),
            chars.empty.repeat(empty),
        )
    } else {
        format!("{}{}", chars.fill.repeat(filled), chars.empty.repeat(empty))
    };

    format!("[{}] {}", bar_content, time_info)
}

fn render_done(total: Duration, bar_width: usize, chars: &BarChars, no_bar: bool) -> String {
    let elapsed_str = format_duration(total);
    let done_text = format!("{} elapsed, Done!", elapsed_str);

    if no_bar {
        return done_text;
    }

    format!("[{}] {}", chars.fill.repeat(bar_width), done_text,)
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

    if let Some(ref f) = cli.fill
        && f.chars().count() != 1
    {
        eprintln!("Error: --fill must be a single character");
        std::process::exit(1);
    }
    if let Some(ref e) = cli.empty
        && e.chars().count() != 1
    {
        eprintln!("Error: --empty must be a single character");
        std::process::exit(1);
    }

    let has_custom_fill = cli.fill.is_some();
    let chars = BarChars {
        fill: cli
            .fill
            .unwrap_or_else(|| cli.style.fill_char().to_string()),
        empty: cli
            .empty
            .unwrap_or_else(|| cli.style.empty_char().to_string()),
        is_arrow: !has_custom_fill && cli.style.is_arrow(),
    };

    // Handle Ctrl+C gracefully
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    })
    .expect("Failed to set Ctrl+C handler");

    let bar_width = 30;
    let start = Instant::now();
    let is_tty = io::stdout().is_terminal();
    let no_bar = if cli.bar {
        false // --bar: force show
    } else if cli.no_bar {
        true // --no-bar: force hide
    } else {
        !is_tty // default: TTY shows bar, non-TTY hides bar
    };
    let mut last_print = Instant::now() - Duration::from_secs(9999); // force first print

    loop {
        if interrupted.load(Ordering::SeqCst) {
            if is_tty {
                print!("\r\x1b[K");
                io::stdout().flush().ok();
            }
            println!("Interrupted.");
            std::process::exit(130);
        }

        let elapsed = start.elapsed();

        if elapsed >= total {
            break;
        }

        if is_tty {
            let output = render_progress(elapsed, total, bar_width, &chars, no_bar);
            print!("\r{}\x1b[K", output);
            io::stdout().flush().ok();
            thread::sleep(Duration::from_millis(100));
        } else {
            let remaining = total.saturating_sub(elapsed);
            let interval = non_tty_interval(remaining, min_interval, max_interval);

            if last_print.elapsed() >= interval {
                let output = render_progress(elapsed, total, bar_width, &chars, no_bar);
                println!("{}", output);
                last_print = Instant::now();
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    // Final state: 100%
    let done_msg = render_done(total, bar_width, &chars, no_bar);

    if is_tty {
        print!("\r{}\x1b[K\n", done_msg);
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
