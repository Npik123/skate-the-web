//! Windows device transport. Raw signed axes/trigger bytes reach the TU3
//! converter without Bevy/gilrs deadzones or normalized-axis reconstruction.
use skate_core::input::xbox::XboxState;

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    #[cfg(all(not(windows), not(target_arch = "wasm32")))]
    UnsupportedPlatform,
}

/// Device identity is metadata; raw input is still sampled every host frame.
/// Refresh periodically as well as after errors, so hot swaps cannot leave a
/// subtype cached indefinitely even if Windows never exposes a disconnect.
#[derive(Default)]
pub(crate) struct CapabilityCache {
    value: Option<(u8, web_time::Instant)>,
}
impl CapabilityCache {
    pub(crate) fn invalidate(&mut self) {
        self.value = None;
    }
    fn get(
        &mut self,
        now: web_time::Instant,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        if let Some((subtype, expires)) = self.value {
            if now < expires {
                return Ok(subtype);
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype, now + std::time::Duration::from_secs(1)));
        Ok(subtype)
    }
}

/// Browser transport. Standard-mapping gamepads are converted to the same raw
/// XInput integers the Windows path hands the converter: buttons keep XInput
/// bit positions, sticks are the browser axis scaled to i16 with Y inverted
/// (XInput up is positive) and triggers are the analog button value scaled to
/// u8. No deadzone or other normalisation is added. With no pad connected,
/// slot 0 reports the keyboard mapping the page provides.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use wasm_bindgen::{JsCast, prelude::*};

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = skateHost, js_name = keyboardPad)]
        fn keyboard_pad() -> Option<Vec<i32>>;
    }

    const XINPUT_DEVSUBTYPE_GAMEPAD: u8 = 1;
    const BUTTONS: [(usize, u16); 16] = [
        (0, 0x1000),
        (1, 0x2000),
        (2, 0x4000),
        (3, 0x8000),
        (4, 0x0100),
        (5, 0x0200),
        (8, 0x0020),
        (9, 0x0010),
        (10, 0x0040),
        (11, 0x0080),
        (12, 0x0001),
        (13, 0x0002),
        (14, 0x0004),
        (15, 0x0008),
        (6, 0),
        (7, 0),
    ];

    fn axis(value: f64, invert: bool) -> i16 {
        let value = if value.is_finite() { value.clamp(-1., 1.) } else { 0. };
        let scaled = (if invert { -value } else { value } * 32767.).round();
        scaled as i16
    }
    fn trigger(value: f64) -> u8 {
        (value.clamp(0., 1.) * 255.).round() as u8
    }

    /// Pads in the browser's standard layout first (Xbox, and PlayStation
    /// 3/4/5 pads in current browsers), then pads the browser could not map
    /// but that are shaped like one (4+ axes, 12+ buttons; some PS3 pads),
    /// read with the standard layout. Wheels and sticks are left out.
    fn connected_pads() -> Vec<web_sys::Gamepad> {
        let Some(window) = web_sys::window() else { return Vec::new() };
        let Ok(list) = window.navigator().get_gamepads() else { return Vec::new() };
        let pads: Vec<web_sys::Gamepad> = list
            .iter()
            .filter_map(|entry| entry.dyn_into::<web_sys::Gamepad>().ok())
            .filter(|pad| pad.connected())
            .collect();
        let standard = |pad: &web_sys::Gamepad| pad.mapping() == web_sys::GamepadMappingType::Standard;
        let pad_shaped = |pad: &web_sys::Gamepad| pad.axes().length() >= 4 && pad.buttons().length() >= 12;
        pads.iter()
            .filter(|pad| standard(pad))
            .chain(pads.iter().filter(|pad| !standard(pad) && pad_shaped(pad)))
            .cloned()
            .collect()
    }

    fn keyboard(index: u32) -> Result<DevicePacket, DeviceError> {
        let values = if index == 0 { keyboard_pad() } else { None };
        let Some(v) = values.filter(|v| v.len() >= 7) else {
            return Err(DeviceError::Disconnected);
        };
        Ok(DevicePacket {
            number: 0,
            state: XboxState {
                buttons: v[0] as u16,
                triggers: [v[1].clamp(0, 255) as u8, v[2].clamp(0, 255) as u8],
                left: [v[3].clamp(-32767, 32767) as i16, v[4].clamp(-32767, 32767) as i16],
                right: [v[5].clamp(-32767, 32767) as i16, v[6].clamp(-32767, 32767) as i16],
            },
            subtype: XINPUT_DEVSUBTYPE_GAMEPAD,
        })
    }

    pub(super) fn poll(index: u32) -> Result<DevicePacket, DeviceError> {
        let pads = connected_pads();
        let Some(pad) = pads.get(index as usize) else {
            return keyboard(if pads.is_empty() { index } else { u32::MAX });
        };
        let buttons = pad.buttons();
        let mut mask = 0u16;
        for (slot, bit) in BUTTONS {
            if bit != 0 && (slot as u32) < buttons.length() {
                let button: web_sys::GamepadButton = buttons.get(slot as u32).unchecked_into();
                if button.pressed() {
                    mask |= bit;
                }
            }
        }
        // A pad the browser could not map may have fewer buttons.
        let analog = |slot: u32| -> f64 {
            if slot >= buttons.length() {
                return 0.;
            }
            let button: web_sys::GamepadButton = buttons.get(slot).unchecked_into();
            button.value()
        };
        let axes = pad.axes();
        let a = |i: u32| axes.get(i).as_f64().unwrap_or(0.);
        Ok(DevicePacket {
            number: pad.timestamp() as u32,
            state: XboxState {
                buttons: mask,
                triggers: [trigger(analog(6)), trigger(analog(7))],
                left: [axis(a(0), false), axis(a(1), true)],
                right: [axis(a(2), false), axis(a(3), true)],
            },
            subtype: XINPUT_DEVSUBTYPE_GAMEPAD,
        })
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::mem::MaybeUninit;

    // ABI from the installed Windows SDK Xinput.h. No OS-owned pointers are
    // retained and only successful calls permit reading output storage.
    #[repr(C)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        left_x: i16,
        left_y: i16,
        right_x: i16,
        right_y: i16,
    }
    #[repr(C)]
    struct State {
        number: u32,
        gamepad: Gamepad,
    }
    #[repr(C)]
    struct Vibration {
        left: u16,
        right: u16,
    }
    #[repr(C)]
    struct Capabilities {
        device_type: u8,
        subtype: u8,
        flags: u16,
        gamepad: Gamepad,
        vibration: Vibration,
    }
    const _: () = assert!(size_of::<Gamepad>() == 12);
    const _: () = assert!(size_of::<State>() == 16);
    const _: () = assert!(size_of::<Capabilities>() == 20);

    #[link(name = "xinput")]
    unsafe extern "system" {
        fn XInputGetState(index: u32, state: *mut State) -> u32;
        fn XInputGetCapabilities(index: u32, flags: u32, capabilities: *mut Capabilities) -> u32;
    }

    pub(super) fn poll(
        index: u32,
        cache: &mut CapabilityCache,
    ) -> Result<DevicePacket, DeviceError> {
        let mut state = MaybeUninit::<State>::uninit();
        // SAFETY: properly aligned writable storage with the SDK's exact C ABI.
        let result = unsafe { XInputGetState(index, state.as_mut_ptr()) };
        if result != 0 {
            cache.invalidate();
        }
        if result == 1167 {
            return Err(DeviceError::Disconnected);
        }
        if result != 0 {
            return Err(DeviceError::State(result));
        }
        let subtype = cache.get(web_time::Instant::now(), || {
            let mut capabilities = MaybeUninit::<Capabilities>::uninit();
            // SAFETY: writable storage with the SDK ABI; read only on success.
            let result = unsafe { XInputGetCapabilities(index, 1, capabilities.as_mut_ptr()) };
            if result != 0 {
                return Err(DeviceError::Capabilities(result));
            }
            Ok(unsafe { capabilities.assume_init() }.subtype)
        })?;
        // SAFETY: successful XInputGetState initialized the complete structure.
        let state = unsafe { state.assume_init() };
        Ok(DevicePacket {
            number: state.number,
            state: XboxState {
                buttons: state.gamepad.buttons,
                triggers: [state.gamepad.left_trigger, state.gamepad.right_trigger],
                left: [state.gamepad.left_x, state.gamepad.left_y],
                right: [state.gamepad.right_x, state.gamepad.right_y],
            },
            subtype,
        })
    }
}

