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
- The only thing the app writes to the printer on its own is the read-only
  status request (`st 01 00 01`, see [Status](#status)). It does this only while
  the CUPS queue has no jobs.
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
| Nozzle Check | Works. Pre-check, 210, "Please wait...", 211 (Good / Cleaning needed); [Clean] starts Head Cleaning |
| Head Cleaning | Works. Pre-check, 220/226, (Power Cleaning: 360 confirm), "Please wait...", progress 223 (with status), 224/227; [Print Nozzle Check Pattern] |
| Power Cleaning | Works, as the second radio option in Head Cleaning, with the transportation-lock warning (dialog 360) |
| EPSON Status Monitor 3 | Works: status title and message from the Windows status table, approximate levels of the 6 ink tanks, polled every 5 s while open |
| Printer and Option Information | Shows what CUPS reports for the queue (`lpstat -l -p/-v/-a/-o`), read-only |
| Print Head Alignment | Disabled ("Not available on Linux yet"): deferred, because it writes persistent values with `DD`/`SV` |
| Paper Guide Cleaning | Disabled |
| Speed & Progress..., Driver Update | Disabled. Monitoring Preferences lives inside Speed & Progress on Windows |
| Language | English only (the driver has no other UI strings) |

The Windows Maintenance tab has no Print Queue button for this driver, so this
app has none either.

### Status path and no-status path

Like Windows (rea/NOTES.md §1.2-1.4), each utility first shows "Please wait..."
(dialog 201) and reads the printer status.

**With a status reply:**
- **Pre-check.** On ST=00 or 11: "A problem has occurred." [Details] [OK]
  (dialog 270; Details opens Status Monitor 3). On ST=01 or field 0x03: "Cannot
  complete Nozzle Check/Head Cleaning." For Head Cleaning only: ST=07 gives "Now
  cleaning!", and an ink-tank warning 0x51/0x52 gives "Head Cleaning can not be
  completed." Otherwise the utility opens.
- **Head Cleaning** uses dialog 220. After spooling, progress dialog 223 shows
  "Please wait...". It polls the status every 2 s. Cleaning has started when
  ST=07 and field 0x0C (the expected time in seconds) is above 0. The bar then
  fills over that time, held at 97 % until 3 polls in a row no longer show ST=07.
  It then shows 100 % for 0.5 s, and dialog 224 says the cleaning cycle has
  finished.
- If cleaning never starts within 16 polls (~32 s), or 3 reads in a row fail,
  the app gives up and shows dialog 227 ("finished when the power light stops
  flashing").
- Just before spooling, the status is read again. ST=00 stops the job with the
  communication error (dialog 270).

**Without a status reply** (no device node, no access, or CUPS has jobs): no
pre-check. Head Cleaning uses dialog 226 (same layout and text as 220) and
goes straight to 227, with no progress dialog.

**Both paths:** before sending, the CUPS queue is checked. A disabled (paused)
queue, or one that is not accepting jobs, gives Windows' communication error
(string 3000, the "Work Offline"/"Paused" case). In `--dry-run` this check is
only a note in the hexdump window. Dry-run never shows the progress dialog,
because no cleaning runs. The "Please wait..." popup stays up at least 0.7 s,
so it does not just flicker.

## Status

`src/status/` reads the status the way Windows does: the `st 01 00 01` request
on the IEEE 1284.4 (D4) EPSON-CTRL channel (socket 2), answered with a binary
`@BDC ST2` reply. It talks 1284.4 directly over the kernel usblp node
`/dev/usb/lpN` (found by `MDL:L800;` in sysfs).

- usblp stays bound, so nothing changes for CUPS. A query takes about 60 ms.
- Each poll opens the node, then sends Enter, Init, OpenChannel, the status
  request, CloseChannel and Exit. Exit is always sent, also after an error.
- The app queries only when `lpstat -o L800` shows no job.
- Bytes on the wire are identical to Epson's own `ecbd` session (usbmon capture
  in the phase-2 spike). The tests replay that live capture and parse the live
  reply.
- Epson's `ecbd` daemon is not used. Its TCP answer is a shortened text
  without the warning list and the cleaning time. Each ecbd query also unbinds
  usblp and takes about 1 s.
- Ink levels come from field 0x0F, whose entries are 4 bytes on the L800:
  `[ink id, colour, level, 02]`, in the order C, M, LM, LC, BK, Y. The levels are
  the printer's estimates (the tanks have no sensor), so they are labelled
  approximate, as on Windows.

**Access.** The usblp node is `root:lp 0660` on a stock system. It works here
only because Epson's `79-udev-epson.rules` makes it `0777`. To give just the
logged-in user access, install the rule in `packaging/`:

```sh
sudo install -m 644 packaging/70-epson-l800-utility.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=usbmisc
```

Without access, the app keeps working on the no-status path.

One read-only status query from the command line:
`cargo run --example status` prints the decoded fields, the Status Monitor text
and the pre-check result. It refuses while the queue has jobs.

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
| Status | `"st" 01 00 01` in a 1284.4 data packet on socket 2 | 5 B payload | **High** (Windows E_IAPRG5P / E_IHUTG5P command tables; live replies on this L800) |
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
| Head Cleaning 1 | 220 / 226 | 420×372 | instructions, (•) Cleaning ( ) Power Cleaning, [Cancel] [**Start**] |
| Power Cleaning confirm | 360 | 435×414 | warning, transportation-lock picture, [Cancel] [**OK**] |
| Head Cleaning progress | 223 | 420×335 | moving print head, progress bar (status path only) |
| Head Cleaning done | 224 / 227 | 420×335 | "cleaning cycle has finished" / "finished when the power light stops flashing", [**Print Nozzle Check Pattern**] [Finish] |
| Messages | 270 / 271 | 371×197 | "A problem has occurred." [Details] [**OK**] / "… can not be completed.", "Now cleaning!" [**OK**] |
| EPSON Status Monitor 3 | `E_IASRG5P` dialog 102 | 479×412 | status title and message, picture, "Approximate Ink Levels" (6 tanks), note, [**OK**] |
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
Start:          201 (read status) --error--> 270/271 --> close   (Details --> Status Monitor 3)
                                  --ok / no status--> utility
Nozzle Check:   210 --Print--> 201 (re-check, queue check, spool) --> 211 --Clean--> Head Cleaning
                 '--Cancel--> close                                   '--Finish--> close
Head Cleaning:  220/226 --Start (Cleaning)----------> 201 (spool) --> 223 (status) --> 224
                 |      --Start (Power Cleaning)--> 360 --OK--^      '-(no status / gave up)--> 227
                 '--Cancel--> close                       '--Cancel--> 220/226
                224/227 --Print Nozzle Check Pattern--> Nozzle Check
Errors:         queue paused / not accepting / lp failed --> 271;  ST=00 before spooling --> 270
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
- `src/status/`: 1284.4 session (`d4.rs`), usblp port (`usblp.rs`), ST2 parser
  (`st2.rs`), and in `mod.rs` the pre-check, Status Monitor messages and
  cleaning-progress logic
- `packaging/70-epson-l800-utility.rules`: udev rule for status access
- `src/main.rs`: flows (dialog sequence) and command-line options
- `ui/app.slint`: all windows, at the Windows pixel rectangles
- `examples/screenshot.rs`: offscreen render of every window
- `examples/status.rs`: one read-only status query
