//! Controllers XInput does not see: PlayStation 3/4/5 pads (and any other
//! pad the platform's game controller API knows), read with gilrs.
//!
//! gilrs runs on its own thread: its device enumeration can block for tens
//! of milliseconds (the reason Bevy's gilrs plugin is off), and gameplay
//! polls every frame. The thread keeps each pad's latest state, already in
//! XInput's integers (button bits, i16 sticks with up positive, u8 triggers),
//! so the game converts it exactly like an Xbox pad. No dead zone is added:
//! the game's own converter applies Skate 3's.
//!
//! Xbox pads are skipped here (XInput reads them), so one pad never fills two
//! slots.
use gilrs::{Axis, Button, GilrsBuilder};
use skate_core::input::xbox::XboxState;
use std::sync::{Mutex, Once};

/// A pad's state and a counter that moves whenever it changes.
#[derive(Clone, Copy)]
pub(crate) struct Pad {
    pub state: XboxState,
    pub number: u32,
}

static PADS: Mutex<Vec<Pad>> = Mutex::new(Vec::new());
static START: Once = Once::new();

/// The connected non-Xbox pads, in the order they were connected.
pub(crate) fn snapshot() -> Vec<Pad> {
    START.call_once(|| {
        let spawned = std::thread::Builder::new().name("gamepads".into()).spawn(run);
        if let Err(error) = spawned {
            bevy::log::warn!("Gamepad thread could not start ({error}); only XInput pads work");
        }
    });
    PADS.lock().map(|p| p.clone()).unwrap_or_default()
}

/// XInput's button bits for the platform's standard buttons.
const BUTTONS: [(Button, u16); 15] = [
    (Button::DPadUp, 0x0001),
    (Button::DPadDown, 0x0002),
    (Button::DPadLeft, 0x0004),
    (Button::DPadRight, 0x0008),
    (Button::Start, 0x0010),
    (Button::Select, 0x0020),
    (Button::LeftThumb, 0x0040),
    (Button::RightThumb, 0x0080),
    (Button::LeftTrigger, 0x0100),
    (Button::RightTrigger, 0x0200),
    (Button::Mode, 0x0400),
    // Cross / Circle / Square / Triangle sit where A / B / X / Y do.
    (Button::South, 0x1000),
    (Button::East, 0x2000),
    (Button::West, 0x4000),
    (Button::North, 0x8000),
];

/// An Xbox pad (XInput reads those) by its maker or name.
fn xbox(vendor: Option<u16>, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    vendor == Some(0x045E) || name.contains("xbox") || name.contains("xinput") || name.contains("x-box")
}

fn stick(value: f32) -> i16 {
    let value = if value.is_finite() { value.clamp(-1., 1.) } else { 0. };
    (value * 32767.).round() as i16
}
fn trigger(value: f32) -> u8 {
    let value = if value.is_finite() { value.clamp(0., 1.) } else { 0. };
    (value * 255.).round() as u8
}

fn run() {
    let mut gilrs = match GilrsBuilder::new().with_default_filters(false).build() {
        Ok(gilrs) => gilrs,
        Err(gilrs::Error::NotImplemented(gilrs)) => {
            bevy::log::info!("No gamepad API on this platform beyond XInput");
            gilrs
        }
        Err(error) => {
            bevy::log::warn!("Gamepads beyond XInput are unavailable ({error})");
            return;
        }
    };
    let mut announced = std::collections::BTreeSet::new();
    let mut number = 0u32;
    loop {
        // Wait for input (or a few ms), then take everything that queued up.
        let mut changed = gilrs.next_event_blocking(Some(std::time::Duration::from_millis(4))).is_some();
        while gilrs.next_event().is_some() {
            changed = true;
        }
        if changed {
            number = number.wrapping_add(1);
        }
        let mut pads = Vec::new();
        for (id, pad) in gilrs.gamepads() {
            if !pad.is_connected() || xbox(pad.vendor_id(), pad.name()) {
                continue;
            }
            if announced.insert(usize::from(id)) {
                bevy::log::info!("Gamepad connected: {} (as an Xbox-layout pad)", pad.name());
            }
            let mut buttons = 0u16;
            for (button, bit) in BUTTONS {
                if pad.is_pressed(button) {
                    buttons |= bit;
                }
            }
            // Triggers are analog buttons on most pads, axes on a few.
            let analog = |button: Button, axis: Axis| {
                let pressed = pad.button_data(button).map_or(0., |d| d.value());
                pressed.max(pad.value(axis))
            };
            pads.push(Pad {
                state: XboxState {
                    buttons,
                    triggers: [trigger(analog(Button::LeftTrigger2, Axis::LeftZ)), trigger(analog(Button::RightTrigger2, Axis::RightZ))],
                    left: [stick(pad.value(Axis::LeftStickX)), stick(pad.value(Axis::LeftStickY))],
                    right: [stick(pad.value(Axis::RightStickX)), stick(pad.value(Axis::RightStickY))],
                },
                number,
            });
        }
        if let Ok(mut shared) = PADS.lock() {
            *shared = pads;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox_pads_are_left_to_xinput() {
        assert!(xbox(Some(0x045E), "Controller"));
        assert!(xbox(None, "Xbox Wireless Controller"));
        assert!(!xbox(Some(0x054C), "DualSense Wireless Controller"));
        assert!(!xbox(Some(0x054C), "PS3 Controller"));
    }

    #[test]
    fn values_become_xinput_integers() {
        assert_eq!(stick(1.), 32767);
        assert_eq!(stick(-1.), -32767);
        assert_eq!(stick(f32::NAN), 0);
        assert_eq!(trigger(0.5), 128);
        assert_eq!(trigger(2.), 255);
    }
}
