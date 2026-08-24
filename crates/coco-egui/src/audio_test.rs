use super::*;

#[test]
fn resampler_upsamples_2x_with_linear_interpolation() {
    // step = 0.5 means the device rate is double the source rate: every
    // input frame should produce two output frames, the second being
    // the midpoint to the next input frame.
    let mut r = Resampler::default();
    let mut out = Vec::new();
    r.process(&[[0.0; 2], [10.0, -10.0]], 0.5, &mut out);
    // prev starts at 0.0 (no prior batch), so:
    // pos=0.0 -> a=prev=0, b=input[0]=0, frac=0.0 -> 0
    // pos=0.5 -> a=prev=0, b=input[0]=0, frac=0.5 -> 0
    // pos=1.0 -> a=input[0]=0, b=input[1], frac=0.0 -> 0
    // pos=1.5 -> a=input[0]=0, b=input[1], frac=0.5 -> midpoint
    assert_eq!(out, vec![[0.0; 2], [0.0; 2], [0.0; 2], [5.0, -5.0]]);
}

#[test]
fn resampler_carries_fractional_position_and_prev_frame_across_calls() {
    // Same math as above, but split across two process() calls to prove
    // the carried `pos`/`prev` state reproduces one continuous stream.
    let mut r = Resampler::default();
    let mut out = Vec::new();
    r.process(&[[0.0; 2]], 0.5, &mut out);
    r.process(&[[10.0, -10.0]], 0.5, &mut out);
    assert_eq!(out, vec![[0.0; 2], [0.0; 2], [0.0; 2], [5.0, -5.0]]);
}

#[test]
fn resampler_downsamples_when_step_exceeds_one() {
    // step = 2.0: device rate is half the source rate, so every other
    // input frame is emitted. The very first output still interpolates
    // against `prev`'s initial 0.0 (no prior batch), which is why it
    // isn't simply input[0].
    let mut r = Resampler::default();
    let mut out = Vec::new();
    r.process(&[[1.0; 2], [2.0; 2], [3.0; 2], [4.0; 2]], 2.0, &mut out);
    assert_eq!(out, vec![[0.0; 2], [2.0; 2]]);
}

#[test]
fn resampler_keeps_channels_independent() {
    let mut r = Resampler::default();
    let mut out = Vec::new();
    r.process(&[[1.0, -1.0], [1.0, -1.0]], 1.0, &mut out);
    assert_eq!(out[1], [1.0, -1.0], "no crosstalk between channels");
}

#[test]
fn dc_blocker_converges_toward_zero_on_constant_input() {
    let mut dc = DCBlocker::default();
    let mut last = 1.0;
    for _ in 0..2000 {
        last = dc.process(1.0);
    }
    assert!(last.abs() < 1e-3, "expected near-zero, got {last}");
}

#[test]
fn dc_blocker_passes_already_centered_signal_without_blowing_up() {
    let mut dc = DCBlocker::default();
    let mut max_abs = 0.0f32;
    for i in 0..1000 {
        let x = if i % 2 == 0 { 1.0 } else { -1.0 };
        max_abs = f32::max(max_abs, dc.process(x).abs());
    }
    // A signal already centered at 0 should stay bounded near its own
    // amplitude, not grow — a highpass shouldn't amplify AC content.
    assert!(max_abs < 2.5, "expected bounded output, got {max_abs}");
}

#[test]
fn lowpass_attenuates_nyquist_rate_alternation_but_passes_dc() {
    // 62.9 kHz source decimated to 48 kHz: a +1/-1 alternation at the
    // source rate (31.45 kHz — pure fold-back material) must be crushed,
    // while a constant passes nearly unchanged.
    let mut lp = LowPass::design(0.45 * 48_000.0, 62_866.0);
    let mut max_late = 0.0f32;
    for i in 0..4000 {
        let y = lp.process(if i % 2 == 0 { 1.0 } else { -1.0 });
        if i > 2000 {
            max_late = f32::max(max_late, y.abs());
        }
    }
    assert!(
        max_late < 0.2,
        "Nyquist alternation not attenuated: {max_late}"
    );

    let mut lp = LowPass::design(0.45 * 48_000.0, 62_866.0);
    let mut last = 0.0;
    for _ in 0..4000 {
        last = lp.process(1.0);
    }
    assert!(
        (last - 1.0).abs() < 0.01,
        "DC gain should be ~1, got {last}"
    );
}

#[test]
fn underrun_decay_reaches_floor_within_fade_window() {
    let device_rate = 48_000.0;
    let fade_frames = (device_rate * UNDERRUN_FADE_SECS) as u32;
    let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
    let mut held = 1.0f32;
    for _ in 0..fade_frames {
        held *= decay;
    }
    assert!(held <= UNDERRUN_FADE_FLOOR * 1.01);
}

#[test]
fn reset_empties_ring_and_returns_filters_to_default() {
    // 62.9 kHz source into a 48 kHz device: decimating, so the low-pass is
    // designed too. A sign-alternating signal (not a DC step, which the DC
    // blocker would have decayed to ~0 by now) leaves every filter holding
    // real, non-negligible history.
    let mut out = AudioOutput::headless(48_000.0);
    let alternating = (0..4000).map(|i| if i % 2 == 0 { [1.0, -1.0] } else { [-1.0, 1.0] });
    out.push_samples(alternating, 62_866.0);
    assert!(out.queued_frames() > 0, "pipeline must be prefilled");
    assert!(out.lowpass.is_some(), "decimating path designs a low-pass");
    let history = out.dc[0].prev_out.abs();
    assert!(
        history > 1e-3,
        "DC blocker must carry history, got {history}"
    );
    assert_ne!(out.resampler.prev, [0.0; 2]);
    assert!(out.resampler.pos > 0.0);

    out.reset();

    assert_eq!(out.queued_frames(), 0);
    assert!(out.lowpass.is_none());
    assert_eq!(out.lowpass_rate, 0.0);
    for ch in out.dc {
        assert_eq!((ch.prev_in, ch.prev_out), (0.0, 0.0));
    }
    assert_eq!(out.resampler.pos, 0.0);
    assert_eq!(out.resampler.prev, [0.0; 2]);
}
