use super::*;

fn store_with_session(now: Instant) -> (SessionStore, String) {
    let mut store = SessionStore::default();
    let id = store
        .create(now, ProtocolVersion::March2025)
        .expect("session admitted");
    (store, id)
}

#[test]
fn session_store_rejects_capacity_until_a_session_expires() {
    let now = Instant::now();
    let mut store = SessionStore::default();
    for _ in 0..MAX_CONTROL_SESSIONS {
        assert!(store.create(now, ProtocolVersion::June2025).is_some());
    }
    assert!(store.create(now, ProtocolVersion::June2025).is_none());

    let expired = now + CONTROL_SESSION_IDLE_TIMEOUT;
    assert!(store.create(expired, ProtocolVersion::June2025).is_some());
    assert_eq!(store.sessions.len(), 1);
}

#[test]
fn starting_a_request_refreshes_the_idle_deadline() {
    let now = Instant::now();
    let (mut store, id) = store_with_session(now);
    let refreshed = now + CONTROL_SESSION_IDLE_TIMEOUT / 2;

    assert_eq!(
        store.start(&id, refreshed),
        Some(ProtocolVersion::March2025)
    );
    store.finish(&id, refreshed);
    assert_eq!(
        store.start(&id, now + CONTROL_SESSION_IDLE_TIMEOUT),
        Some(ProtocolVersion::March2025)
    );
}

#[test]
fn an_idle_session_expires() {
    let now = Instant::now();
    let (mut store, id) = store_with_session(now);
    assert_eq!(store.start(&id, now + CONTROL_SESSION_IDLE_TIMEOUT), None);
}

#[test]
fn a_request_in_flight_keeps_its_session_past_the_idle_timeout() {
    let now = Instant::now();
    let (mut store, id) = store_with_session(now);
    store.start(&id, now).expect("session known");

    // Another connection's request runs expiry long after this one started.
    let much_later = now + CONTROL_SESSION_IDLE_TIMEOUT * 3;
    assert!(
        store
            .create(much_later, ProtocolVersion::June2025)
            .is_some()
    );
    assert!(store.sessions.contains_key(&id));

    // The idle clock restarts when the long request is answered.
    store.finish(&id, much_later);
    let before_timeout = much_later + CONTROL_SESSION_IDLE_TIMEOUT / 2;
    assert_eq!(
        store.start(&id, before_timeout),
        Some(ProtocolVersion::March2025)
    );
}

#[test]
fn dropping_an_active_session_finishes_its_request() {
    let now = Instant::now();
    let (store, id) = store_with_session(now);
    let sessions = Mutex::new(store);

    let (version, active) = ActiveSession::start(&sessions, &id, now).expect("session known");
    assert_eq!(version, ProtocolVersion::March2025);
    assert_eq!(sessions.lock().unwrap().sessions[&id].in_flight, 1);
    drop(active);
    let store = sessions.lock().unwrap();
    assert_eq!(store.sessions[&id].in_flight, 0);
    assert!(store.sessions[&id].last_used >= now);
}

#[test]
fn an_unknown_session_does_not_start() {
    let sessions = Mutex::new(SessionStore::default());
    assert!(ActiveSession::start(&sessions, "nope", Instant::now()).is_none());
}

#[test]
fn finishing_a_deleted_session_is_a_no_op() {
    let now = Instant::now();
    let (mut store, id) = store_with_session(now);
    store.start(&id, now).expect("session known");
    store.remove(&id);
    store.finish(&id, now);
    assert!(store.sessions.is_empty());
}
