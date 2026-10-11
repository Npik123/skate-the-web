//! Kicking players, as CS:GO does it. The host can kick anyone outright;
//! anyone else can call a vote to kick a player. A kicked player is removed
//! and banned from the session for as long as it lasts (see
//! `skate_net::lobby::Session::kick`, and the browser relay's own lobby ban).
//!
//! Votes run on the host, which owns the session:
//! * a player calls a vote (`mp:kickcall`); the caller counts as a yes;
//! * everyone except the player in question votes yes or no
//!   (`mp:kickballot`): F1 / F2, the pause menu's Online column, or `/yes`
//!   and `/no` in chat;
//! * it passes when more than half of the other players vote yes, fails as
//!   soon as that can no longer happen, and times out after 30 seconds;
//! * a failed vote keeps its caller from calling another for two minutes;
//! * the host cannot be vote-kicked (the host runs the session).
//!
//! The host shares the vote (`mp:kickvote`) and the session's bans
//! (`mp:bans`), so a new host after migration keeps refusing them.
use super::Multiplayer;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

const CALL_KEY: &str = "mp:kickcall";
const VOTE_KEY: &str = "mp:kickvote";
const BALLOT_KEY: &str = "mp:kickballot";
const BANS_KEY: &str = "mp:bans";
/// How long a vote stays open.
const VOTE_SECONDS: f64 = 30.;
/// A failed vote's caller waits this long before calling another.
const COOLDOWN_SECONDS: f64 = 120.;
/// A finished vote's result stays up this long.
const RESULT_SECONDS: f64 = 5.;

/// A banned player: the endpoint they used, their player id and name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Ban {
    pub peer: u64,
    pub actor: u64,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Call {
    seq: u32,
    target: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Ballot {
    vote: u32,
    yes: bool,
}

/// A vote as the host runs it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Vote {
    pub id: u32,
    pub target: u64,
    pub caller: u64,
    pub yes: Vec<u64>,
    pub no: Vec<u64>,
    /// Players who may vote (everyone but the target), when last counted.
    pub voters: usize,
    /// Seconds left when published.
    pub left: f32,
    /// Some(true): kicked; Some(false): failed.
    pub result: Option<bool>,
}

/// The host's shared record: the vote, and the last call it refused (to
/// whom, which call, why).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Shared {
    vote: Option<Vote>,
    refused: Option<(u64, u32, String)>,
}

/// What the menus and chat ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Request {
    /// Host: kick now.
    Kick(u64),
    /// Call a vote to kick.
    Call(u64),
    /// Vote in the open vote.
    Ballot(bool),
}

static REQUESTS: Mutex<Vec<Request>> = Mutex::new(Vec::new());
/// The open vote for the menu: (target name, my ballot).
static OPEN: Mutex<Option<(u64, String, Option<bool>)>> = Mutex::new(None);
/// The menu's player picker, in its order.
static TARGETS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

pub(crate) fn request(request: Request) {
    REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).push(request);
}
/// The open vote, if any, that this player can vote in: (target, name, ballot).
pub(crate) fn open_vote() -> Option<(u64, String, Option<bool>)> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
/// The players the menu offers to kick (everyone else; not the host for a
/// vote), remembered so the picked row maps back to a player.
pub(crate) fn menu_targets(net: &Multiplayer) -> Vec<(u64, String)> {
    let local = net.session_clock().map_or(0, |c| c.0);
    let host = net.host_actor();
    let hosting = net.is_host();
    let rows: Vec<(u64, String)> = net
        .player_ids()
        .into_iter()
        .filter(|&id| id != local && (hosting || id != host))
        .map(|id| (id, net.display_name(id)))
        .collect();
    *TARGETS.lock().unwrap_or_else(|e| e.into_inner()) = rows.iter().map(|r| r.0).collect();
    rows
}
/// The menu's picked row (1-based; 0 is the "choose a player" prompt).
pub(crate) fn menu_pick(value: usize, hosting: bool) {
    let target = value.checked_sub(1).and_then(|i| TARGETS.lock().unwrap_or_else(|e| e.into_inner()).get(i).copied());
    if let Some(target) = target {
        request(if hosting { Request::Kick(target) } else { Request::Call(target) });
    }
}
/// Chat commands: `/kick name` (host), `/votekick name`, `/yes`, `/no`.
/// Returns the reply to show, or None when the text is not a command.
pub(crate) fn chat_command(net: &Multiplayer, text: &str) -> Option<String> {
    let text = text.trim();
    let (command, rest) = text.split_once(' ').unwrap_or((text, ""));
    let find = |name: &str| -> Result<u64, String> {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("Name a player: /votekick name".into());
        }
        let local = net.session_clock().map_or(0, |c| c.0);
        let matches: Vec<u64> = net
            .player_ids()
            .into_iter()
            .filter(|&id| id != local && net.display_name(id).to_ascii_lowercase().starts_with(&name))
            .collect();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(format!("No player called {}.", rest.trim())),
            _ => Err("More than one player matches; type more of the name.".into()),
        }
    };
    match command.to_ascii_lowercase().as_str() {
        "/kick" if net.is_host() => Some(find(rest).map_or_else(|e| e, |id| {
            request(Request::Kick(id));
            String::new()
        })),
        "/kick" => Some("Only the host can kick. Use /votekick name.".into()),
        "/votekick" => Some(find(rest).map_or_else(|e| e, |id| {
            request(if net.is_host() { Request::Kick(id) } else { Request::Call(id) });
            String::new()
        })),
        "/yes" | "/f1" => {
            request(Request::Ballot(true));
            Some(String::new())
        }
        "/no" | "/f2" => {
            request(Request::Ballot(false));
            Some(String::new())
        }
        _ => None,
    }
}

