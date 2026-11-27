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

// ---- environment variables ----

/// Runs a short sleep with `SLEEPX_SAY` set and no `say` on PATH, so a true
/// value shows up as a warning instead of speech. Returns code and stderr.
fn run_with_say_env(value: &str) -> (Option<i32>, String) {
    let mut cmd = sleepx();
    cmd.arg("0.1")
        .env("SLEEPX_SAY", value)
        .env("PATH", "/nonexistent");
    let out = output_with_timeout(cmd);
    (out.status.code(), stderr_of(&out))
}

/// Happy: `SLEEPX_SAY` accepts common spellings of "on".
///
/// Scenario: `export SLEEPX_SAY=1` in a shell profile.
/// Purpose: only `true`/`false` used to parse; `1` or `yes` failed with exit 2.
/// Means: set `1`, `yes`, `on` and `TRUE`, with `say` missing from PATH.
/// Checks: exit 0, and the missing-`say` warning proves the flag is on.
#[test]
fn say_env_accepts_boolish_true_values() {
    for value in ["1", "yes", "on", "TRUE"] {
        let (code, stderr) = run_with_say_env(value);
        assert_eq!(code, Some(0), "SLEEPX_SAY={value}, stderr: {stderr}");
        assert!(
            stderr.contains("`say`"),
            "SLEEPX_SAY={value}, stderr: {stderr}"
        );
    }
}

/// Happy: `SLEEPX_SAY` accepts common spellings of "off", and empty.
///
/// Scenario: `SLEEPX_SAY=0`, or `SLEEPX_SAY=` left blank in a profile.
/// Purpose: an empty value must behave like an unset variable, not an error.
/// Means: set `0`, `no`, `off`, `false` and the empty string.
/// Checks: exit 0 and no `say` warning.
#[test]
fn say_env_accepts_boolish_false_values_and_empty() {
    for value in ["0", "no", "off", "false", ""] {
        let (code, stderr) = run_with_say_env(value);
        assert_eq!(code, Some(0), "SLEEPX_SAY={value:?}, stderr: {stderr}");
        assert!(
            !stderr.contains("`say`"),
            "SLEEPX_SAY={value:?}, stderr: {stderr}"
        );
    }
}

/// Sad: an unrecognized `SLEEPX_SAY` value is a usage error.
///
/// Scenario: `SLEEPX_SAY=maybe`.
/// Purpose: a typo should be reported, not silently read as off.
/// Means: set `maybe`.
/// Checks: exit 2 with an `error:` prefix.
#[test]
fn say_env_rejects_unrecognized_value() {
    let (code, stderr) = run_with_say_env("maybe");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(stderr.starts_with("error:"), "stderr: {stderr}");
}

// ---- duration arguments ----

/// Happy: several durations are added together.
///
/// Scenario: `sleepx 1m 30s`, as coreutils `sleep` allows.
/// Purpose: a second argument used to be rejected as unexpected.
/// Means: run with `0.1 0.2`.
/// Checks: exit 0; the final line shows a 0.3s total.
#[test]
fn sums_multiple_durations_like_coreutils_sleep() {
    let out = run(&["0.1", "0.2"]);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert!(stderr.contains("0.3s - 0.0s = Done!"), "stderr: {stderr}");
}

/// Sad: at least one duration is required.
///
/// Scenario: a bare `sleepx`.
/// Purpose: clap should name the missing argument.
/// Means: run with no arguments.
/// Checks: usage error mentioning `<DURATION>`.
#[test]
fn requires_at_least_one_duration() {
    assert_usage_error(&[], "<DURATION>");
}

/// Boundary: a sum beyond what `Duration` can hold is a usage error.
///
/// Scenario: `sleepx 1.8e19 1.8e19`; each fits, the total does not.
/// Purpose: adding must not overflow or panic.
/// Means: run with two values just under `Duration::MAX`.
/// Checks: usage error mentioning "too large".
#[test]
fn rejects_sum_too_large_to_represent() {
    assert_usage_error(&["1.8e19", "1.8e19"], "too large");
}

