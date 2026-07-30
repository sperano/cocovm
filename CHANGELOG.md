# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
