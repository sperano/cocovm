//! Process-lifetime host gamepad backend and its shared input snapshot.

use std::cell::RefCell;
#[cfg(test)]
use std::collections::VecDeque;
use std::rc::Rc;

use gilrs::{Axis, Button, EventType};

/// Current host gamepad values, copied into every VM that uses a gamepad source.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct GamepadState {
    /// Left analog stick, in gilrs' -1.0..=1.0 range.
    pub(super) axes: [f32; 2],
    /// South (button 0) and East (button 1).
    pub(super) buttons: [bool; 2],
}

/// Cloneable main-thread handle to the application's single gamepad backend.
///
/// `gilrs` is polled from egui's main thread. Keeping the backend behind `Rc`
/// lets each VM consume the same retained state after the first VM drains the
/// process event queue, without opening device handles or helper threads per VM.
#[derive(Clone)]
pub(crate) struct SharedGamepad(Rc<RefCell<GamepadHost>>);

struct GamepadHost {
    backend: Option<gilrs::Gilrs>,
    state: GamepadState,
    #[cfg(test)]
    queued_states: VecDeque<GamepadState>,
    #[cfg(test)]
    service_count: usize,
    #[cfg(test)]
    test_service_enabled: bool,
}

impl SharedGamepad {
    /// Open the one application-lifetime backend. CocoVM doesn't use rumble,
    /// so force-feedback device handles and worker resources stay disabled.
    pub(crate) fn new() -> Self {
        let backend = match gilrs::GilrsBuilder::new()
            .with_force_feedback(false)
            .build()
        {
            Ok(backend) => Some(backend),
            Err(error) => {
                tracing::warn!("gamepad input unavailable: {error}");
                None
            }
        };
        Self(Rc::new(RefCell::new(GamepadHost {
            backend,
            state: GamepadState::default(),
            #[cfg(test)]
            queued_states: VecDeque::new(),
            #[cfg(test)]
            service_count: 0,
            #[cfg(test)]
            test_service_enabled: false,
        })))
    }

    pub(super) fn available(&self) -> bool {
        self.0.borrow().backend.is_some()
    }

    /// Drain host events once and return the retained current state. Later VM
    /// updates in the same egui pass see the same state even when no events remain.
    pub(super) fn poll(&self) -> GamepadState {
        self.service();
        self.0.borrow().state
    }

    /// Drain the backend queue while retaining its latest values for every VM.
    /// Returns whether a backend exists and therefore needs periodic service.
    pub(crate) fn service(&self) -> bool {
        let mut host = self.0.borrow_mut();
        host.poll();
        let service_enabled = host.backend.is_some();
        #[cfg(test)]
        let service_enabled = service_enabled || host.test_service_enabled;
        service_enabled
    }

    #[cfg(test)]
    pub(crate) fn without_backend() -> Self {
        Self(Rc::new(RefCell::new(GamepadHost {
            backend: None,
            state: GamepadState::default(),
            queued_states: VecDeque::new(),
            service_count: 0,
            test_service_enabled: false,
        })))
    }

    #[cfg(test)]
    pub(crate) fn service_test_backend() -> Self {
        let gamepad = Self::without_backend();
        gamepad.0.borrow_mut().test_service_enabled = true;
        gamepad
    }

    #[cfg(test)]
    pub(super) fn set_test_state(&self, state: GamepadState) {
        self.0.borrow_mut().state = state;
    }

    #[cfg(test)]
    pub(super) fn queue_test_state(&self, state: GamepadState) {
        self.0.borrow_mut().queued_states.push_back(state);
    }

    #[cfg(test)]
    pub(crate) fn service_count(&self) -> usize {
        self.0.borrow().service_count
    }

    #[cfg(all(test, feature = "perf"))]
    pub(super) fn shares_backend(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl GamepadHost {
    fn poll(&mut self) {
        #[cfg(test)]
        {
            self.service_count += 1;
            if let Some(state) = self.queued_states.pop_front() {
                self.state = state;
            }
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        while let Some(gilrs::Event { event, .. }) = backend.next_event() {
            update_state(&mut self.state, event);
        }
    }
}

fn update_state(state: &mut GamepadState, event: EventType) {
    match event {
        EventType::AxisChanged(Axis::LeftStickX, value, _) => state.axes[0] = value,
        EventType::AxisChanged(Axis::LeftStickY, value, _) => state.axes[1] = value,
        EventType::ButtonPressed(Button::DPadLeft, _) => state.axes[0] = -1.0,
        EventType::ButtonPressed(Button::DPadRight, _) => state.axes[0] = 1.0,
        EventType::ButtonReleased(Button::DPadLeft | Button::DPadRight, _) => state.axes[0] = 0.0,
        EventType::ButtonPressed(Button::DPadUp, _) => state.axes[1] = -1.0,
        EventType::ButtonPressed(Button::DPadDown, _) => state.axes[1] = 1.0,
        EventType::ButtonReleased(Button::DPadUp | Button::DPadDown, _) => state.axes[1] = 0.0,
        EventType::ButtonPressed(Button::South, _) => state.buttons[0] = true,
        EventType::ButtonReleased(Button::South, _) => state.buttons[0] = false,
        EventType::ButtonPressed(Button::East, _) => state.buttons[1] = true,
        EventType::ButtonReleased(Button::East, _) => state.buttons[1] = false,
        _ => {}
    }
}

#[cfg(test)]
#[path = "gamepad_test.rs"]
mod tests;
