//! End-to-end tests that run the compiled `sleepx` binary.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_sleepx");

const ENV_VARS: [&str; 11] = [
    "SLEEPX_STYLE",
    "SLEEPX_BAR_LEFT",
    "SLEEPX_BAR_FILL",
    "SLEEPX_BAR_TIP",
    "SLEEPX_BAR_EMPTY",
    "SLEEPX_BAR_RIGHT",
    "SLEEPX_BAR",
    "SLEEPX_COLOR",
    "SLEEPX_SAY",
    "SLEEPX_MIN_INTERVAL",
    "SLEEPX_MAX_INTERVAL",
];

/// A `sleepx` command isolated from the caller's `SLEEPX_*` settings.
fn sleepx() -> Command {
    let mut cmd = Command::new(BIN);
    for var in ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

/// Upper bound for any single run; a regression that sleeps for real
/// (e.g. reading `5M` as months) fails instead of hanging the suite.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `cmd` to completion, killing it and failing the test after `RUN_TIMEOUT`.
fn output_with_timeout(mut cmd: Command) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn sleepx");
    let deadline = Instant::now() + RUN_TIMEOUT;
    while child.try_wait().expect("poll sleepx").is_none() {
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out sleepx");
            let out = child.wait_with_output().expect("collect output");
            panic!("sleepx timed out; stderr: {}", stderr_of(&out));
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("collect output")
}

fn run(args: &[&str]) -> Output {
    let mut cmd = sleepx();
    cmd.args(args);
    output_with_timeout(cmd)
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Asserts a usage error: exit code 2, clap-style `error:` prefix, no panic.
fn assert_usage_error(args: &[&str], expected_fragment: &str) {
    let out = run(args);
    let stderr = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "args {args:?}, stderr: {stderr}"
    );
    assert!(
        stderr.starts_with("error:"),
        "args {args:?}, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "args {args:?}, stderr: {stderr}"
    );
    assert!(
        stderr.contains(expected_fragment),
        "args {args:?}: expected {expected_fragment:?} in stderr: {stderr}"
    );
}

// ---- argument validation ----

/// Sad: a non-finite duration is a usage error, not a crash.
///
/// Scenario: a user types `sleepx inf` expecting coreutils' sleep-forever.
/// Purpose: non-finite input used to panic with exit 101; it must fail cleanly.
/// Means: run the binary with `inf`, `nan` and `infinity`.
/// Checks: exit code 2; stderr starts with `error:`, mentions "finite", no panic.
#[test]
fn rejects_non_finite_durations_without_panicking() {
    for input in ["inf", "nan", "infinity"] {
        assert_usage_error(&[input], "finite");
    }
}

/// Boundary: a duration beyond what `Duration` can hold is a usage error.
///
/// Scenario: a user types `sleepx 1e30`.
/// Purpose: the value is finite but too large to convert; it must not panic.
/// Means: run the binary with `1e30`.
/// Checks: exit code 2; stderr mentions "too large".
#[test]
fn rejects_duration_too_large_to_represent() {
    assert_usage_error(&["1e30"], "too large");
}

/// Sad: text that is not a duration is a usage error.
///
/// Scenario: a user mistypes `sleepx abc`.
/// Purpose: the message should say what a valid duration looks like.
/// Means: run the binary with `abc`.
/// Checks: exit code 2; stderr mentions "invalid duration".
#[test]
fn rejects_unparsable_duration() {
    assert_usage_error(&["abc"], "invalid duration");
}

/// Sad: non-finite output intervals are usage errors, not crashes.
///
/// Scenario: `SLEEPX_MIN_INTERVAL=inf` or a typo like `-x nan`.
/// Purpose: converting these to a Duration used to panic.
/// Means: pass `inf` and `nan` to each interval flag.
/// Checks: exit code 2; stderr mentions "finite".
#[test]
fn rejects_non_finite_intervals_without_panicking() {
    for flag in ["--min-interval", "--max-interval"] {
        for value in ["inf", "nan"] {
            assert_usage_error(&["0.1", flag, value], "finite");
        }
    }
}

/// Sad: zero, negative and sub-nanosecond intervals are rejected.
///
/// Scenario: `-n 0`, `-n -1`, or `-n 1e-10 -x 1e-10`.
/// Purpose: a zero interval would schedule output with no gap at all.
/// Means: pass each value as both bounds.
/// Checks: exit code 2; stderr mentions "positive".
#[test]
fn rejects_intervals_that_are_not_positive() {
    for value in ["0", "-1", "1e-10"] {
        assert_usage_error(
            &[
                "0.1",
                &format!("--min-interval={value}"),
                &format!("--max-interval={value}"),
            ],
            "positive",
        );
    }
}

/// Sad: a minimum interval above the maximum is a usage error.
///
/// Scenario: `-n 5 -x 2`.
/// Purpose: the clamp would be meaningless; report it like other usage errors.
/// Means: run with min 5 and max 2.
/// Checks: exit code 2; stderr mentions "must not exceed".
#[test]
fn rejects_min_interval_above_max_interval() {
    assert_usage_error(&["0.1", "-n", "5", "-x", "2"], "must not exceed");
}

/// Boundary: equal minimum and maximum intervals are allowed.
///
/// Scenario: `-n 2 -x 2` for a fixed output cadence.
/// Purpose: the min <= max check must not reject the equal case.
/// Means: run a short sleep with both bounds at 2.
/// Checks: exit code 0.
#[test]
fn accepts_min_interval_equal_to_max_interval() {
    let out = run(&["0.1", "-n", "2", "-x", "2"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
}

/// Sad: every bar character option takes exactly one character.
///
/// Scenario: `--bar-fill ab` or `--bar-left ""`.
/// Purpose: a wrong width would break the bar layout; reject it up front.
/// Means: pass a two-character value to each bar character option.
/// Checks: exit code 2; stderr mentions "single character".
#[test]
fn rejects_bar_characters_that_are_not_single_characters() {
    for flag in [
        "--bar-left",
        "--bar-fill",
        "--bar-tip",
        "--bar-empty",
        "--bar-right",
    ] {
        assert_usage_error(&["0.1", flag, "ab"], "single character");
    }
    assert_usage_error(&["0.1", "--bar-left", ""], "single character");
}

// ---- output streams ----

/// Happy: progress goes to stderr and stdout stays empty.
///
/// Scenario: `x=$(sleepx 30)` in a script, or `sleepx 5 && next` in a pipeline.
/// Purpose: progress is diagnostic output; capturing stdout must not pick it up.
/// Means: run a 0.2s sleep with both streams captured.
/// Checks: exit 0; stdout is empty; stderr contains the "Done!" line.
#[test]
fn writes_progress_to_stderr_and_nothing_to_stdout() {
    let out = run(&["0.2"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert!(stderr.contains("Done!"), "stderr: {stderr}");
}

/// Sad: a closed progress pipe ends the run quietly with status 141.
///
/// Scenario: `sleepx 5 2>&1 | head -1`.
/// Purpose: writing to a closed pipe used to panic; exit like a SIGPIPE death.
/// Means: read the first stderr line, close the reader, wait for the process.
/// Checks: exit status 141.
#[test]
fn exits_quietly_with_141_when_progress_reader_goes_away() {
    let mut child = sleepx()
        .arg("5")
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn sleepx");
    let mut reader = BufReader::new(child.stderr.take().expect("stderr is piped"));
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read first progress line");
    drop(reader);

    let status = child.wait().expect("wait for sleepx");
    assert_eq!(status.code(), Some(141), "first line: {first_line}");
}

// ---- signals ----

/// Starts a long sleep, waits until it prints (so its signal handlers are
/// installed), sends `signal` via kill(1), and returns the exit status and
/// the rest of stderr.
fn status_after_signal(signal: &str) -> (ExitStatus, String) {
    let mut child = sleepx()
        .arg("30")
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn sleepx");
    let mut reader = BufReader::new(child.stderr.take().expect("stderr is piped"));
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("read first progress line");

    let kill = Command::new("kill")
        .args([format!("-{signal}"), child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(kill.success(), "kill -{signal} failed");

    let mut rest = String::new();
    reader
        .read_to_string(&mut rest)
        .expect("read remaining stderr");
    let status = child.wait().expect("wait for sleepx");
    (status, rest)
}

/// Happy: sleepx dies by the same signal it received.
///
/// Scenario: Ctrl+C, `kill`, or closing the terminal during a sleep, inside a
/// wrapper script that checks how its child ended.
/// Purpose: dying by the real signal lets the shell report 130/143/129 and
/// lets wrappers stop on the first Ctrl+C; re-raising SIGINT for every
/// signal hid SIGTERM and SIGHUP.
/// Means: send INT, HUP and TERM to a 30s sleep.
/// Checks: the process is terminated by that signal, with no exit code.
#[test]
fn dies_by_the_signal_it_received() {
    for (name, number) in [("INT", 2), ("HUP", 1), ("TERM", 15)] {
        let (status, stderr) = status_after_signal(name);
        assert_eq!(status.signal(), Some(number), "{name}, stderr: {stderr}");
        assert_eq!(status.code(), None, "{name}, stderr: {stderr}");
    }
}

/// Happy: the last progress line is printed before dying.
///
/// Scenario: a user presses Ctrl+C and wants to see how long it slept.
/// Purpose: the final line records elapsed time at the moment of interrupt.
/// Means: send INT to a 30s sleep and read the rest of stderr.
/// Checks: stderr ends with a progress line showing the 30.0s total.
#[test]
fn prints_final_progress_line_before_dying() {
    let (_, stderr) = status_after_signal("INT");
    assert!(
        stderr.trim_end().starts_with("30.0s - "),
        "stderr: {stderr}"
    );
}
