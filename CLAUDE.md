# CLAUDE.md

Guidance for Claude Code (and other coding agents) working in this repo.
Read README.md for what the app does and CONTRIBUTING.md for the rules on
adding a printer model. This file is the short version of both, plus the
things that are easy to get wrong.

## What this is

escp-utility: Printing Preferences and maintenance (Nozzle Check, Head
Cleaning, status and ink levels) for Epson inkjet printers on Linux, laid out
like the Windows Epson L800 driver. Rust (edition 2024) and Slint 1.18
(fluent style, winit backend). MIT.

## Commands

```sh
cargo build --release
cargo test                                   # unit, golden, integration, e2e
cargo test -- --ignored                      # GUI start-up smoke test (opens a window for ~3 s)
cargo clippy --all-targets -- -D warnings    # must be clean
PPD_FIXTURES=DIR ESCPR_PPD_DIR=DIR cargo test   # extra tests on Epson PPDs (CONTRIBUTING.md)
cargo run --example screenshot -- OUT_DIR    # offscreen render of every window
cargo run -- --dry-run                       # run the app, jobs go to ~/.cache/escp-utility/
```

Tests that need Epson files (`PPD_FIXTURES`, `ESCPR_PPD_DIR`,
`PPD_LIVE_QUEUE`) skip when the variable is unset. Never commit those files.

## Layout

| Path | What |
|---|---|
| `src/main.rs` | CLI flags, Slint wiring, the maintenance flows (pre-check, confirm, send, progress) |
| `src/commands.rs` | ESC/P2 remote-mode job builders (`NC`, `CH`), each byte next to its evidence |
| `src/transport.rs` | Send a job as one raw CUPS job (`lp -o raw`), or write it to a file in dry-run |
| `src/printer.rs` | Epson queues from `lpstat -v`, IEEE 1284 device ID from `/sys/class/usbmisc/lpN`, matched by USB serial |
| `src/profile.rs`, `profiles.toml` | Per-model maintenance data keyed by the device ID `MDL`; generic fallback |
| `src/status/` | Read-only status: `d4.rs` (IEEE 1284.4 client), `usblp.rs` (`/dev/usb/lpN`), `st2.rs` (`@BDC ST2` parser), `mod.rs` (messages, pre-check), `e2e.rs` (fake printer on a pty) |
| `src/settings.rs` | L800 Printing Preferences: Windows tables mapped to PPD/CUPS options, saved with `lpoptions` |
| `src/ppd.rs`, `src/ppd_sheet.rs` | Any other PPD: parse options and `*UIConstraints`, keep the selection conflict-free, drive the same pages |
| `src/report.rs` | `--report` for contributors (serial masked) |
| `src/texts.rs`, `src/layouts.rs` | Dialog texts (our wording) and generated sub-dialog rectangles |
| `ui/*.slint`, `ui/icons/` | The windows and our own SVG icons |
| `tests/golden/<MDL>/` | Golden job bytes; `tests/cli.rs` binary tests with fake `lpstat`/`lpoptions`/`lp` on `PATH` |

Comments that cite `phase2/...`, `rea/...` or `MULTI-MODEL-PLAN` point to the
original research notes, which are not in the repo. `docs/DESIGN.md` has the
evidence that matters.

## Hard rules

- **Nothing reaches a real printer from tests or from exploring.** No `lp`/`lpr`
  jobs, no writes to `/dev/usb/lp*`, no `lpoptions -o`/`lpadmin` on real queues.
  Use `--dry-run`, `--dump-dir`, the fake CLI shims in `tests/cli.rs`, or the
  pty printer in `src/status/e2e.rs`. A test that would touch hardware is
  `#[ignore]` with a reason.
- **No guessed bytes.** Every byte sent to a printer is written out with its
  source (driver resource, capture, Epson's Linux tools) and covered by a
  golden. New commands start in `--dry-run`. Print Head Alignment stays
  disabled (it writes permanent calibration values).
- **The status request only on an idle queue** (`lpstat -o` empty), and open,
  query, close per poll: usblp allows one opener at a time.
- **Keep the L800 byte- and pixel-identical.** The L800 goldens, the L800
  status tests and the screenshot renders change only when that is the point
  of the change.
- **No Epson assets.** No PPDs, driver binaries, bitmaps, icons or copied
  texts. UI texts are our own wording; icons are our own SVGs.
- **No real serial numbers** in tests or fixtures; use a fake like
  `1234567X8901234567`.

## Style

- Smallest design that works. No options, abstractions or branches without
  something real behind them (a printer reply, a PPD, a user action).
- Short, plain comments; cite the evidence for printer behaviour.
- UI is English. Commit messages are English, imperative, no attribution
  trailers.
- Before calling a change done: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
  and for UI changes compare `examples/screenshot` renders before and after.
