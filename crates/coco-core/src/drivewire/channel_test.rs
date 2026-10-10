use super::*;

fn open_with_data(channels: &mut Channels, channel: u8, bytes: &[u8]) -> ChannelHandle {
    channels.guest_open(channel);
    let handle = channels.info(channel).unwrap().handle.unwrap();
    assert_eq!(channels.send(handle, bytes), Ok(bytes.len()));
    handle
}

#[test]
fn poll_cursor_wraps_past_the_last_channel() {
    let mut channels = Channels::default();
    let last = (CHANNEL_COUNT - 1) as u8;
    open_with_data(&mut channels, last, b"z");
    open_with_data(&mut channels, 0, b"a");

    assert_eq!(
        channels.poll(),
        PollReply::Block {
            channel: 0,
            count: 1
        }
    );
    assert_eq!(
        channels.poll(),
        PollReply::Block {
            channel: last,
            count: 1
        }
    );
    assert_eq!(
        channels.poll(),
        PollReply::Block {
            channel: 0,
            count: 1
        }
    );
}

#[test]
fn every_waiting_channel_is_served_within_one_rotation() {
    let mut channels = Channels::default();
    for channel in 0..CHANNEL_COUNT as u8 {
        open_with_data(&mut channels, channel, &[channel; 300]);
    }
    let served: Vec<u8> = (0..CHANNEL_COUNT)
        .map(|_| match channels.poll() {
            PollReply::Block { channel, count } => {
                assert_eq!(count, u8::MAX);
                channel
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(served, (0..CHANNEL_COUNT as u8).collect::<Vec<_>>());
}

#[test]
fn closed_channels_are_never_polled() {
    let mut channels = Channels::default();
    let handle = open_with_data(&mut channels, 2, b"x");
    channels.guest_close(2);
    assert_eq!(channels.poll(), PollReply::Idle);
    assert_eq!(channels.hangup(handle), Err(ChannelError::NotOpen));
}

#[test]
fn info_is_absent_outside_the_channel_range() {
    let channels = Channels::default();
    assert!(channels.info(CHANNEL_COUNT as u8).is_none());
    let info = channels.info(0).unwrap();
    assert!(
        !info.open && info.handle.is_none(),
        "never opened: no handle"
    );
}

#[test]
fn diagnostics_total_every_channel() {
    let mut channels = Channels::default();
    open_with_data(&mut channels, 1, b"abc");
    open_with_data(&mut channels, 4, b"de");
    channels.guest_write(4, b"xyz");
    channels.guest_write(5, b"lost");

    let diagnostics = channels.diagnostics();
    assert_eq!(diagnostics.open, 2);
    assert_eq!(diagnostics.to_guest, 5);
    assert_eq!(diagnostics.from_guest, 3);
    assert_eq!(diagnostics.dropped_bytes, 4);

    channels.clear_counters();
    assert_eq!(channels.diagnostics().dropped_bytes, 0);
}

#[test]
fn reset_takes_a_new_epoch_for_every_handle() {
    let mut channels = Channels::default();
    let handle = open_with_data(&mut channels, 1, b"a");
    channels.reset();
    channels.guest_open(1);
    let reopened = channels.info(1).unwrap().handle.unwrap();
    assert_ne!(handle, reopened);
    assert_eq!(channels.receive(handle, 1), Err(ChannelError::Stale));
}
