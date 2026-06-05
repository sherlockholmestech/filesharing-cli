# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 command-line application. The binary is named `fsc` and starts at `src/main.rs`. Core modules live directly under `src/`: `download.rs`, `http.rs`, `progress.rs`, `settings.rs`, `style.rs`, and `token.rs`. Provider integrations are grouped in `src/providers/`; add new services as their own module and export them from `src/providers/mod.rs`. Build artifacts are written to `target/` and should not be edited or committed.

## Build, Test, and Development Commands

- `cargo build` compiles the CLI in debug mode.
- `cargo run -- providers` runs the local binary and lists supported providers.
- `cargo run -- upload <FILE> --provider <NAME>` tests an upload flow against a provider.
- `cargo test` runs unit and integration tests.
- `cargo fmt --check` verifies Rust formatting; use `cargo fmt` to apply it.
- `cargo clippy --all-targets --all-features -- -D warnings` checks for lints that should be fixed before review.
- `cargo install --path .` installs the local `fsc` binary.

## Coding Style & Naming Conventions

Follow standard `rustfmt` formatting with four-space indentation. Use `snake_case` for functions, variables, modules, and filenames; use `PascalCase` for types and provider structs. Prefer `anyhow::Result` for fallible command flows, keep user-facing errors clear, and avoid logging tokens or provider credentials. Match existing async patterns with `tokio`, `reqwest`, and small provider-specific helpers.

## Testing Guidelines

There is no dedicated `tests/` directory yet. Add focused unit tests beside the code they exercise with `#[cfg(test)] mod tests`, or create integration tests under `tests/` for CLI-level behavior. Name tests after the behavior being checked, such as `parses_pixeldrain_share_url`. Run `cargo test`, `cargo fmt --check`, and `cargo clippy --all-targets --all-features -- -D warnings` before submitting changes.

## Commit & Pull Request Guidelines

Recent commits use short Conventional Commit-style subjects, such as `feat: new providers`. Use concise subjects like `fix: handle missing download filename` or `refactor: simplify provider lookup`. Pull requests should describe the change, mention affected providers or commands, list validation performed, and link related issues when available. Include terminal output or screenshots only when CLI behavior or formatting changes.
