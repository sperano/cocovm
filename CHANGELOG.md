# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- The application icon now reaches every platform's launcher, not just the
  macOS bundle. The Windows executable embeds the icon and a version block,
  so Explorer shows them without an installer. The Linux archive carries a
  `cocovm.desktop` entry, the icon in the standard sizes, and an
  `install.sh` that places them under `~/.local` (or a prefix you choose);
  the window now also reports `cocovm` as its Wayland app id, so GNOME and
  KDE pair the running window with the launcher. The macOS bundle step is a
  script under `packaging/` that builds the same `cocovm.app` from a local
  build.
- The machine list's right-click menu item "Show config" is now "Show
  config in Finder" (macOS), "Show config in File Explorer" (Windows), or
  "Show config in File Manager" (Linux), and it does what the name says:
  it reveals the machine's definition file on disk. Before, it only
  selected the row, which looked like nothing happened when the row was
  already selected. On Linux the containing folder opens, since desktops
  have no portable way to select a file.
- A `fast_forward` option on the MCP `wait` and `wait_for_text` tools. With
  it, the VM runs as fast as the host allows until the call returns, with
  audio dropped, instead of at real-time speed; a 3,600-field wait no longer
  takes a minute. `wait_for_text` stops on the first field whose screen
  matches. The run ends with the call, including on timeout or a dropped
  connection, and a VM fast-forwards for one call at a time.
- An optional slug argument to start a saved machine as the manager opens:
  `cocovm <slug>` starts that machine without selecting its row in the list.
- `stop_vm` and `suspend_vm` MCP tools, which work like the manager's Stop
  and Suspend: both write modified floppies and tapes back to their files
  first. If a write-back fails, `stop_vm` still powers the VM off and
  returns an error naming the file, while `suspend_vm` leaves the VM
  running.
- Structured MCP tool results. For clients on protocol 2025-06-18,
  `list_vms`, `screen_text`, and `peek` declare an `outputSchema` and return
  `structuredContent` next to the existing text: the VM list; the screen
  lines, mode, and cursor; and the address and bytes. Older clients get the
  text alone. Requests with an unsupported `MCP-Protocol-Version` header are
  rejected with HTTP 400.
- MCP `enter_basic` tool. It types a multi-line BASIC listing one line at a
  time, without `type_text`'s 600-character limit, and can type `NEW` first.
  It rejects the whole listing before typing when the listing is longer than
  8,192 characters, a line is longer than BASIC's 249-character input line,
  or a character has no CoCo key. It stops at the first line that BASIC
  answers with an error and reports that line, the error, and the screen.
- Save to File… and Load from File… at the end of the toolbar's State
  selector, and of the States menu when the toolbar is narrow. They save
  the machine state to, or load it from, a `.ccstate` file anywhere on
  disk, and leave the selected state unchanged.

### Fixed
- An MCP session no longer expires while one of its requests is still
  running. The 5-minute idle timeout now starts when the response is sent,
  so a long `enter_basic` call doesn't end the client's session.

### Changed
- Five quick states instead of ten, and every one of them has keyboard
  shortcuts: ⌘1 to ⌘5 (Ctrl+1 to Ctrl+5 on Windows and Linux) load States
  1 to 5 and ⇧⌘1 to ⇧⌘5 (Ctrl+Shift+1 to Ctrl+Shift+5) save them. Before,
  only States 1 to 3 had shortcuts. The shortcuts are editable in the
  Settings dialog's Hotkeys tab like the other hotkeys, and are stored as
  `hotkey_load_state_<n>` and `hotkey_save_state_<n>` in `config.toml`.
  States 6 to 10 are gone; their files, if any, stay in the `save-states`
  folder but are no longer listed. A `config.toml` that already gives ⌘4,
  ⌘5, ⇧⌘4, or ⇧⌘5 (or the Ctrl equivalents) to another hotkey is refused
  at startup, naming both actions, until one of them is rebound.
- The MCP server's `list_vms` tool also reports each VM's model, RAM size,
  CPU, cartridge, and mounted media.
