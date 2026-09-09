use super::*;

const TEST_DEVICE_RATE: f64 = 48_000.0;
const CHANNEL_COUNTS: [usize; 3] = [1, 2, 4];
const QUEUED_FRAMES: [[f32; 2]; 2] = [[0.8, -0.4], [0.5, -0.25]];

#[test]
fn callback_maps_channels_and_fades_underruns_across_calls() {
    for channels in CHANNEL_COUNTS {
        let ring = Mutex::new(VecDeque::from(QUEUED_FRAMES));
        let mut callback = Callback::new(channels, TEST_DEVICE_RATE);
        let mut held = [0.0; 2];
        for frames in [1, 0, 3, 2] {
            let queued: Vec<_> = lock(&ring).iter().copied().collect();
            let mut expected = Vec::new();
            for index in 0..frames {
                held = queued
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| [held[0] * callback.decay, held[1] * callback.decay]);
                for channel in 0..channels {
                    expected.push(if channels == 1 {
                        (held[0] + held[1]) * 0.5
                    } else {
                        held[channel % 2]
                    });
                }
            }
            let mut actual = vec![f32::NAN; frames * channels];
            callback.render(&ring, &mut actual);
            assert_eq!(actual, expected);
        }
        assert!(lock(&ring).is_empty());
        lock(&ring).push_back(QUEUED_FRAMES[0]);
        let mut resumed = vec![0.0; channels];
        callback.render(&ring, &mut resumed);
        assert_eq!(callback.held, QUEUED_FRAMES[0]);
    }
}

#[test]
fn empty_callback_starts_silent_and_reaches_fade_floor() {
    const STEREO_CHANNELS: usize = 2;
    let ring = Mutex::new(VecDeque::new());
    let mut callback = Callback::new(STEREO_CHANNELS, TEST_DEVICE_RATE);
    let mut empty = [f32::NAN; STEREO_CHANNELS];
    callback.render(&ring, &mut empty);
    assert_eq!(empty, [0.0; STEREO_CHANNELS]);
    lock(&ring).push_back([1.0; 2]);
    callback.render(&ring, &mut empty);
    let fade_frames = (TEST_DEVICE_RATE * UNDERRUN_FADE_SECS) as usize;
    let mut faded = vec![0.0; fade_frames * STEREO_CHANNELS];
    callback.render(&ring, &mut faded);
    assert!(callback.held[0] <= UNDERRUN_FADE_FLOOR * 1.01);
    assert_eq!(callback.held[0], callback.held[1]);
}
