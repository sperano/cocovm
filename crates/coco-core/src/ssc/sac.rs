//! Sound Activity Circuit: an envelope follower on the cartridge's own
//! (pre-mux) audio output, purely so `$FF7E` bit 5 can report whether the
//! PSG is making sound. Runs on every `audio_sample` call regardless of
//! whether the CoCo's sound mux is actually listening to the cartridge
//! (MAME `coco_ssc.cpp` `sac_update`; time constants below are tuned for
//! MAME's own ~44.1 kHz-ish audio-stream sampling rate — the audio grid's
//! ~62.9 kHz call rate is close enough that the same constants serve, and
//! much closer than the old once-per-scanline 15.7 kHz rate was).

use super::Ssc;

/// One-pole DC-blocking high-pass filter coefficient:
/// `y = ALPHA * (y_prev + x - x_prev)`.
const HPF_ALPHA: f32 = 0.99;
/// Leaky-integrator attack rate (envelope rising toward a louder rectified
/// sample).
const ATTACK_COEFF: f32 = 0.0026;
/// Leaky-integrator decay rate (envelope falling toward a quieter one).
const DECAY_COEFF: f32 = 0.0003;
/// Envelope level above which sound is considered "active".
const THRESH_ON: f32 = 0.05;
/// Envelope level below which sound is considered "quiet" again (hysteresis:
/// lower than [`THRESH_ON`] so the status bit doesn't chatter around a
/// single threshold).
const THRESH_OFF: f32 = 0.01;

impl Ssc {
    /// Sound Activity Circuit envelope follower — see the module doc
    /// comment. Runs unconditionally on every [`Ssc::audio_sample`] call.
    pub(super) fn update_sac(&mut self, x: f32) {
        let y = HPF_ALPHA * (self.sac_hpf_prev_out + x - self.sac_hpf_prev_in);
        self.sac_hpf_prev_in = x;
        self.sac_hpf_prev_out = y;

        let rectified = y.abs();
        let coeff = if rectified > self.sac_envelope { ATTACK_COEFF } else { DECAY_COEFF };
        self.sac_envelope += coeff * (rectified - self.sac_envelope);

        if self.sac_envelope > THRESH_ON {
            self.sac_sound_active = true;
        } else if self.sac_envelope < THRESH_OFF {
            self.sac_sound_active = false;
        }
    }
}
