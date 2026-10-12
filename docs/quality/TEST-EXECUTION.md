# Verification and reproducibility

Rust 1.95+ is required. No Rust toolchain was available in the present local sandbox, so **no local cargo command was executed**. GitHub Actions is the authoritative verification environment.

```sh
cargo fmt --all -- --check
cargo test -p photocraft-geom --locked --lib
cargo llvm-cov -p photocraft-geom --lib --locked --json --output-path coverage.json
cargo llvm-cov report --lcov --output-path coverage.lcov
cargo test --workspace --locked --features heif
cargo clippy --workspace --all-targets --locked --features heif -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo xtask scorecard --check
cargo xtask test-corpus
```

The existing CI installs platform packages, pinned corpus inputs and fonts as documented in `AGENTS.md` and `docs/development.md`. macOS is release-tested, not run on ordinary PR CI. Track exact SHA, job conclusion, test counts and coverage artifact for each iteration; skipped/blocked never means passed.