- A pending MCP `wait`, `wait_for_text`, `type_text`, or `press_keys` call
  fails as soon as its VM is paused or suspended, instead of timing out.
  The error names the tool that resumes the VM.
- The MCP server's `screen_text` tool reports where BASIC's next character
  lands, as a 0-based row and column, on the 32-column screen and the
  `WIDTH 40`/`WIDTH 80` screens. In PMODE graphics it now says there is no
  text buffer instead of decoding graphics bytes as characters.
- Start Print Capture, Stop Print Capture, and Translate CR to LF are in
  the status bar's Printer menu, next to Open Print Capture, now that the
  Machine menu is gone.
- Loading a state saved on another machine type, such as a CoCo 2 state
  in a CoCo 3 window, asks first. Before, the window silently turned into
  the other type. Loading never changes the machine's settings.

### Removed
- The `book/` course and its `tools/build-book.sh` EPUB/PDF builder. The
  per-PR book-update gate is gone with it.
- The Machine menu's Quick Save and Quick Load submenus. Save and load
  quick states with the toolbar's State selector and its Save and Load
  tiles, or with each state's keyboard shortcuts.
- The Machine menu. Its Save State and Load State items are now Save to
  File… and Load from File… in the toolbar's State selector, and print
  capture moved to the Printer menu.
- The VM window's menu bar. Its View menu's Orchestra-90 Levels item is
  now in the status bar's Sound menu, shown while an Orchestra-90 is
  inserted. Its Help menu's About item is now About cocovm, in the menu of
  the manager toolbar's Help tile, which did nothing before.

## [0.7.8] - 2026-10-03

