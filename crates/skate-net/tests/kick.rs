use skate_net::lobby::{Info, Session};
fn info(id: u64) -> Info {
    Info { id, map: 1, rig: 2, physics: 3, appearance: 4 }
}
/// Guest i talks to the host from endpoint 10 + i; the host is endpoint 1.
fn pump(host: &mut Session, guests: &mut [Session], now: u64) {
    for (i, g) in guests.iter_mut().enumerate() {
        for p in g.service(now) {
            host.receive(i as u64 + 10, &p.data, now);
        }
    }
    for p in host.service(now) {
        if let Some(g) = guests.get_mut((p.peer - 10) as usize) {
            g.receive(1, &p.data, now);
        }
    }
}

#[test]
fn a_kicked_player_is_removed_and_refused_for_the_session() {
    let mut h = Session::new(7, info(1), None);
    let mut g = vec![Session::new(7, info(2), Some(1)), Session::new(7, info(3), Some(1))];
    for t in (0..2000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert!(h.actors.contains_key(&3) && g[0].actors.contains_key(&3));

    // Only the host can kick; it reports the endpoint the player used.
    assert_eq!(g[0].kick(3), None);
    assert_eq!(h.kick(3), Some(11));
    for t in (2000..4000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert!(g[1].kicked, "the kicked player is told why");
    assert!(!h.actors.contains_key(&3), "it keeps knocking but is not let back in");
    assert!(!g[0].actors.contains_key(&3), "the others drop it from the roster");
    assert!(!g[0].kicked);

    // Rejoining from the same endpoint as a new player is refused too.
    g[1] = Session::new(7, info(30), Some(1));
    for t in (4000..6000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert!(g[1].kicked && !h.actors.contains_key(&30));

    // A new host after migration keeps the session's bans.
    let mut next = Session::new(7, info(2), None);
    next.ban(11, 3);
    assert!(next.is_banned(11, 99) && next.is_banned(55, 3) && !next.is_banned(10, 2));
}
