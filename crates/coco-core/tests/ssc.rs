//! Tandy Sound/Speech Cartridge (SSC): the `$FF7D`/`$FF7E` handshake, bus
//! routing (standard SCS-slot behaviour plus the Multi-Pak's `$FF60-$FF7E`
//! broadcast — see `crate::cart::MultiPak`'s doc comment), and the
//! AY-3-8913's audio/Sound Activity Circuit integration. Facts per
//! `docs/ssc-spec.md` / MAME `coco_ssc.cpp`. AY-3-8913 core coverage lives in
//! `crates/coco-core/src/ay8913.rs`'s own inline tests.

#[path = "ssc/common.rs"]
mod common;

#[path = "ssc/audio_sac.rs"]
mod audio_sac;
#[path = "ssc/bus_routing.rs"]
mod bus_routing;
#[path = "ssc/handshake.rs"]
mod handshake;
#[path = "ssc/host_protocol.rs"]
mod host_protocol;