### Added
- Tandy Hi-Res Joystick Interface (26-3025) as a joystick option ("Tandy
  Hi-Res"), timing the pot position through the cassette DAC and comparator
  like the real RC-timer box.
- CoCo Max Hi-Res Input Module cartridge for the CoCo 1/2, in the Cartridge
  and MPI Slot combos. It is refused on a CoCo 3, where its I/O window
  belongs to the GIME.
- FD-502 controllers can boot Disk BASIC 1.1 or HDB-DOS (CoCo 3 only), in
  the cartridge port or a MultiPak slot. With HDB-DOS, `DRIVE ON` reaches
  the DriveWire disks and `DRIVE OFF` the controller's floppies. Existing
  machines keep Disk BASIC.
- DriveWire settings are saved with the machine: Enable DriveWire, HDB-DOS
  mode, and the DW0–DW3 disk images live in a DriveWire tab of the VM
  settings, edited as file paths with Browse and clear buttons, instead of
  a Machine menu that forgot them on a cold start.
- The machine list can be sorted by creation date or name, ascending or
  descending, from the "Sort by" control above it. The order is remembered
  in `config.toml` (`manager_sort`); newest first is the default.
- Up and Down walk the machine list's selection when no text field has the
  keyboard.
- Rebindable hotkeys. The Settings dialog has a Hotkeys section for the key
  layout window (F10), the keyboard mode toggle (F12), New machine (⌘N /
  Ctrl+N), and the debugger in `debug-ui` builds (⌘D / Ctrl+D): click a
  binding, then press the new key. They are stored as `hotkey_*` keys in
  `config.toml` and apply to open VM windows on Save. A hotkey must not type
  into the machine, clash with another hotkey, or take a built-in shortcut.

### Changed
- VM settings are split into General, Display, Devices, Input, and
  DriveWire tabs under a tab strip.
- DriveWire disk reads and writes run off the emulation thread, so a slow
  host file no longer stalls the machine. The status bar shows DriveWire
  queue state and errors.
- Sound's Mute and Volume moved from a top-level menu to a speaker entry
  in the status bar.
- The printer paper window opens from the status bar's printer menu only;
  the View menu entry is gone.
- The machine's slug shows dimmed under the name field instead of as a
  bold "Slug ID" item.
- The product name is written CoCoVM throughout, including the window
  title.
- DriveWire settings apply to a running machine as they are edited: enabling
  or disabling DriveWire, HDB-DOS mode, and the DW0–DW3 images no longer wait
  for the next start from power off. A suspended machine still resumes its
  saved session.
- Floppy disks are inserted, created, and ejected from the status bar's disk
  entries, like the cassette deck: the "No disks" entry and each mounted
  disk's entry open a menu covering every drive, the clicked drive first. The
  Machine menu no longer lists them.

### Fixed
- Choosing the HDB-DOS ROM for an FD-502 switches DriveWire and HDB-DOS mode
  on once instead of locking them on, so both can be turned off again.

### Removed
- The per-cartridge `autostart` setting and its Auto-start checkbox in the
  New/Edit form. ROM Paks, banked ROM Paks, and the Games Master Cartridge
  always start at power-up, like real game paks; the FD-502's DOS ROM never
  did. Machine files that still carry the key load unchanged.
- The optional 4:3 aspect setting, its View menu item, and the F9 shortcut.
  Displays use a fixed 4:3 aspect ratio across RGB, composite, and TV modes.

## [0.7.7] - 2026-09-17

### Added
- NTSC artifact colors. CoCo 1/2 PMODE 4 (RG6) screens with the color-set
  bit on now show the red/blue artifact hues on a composite monitor or TV,
  with the phase picked at reset like real hardware and preserved in save
  states. A CoCo 3 on NTSC composite gets the same treatment, following its
  live burst-phase bit; RGB monitors and native GIME modes are unchanged. A
  color TV keeps the artifact chroma, a black-and-white TV collapses it.
- Composite monitor as a display choice on the CoCo 1/2, standing in for the
  common composite video-output mod. It is the new default there (matching
  the CoCo 3's RGB default); a TV stays one click away, and definitions that
  saved `display = "tv"` keep it. RGB is still refused, since the VDG has no
  RGB output.
- Orchestra-90 CC is a plain entry in the Cartridge and MPI Slot combos, like
  the Sound/Speech Cartridge, loading its fixed `roms/orch90.rom`; insertion
  is refused with a message naming the file when it is missing. The `path`
  key is gone from machine definitions; old files still load.
- Asset bundle v8: `orch90.rom` and `rs232.rom` join the bundled ROMs, the
  cartridge set gains new and refreshed dumps (99 images), and the ROM
  database knows which machine family each ROM and cartridge needs. The
  startup banner counts cartridges, and Known Cartridges hover text shows the
  machine family.
- The first-run asset download also checks for the bundled cartridge images,
  so an install from before the cartridges shipped picks them up and the
  Known Cartridges picker fills in.

### Changed
- CoCo 1/2 screens now render on the same canvas as CoCo 3 legacy modes, so a
  CoCo 2 on Color TV is exactly as sharp as a CoCo 3 showing the same screen
  instead of about twice as blurry.
- The machine form no longer offers a Video (NTSC/PAL) row. PAL was disabled
  on the CoCo 1/2 and on the CoCo 3 only ran the NTSC ROM on unverified
  50 Hz timing. Hand-edited definitions with `video = "pal"` still load.

### Fixed
- Pasting a BASIC program dropped characters after ENTER on long lines (a
  line such as `30 PMODE 4,1` could arrive as line 0). Paste and the remote
  `type_text` tool now pace each key by the machine's actual keyboard scans,
  so a key is held until Color BASIC has seen it and released until it has
  seen the release.
- GitHub releases ship binaries again. Every release since v0.7.0 was
  published before the build workflow could attach its archives; the
  workflow now uploads to a draft and publishes once every target has
  built.

## [0.7.6] - 2026-09-14

### Added
- Suspended machines are now obvious at a glance: the VM window's display
  dims under a translucent scrim with a large Play glyph and a "Suspended"
  marker, and clicking anywhere on the screen resumes the machine, same as
  the toolbar's Start tile.
- Known Cartridges picker: the New VM form's Cartridge combo and each MPI
  Slot combo list the cartridge images the asset bundle ships, next to the
  unchanged manual file picker. Cartridge hardware (fixed ROM, legacy
  banked, Games Master) is detected from the image's size and CRC32
  fingerprint, so the separate ROM Pak, banked ROM Pak, and Games Master
  entries collapse into one Cartridge ROM choice.
- The manager's detail pane gains a read-only ROMs group listing every ROM
  image the selected definition will load at its next cold start, so a
  missing or doubtful dump shows before Start instead of as a boot failure.
- The manager's welcome image can change on a timer, in file-name order or
  shuffled, with a short crossfade (config keys/flags follow the
  `toolbar_icons_only` pattern).
- Icon-only VM status bar via `status_bar_icons_only`
  (`--status-bar-icons-only` / `COCOVM_STATUS_BAR_ICONS_ONLY`), with a
  matching Settings checkbox.

### Changed
- **Breaking: save states from 0.7.5 no longer load.** The snapshot schema
  was renumbered to version 1; re-create save states after upgrading.
- The manager's detail pane shows a much larger screen preview, spanning
  the right half of the header next to the identity and hardware groups.
- The paper window starts printing below a top-of-form margin: the first
  line no longer lands on the paper's edge and page perforations no longer
  cut through a character row.
- The status bar shows an installed FD-502 with a "No disks" readout while
  its drives are empty, instead of drawing nothing.
- Settings save applies immediately: changing the log level re-levels the
  live logger, and changing the control port moves the built-in MCP
  listener without a restart.
- The Disto RTC now models only the 4-N-1 (OKI MSM6242); the 2-N-1 chip
  fit was dropped.
- Performance and memory: printer PNG/PDF exports run on cancellable
  background workers with bounded memory and atomic output files; the
  machine list and saved previews load visible-first under strict decode
  and cache budgets; VM repaint scheduling coalesces deadlines instead of
  requesting immediate repaints.

### Fixed
- The Becker port can no longer be enabled alongside a Games Master
  Cartridge — they share the $FF41 sound port; the disabled control's
  hover text explains the conflict.
- DMP printers print the manuals' verified character tables for the
  European range $A0–$BF and the block graphics $E0–$FE, replacing
  placeholder and invented glyphs.
- Icon-only toolbar style now applies to already-open VM windows instead
  of only newly opened ones.

## [0.7.5] - 2026-09-09

### Added
- DMP-130 printer, selectable alongside the DMP-105. It supports the
  Tandy DP/WP and graphics command sets plus IBM emulation, with buffered
  text, counted graphics, forms, margins, tabs, and style controls. Output
  uses the same paper window, PNG/PDF exports, and save-state flow as the
  DMP-105. Fonts remain approximations and some extended characters and
  country substitutions are incomplete.
- Global configuration file: `~/.config/cocovm/config.toml`
  (`%APPDATA%\spe\cocovm\config.toml` on Windows) holds the log level,
  control port, asset bundle URL, and toolbar style. Precedence per
  parameter is CLI flag > environment variable > config file > built-in
  default. A commented-out template listing every parameter and its
  default is seeded on first start; a malformed file or unknown key is a
  startup error naming the path.
- Settings dialog: the manager toolbar's Settings tile edits the config
  file in place, preserving its comments. Fields left at their defaults
  are removed from the file so they keep tracking future defaults.
  Toolbar style applies immediately; the other keys apply on next start.
- Icon-only toolbars via `toolbar_icons_only` (also
  `--toolbar-icons-only` / `COCOVM_TOOLBAR_ICONS_ONLY`): manager and VM
  window toolbars draw square icon tiles with the caption as hover text.
- Clickable on-screen keyboard: keycaps in the keyboard help window send
  the CoCo key directly to the matrix. Shift, Ctrl, and Alt caps toggle
  visible latches so combinations are reachable by successive clicks.
  Disabled while a remote keyboard session is active.
- The VM details panel shows the machine's slug under its name.

### Changed
- **Breaking: save states from earlier versions no longer load.** The
  snapshot schema moved to version 2 for the new printer representation
  and paper geometry; schema 1 snapshots are rejected and there is no
  migration. Re-create save states after upgrading.
- DMP-105 graphics output was distorted: picture dumps used text-dot
  spacing horizontally and the text line feed for graphics carriage
  returns. Graphics geometry and positioning now follow the command
  table, and paper coordinates represent the documented feed increments
  exactly. Existing DMP-105 machine definitions keep their meaning.
- Asset bundle v6 adds `hdbdw3bc3.rom` (HDB-DOS 1.4 Becker) and moves to a
  new location on the asset host. Existing installs are prompted for the
  new bundle on their next start.
- Rendering and audio allocate far less: unchanged framebuffers reuse
  their GPU texture instead of re-uploading on every UI repaint, TV
  processing reuses its scratch buffers, audio buffers are retained
  across scanlines, and the audio queue is preallocated and drops the
  oldest frames rather than growing past its bound. Static or paused
  windows no longer request animation repaints. TV noise now advances on a
  60 Hz clock, including on paused TVs.
- Sound/Speech Cartridge: the TMS7040 core now models the external
  interrupt pulse latch, Timer 1 event-counter mode, and IOCNT0 memory
  expansion modes per the TI data manuals, and validates restored
  snapshots so a corrupted state cannot hang or panic the emulator. The
  disassembler now decodes `MOV B,A` (0xB1). None of these paths are
  exercised by the cartridge firmware, so its behavior is unchanged.
- The `tms7000` crate ships MAME's BSD-3-Clause notice and is
  publishable on its own.

### Fixed
- The disk/VHD-based integration tests resolved their images from a
  directory nothing ever created and skipped silently. They now fetch a
  separate test bundle into `assets/tests` on first use (override with
  `COCOVM_TEST_ASSETS_URL`; empty disables the fetch).

## [0.7.4] - 2026-09-06

### Added
- Sound/Speech Cartridge: speech synthesis. The cartridge now carries its
  GI SP0256-AL2 speech chip and runs the real TMS7040 firmware on a new
  cycle-exact TMS7040 core, so allophone streams and full text-to-speech
  work as on the board — text mode, allophone loads, and the manual's
  demo programs all speak. Verified instruction-for-instruction against
  MAME over the firmware's boot and host-command paths.
- Printer: the status-bar icon is always visible, flashes red on
  serial-port output, and gains a context menu with "View Papers" and
  "Open Print Capture".
- First-run download dialog: when ROMs or images are missing at startup,
  the app opens a small window asking before it downloads anything —
  Download fetches the bundle in the background with a spinner and then
  opens the manager; Cancel quits. Failed downloads show in the dialog
  for a retry.
- The asset bundle URL is configurable via `--assets-url` or the
  `COCOVM_ASSETS_URL` environment variable.

### Changed
- Sound/Speech Cartridge host-port behavior now matches the real board:
  a byte written while busy overwrites the latch instead of being dropped,
  the reset line acts on its falling edge, and busy releases when the
  firmware says so rather than on approximated timing. Software that
  polls right after a command sees the documented delays.
- Snapshots record both Sound/Speech Cartridge ROM images (firmware and
  speech ROM) and restore reattaches them; snapshots from earlier versions
  still load. The asset download moves to the v3 bundle, which adds
  `sp0256-al2.rom` and `ssc-tms7040.rom`.
- Downloaded assets now install under `~/.local/share/cocovm/assets/`
  (`assets/roms`, `assets/images`) instead of the data directory root.
  There is no automatic migration — on the next start the download dialog
  offers a fresh download, and the old `roms/` and `images/` directories
  can be deleted.

## [0.7.3] - 2026-08-30

### Added
- MCP endpoint: the app serves an MCP server over HTTP at
  `http://127.0.0.1:6809/mcp` so an AI agent can drive a running VM —
  list/start VMs, read the screen as text or a PNG screenshot, type text,
  press key chords, move the joystick, insert/eject disks, reset, pause,
  wait, peek and poke memory. Register with
  `claude mcp add --transport http cocovm http://127.0.0.1:6809/mcp`.
  `--control-port` / `COCOVM_CONTROL_PORT` change the port; `0` disables it.
- Disto RTC: the 2-N-1 model (OKI MSM5832) alongside the 4-N-1, so NitrOS-9's
  `clock2_disto2` driver reads the correct date. Machine files take
  `model = "4n1" | "2n1"` on the `rtc` peripheral; the New dialog offers both.
- CPU: the MC6809's undocumented read-modify-write opcode aliases (including
  the XCLR accumulator behavior) execute and disassemble as on real silicon.

### Changed
- Suspended VMs keep their menu bar, toolbar and status bar (read-only, with
  a "Suspended" marker); Start resumes the machine, Stop closes the window.
- While no cocovm window is focused, redraws drop to ~10 Hz; emulation and
  audio continue in real time, and a short audio cushion prevents warble.

### Fixed
- Keys, modifiers and key-driven joystick input are released when a VM
  window loses focus, so nothing stays stuck down after switching apps.
- Sound: the speaker mux holds its level while sound is disabled and
  crossfades source changes, removing the hum in Sokoban and similar games.

## [0.7.2] - 2026-08-28

### Changed
- Debugger: its toolbar button and keyboard shortcut exist only in builds
  made with `--features debug-ui`; regular builds no longer show it.
- Running from source: debug builds optimize dependencies and the emulation
  core, so a running VM no longer saturates a CPU core.

### Fixed
- CPU: hardware interrupt entry is cycle-accurate — IRQ and NMI cost 19
  cycles, FIRQ 10; CWAI totals 20 cycles (was 26); leaving SYNC charges its
  1-cycle escape. Verified against the MAME microcode.
- Save states: a corrupt or hand-edited `.ccstate` whose scanline scheduler
  counters are out of range, or whose bus variant disagrees with the machine
  config, is rejected as invalid instead of resuming into an impossible
  state or crashing.

## [0.7.1] - 2026-08-26

### Added
- Machine config: the Multi-Pak's front-panel switch position (`switch`,
  1–4) and the Deluxe RS-232 Pak's host wiring (`endpoint`: loopback, TCP
  listen address, or PTY) are saved in `[peripherals]`; ROM Paks and the
  Game Master Cartridge take an `autostart` flag for paks that must be
  started by hand. Existing machine files load unchanged.