/// Sad: negative numbers get a duration error, not "unexpected argument".
///
/// Scenario: `sleepx -1`.
/// Purpose: clap used to treat `-1` as an unknown flag.
/// Means: run with `-1` and `-0.5`.
/// Checks: usage error mentioning "must not be negative".
#[test]
fn rejects_negative_numbers_with_a_clear_message() {
    for input in ["-1", "-0.5"] {
        assert_usage_error(&[input], "must not be negative");
    }
}

/// Sad: an uppercase `M` is rejected instead of meaning months.
///
/// Scenario: `sleepx 5M`, almost always meant as five minutes.
/// Purpose: humantime reads `M` as months, so this silently slept ~5 months.
/// Means: run with `5M` (the run timeout guards against a real sleep).
/// Checks: usage error suggesting `m` for minutes.
#[test]
fn rejects_uppercase_m_instead_of_reading_it_as_months() {
    assert_usage_error(&["5M"], "'m' for minutes");
}

// ---- say and help ----

/// Sad: a missing `say` is reported at startup, and the sleep still runs.
///
/// Scenario: `SLEEPX_SAY=1` in a profile shared with a Linux machine.
/// Purpose: the warning used to appear only after the whole sleep.
/// Means: run `--say` with PATH pointing nowhere.
/// Checks: exit 0; stderr starts with the missing-`say` warning.
#[test]
fn warns_at_startup_when_say_is_missing() {
    let mut cmd = sleepx();
    cmd.args(["0.1", "--say"]).env("PATH", "/nonexistent");
    let out = output_with_timeout(cmd);
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert!(
        stderr.starts_with("warning: `say` not found"),
        "stderr: {stderr}"
    );
}

fn help_text(flag: &str) -> String {
    let out = run(&[flag]);
    assert_eq!(out.status.code(), Some(0), "{flag} failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Happy: short help shows the tool's own description and examples.
///
/// Scenario: a first-time user runs `sleepx -h`.
/// Purpose: `-h` showed only Cargo's one-line description and no examples.
/// Means: run `-h`.
/// Checks: it mentions the equation layout and has an "Examples:" section.
#[test]
fn short_help_uses_the_tool_description_and_examples() {
    let help = help_text("-h");
    assert!(help.contains("total - remaining = elapsed"), "help: {help}");
    assert!(help.contains("Examples:"), "help: {help}");
}

/// Happy: help names values by meaning.
///
/// Scenario: reading `--help` to learn what each option takes.
/// Purpose: names like `<BAR_FILL>` or `<MIN_INTERVAL>` repeat the flag.
/// Means: run `--help`.
/// Checks: it shows `<DURATION>...`, `<CHAR>` and `<SECONDS>`.
#[test]
fn help_names_values_by_meaning() {
    let help = help_text("--help");
    for name in ["<DURATION>...", "<CHAR>", "<SECONDS>"] {
        assert!(help.contains(name), "missing {name} in help: {help}");
    }
}

/// Happy: help lists env vars without their current values.
///
/// Scenario: `--help` with no `SLEEPX_*` set.
/// Purpose: `[env: SLEEPX_STYLE=]` looks like a broken empty assignment.
/// Means: run `--help`.
/// Checks: `[env: SLEEPX_STYLE]` appears; `SLEEPX_STYLE=` does not.
#[test]
fn help_lists_env_vars_without_values() {
    let help = help_text("--help");
    assert!(help.contains("[env: SLEEPX_STYLE]"), "help: {help}");
    assert!(!help.contains("SLEEPX_STYLE="), "help: {help}");
}

/// Happy: long help explains how output intervals are chosen.
///
/// Scenario: `-n 0.5` seems to have no effect.
/// Purpose: the base gap steps from 1s to 60s; the flags only clamp it.
/// Means: run `--help`.
/// Checks: it mentions "1s to 60s".
#[test]
fn help_explains_interval_range() {
    let help = help_text("--help");
    assert!(help.contains("1s to 60s"), "help: {help}");
}
