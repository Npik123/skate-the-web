//! What a session costs the browser relay (scripts/net-relay.js): every
//! datagram enters the server once and leaves it once, plus the relay's own
//! framing. Four skaters ride near each other for 30 s at the game's send
//! rates (body 20 Hz, pose 10 Hz, name and ping records).
use skate_net::{
    Body, Bone, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};
use std::collections::BTreeMap;

/// Per datagram on the relay: the 8-byte peer id and a WebSocket header, each way.
const RELAY_OVERHEAD: usize = 8 + 4;
const PLAYERS: usize = 4;
const SECONDS: u64 = 30;

fn info(id: u64) -> Info {
    Info { id, map: 1, rig: 2, physics: 3, appearance: 4 }
}

fn body(player: usize, t: f32, spacing: f32) -> BodyState {
    // Players ride circles of a few metres, close enough for full rate.
    let angle = t * 0.8 + player as f32;
    let centre = [player as f32 * spacing, 0., (player % 2) as f32 * spacing * 0.5];
    let root = Pose { p: [centre[0] + angle.cos() * 4., 0.1, angle.sin() * 4.], q: [0., (angle * 0.5).sin(), 0., (angle * 0.5).cos()] };
    let bodies = (0..33)
        .map(|i| {
            // Limbs swing on their own: every body differs a little, as a
            // riding ragdoll's do.
            let k = i as f32;
            let swing = (t * (2. + k * 0.37) + k).sin();
            let half = (angle * 0.5 + swing * 0.4 + k * 0.21).rem_euclid(std::f32::consts::TAU);
            let (s1, c1) = half.sin_cos();
            let tilt = swing * 0.3;
            let norm = (s1 * s1 + c1 * c1 + tilt * tilt).sqrt();
            Body {
                pose: Pose {
                    p: [root.p[0] + (i % 3) as f32 * 0.1 + swing * 0.05, root.p[1] + k * 0.05 + (t * 3. + k).sin() * 0.02, root.p[2] + swing * 0.03],
                    q: [tilt / norm, s1 / norm, 0., c1 / norm],
                },
                velocity: [-angle.sin() * 3.2 + swing * 0.7, (t * 5. + k).cos() * 0.4, angle.cos() * 3.2 - swing * 0.5],
                angular: [swing * 1.3, 0.4 + (t + k).sin(), (t * 2. + k).cos() * 0.8],
            }
        })
        .collect();
    BodyState { root, enabled: (1u64 << 33) - 1, bodies }
}

fn pose(player: usize, t: f32, spacing: f32) -> PoseState {
    let root = body(player, t, spacing).root;
    let bones = (0..20u16)
        .map(|i| {
            let a = (t * 4. + i as f32).sin() * 0.3;
            Bone { index: i, pose: Pose { p: [0., i as f32 * 0.08, 0.], q: [a.sin() * 0.5, 0., 0., (1. - a.sin() * a.sin() * 0.25).sqrt()] } }
        })
        .collect();
    PoseState { root, bones }
}

/// Relay bytes by packet kind for one simulated session.
pub fn measure(spacing: f32) -> (BTreeMap<u8, (usize, usize)>, usize) {
    let mut host = Session::new(7, info(1), None);
    let mut guests: Vec<_> = (0..PLAYERS - 1).map(|i| Session::new(7, info(10 + i as u64), Some(1))).collect();
    let mut by_kind: BTreeMap<u8, (usize, usize)> = BTreeMap::new();
    let mut count = |data: &[u8]| {
        let entry = by_kind.entry(data.get(24).copied().unwrap_or(0)).or_default();
        entry.0 += 1;
        entry.1 += (data.len() + RELAY_OVERHEAD) * 2;
    };
    for frame in 0..(SECONDS * 60) {
        let now = frame * 1000 / 60;
        let t = now as f32 / 1000.;
        if now % 50 < 17 {
            host.publish(packed::BODY, Packed::body(&body(0, t, spacing)).unwrap(), now);
            for (i, g) in guests.iter_mut().enumerate() {
                g.publish(packed::BODY, Packed::body(&body(i + 1, t, spacing)).unwrap(), now);
            }
        }
        if now % 100 < 17 {
            host.publish(packed::POSE, Packed::pose(&pose(0, t, spacing)).unwrap(), now);
            for (i, g) in guests.iter_mut().enumerate() {
                g.publish(packed::POSE, Packed::pose(&pose(i + 1, t, spacing)).unwrap(), now);
            }
        }
        if now % 1000 < 17 {
            let ping = ((now / 1000) % 40 + 20) as u32;
            host.publish_application("name", b"Host".to_vec(), now);
            for g in guests.iter_mut() {
                g.publish_application("name", b"Skater".to_vec(), now);
                g.publish_application("ping", ping.to_le_bytes().to_vec(), now);
            }
        }
        for (i, g) in guests.iter_mut().enumerate() {
            for p in g.service(now) {
                count(&p.data);
                host.receive(i as u64 + 10, &p.data, now + 15);
            }
        }
        for p in host.service(now) {
            count(&p.data);
            if let Some(g) = guests.get_mut((p.peer - 10) as usize) {
                g.receive(1, &p.data, now + 15);
            }
        }
    }
    let total = by_kind.values().map(|(_, b)| b).sum();
    (by_kind, total)
}

#[test]
fn relay_bandwidth_report() {
    report("together (a few metres apart)", 6.);
    report("spread over the map (~60 m apart)", 60.);
}

fn report(label: &str, spacing: f32) {
    let (by_kind, total) = measure(spacing);
    let name = |k: u8| match k {
        1 => "BODY", 2 => "POSE", 3 => "HELLO", 4 => "ROSTER", 5 => "ACK", 6 => "PING", 7 => "PONG",
        10 => "APPLICATION", 11 => "APP_ACK", _ => "other",
    };
    println!("relay traffic, {PLAYERS} players {label}, per second (in + out):");
    for (kind, (packets, bytes)) in &by_kind {
        println!("  {:12} {:7.1} pkt/s {:8.1} kB/s", name(*kind), *packets as f64 / SECONDS as f64, *bytes as f64 / SECONDS as f64 / 1000.);
    }
    println!("  TOTAL {:.1} kB/s = {:.2} GB per hour", total as f64 / SECONDS as f64 / 1000., total as f64 / SECONDS as f64 * 3600. / 1e9);
    assert!(total > 0);
}
