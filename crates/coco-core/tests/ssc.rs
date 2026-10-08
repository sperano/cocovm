//! Tandy Sound/Speech Cartridge (SSC): the `$FF7D`/`$FF7E` handshake, bus
//! routing (standard SCS-slot behaviour plus the Multi-Pak's `$FF60-$FF7E`
//! broadcast — see `crate::cart::MultiPak`'s doc comment), and the
//! AY-3-8913's audio/Sound Activity Circuit integration, and allophone
//! speech through the SP0256-AL2. Semantics follow wiki `cocovm/ssc-spec` and
//! MAME `coco_ssc.cpp`. AY-3-8913 and SP0256 core coverage lives in
//! `crates/coco-core/src/ay8913.rs`'s and `sp0256.rs`'s own inline tests.

#[path = "ssc/common.rs"]
mod common;

#[path = "ssc/audio_sac.rs"]
mod audio_sac;
#[path = "ssc/bus_routing.rs"]
mod bus_routing;
#[path = "ssc/firmware_boot.rs"]
mod firmware_boot;
#[path = "ssc/handshake.rs"]
mod handshake;
#[path = "ssc/host_protocol.rs"]
mod host_protocol;
#[path = "ssc/snapshot_compat.rs"]
mod snapshot_compat;
#[path = "ssc/sound_duration.rs"]
mod sound_duration;
#[path = "ssc/speech.rs"]
mod speech;
#[path = "ssc/text_to_speech.rs"]
mod text_to_speech;
