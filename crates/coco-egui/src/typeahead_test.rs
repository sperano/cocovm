use super::*;

/// Whether `pos` reads as pressed through the PIA sense path.
fn is_down(kb: &kbd::Keyboard, pos: Pos) -> bool {
    kb.sense(!(1 << pos.1)) & (1 << pos.0) == 0
}

#[test]
fn hold_survives_a_release_all_between_fields() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('a').unwrap();
    ta.queue.push_back((pos, false));
    let mut kb = kbd::Keyboard::new();

    ta.advance(&mut kb);
    assert!(is_down(&kb, pos));
    // A focus-loss frame between fields clears the matrix.
    kb.release_all();
    ta.advance(&mut kb);
    assert!(is_down(&kb, pos), "hold must be re-asserted");
    assert!(ta.is_active());
}

#[test]
fn one_tap_takes_fields_per_tap_advances() {
    let mut ta = TypeAhead::default();
    ta.queue.push_back(kbd::char_key('a').unwrap());
    let mut kb = kbd::Keyboard::new();
    for _ in 0..FIELDS_PER_TAP - 1 {
        ta.advance(&mut kb);
        assert!(ta.is_active());
    }
    ta.advance(&mut kb);
    assert!(!ta.is_active());
}

#[test]
fn shifted_tap_holds_shift_and_releases_both() {
    let mut ta = TypeAhead::default();
    let (pos, shift) = kbd::char_key('!').unwrap();
    assert!(shift);
    ta.queue.push_back((pos, shift));
    let mut kb = kbd::Keyboard::new();

    for _ in 0..=TYPE_HOLD_FIELDS {
        ta.advance(&mut kb);
        assert!(is_down(&kb, pos) && is_down(&kb, kbd::SHIFT));
    }
    ta.advance(&mut kb);
    assert!(!is_down(&kb, pos) && !is_down(&kb, kbd::SHIFT));
}
