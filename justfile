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
    cargo run -- 3 --no-bell
