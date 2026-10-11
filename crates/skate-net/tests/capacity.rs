use skate_net::lobby::{Info, MAX_PLAYERS, Session};
fn info(id: u64) -> Info {
    Info { id, map: 1, rig: 2, physics: 3, appearance: 4 }
}
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
fn the_host_sets_how_many_players_get_in() {
    let mut h = Session::new(7, info(1), None);
    h.set_capacity(2);
    let mut g = vec![Session::new(7, info(2), Some(1)), Session::new(7, info(3), Some(1))];
    for t in (0..2000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert!(h.actors.contains_key(&2) && !h.actors.contains_key(&3), "two players: the third is refused");
    assert_eq!(g[1].notice, "Lobby full");

    // Room for more: the waiting player gets in on their next hello.
    h.set_capacity(3);
    for t in (2000..4000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert!(h.actors.contains_key(&3));

    // Limits stay within 2..=MAX_PLAYERS.
    h.set_capacity(1);
    assert_eq!(h.capacity(), 2);
    h.set_capacity(500);
    assert_eq!(h.capacity(), MAX_PLAYERS);
}

#[test]
fn a_full_twenty_player_roster_fits_one_datagram() {
    let mut h = Session::new(7, info(1), None);
    h.set_capacity(MAX_PLAYERS);
    let mut g: Vec<Session> = (2..=MAX_PLAYERS as u64).map(|id| Session::new(7, info(id), Some(1))).collect();
    for t in (0..3000).step_by(50) {
        pump(&mut h, &mut g, t);
    }
    assert_eq!(h.actors.len(), MAX_PLAYERS);
    assert!(g.iter().all(|s| s.actors.len() == MAX_PLAYERS), "every guest sees all twenty");
}
