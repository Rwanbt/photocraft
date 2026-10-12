# Remaining production-readiness risks — 2026-10-12

| Priority | Risk / missing evidence | Next gate |
| --- | --- | --- |
| P0 | Full workspace line/function/branch coverage unknown | Expand LLVM artifacts per crate |
| P0 | Malformed PSD/.pcraft, lossless save/reopen and oracle fidelity not fully verified | Real-file corpus, corrupt/truncated fixtures |
| P1 | Ignored command `panic_hunt` is outside ordinary test collection | Explicit adverse-parameter CI job |
| P1 | Native UI save/close/reopen/export interactions not measured as E2E | Desktop smoke + interaction automation |
| P1 | Browser interactive flows not measured | Real-browser E2E and console errors |
| P1 | macOS platform not tested on regular PRs | Platform release or dedicated gate |
| P1 | Deterministic fuzz replay and performance budgets not consolidated | Seed/corpus replay and budget checks |
| P2 | Half-open `i32` rectangles cannot represent an exclusive edge greater than `i32::MAX` | Audit saturation-aware downstream callers |

Do not merge PR #1 or assert production-ready until applicable gates pass. No upstream PR or forced Git update.