/// Which XInput slots last answered: the other pads fill the empty ones.
#[cfg(not(target_arch = "wasm32"))]
static XINPUT: [std::sync::atomic::AtomicBool; 4] = [const { std::sync::atomic::AtomicBool::new(false) }; 4];

/// A PlayStation (or other non-XInput) pad for an empty XInput slot: the
/// first such pad takes the first empty slot, and so on.
#[cfg(not(target_arch = "wasm32"))]
fn other_pad(index: usize) -> Option<DevicePacket> {
    let empty_before = (0..index).filter(|&k| !XINPUT[k].load(std::sync::atomic::Ordering::Relaxed)).count();
    super::pads::snapshot().get(empty_before).map(|pad| DevicePacket {
        number: pad.number,
        state: pad.state,
        // XINPUT_DEVSUBTYPE_GAMEPAD: a standard pad.
        subtype: 1,
    })
}

pub(crate) fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    {
        let result = windows::poll(index as u32, cache);
        XINPUT[index].store(result.is_ok(), std::sync::atomic::Ordering::Relaxed);
        if matches!(result, Err(DeviceError::Disconnected)) {
            if let Some(packet) = other_pad(index) {
                return Ok(packet);
            }
        }
        return result;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = cache;
        return web::poll(index as u32);
    }
    #[cfg(all(not(windows), not(target_arch = "wasm32")))]
    {
        let _ = cache;
        other_pad(index).ok_or(DeviceError::Disconnected)
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn capability_cache_refreshes_and_never_caches_errors() {
        let start = web_time::Instant::now();
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.get(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get(start + std::time::Duration::from_millis(999), || panic!(
                "redundant capability query"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get(start + std::time::Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
        cache.invalidate();
        assert_eq!(
            cache.get(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get(start, || Ok(4)), Ok(4));
    }
}

// Preserve the uncached API for menu-only polling.
pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}