- Deluxe RS-232 Pak: can be installed in a Multi-Pak slot, not only the
  bare cartridge port; it answers from any slot regardless of the switch
  position, as on real hardware.
- VM manager: the New/Edit form exposes the switch, RS-232 endpoint, and
  auto-start settings.

### Changed
- VM window: peripherals are configured only from the machine definition.
  The Machine menu's insert/eject/remove actions for cartridges, the
  Multi-Pak, the RS-232 Pak, the Disto RTC, and VHD images are gone, as is
  the Reset item (Reset stays on the toolbar). The menu now holds Save/Load
  State, floppy drives, DriveWire, and print capture.

### Fixed
- Load State keeps the RS-232 Pak's configured TCP/PTY wiring instead of
  silently dropping back to loopback.
- Debugger: peeking cartridge registers in `$FF60–$FF7E` through a Multi-Pak
  reaches the same slot a real read does.
- CPU: the cycle counter restarts from zero on reset.

## [0.7.0] - 2026-08-26

### Added
- Display: choose what the machine is plugged into — an RGB or composite
  monitor (CoCo 3) or a color or black-and-white TV (any machine), with
  scanline and RF-noise controls — from a new display entry in the status
  bar.
- Display: adjustable TV overscan crop (0–10% per edge, 5% default), also
  applied to manager thumbnails; monitors stay uncropped.
