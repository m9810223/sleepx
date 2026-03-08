[default]
[group("just")]
list:
    {{ just_executable() }} --justfile {{ justfile() }} --fmt --unstable 2> /dev/null
    {{ just_executable() }} --justfile {{ justfile() }} --list --unsorted

[group("just")]
help RECIPE:
    {{ just_executable() }} --justfile {{ justfile() }} --usage {{ RECIPE }}

[group("just")]
evaluate *ARGS:
    {{ just_executable() }} --justfile {{ justfile() }} --evaluate {{ ARGS }}

[doc("Build debug binary")]
[group("dev")]
build:
    cargo build

[doc("Build release binary")]
[group("dev")]
release:
    cargo build --release

[doc("Run with arguments")]
[group("dev")]
run *ARGS:
    cargo run -- {{ ARGS }}

[doc("Run clippy linter")]
[group("dev")]
lint:
    cargo clippy -- -D warnings

[doc("Format code")]
[group("dev")]
fmt:
    cargo fmt

[doc("Check formatting without modifying")]
[group("dev")]
fmt-check:
    cargo fmt -- --check

[doc("Install to ~/.cargo/bin")]
[group("dev")]
install:
    cargo install --path .

[doc("Run a quick 3s demo")]
[group("dev")]
demo:
    cargo run -- 3

[doc("Demo all bar styles")]
[group("dev")]
demo-all:
    @echo "=== dot (default) ===" && cargo run -- 3
    @echo "=== block ===" && cargo run -- 3 --style block
    @echo "=== hash ===" && cargo run -- 3 --style hash
    @echo "=== arrow ===" && cargo run -- 3 --style arrow
    @echo "=== no-bar ===" && cargo run -- 3 --no-bar

[doc("Test TTY mode (interactive, with progress bar and milliseconds)")]
[group("test")]
test-tty *ARGS:
    cargo run -- {{ ARGS }}

[doc("Test non-TTY mode (piped, no bar, no milliseconds)")]
[group("test")]
test-no-tty *ARGS:
    cargo run -- {{ ARGS }} | cat

[doc("Test both TTY and non-TTY modes")]
[group("test")]
test-modes:
    @echo "=== TTY mode (interactive) ===" && cargo run -- 3
    @echo ""
    @echo "=== non-TTY mode (piped) ===" && cargo run -- 3 | cat

[doc("Remove build artifacts")]
[group("dev")]
clean:
    cargo clean
