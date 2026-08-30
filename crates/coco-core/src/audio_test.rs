use super::*;

const TEST_SAMPLE_RATE: f64 = 62_866.0;

#[test]
fn legacy_default_adopts_current_source_without_fade() {
    let inputs = AudioInputs {
        dac: DAC_MAX as u8,
        snden: true,
        ..AudioInputs::default()
    };
    let mut mux = AudioMux::default();

    assert_eq!(
        mux.sample(&inputs, false, 0.0, TEST_SAMPLE_RATE),
        DAC_GAIN,
        "a snapshot without mux state must preserve the old immediate output"
    );
}

#[test]
fn live_mux_crossfades_from_power_on_inhibit() {
    let inputs = AudioInputs {
        dac: DAC_MAX as u8,
        snden: true,
        ..AudioInputs::default()
    };
    let mut mux = AudioMux::new();

    assert_eq!(
        mux.sample(&inputs, false, 0.0, TEST_SAMPLE_RATE),
        0.0,
        "live hardware must ramp away from its inhibited power-on output"
    );
}