- Cartridges: Game Master Cartridge (GMC), Orchestra-90 CC, and the
  Speech/Sound Cartridge (SSC) can be configured in a machine's
  peripherals, in the cartridge port or in a Multi-Pak slot.
- Machine config: `[peripherals]` now describes the cartridge port as one
  device (`none`, `fd502`, `rompak`, `rtc`, `rs232`, `gmc`, `orch90`,
  `ssc`, or `mpi` with per-slot contents), so a Multi-Pak's slot layout
  is saved with the machine.
- Cassette: a status-bar tape deck entry with Insert / New / Rewind / Seek
  / Eject; recording writes at the head position, the counter moves
  during CSAVE, and recordings auto-save shortly after the motor stops.
- Status bar: the keyboard entry shows the active input mode and opens the
  Keyboard menu on click; a joystick entry shows each port's source and
  opens the Joysticks menu.
- Keyboard help (F10) draws the real CoCo 1/2 or CoCo 3 key layout instead
  of a grid of squares.
- VM manager: per-machine statistics — cumulative powered-on time and boot
  count — in the detail pane.
- VM window: a Debug toolbar tile and ⌘D (Ctrl+D) toggle the debugger,
  replacing F11.
- `--log-level`/`-L` flag, `COCOVM_LOG_LEVEL`, and `.env` file support;
  the startup banner reports the installed ROMs, machines, and renderer.
