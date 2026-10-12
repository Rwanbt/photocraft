# Feature/test traceability — initial risk-based slice

A test source file is not execution evidence; VERIFIED requires the applicable behavior and environment to be checked at the exact commit.

| ID | Feature / contract | Risk | Unit evidence | Integration/E2E evidence | Status |
| --- | --- | --- | --- | --- | --- |
| GEO-001 | `Rect::from_xywh`: unsigned dimensions never wrap signed boundaries | Incorrect crop/export | Large-extent regressions, 23/23 geometry tests passed on SHA `0ca14ce` | Not measured | PARTIAL |
| GEO-002 | `TileCoord::rect`: extreme tile indexes do not overflow | Panic / incorrect tile | Negative and extreme index regressions, same run | Not measured | PARTIAL |
| GEO-003 | `Affine`: composition, inverse, nonfinite numbers | Invalid transforms | Existing nominal tests; invalid-float review open | Not consolidated | PARTIAL |
| FMT-001 | PSD/.pcraft import-export and corruption recovery | Data loss | Tests/fuzz files identified | Full corpus CI pending | UNVERIFIED |
| ENG-001 | Command validation, undo/redo, `panic_hunt` | Crash/corruption | Tests exist | Ignored panic-hunt job not validated | UNVERIFIED |
| CLI-001 | CLI, MCP/control, malformed commands | Incorrect automation | Tests exist | Actual CLI E2E not consolidated | UNVERIFIED |
| UI-001 | Native image-edit/reopen/export workflow | Document loss | Tests exist | UI E2E not confirmed | UNVERIFIED |
| WEB-001 | WASM build and interactive browser editing | Browser regression | Existing WASM workflow | Browser E2E not confirmed | UNVERIFIED |

This matrix must expand from the actual code; it does not certify the full application.
