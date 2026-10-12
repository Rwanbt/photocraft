# PhotoCraft test baseline — 2026-10-12

Base `main`: `0b4b1e4286436a3ce4cd8bb14199174fbc7482a5`. Work is isolated in draft [PR #1](https://github.com/Rwanbt/photocraft/pull/1).

Git-tree inventory: 25 Rust crates, 3 app manifests and xtask; 1,109 Rust files, 178 directly nested integration-test files and 16 fuzz targets. **These file counts are not test counts or coverage.** The fork had zero reported GitHub Actions runs before this PR.

First LLVM baseline: [geometry run 38267483131](https://github.com/Rwanbt/photocraft/actions/runs/38267483131), SHA `0ca14ce6860d4b72dcf94554a4a3146b6f9f42bf`. **23 geometry library tests passed, none failed**. JSON and LCOV coverage reports were uploaded to artifact `photocraft-geom-llvm-coverage` (ID 11755745928). The exact percentages belong to the artifact and the run summary, not to this file-count inventory. Full-workspace coverage and full test collection remain **unmeasured**.

The `Geometry coverage` workflow runs `cargo llvm-cov -p photocraft-geom --lib --locked --json --output-path coverage.json` followed by `cargo llvm-cov report --lcov --output-path coverage.lcov`. Expand to per-crate/feature/OS baselines and exclude only justified untestable code. Do not enforce a 100% gate without an instrumented baseline.
