# Contributing

Thanks for contributing to `cryptoprice`.

## Development Setup

1. Install stable Rust (1.85+; edition 2024).
2. Clone the repository.
3. Build once to confirm your environment:

```sh
cargo build
```

## Local Quality Checks

Run the project CI script before committing:

```sh
bash ./scripts/ci.sh
```

This runs:

- `cargo fmt --all --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked`

## Pre-commit Hook (Recommended)

Enable the repository-managed hook so checks run automatically on commit:

```sh
git config core.hooksPath .githooks
```

## Code Guidelines

The conventions below match how the current codebase is written in `src/`.
Follow these patterns unless the PR is intentionally refactoring them.

- Keep network I/O async; use `tokio` + `reqwest` and never `reqwest::blocking`.
- Keep orchestration in `src/main.rs`; put provider logic in `src/provider/*`, formatting in `src/output/*`, and conversion logic in `src/calc.rs`.
- Implement new providers behind the `PriceProvider` trait (`name`, `id`, `get_prices`) in `src/provider/mod.rs`.
- Prefer batched provider requests when an API supports it (single request for multiple symbols).
- Use the unified `crate::error::Error` and `crate::error::Result<T>` across modules.
- Avoid `unwrap()` in non-test code; in rare startup/invariant spots, use explicit `expect(...)` messages.
- Keep public items documented with brief doc comments.
- Keep modules focused and relatively small (rough target: around 300 lines max per file).
- Use `tracing` for diagnostics: `info` for app flow, `debug` for request/response metadata, `trace` for payload-level details.
- Keep machine-readable output on stdout (`--json`), and keep logs on stderr.
- Preserve output contracts: table output in `src/output/table.rs`, JSON serialization in `src/output/json.rs`.
- Keep symbol/currency normalization explicit (`to_uppercase`/`to_lowercase`) at API boundaries.
- When changing behavior, update docs (`README.md`, this file) in the same PR.

## Testing Guidelines

- Add or update tests for behavior changes.
- Prefer unit tests with fixture JSON strings for parsing/validation logic.
- Do not make live network calls in default tests.
- For HTTP behavior tests, prefer mock-based tests (for example with `wiremock`).

## Pull Requests

- Keep PRs focused and small when possible.
- Include a clear description of what changed and why.
- Ensure CI passes before requesting review.

## Release Notes

Tag releases with a `v*` tag (for example `v0.2.0`).

Tag pushes trigger automated release workflows that:

- run tests,
- publish platform binaries,
- publish Docker images to GHCR.