#[derive(Resource, Default)]
pub(crate) struct Kicks {
    call: Option<Call>,
    ballot: Option<Ballot>,
    // The host's side.
    shared: Shared,
    ends: f64,
    next_id: u32,
    seen_calls: BTreeMap<u64, u32>,
    cooldowns: BTreeMap<u64, f64>,
    published: Option<Shared>,
    publish_in: f32,
    published_bans: usize,
    // Everyone's side: what was announced.
    announced: Option<(u32, Option<bool>)>,
    refusal_seen: u32,
}

/// The host's vote count: passed, failed, or still open.
pub(crate) fn tally(yes: usize, no: usize, voters: usize) -> Option<bool> {
    if yes * 2 > voters {
        Some(true)
    } else if (voters - no.min(voters)) * 2 <= voters {
        Some(false)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    mut kicks: ResMut<Kicks>,
    mut net: ResMut<Multiplayer>,
    mut chat: ResMut<super::chat::Chat>,
    mut modes: ResMut<crate::game_modes::GameModes>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    if let Some(text) = net.kicked_notice.take() {
        modes.announce(text, 5.);
    }
    let requests = std::mem::take(&mut *REQUESTS.lock().unwrap_or_else(|e| e.into_inner()));
    let Some((local, hosting, _)) = net.session_clock() else {
        *kicks = Kicks::default();
        *OPEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return;
    };
    let players = net.player_ids();
    let name = |net: &Multiplayer, id: u64| net.display_name(id);

    // The open vote as everyone sees it: the host's own, or the host's record.
    let host = net.host_actor();
    let shared: Shared = if hosting {
        kicks.shared.clone()
    } else {
        net.records(VOTE_KEY)
            .into_iter()
            .find(|(id, _, _)| *id == host)
            .and_then(|(_, v, _)| serde_json::from_slice(&v).ok())
            .unwrap_or_default()
    };
    let open = shared.vote.as_ref().filter(|v| v.result.is_none()).cloned();

    // Requests from the menu, chat and keys.
    let mut requests = requests;
    if open.is_some() && !chat.has_keys() {
        if keys.just_pressed(KeyCode::F1) {
            requests.push(Request::Ballot(true));
        } else if keys.just_pressed(KeyCode::F2) {
            requests.push(Request::Ballot(false));
        }
    }
    for request in requests {
        match request {
            Request::Kick(target) if hosting => {
                let who = name(&net, target);
                if net.kick(target) {
                    chat.notify(format!("{who} was kicked by the host."), now);
                }
            }
            Request::Kick(_) => chat.notify("Only the host can kick. Call a vote instead.".into(), now),
            Request::Call(target) => {
                let seq = kicks.call.map_or(1, |c| c.seq.wrapping_add(1).max(1));
                kicks.call = Some(Call { seq, target });
                if !hosting {
                    if let Ok(bytes) = serde_json::to_vec(&kicks.call) {
                        net.publish_application(CALL_KEY, bytes);
                    }
                }
            }
            Request::Ballot(yes) => match &open {
                Some(vote) if vote.target != local => {
                    kicks.ballot = Some(Ballot { vote: vote.id, yes });
                    if let Ok(bytes) = serde_json::to_vec(&kicks.ballot) {
                        net.publish_application(BALLOT_KEY, bytes);
                    }
                    chat.notify(if yes { "You voted yes.".into() } else { "You voted no.".into() }, now);
                }
                Some(_) => chat.notify("You can't vote on your own kick.".into(), now),
                None => chat.notify("There is no vote to kick open.".into(), now),
            },
        }
    }

    if hosting {
        host_votes(&mut kicks, &mut net, local, &players, now, time.delta_secs());
    } else {
        // Take on the host's bans, ready in case we become the host.
        let bans = net
            .records(BANS_KEY)
            .into_iter()
            .find(|(id, _, _)| *id == host)
            .and_then(|(_, v, _)| serde_json::from_slice::<Vec<Ban>>(&v).ok());
        if let Some(bans) = bans {
            net.adopt_bans(bans);
        }
    }

    // Tell everyone what happens.
    let shared = if hosting { kicks.shared.clone() } else { shared };
    if let Some(vote) = &shared.vote {
        let state = (vote.id, vote.result);
        if kicks.announced != Some(state) {
            let target = name(&net, vote.target);
            let text = match vote.result {
                None if vote.target == local => format!("{} called a vote to kick you.", name(&net, vote.caller)),
                None => format!("Vote to kick {target}? F1 yes, F2 no (or Pause > Online)."),
                Some(true) => format!("{target} was kicked by vote."),
                Some(false) => format!("The vote to kick {target} failed."),
            };
            if kicks.announced.is_none_or(|(id, _)| id != vote.id) || vote.result.is_some() {
                modes.announce(text.clone(), 4.);
                chat.notify(text, now);
            }
            kicks.announced = Some(state);
        }
    }
    if let Some((caller, seq, reason)) = &shared.refused {
        if *caller == local && *seq != kicks.refusal_seen {
            kicks.refusal_seen = *seq;
            chat.notify(reason.clone(), now);
            modes.announce(reason.clone(), 3.);
        }
    }
    *OPEN.lock().unwrap_or_else(|e| e.into_inner()) = shared
        .vote
        .as_ref()
        .filter(|v| v.result.is_none() && v.target != local)
        .map(|v| (v.target, name(&net, v.target), kicks.ballot.filter(|b| b.vote == v.id).map(|b| b.yes)));
}

/// The host: start called votes, count ballots, kick on a pass, share it all.
fn host_votes(kicks: &mut Kicks, net: &mut Multiplayer, local: u64, players: &[u64], now: f64, dt: f32) {
    // Calls: the host's own, and each player's latest.
    let mut calls: Vec<(u64, Call)> = net
        .records(CALL_KEY)
        .into_iter()
        .filter_map(|(id, v, _)| serde_json::from_slice::<Option<Call>>(&v).ok().flatten().map(|c| (id, c)))
        .collect();
    if let Some(call) = kicks.call {
        calls.push((local, call));
    }
    for (caller, call) in calls {
        if kicks.seen_calls.get(&caller).is_some_and(|&seen| seen >= call.seq) {
            continue;
        }
        kicks.seen_calls.insert(caller, call.seq);
        let refuse = if kicks.shared.vote.as_ref().is_some_and(|v| v.result.is_none()) {
            Some("A vote is already open.".to_string())
        } else if call.target == local {
            Some("The host can't be vote-kicked.".to_string())
        } else if call.target == caller || !players.contains(&call.target) {
            Some("That player can't be kicked.".to_string())
        } else if let Some(&until) = kicks.cooldowns.get(&caller).filter(|&&until| until > now) {
            Some(format!("Your last vote failed. Wait {:.0}s to call another.", (until - now).ceil()))
        } else {
            None
        };
        match refuse {
            Some(reason) => {
                info!("MULTIPLAYER_VOTE_REFUSED caller={caller} reason={reason:?}");
                kicks.shared.refused = Some((caller, call.seq, reason));
            }
            None => {
                kicks.next_id = kicks.next_id.wrapping_add(1).max(1);
                kicks.shared.vote = Some(Vote {
                    id: kicks.next_id,
                    target: call.target,
                    caller,
                    yes: vec![caller],
                    no: vec![],
                    voters: 0,
                    left: VOTE_SECONDS as f32,
                    result: None,
                });
                kicks.ends = now + VOTE_SECONDS;
                info!("MULTIPLAYER_VOTE_KICK target={} caller={caller}", call.target);
            }
        }
    }

    // Count the open vote.
    if let Some(vote) = kicks.shared.vote.clone() {
        if vote.result.is_none() {
            let mut ballots: BTreeMap<u64, bool> = net
                .records(BALLOT_KEY)
                .into_iter()
                .filter_map(|(id, v, _)| serde_json::from_slice::<Option<Ballot>>(&v).ok().flatten().map(|b| (id, b)))
                .filter(|(_, b)| b.vote == vote.id)
                .map(|(id, b)| (id, b.yes))
                .collect();
            if let Some(b) = kicks.ballot.filter(|b| b.vote == vote.id) {
                ballots.insert(local, b.yes);
            }
            ballots.insert(vote.caller, true);
            let voters: Vec<u64> = players.iter().copied().filter(|&id| id != vote.target).collect();
            let yes: Vec<u64> = voters.iter().copied().filter(|id| ballots.get(id) == Some(&true)).collect();
            let no: Vec<u64> = voters.iter().copied().filter(|id| ballots.get(id) == Some(&false)).collect();
            let gone = !players.contains(&vote.target);
            let mut result = if gone { Some(false) } else { tally(yes.len(), no.len(), voters.len()) };
            if result.is_none() && now >= kicks.ends {
                result = Some(false);
            }
            if result == Some(true) && !net.kick(vote.target) {
                result = Some(false);
            }
            if result == Some(false) && !gone {
                kicks.cooldowns.insert(vote.caller, now + COOLDOWN_SECONDS);
            }
            if result.is_some() {
                kicks.ends = now + RESULT_SECONDS;
                info!("MULTIPLAYER_VOTE_RESULT target={} kicked={:?}", vote.target, result);
            }
            kicks.shared.vote = Some(Vote {
                yes,
                no,
                voters: voters.len(),
                left: (kicks.ends - now).max(0.) as f32,
                result,
                ..vote
            });
        } else if now >= kicks.ends {
            kicks.shared.vote = None;
        }
    }

    // Share the vote (on change, and its clock every second) and the bans.
    kicks.publish_in -= dt;
    let changed = kicks.published.as_ref().is_none_or(|p| {
        p.refused != kicks.shared.refused
            || p.vote.as_ref().map(|v| (v.id, &v.yes, &v.no, v.result)) != kicks.shared.vote.as_ref().map(|v| (v.id, &v.yes, &v.no, v.result))
    });
    if changed || kicks.publish_in <= 0. {
        kicks.publish_in = 1.;
        if let Ok(bytes) = serde_json::to_vec(&kicks.shared) {
            net.publish_application(VOTE_KEY, bytes);
        }
        kicks.published = Some(kicks.shared.clone());
    }
    if net.bans().len() != kicks.published_bans {
        kicks.published_bans = net.bans().len();
        if let Ok(bytes) = serde_json::to_vec(net.bans()) {
            net.publish_application(BANS_KEY, bytes);
        }
    }
    // A new host (after migration) refuses the old host's bans too.
    let bans = net.bans().to_vec();
    net.adopt_bans(bans);
}

/// Test hook (SKATE_VERIFY_KICK=kick|call|call2|yes|no, with SKATE_VERIFY_PLAYERS=n):
/// once n players are in, `kick` (host) kicks the last other player, `call`
/// calls a vote against the first player it may, and `yes` votes yes in any
/// open vote. Logs the session's players every second.
pub(super) fn verify(net: Res<Multiplayer>, time: Res<Time<Real>>, mut state: Local<(f64, bool, f64)>, mut again: Local<bool>) {
    let Ok(mode) = std::env::var("SKATE_VERIFY_KICK") else { return };
    let wanted = std::env::var("SKATE_VERIFY_PLAYERS").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(3);
    let now = time.elapsed_secs_f64();
    if now - state.2 >= 1. {
        state.2 = now;
        info!("MULTIPLAYER_VERIFY_KICK players={} active={} host={}", net.player_ids().len(), net.active(), net.is_host());
    }
    if net.player_ids().len() >= wanted && state.0 == 0. {
        state.0 = now + 3.;
    }
    if state.0 == 0. || now < state.0 {
        return;
    }
    match mode.as_str() {
        "kick" if net.is_host() && !state.1 => {
            state.1 = true;
            let local = net.session_clock().map_or(0, |c| c.0);
            if let Some(&target) = net.player_ids().iter().rev().find(|&&id| id != local) {
                request(Request::Kick(target));
            }
        }
        "call" if !state.1 => {
            state.1 = true;
            if let Some(&(target, _)) = menu_targets(&net).first() {
                request(Request::Call(target));
            }
        }
        "yes" | "no" => {
            if open_vote().is_some_and(|(_, _, ballot)| ballot.is_none()) {
                request(Request::Ballot(mode == "yes"));
            }
        }
        // `call2`: call, then call again 5 s later (the cooldown refuses it).
        "call2" => {
            let second = state.1 && !*again && now > state.0 + 5.;
            if !state.1 || second {
                *again |= second;
                state.1 = true;
                if let Some(&(target, _)) = menu_targets(&net).first() {
                    request(Request::Call(target));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::tally;

    #[test]
    fn more_than_half_of_the_other_players_kicks() {
        // Four voters (five players, one the target): three yes passes.
        assert_eq!(tally(2, 0, 4), None);
        assert_eq!(tally(3, 0, 4), Some(true));
        // Two no of four: three yes is out of reach.
        assert_eq!(tally(1, 2, 4), Some(false));
        assert_eq!(tally(2, 1, 4), None);
        // Two players: the caller alone is everyone else.
        assert_eq!(tally(1, 0, 1), Some(true));
        // Three players: both others must agree.
        assert_eq!(tally(1, 0, 2), None);
        assert_eq!(tally(1, 1, 2), Some(false));
    }
}
