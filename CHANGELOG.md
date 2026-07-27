# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
