# Repository Guidelines

## Project Structure & Module Organization

`src/main.rs` parses CLI args and delegates to `src/cli.rs`. `src/lib.rs` exports cache and decoder modules: `cache`, `js5`, `config`, `script`, `model`, `audio`, `interface`, `map`, `vfx`, `animator`, `cutscene2d`, `vars`, plus shared `constants` and `fixture`. Integration tests live in `tests/real_cache.rs`; they use external RS3 cache data and Java repo lookup files. `README.md` documents target snapshot, CLI examples, and expected unpack output.

## Build, Test, and Development Commands

- `cargo build`: compile debug binary for local iteration.
- `cargo build --release`: produce optimized extractor.
- `cargo run -- --help`: show CLI commands and flags.
- `cargo run -- unpack --out-dir /tmp/rs3-cache-rs-out --sample-models --skip-audio`: quick unpack smoke run with default paths.
- `cargo clippy --all-targets --all-features -- -D warnings`: enforce crate lint policy.
- `cargo fmt --all --check`: verify rustfmt output before review.
- `cargo test`: run integration tests; cache assets must exist or env overrides must point to them.

## Coding Style & Naming Conventions

Use Rust 2024, 4-space indentation, and rustfmt defaults. Keep `unsafe` absent; manifest forbids `unsafe_code`. Use `anyhow::Result<T>` for CLI and cache flows. Follow Rust naming: `snake_case` functions and modules, `PascalCase` types, `UPPER_SNAKE_CASE` constants. Keep parser and decoder modules focused by cache domain; add exports in `src/lib.rs` when new module is public.

## Testing Guidelines

Tests use Rust built-in harness and often return `anyhow::Result<()>`. Put cross-module tests in `tests/`; use clear behavior names like `parses_every_interface_file`. Set `RS3_CACHE_DIR`, `RS3_CACHE_TAR`, and `RS3_DATA_DIR` when cache assets are outside README defaults. Prefer targeted tests for new decoder behavior, plus one CLI smoke path when output shape changes.

## Commit & Pull Request Guidelines

Git history uses short imperative subjects, such as `Add 910 cache compatibility support` and `Remove parity artifacts`. Keep subject concise and scoped. PRs should state cache build tested, commands run, output directory or artifact touched, and linked issue when relevant. Include screenshots only for rendered assets such as interface or map image changes.

## Security & Configuration Tips

Do not commit cache dumps, generated unpack output, or local absolute paths. Keep large outputs under `/tmp` or ignored workspace dirs. Verify data-dir changes against `../rs3-cache/data` compatibility.
