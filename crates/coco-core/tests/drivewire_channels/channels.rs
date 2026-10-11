//! Plain channel traffic that no host service claims.

use std::collections::HashMap;

use crate::common::{boot_nitros9, collect, dw, run_until, type_line};

#[test]
fn nitros9_shell_redirection_writes_to_a_numbered_channel() {
    /// `/N5`: a numbered descriptor that the boot leaves unused.
    const CHANNEL: u8 = 5;
    let Some(mut m) = boot_nitros9() else {
        return;
    };
    assert!(!dw(&mut m).channel_info(CHANNEL).unwrap().open);

    type_line(&mut m, "echo HELLO FROM NITROS9 >/n5");
    let mut inbox = HashMap::new();
    let mut session = None;
    run_until(&mut m, "echo output on /N5", |m| {
        session = collect(m, &mut inbox, |bytes| bytes.contains(&b'\r'));
        session.is_some()
    });
    let handle = session.unwrap();
    assert_eq!(handle.channel(), CHANNEL);

    run_until(&mut m, "the guest to close /N5", |m| {
        !dw(m).channel_info(CHANNEL).unwrap().open
    });
    let mut received = inbox.remove(&handle).unwrap();
    received.extend(dw(&mut m).channel_receive(handle, usize::MAX).unwrap());
    // Shell+ passes the space before the redirection as part of the argument.
    assert_eq!(
        String::from_utf8_lossy(&received),
        "HELLO FROM NITROS9 \r",
        "output written before the close stays readable"
    );
    assert_eq!(dw(&mut m).channel_diagnostics().dropped_bytes, 0);
}
