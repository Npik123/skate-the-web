//! Bounded control messages between the game and the optional discovery helper.
use serde::{Deserialize, Serialize};
pub const NAMESPACE: &str = "skate3rust-free-skate-v5";
// Spacewar is shared with other games; identify ours without filtering versions.
pub fn is_game_lobby(namespace: &str) -> bool {
    namespace.starts_with("skate3rust-free-skate-v")
}
pub const PAGE_SIZE: usize = 5;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: Command,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Browse {
        page: usize,
        map: u64,
        physics: u64,
    },
    Host {
        map: u64,
        physics: u64,
        label: String,
        /// Players the lobby holds (0: the default).
        #[serde(default)]
        capacity: usize,
    },
    /// The owner changes how many players the lobby holds.
    Capacity {
        capacity: usize,
    },
    Join {
        lobby: u64,
        map: u64,
        physics: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub id: u64,
    pub map: String,
    pub players: usize,
    pub capacity: usize,
    pub compatible: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Event {
    Rows {
        page: usize,
        total: usize,
        rows: Vec<Row>,
    },
    Owner {
        lobby: u64,
        owner: u64,
        own: u64,
    },
    Error(String),
    /// A request with nothing to report succeeded.
    Done,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub request: u64,
    pub event: Event,
}
pub fn encode<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("Control serialization")
}
pub fn request(text: &str) -> Option<Request> {
    if text.len() > 1200 {
        return None;
    }
    let r: Request = serde_json::from_str(text).ok()?;
    match &r.command {
        Command::Browse { page, .. } if *page > 100 => return None,
        Command::Host { label, .. } if label.len() > 64 || label.contains('\0') => return None,
        _ => (),
    };
    Some(r)
}
pub fn response(text: &str) -> Option<Response> {
    if text.len() > 1400 {
        return None;
    }
    let r: Response = serde_json::from_str(text).ok()?;
    match &r.event {
        Event::Rows { rows, .. }
            if rows.len() > PAGE_SIZE
                || rows
                    .iter()
                    .any(|r| r.map.len() > 64 || r.capacity > crate::lobby::MAX_PLAYERS || r.players > crate::lobby::MAX_PLAYERS) =>
        {
            return None;
        }
        Event::Owner { lobby, owner, own } if *lobby == 0 || *owner == 0 || *own == 0 => {
            return None;
        }
        _ => (),
    };
    Some(r)
}

/// A session label's limits: room for "Map | host name" (a 16-character name).
pub const LABEL_CHARS: usize = 48;
pub const LABEL_BYTES: usize = 64;

/// How a session shows in the browser: "Map | host name". The host's name is
/// what players look for, so it stays whole and a long map name is shortened.
pub fn session_label(map: &str, host: &str) -> String {
    let host = label(host);
    if host.is_empty() {
        return label(map);
    }
    let room = LABEL_CHARS.saturating_sub(host.chars().count() + 3);
    let mut map: String = map.chars().filter(|c| !c.is_control()).take(room).collect();
    while !map.is_empty() && map.len() + host.len() + 3 > LABEL_BYTES {
        map.pop();
    }
    if map.is_empty() { host } else { label(&format!("{map} | {host}")) }
}

/// Keep labels readable and bounded in bytes, including Unicode map names.
pub fn label(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars().filter(|c| !c.is_control()).take(LABEL_CHARS) {
        if result.len() + c.len_utf8() > LABEL_BYTES {
            break;
        }
        result.push(c);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_show_map_and_host() {
        assert_eq!(session_label("DownTown", "aaddpp"), "DownTown | aaddpp");
        // The whole name survives; the map gives way.
        let long = session_label(&"M".repeat(60), "SixteenCharName1");
        assert!(long.ends_with(" | SixteenCharName1") && long.chars().count() <= LABEL_CHARS, "{long}");
        assert_eq!(session_label("DownTown", ""), "DownTown");
    }
    #[test]
    fn unicode_labels_and_full_page_fit_control_datagram() {
        let name = label(&"\u{1f600}".repeat(80));
        assert!(name.len() <= LABEL_BYTES);
        let rows = (1..=5)
            .map(|id| Row {
                id,
                map: name.clone(),
                players: 10,
                capacity: 10,
                compatible: true,
            })
            .collect();
        assert!(
            response(&encode(&Response {
                request: 1,
                event: Event::Rows {
                    page: 0,
                    total: 5,
                    rows
                }
            }))
            .is_some()
        );
        assert!(
            request(&encode(&Request {
                id: 1,
                command: Command::Host {
                    map: 1,
                    physics: 2,
                    label: name,
                    capacity: crate::lobby::MAX_PLAYERS,
                }
            }))
            .is_some()
        );
    }
    #[test]
    fn rejects_invalid_owners_and_unbounded_requests() {
        assert!(
            response(&encode(&Response {
                request: 0,
                event: Event::Owner {
                    lobby: 1,
                    owner: 0,
                    own: 2
                }
            }))
            .is_none()
        );
        assert!(
            request(&encode(&Request {
                id: 1,
                command: Command::Browse {
                    page: 101,
                    map: 1,
                    physics: 2
                }
            }))
            .is_none()
        );
        assert!(request(&"x".repeat(1201)).is_none());
    }
}
