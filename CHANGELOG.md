# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
