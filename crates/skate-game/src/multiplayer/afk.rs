//! Idle players leave the session on their own: anyone who touches no key,
//! button, stick, mouse or screen for `KICK_AFTER` is warned, then removed.
//! An idle player still streams a full skater to everyone through the relay.
//! A tab in the background stops sending at once, and the host drops it after
//! its usual five-second timeout.
use bevy::{
    input::{gamepad::Gamepad, mouse::{MouseMotion, MouseWheel}},
    prelude::*,
};

const KICK_AFTER: f32 = 300.;
const WARN_BEFORE: f32 = 30.;

#[derive(Default)]
pub(crate) struct Idle {
    since: f32,
    warned: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check(
    mut idle: Local<Idle>,
    mut net: ResMut<super::Multiplayer>,
    mut modes: ResMut<crate::game_modes::GameModes>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    nav: Res<crate::customiser::Navigation>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs();
    let moved = motion.read().count() > 0;
    let scrolled = wheel.read().count() > 0;
    let pad = pads.iter().any(|pad| {
        pad.get_just_pressed().next().is_some()
            || [pad.left_stick(), pad.right_stick()].iter().any(|stick| stick.length() > 0.3)
    });
    let active = keys.get_just_pressed().next().is_some()
        || mouse.get_just_pressed().next().is_some()
        || touches.iter_just_pressed().next().is_some()
        || nav.pressed != 0
        || moved
        || scrolled
        || pad;
    if active || !net.active() {
        if idle.warned && net.active() {
            modes.announce("Welcome back", 2.);
        }
        idle.since = now;
        idle.warned = false;
        return;
    }
    let quiet = now - idle.since;
    if quiet >= KICK_AFTER {
        info!("MULTIPLAYER_AFK leaving after {quiet:.0} s idle");
        net.leave();
        net.status = "Removed from the session for being idle. Host or join again from ONLINE.".into();
        modes.announce("Removed from the session for being idle", 6.);
        idle.since = now;
        idle.warned = false;
    } else if quiet >= KICK_AFTER - WARN_BEFORE && !idle.warned {
        idle.warned = true;
        modes.announce(format!("Idle: leaving the session in {WARN_BEFORE:.0} seconds"), WARN_BEFORE);
    }
}
