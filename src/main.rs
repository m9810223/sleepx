use std::io::{self, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

/// A sleep command with an inline progress bar and countdown display.
#[derive(Parser)]
#[command(name = "sleepp", version, about)]
struct Cli {
    /// Duration to sleep. Examples: 30, 1m30s, 2h5m
    duration: String,

    /// Use macOS `say` to announce when done
    #[arg(long)]
    say: bool,

    /// Don't ring terminal bell at the end
    #[arg(long)]
    no_bell: bool,
}

fn parse_duration(input: &str) -> Result<Duration, String> {
    // Try parsing as plain number (seconds)
    if let Ok(secs) = input.parse::<f64>() {
        if secs <= 0.0 {
            return Err("Duration must be positive".to_string());
        }
        return Ok(Duration::from_secs_f64(secs));
    }

    // Try parsing as humantime format (e.g. 1m30s, 2h5m)
    humantime::parse_duration(input).map_err(|e| format!("Invalid duration '{}': {}", input, e))
}

fn format_remaining(remaining: Duration) -> String {
    let total_secs = remaining.as_secs();
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

fn render_progress_bar(elapsed: Duration, total: Duration, bar_width: usize) -> String {
    let progress = if total.as_secs_f64() > 0.0 {
        (elapsed.as_secs_f64() / total.as_secs_f64()).min(1.0)
    } else {
        1.0
    };

    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);

    let remaining = if total > elapsed {
        total - elapsed
    } else {
        Duration::ZERO
    };

    let remaining_str = format_remaining(remaining);
    let pct = (progress * 100.0) as u32;

    format!(
        "\r[{}{}] {} remaining ({:3}%)",
        "#".repeat(filled),
        "-".repeat(empty),
        remaining_str,
        pct,
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

    // Handle Ctrl+C gracefully
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    })
    .expect("Failed to set Ctrl+C handler");

    let bar_width = 30;
    let start = Instant::now();

    loop {
        if interrupted.load(Ordering::SeqCst) {
            // Clear the line and print newline before exiting
            print!("\r{}\r", " ".repeat(bar_width + 40));
            io::stdout().flush().ok();
            println!("Interrupted.");
            std::process::exit(130);
        }

        let elapsed = start.elapsed();

        if elapsed >= total {
            break;
        }

        let bar = render_progress_bar(elapsed, total, bar_width);
        print!("{}", bar);
        io::stdout().flush().ok();

        thread::sleep(Duration::from_millis(100));
    }

    // Final state: 100%
    print!(
        "\r[{}] Done!{}\n",
        "#".repeat(bar_width),
        " ".repeat(20), // clear trailing characters
    );
    io::stdout().flush().ok();

    // Terminal bell
    if !cli.no_bell {
        print!("\x07");
        io::stdout().flush().ok();
    }

    // macOS say
    if cli.say {
        Command::new("say").arg("Timer complete").spawn().ok();
    }
}
