# Epson L800 Utility for Linux

A small Rust + [Slint](https://slint.dev) (fluent style) desktop app that
reproduces the **Maintenance** tab of the Windows Epson L800 printer driver: the
same layout, labels, buttons and step-by-step dialogs. Everything shown was
derived from the Windows driver's own resources and code (driver family "G5P").

It contains no Epson code, icons or bitmaps. The pictures in `ui/icons/` are
our own SVGs, drawn at the size and position where Windows draws its bitmaps.
The long instruction texts are written in our own words, with the same meaning
as the Windows texts.

## Safety first

- Nothing is sent when the app starts. A job is sent only when you click
  **Print** (Nozzle Check), **Start** (Head Cleaning) or **OK** (Power Cleaning
  confirmation).
- Jobs go to CUPS as one raw job, `lp -d L800 -o raw`. This is the same as
  Windows, which spools a `RAW` job. CUPS keeps the USB device, so the app does
  not compete with the CUPS usb backend or with Epson's `ecbd`.
- **`--dry-run`** never calls `lp`. It writes the exact job bytes to a file and
  shows a hexdump.
- A nozzle check uses one sheet of paper. A cleaning uses ink, and a Power
  Cleaning uses much more ink. As of this version, none of these jobs has
  been sent to a real printer. Try `--dry-run` first.

## What works

| Maintenance item | State |
|---|---|
| Nozzle Check | Works. Windows flow: 210 → "Please wait..." → 211 (Good / Cleaning needed) → [Clean] starts Head Cleaning |
| Head Cleaning | Works. Flow 226 → (Power Cleaning: 360 confirm) → "Please wait..." → 227 → [Print Nozzle Check Pattern] |
| Power Cleaning | Works, as the second radio option in Head Cleaning, with the transportation-lock warning (dialog 360) |
| Printer and Option Information | Shows what CUPS reports for the queue (`lpstat -l -p/-v/-a/-o`), read-only |
| EPSON Status Monitor 3 | Disabled ("Not available on Linux yet"): needs the bidirectional status channel |
| Print Head Alignment | Disabled: deferred (it writes persistent values with `DD`/`SV`) |
| Paper Guide Cleaning | Disabled |
| Speed & Progress..., Driver Update | Disabled. Monitoring Preferences lives inside Speed & Progress on Windows |
| Language | English only (the driver has no other UI strings) |

The Windows Maintenance tab has no Print Queue button for this driver, so this
app has none either.

### No status channel yet: the Windows "no status" path

Windows reads the printer status over IEEE 1284.4 before and during a job. When
it gets no status reply, it takes a simpler path, and this app follows that
path:

- Head Cleaning uses dialog **226** (same layout as 220) and finishes with
  **227** (same layout as 224). It shows the text saying cleaning is done when
  the power light stops flashing.
- There is no progress dialog (223). Windows shows that one only when it can
  read the expected cleaning time from the status reply (field 0x0C).
- Before sending, Windows checks the spooler. If the printer is "Work Offline"
  or "Paused", it shows a communication error (string 3000). Here, a disabled
  (paused) CUPS queue, or one that is not accepting jobs, gives the same error.
  In `--dry-run`, this check is only a note in the hexdump window.
- Status-based pre-checks ("A problem has occurred", "Now cleaning!") are left
  out until status exists. The worker in `src/main.rs` (`run_job`) is where a
  `status` module will plug in.
- The "Please wait..." popup (dialog 201) stays up for at least 0.7 s, so it
  does not just flicker. Windows keeps it up while it reads status.

## Build and run

```sh
cargo build --release
cargo test
./target/release/epson-l800-utility --dry-run      # safe: writes files, never calls lp
./target/release/epson-l800-utility                # live: sends to the CUPS queue "L800"
```

Options:

```
--dry-run       write each job to a file and show a hexdump; never calls lp
--dump-dir DIR  folder for dry-run files (default: ~/.cache/epson-l800-utility)
--queue NAME    CUPS queue of the printer (default: L800)
```

No `-dev` packages are needed. fontconfig is loaded at run time
(`fontique/fontconfig-dlopen`). The UI font is Liberation Sans 11 px, which
stands in for Tahoma 8 pt.

Desktop entry: `~/.local/share/applications/epson-l800-utility.desktop` runs the
release binary. Its right-click action "Dry Run" starts the app with `--dry-run`.

Layout screenshots without a display:
`cargo run --example screenshot -- OUT_DIR` writes one PPM per window, using
Slint's software renderer.

## Commands and evidence

The Windows utility (`E_IHUTG5P.DLL`) builds each job by running a small
script from the model data DLL (`E_IHBRG5P.DLL`, RT_RCDATA 1800 = nozzle check,
1802 = head cleaning). The interpreter is `FUN_004083dc`. `FUN_004082ec` then
appends the job-end trailer (string 4302). The bytes are in `src/commands.rs`,
with one comment per opcode and source. Unit tests compare them byte for byte
with golden vectors that an independent verifier emulated from the same
resources.

Shared envelope (all jobs):

| bytes | meaning | source |
|---|---|---|
| `00 00 00 1B 01 "@EJL 1284.4" 0A "@EJL     " 0A` | exit IEEE 1284.4 packet mode | string 4300, opcode 0x104 |
| `1B 40 1B 40` | ESC @ twice (reset) | opcode 0x102 |
| `1B 28 52 08 00 00 "REMOTE1"` | enter remote mode | opcode 0x100 |
| `"TI" 08 00 00 YYhi YYlo MM DD hh mm ss` | set the printer clock (local time) | opcode 0x106, `FUN_0040b3ac` |
| … command … | | |
| `1B 00 00 00` | exit remote mode | opcode 0x101 |
| `1B 40 1B 40` + REMOTE1 + `"JE" 01 00 00` + `1B 00 00 00` | job end | string 4302, `FUN_004082ec` |

| Job | Command part | Size | Confidence |
|---|---|---|---|
| Nozzle Check | `"JS" 04 00 00 00 00 00`, `"NC" 02 00 00 00`, exit, `0D 0A 0D 0A`, REMOTE1, `"VI" 02 00 00 00`, `"LD" 00 00`, exit, `0C` | 132 B | **High** (RCDATA 1800 + string 1528; Gutenprint escputil also uses `NC 02 00 00 00`) |
| Head Cleaning | `"CH" 02 00 00 00` | 92 B | **High** (RCDATA 1802 + string 1527; identical to Epson's Linux `ecbd` and Gutenprint `CH`) |
| Power Cleaning | `"CH" 02 00 00 10` | 92 B | **High** that this is what Windows sends for the L800: 0x10 is the "Power Cleaning" radio value in RCDATA 10460, and `FUN_00409b3c` writes it into byte 5. Not yet tested on hardware. `ecbd`'s `CH … 20` is a different operation and is not used. |
| Status (not used yet) | `"st" 01 00 01` on the 1284.4 EPSON-CTRL channel | | Medium |
| Print Head Alignment | deferred | | Low |

Only the 7 TI bytes (offsets 49–55) depend on the clock.

## SPEC

### Screens

Pixel sizes are client areas at 96 dpi. They come from the DLU rectangles in the
resources, using the dialog font Tahoma 8 pt (base units 6×13 px:
`x = MulDiv(dlu, 6, 4)`, `y = MulDiv(dlu, 13, 8)`). Each control's rectangle is
in `ui/app.slint`.

| Screen | Windows source | Size | Contents |
|---|---|---|---|
| Main window | property sheet + `E_IUIRG5P` dialog 903 "Maintenance" | page 443×434 | 6 rows (icon button 41×42, title, etched line, description), Language, Speed & Progress..., Driver Update, version, [Close] |
| Please wait... | `E_IHSRG5P` 201 | 240×65 | icon, "Please wait..." |
| Nozzle Check 1 | 210 | 420×374 | picture, instructions, [Cancel] [**Print**] |
| Nozzle Check 2 | 211 | 435×414 | Good / Cleaning needed samples, instructions, [Clean] [**Finish**] |
| Head Cleaning 1 | 226 (= 220) | 420×372 | instructions, (•) Cleaning ( ) Power Cleaning, [Cancel] [**Start**] |
| Power Cleaning confirm | 360 | 435×414 | warning, transportation-lock picture, [Cancel] [**OK**] |
| Head Cleaning done | 227 (= 224) | 420×335 | "finished when the power light stops flashing", [**Print Nozzle Check Pattern**] [Finish] |
| Error | 271 | 371×197 | "… can not be completed." + reason, [OK] |
| Printer and Option Information / dry-run hexdump | size of 5050 | 485×505 | read-only text, [OK] |

The **bold** button is the default (drawn as the highlighted button, run by Return), as set at run time by `FUN_00425ef8`. Escape runs Cancel or Finish.

The slot set for the L800 is SM3, Nozzle Check, Head Cleaning, Print Head
Alignment, Paper Guide Cleaning, and Printer and Option Information. It follows
the table order in `E_IUICG5P` `FUN_68fd966c`. Rows 2–4 are certain. Rows 1, 5
and 6 are probable, because their capability checks were not resolved
statically. The property-sheet frame (tab strip height, insets, button row) is
not in the resources and depends on the Windows theme, so it is approximate.
Windows has OK/Cancel/Help; this app has only Close, because there are no
settings to apply.

### Flows

```
Nozzle Check:   210 --Print--> [queue check, spool] (201) --> 211 --Clean--> Head Cleaning
                 |                                              '--Finish--> close
                 '--Cancel--> close
Head Cleaning:  226 --Start (Cleaning)-------------> [queue check, spool] (201) --> 227
                 |   --Start (Power Cleaning)--> 360 --OK--^        227 --Print Nozzle Check Pattern--> 210
                 '--Cancel--> close                 '--Cancel--> 226
Errors:         queue paused / not accepting / lp failed --> 271
```

### Commands

| UI action | Job | Bytes |
|---|---|---|
| Nozzle Check → Print | RCDATA 1800 + trailer | 132 |
| Head Cleaning → Start (Cleaning) | RCDATA 1802, CH byte 5 = 00, + trailer | 92 |
| Head Cleaning → Start (Power Cleaning) → OK | RCDATA 1802, CH byte 5 = 10, + trailer | 92 |

## Layout of the code

- `src/commands.rs`: job bytes, with evidence comments and golden-vector tests
- `src/transport.rs`: `lp -o raw`, dry-run files, CUPS queue check and info
- `src/main.rs`: flows (dialog sequence) and command-line options
- `ui/app.slint`: all windows, at the Windows pixel rectangles
- `examples/screenshot.rs`: offscreen render of every window
