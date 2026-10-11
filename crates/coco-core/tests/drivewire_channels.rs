//! Real-guest acceptance for DriveWire virtual serial channels and the `dw`
//! command service: the stock NitrOS-9 Level 2 3.3.0 Becker image
//! (`tests/nos96809l2v030300coco3_becker.dsk`) boots with `dwio`, `scdwv`,
//! `/N`, `/N1`–`/N13`, and `/X1`–`/X3` in its bootfile and `dw` in `CMDS`.
//! Its `dw` client prints the reply and exits only after its poller
//! delivers the server's hangup as `S$HUP`, so the shell prompt returning
//! after the payload is the guest observing EOF. Skips when an asset is
//! absent.

#[path = "drivewire_channels/common.rs"]
mod common;

#[path = "drivewire_channels/channels.rs"]
mod channels;
#[path = "drivewire_channels/commands.rs"]
mod commands;