- CoCo 1/2 video: the SAM's display address stream follows the discrete
  MC6883 counters, so mismatched SAM/VDG mode pairings render as on real
  hardware; the original MC6847 draws semigraphics-6 while the MC6847T1
  does not.
- macOS release binaries are signed and notarized, shipped as a `.dmg`
  and a `.tar.gz`.

### Changed
- The VM window's toolbar uses the same transport tiles as the manager.
- Inserting or creating a disk no longer silently adds an FD-502 and
  power-cycles the machine; those menu items stay disabled until a
  controller is configured in the machine's peripherals.
- The direct-boot command-line flags are gone; the app always opens the VM
  manager, and every media/peripheral option lives in the VM window or
  the machine editor.
- Mouse-as-joystick maps over the active picture (not the borders) and
  only fires for presses that start on the display, not on menus or
  dialogs.
- The CoCo 3's `$FF40–$FF5F` cartridge I/O window honours the GIME's
  INIT0 MC2 bit, as on real hardware.

### Fixed
- Dirty floppies and tapes are never lost: a failed write-back aborts the
  insert, eject, new-media, cartridge swap, save-state, or suspend that
  triggered it and keeps the medium mounted for retry.
- Resume no longer reports a machine as Running if its suspend checkpoint
  could not be consumed.
