# Contributing

## Adding a printer model

A model needs a profile only for what differs from the generic baseline
(`NC 00`, one Head Cleaning `CH 00`): extra cleaning types such as Power
Cleaning, a different nozzle check parameter, or the Windows texts. Only
values that were measured go into `profiles.toml`; nothing is guessed.

1. **Report.** With the printer on USB and its CUPS queue idle, run
   `escp-utility --report --queue NAME` and attach the output. It shows the
   IEEE 1284 device ID (the `MDL` is the profile key), the PPD NickName,
   `lpoptions -p NAME -l` and one status reply as hex, with the serial number
   masked. It reads the status once and sends no job.
2. **Evidence.** Show where each value comes from: the model's Windows driver
   resources (the job scripts and the cleaning-type table, read for
   interoperability), or a usbmon capture of the Windows utility running the
   job in a virtual machine. Name the source for each byte, as
   `src/commands.rs` does.
3. **Profile and golden.** Add a `[[printer]]` entry (the fields are
   documented at the top of `profiles.toml`), with `tested = ""`. Run the app
   with `--dry-run --profile FILE`, compare the dumps in
   `~/.cache/escp-utility/` with the evidence, and add them as goldens:
   `tests/golden/<MDL>/nozzle-check.hex` and
   `tests/golden/<MDL>/head-cleaning-<ch>.hex` (hex bytes, `#` comment lines
   allowed). The tests rebuild each golden from the profile with the clock in
   its own `TI` bytes.
4. **Hardware run.** Run the profile on the printer with
   `escp-utility --profile FILE`: Nozzle Check, then every cleaning type.
   Then fill `tested` with who ran it, when, and the firmware version if
   known. Until then the app shows "Experimental on this model".

A lint test checks the profiles: each MDL once, the first cleaning `ch 0`,
no `ch` twice, and `windows_family` "G5P" or "". A non-zero `ch` always asks
for confirmation before it runs.

## Tests

```sh
cargo test                        # unit tests and goldens
cargo test -- --ignored           # start-up smoke test (opens a window for 3 s)
cargo clippy --all-targets        # no warnings
```

Tests that need files we may not ship read them from these variables and
skip when they are unset:

| Variable | Contents |
|---|---|
| `PPD_FIXTURES` | A folder with `L800.ppd`, the L800 queue's PPD as cupsd serves it (`curl http://localhost:631/printers/<queue>.ppd`) |
| `ESCPR_PPD_DIR` | The PPDs of Epson's open-source escpr driver (epson-inkjet-printer-escpr), one `.ppd` per model |
| `PPD_LIVE_QUEUE` | A queue name: one read-only PPD download from the running cupsd |

Never commit Epson files (PPDs, driver binaries, bitmaps, icons). The
screenshot example (`cargo run --example screenshot -- DIR`) renders every
window offscreen; check a UI change against its renders before and after.

## Code style

- No over-engineering: the smallest design that works, no options or
  abstractions without a present use, no invented cases. Every field and
  branch must be reachable by something real (a printer reply, a PPD, a user
  action).
- Every byte sent to a printer is written out next to its evidence, and is
  covered by a golden test.
- Nothing is sent to a printer except on an explicit user action, and the
  status request only while the CUPS queue has no jobs. New commands start
  in `--dry-run`.
- Keep the L800 byte- and pixel-identical: the job goldens, the L800 status
  tests and the screenshot renders must not change unless that is the point
  of the change.
- Plain, short comments and texts; user-facing texts in our own words.