- Power cycle resets latched keyboard/CART* edges and audio state, and
  loading a state or power-cycling clears the host audio pipeline instead
  of playing a burst of the previous machine's sound.
- Renaming a running machine and then quitting migrates its folder.
- 6809: `EXG` between 8- and 16-bit registers follows the real chip's
  widening rules and takes 8 cycles; `PULU` loading S arms NMI
  recognition.
- The cassette calibration probe no longer races on an idle tape.

## [0.6.2] - 2026-07-30

### Added
- VM manager: machines are now Powered Off, Running, or Suspended — ⏸
  freezes a running machine to disk and ▶ resumes it, surviving quitting
  the manager entirely.
- VM manager: multi-select in the machine list (shift-click ranges,
  cmd-click toggles, ⌘A) with bulk Start/Suspend/Stop/Reset and bulk
  delete from the context menu or the selection pane.
- VM manager: the detail pane is a full machine editor — Machine, RAM,
  Display, Peripherals, Ports, Joysticks, and Keyboard sections with
  autosave — and the transport controls moved into the main toolbar.
- Status bar: every device shows a small hardware icon with a live
  activity light — the cassette's reels spin with the tape, the floppy,
  VHD, DriveWire, and RS-232 icons blink red during I/O, joystick and
  printer entries light while in use, and each icon has a hover tooltip.

### Changed
- The executable is named `cocovm`.
- The manager toolbar uses icon-over-label buttons, Reset joins the ↻
  transport controls, and clicking empty list space clears the selection.
- The book grew to full depth across chapters 2–16, with EPUB and PDF
  editions built by `tools/build-book.sh`.
- Internal identifiers now spell acronyms all-caps (RS232, VHD, SAM,
  PIAPort, …). Save states from earlier versions that contain a
  cartridge no longer load; save-state compatibility is not yet
  guaranteed pre-1.0.

## [0.6.1] - 2026-07-27

### Added
- Prebuilt release binaries for macOS (Intel and Apple Silicon), Linux
  (x86_64 and arm64), and Windows, attached to each GitHub release.

### Fixed
- The emulator now builds on Linux arm64 (`c_char` signedness in the RS-232
  PTY endpoint) and on Windows, where the PTY wiring option is Unix-only and
  the RS-232 pak offers Loopback and TCP.

## [0.6.0] - 2026-07-26

First release of cocovm, a Tandy Color Computer emulator written in Rust.
