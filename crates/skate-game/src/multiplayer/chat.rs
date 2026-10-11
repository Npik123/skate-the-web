//! Text chat for a session. Skate 3 had voice chat only, so there is no
//! original screen to host: this is a small engine-drawn box, bottom left.
//!
//! Messages travel as each player's `mp:chat` application record (reliable,
//! resent until acknowledged, forwarded by the host) holding their last few
//! messages, so it works over Steam, direct and the browser relay alike.
//!
//! Spam is limited twice: the sender refuses to send faster than the limit,
//! and every receiver applies the same limit per player, so a modified client
//! cannot flood anyone either. Text is plain ASCII, short, and drawn as game
//! text, never as HTML, so it cannot inject markup or script into the page.
use super::Multiplayer;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use std::collections::{BTreeMap, VecDeque};

pub(super) const CHAT_KEY: &str = "mp:chat";
/// Longest message, in characters.
pub(crate) const MAX_CHARS: usize = 100;
/// Messages carried in the record: enough to survive a few quick ones in a row.
const CARRIED: usize = 4;
/// At most this many messages at once, then one more every REFILL seconds,
/// and never two closer together than MIN_GAP.
const BURST: f32 = 3.;
const REFILL: f32 = 2.;
const MIN_GAP: f64 = 0.75;
/// Trying again and again while held back earns a longer pause.
const STRIKES: usize = 4;
const STRIKE_WINDOW: f64 = 10.;
const COOLDOWN: f64 = 15.;
/// The same message again within this long is refused.
const REPEAT_WINDOW: f64 = 20.;
/// Lines shown, and how long they stay up when the box is closed.
const SHOWN: usize = 8;
const LINGER: f64 = 15.;
const KEPT: usize = 40;

/// Messages per player over time: a burst, then a steady trickle.
#[derive(Clone, Debug)]
pub(crate) struct Limiter {
    tokens: f32,
    at: f64,
    last: f64,
}
impl Default for Limiter {
    fn default() -> Self {
        Self { tokens: BURST, at: 0., last: f64::NEG_INFINITY }
    }
}
impl Limiter {
    /// Ok when a message may go now; otherwise the seconds to wait.
    pub(crate) fn allow(&mut self, now: f64) -> Result<(), f64> {
        self.tokens = (self.tokens + (now - self.at).max(0.) as f32 / REFILL).min(BURST);
        self.at = now;
        let gap = MIN_GAP - (now - self.last);
        if gap > 0. {
            return Err(gap);
        }
        if self.tokens < 1. {
            return Err(f64::from((1. - self.tokens) * REFILL));
        }
        self.tokens -= 1.;
        self.last = now;
        Ok(())
    }
}

/// What a message may contain: printable ASCII (what the game's font draws),
/// single spaces, no character more than four times in a row, MAX_CHARS long.
/// Applied when typing, when sending and again to everything received.
pub(crate) fn clean(raw: &str) -> String {
    let mut out = String::new();
    let mut run = (' ', 0usize);
    for c in raw.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if !(' '..='~').contains(&c) {
            continue;
        }
        if c == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        if c == run.0 {
            run.1 += 1;
            if run.1 > 4 {
                continue;
            }
        } else {
            run = (c, 1);
        }
        if out.len() >= MAX_CHARS {
            break;
        }
        out.push(c);
    }
    out.trim_end().to_owned()
}

/// `[u32 LE number][u8 length][text]` for each carried message, oldest first.
fn encode(sent: &VecDeque<(u32, String)>) -> Vec<u8> {
    let mut out = Vec::new();
    for (number, text) in sent {
        out.extend(number.to_le_bytes());
        out.push(text.len() as u8);
        out.extend(text.as_bytes());
    }
    out
}

/// A record's messages; anything malformed ends the list.
fn decode(bytes: &[u8]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while out.len() < CARRIED && rest.len() >= 5 {
        let number = u32::from_le_bytes(rest[..4].try_into().unwrap());
        let length = rest[4] as usize;
        let Some(text) = rest.get(5..5 + length) else { break };
        let Ok(text) = std::str::from_utf8(text) else { break };
        out.push((number, text.to_owned()));
        rest = &rest[5 + length..];
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Player,
    Own,
    Notice,
}
struct Line {
    name: String,
    text: String,
    at: f64,
    kind: Kind,
}

#[derive(Default)]
struct Peer {
    last: u32,
    limiter: Limiter,
}

#[derive(Resource, Default)]
pub(crate) struct Chat {
    /// The box is open and has the keyboard.
    pub(crate) typing: bool,
    /// The box closed this frame: the key that closed it is not for the menus.
    pub(crate) closed: bool,
    draft: String,
    lines: VecDeque<Line>,
    sent: VecDeque<(u32, String)>,
    next: u32,
    limiter: Limiter,
    recent: VecDeque<(f64, String)>,
    strikes: VecDeque<f64>,
    cooldown: f64,
    peers: BTreeMap<u64, Peer>,
    /// The page's keyboard controller is off while typing (web).
    page_typing: bool,
}
impl Chat {
    /// Keys belong to the chat box this frame.
    pub(crate) fn has_keys(&self) -> bool {
        self.typing || self.closed
    }
    fn notice(&mut self, text: String, now: f64) {
        self.push(Line { name: String::new(), text, at: now, kind: Kind::Notice });
    }
    /// A line from the game (kicks, votes), shown like the chat's own notices.
    pub(crate) fn notify(&mut self, text: String, now: f64) {
        self.notice(text, now);
    }
    fn push(&mut self, line: Line) {
        self.lines.push_back(line);
        while self.lines.len() > KEPT {
            self.lines.pop_front();
        }
    }
    /// Try to send what was typed. Err is the reason it did not go.
    fn submit(&mut self, text: &str, now: f64) -> Result<String, String> {
        let text = clean(text);
        if text.is_empty() {
            return Err(String::new());
        }
        if now < self.cooldown {
            return Err(format!("Chat paused for spamming. Wait {:.0}s.", (self.cooldown - now).ceil()));
        }
        let key = text.to_ascii_lowercase();
        self.recent.retain(|(at, _)| now - at < REPEAT_WINDOW);
        let refused = if self.recent.iter().any(|(_, said)| *said == key) {
            Some("You just said that.".to_owned())
        } else {
            self.limiter.allow(now).err().map(|wait| format!("Slow down. Wait {:.0}s.", wait.ceil().max(1.)))
        };
        if let Some(reason) = refused {
            self.strikes.retain(|at| now - at < STRIKE_WINDOW);
            self.strikes.push_back(now);
            if self.strikes.len() >= STRIKES {
                self.strikes.clear();
                self.cooldown = now + COOLDOWN;
                return Err(format!("Too many messages. Chat paused for {COOLDOWN:.0}s."));
            }
            return Err(reason);
        }
        self.recent.push_back((now, key));
        self.next = self.next.wrapping_add(1).max(1);
        self.sent.push_back((self.next, text.clone()));
        while self.sent.len() > CARRIED {
            self.sent.pop_front();
        }
        Ok(text)
    }
    /// New messages in a player's record, within that player's limit.
    fn receive(&mut self, peer: u64, record: &[u8], now: f64) -> Vec<String> {
        let state = self.peers.entry(peer).or_default();
        let mut shown = Vec::new();
        let mut messages = decode(record);
        messages.sort_by_key(|(number, _)| *number);
        for (number, text) in messages {
            if number <= state.last {
                continue;
            }
            state.last = number;
            let text = clean(&text);
            // Over the limit: dropped here, whatever the sender's game did.
            if !text.is_empty() && state.limiter.allow(now).is_ok() {
                shown.push(text);
            }
        }
        shown
    }
}

/// Send what was typed, or say why not.
fn send(chat: &mut Chat, net: &mut Multiplayer, draft: &str, now: f64) {
    // Commands (/votekick name, /kick name, /yes, /no) are not chat.
    if let Some(reply) = super::kick::chat_command(net, draft) {
        if !reply.is_empty() {
            chat.notice(reply, now);
        }
        return;
    }
    match chat.submit(draft, now) {
        Ok(text) => {
            let record = encode(&chat.sent);
            net.publish_application(CHAT_KEY, record);
            let name = net.published_name();
            chat.push(Line { name, text, at: now, kind: Kind::Own });
        }
        Err(reason) if !reason.is_empty() => chat.notice(reason, now),
        Err(_) => {}
    }
}

/// Test hook (SKATE_VERIFY_CHAT=<n>): once another player is in the session,
/// try to send n messages within one second, as a key-masher would, and log
/// every line the box shows. Off in an ordinary session.
pub(super) fn verify(
    mut chat: ResMut<Chat>,
    mut net: ResMut<Multiplayer>,
    time: Res<Time<Real>>,
    mut state: Local<(Option<f64>, u32, usize)>,
) {
    let Some(count) = std::env::var("SKATE_VERIFY_CHAT").ok().and_then(|n| n.parse::<u32>().ok()) else { return };
    let now = time.elapsed_secs_f64();
    let others = net.player_ids().len() > 1;
    let start = *state.0.get_or_insert(f64::MAX);
    if others && start == f64::MAX {
        state.0 = Some(now + 5.);
    }
    let start = state.0.unwrap_or(f64::MAX);
    while state.1 < count && now >= start + f64::from(state.1) / f64::from(count) {
        state.1 += 1;
        let text = format!("message {} <b>bold</b>", state.1);
        send(&mut chat, &mut net, &text, now);
    }
    for line in chat.lines.iter().skip(state.2) {
        info!("chat verify: {:?} [{}] {}", line.kind, line.name, line.text);
    }
    state.2 = chat.lines.len();
}

/// Opening, typing and sending. Runs before the menus read the keyboard.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    mut chat: ResMut<Chat>,
    mut net: ResMut<Multiplayer>,
    keys: Res<ButtonInput<KeyCode>>,
    mut typing: MessageReader<KeyboardInput>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    customiser: Option<Res<crate::customiser::Customiser>>,
    travel: Option<Res<crate::teleport_menu::Travel>>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    chat.closed = false;
    let free = net.active()
        && menu.as_ref().is_none_or(|m| !m.open)
        && customiser.as_ref().is_none_or(|c| !c.open)
        && travel.as_ref().is_none_or(|t| !t.open)
        && !crate::object_dropper::is_open();
    if chat.typing && !free {
        chat.typing = false;
        chat.draft.clear();
    }
    if !chat.typing {
        typing.clear();
        if free && (keys.just_pressed(KeyCode::KeyT) || keys.just_pressed(KeyCode::Enter)) {
            chat.typing = true;
            chat.draft.clear();
        }
    } else {
        for event in typing.read() {
            if !event.state.is_pressed() {
                continue;
            }
            match event.key_code {
                KeyCode::Escape => {
                    chat.typing = false;
                    chat.closed = true;
                    chat.draft.clear();
                    break;
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let draft = std::mem::take(&mut chat.draft);
                    chat.typing = false;
                    chat.closed = true;
                    send(&mut chat, &mut net, &draft, now);
                    break;
                }
                KeyCode::Backspace => {
                    chat.draft.pop();
                }
                _ => {
                    if let Some(text) = &event.text {
                        let joined = format!("{}{}", chat.draft, text);
                        // Typing is held to what can be sent, so nothing is
                        // cut or changed after Enter.
                        let cleaned = clean(&joined);
                        let keep_space = joined.ends_with(' ') && !cleaned.is_empty() && cleaned.len() < MAX_CHARS;
                        chat.draft = if keep_space { format!("{cleaned} ") } else { cleaned };
                    }
                }
            }
        }
    }
    // The page turns keys into a controller; it must not while typing.
    if chat.page_typing != chat.typing {
        chat.page_typing = chat.typing;
        #[cfg(target_arch = "wasm32")]
        crate::web::set_typing(chat.typing);
    }
}

/// Messages from the others.
pub(super) fn receive(mut chat: ResMut<Chat>, net: Res<Multiplayer>, time: Res<Time<Real>>) {
    let now = time.elapsed_secs_f64();
    if !net.active() {
        chat.peers.clear();
        chat.lines.retain(|line| now - line.at < LINGER);
        return;
    }
    let records = net.application_values(CHAT_KEY);
    for (peer, record) in records {
        let name = net.display_name(peer);
        for text in chat.receive(peer, &record, now) {
            chat.push(Line { name: name.clone(), text, at: now, kind: Kind::Player });
        }
    }
    let live = net.player_ids();
    chat.peers.retain(|id, _| live.contains(id));
}

#[derive(Component)]
pub(super) struct ChatRoot;
#[derive(Component)]
pub(super) struct ChatLine(usize);
#[derive(Component)]
pub(super) struct ChatName(usize);
#[derive(Component)]
pub(super) struct ChatText(usize);
#[derive(Component)]
pub(super) struct ChatDraft;

pub(super) fn setup(mut commands: Commands) {
    commands
        .spawn((
            ChatRoot,
            GlobalZIndex(5),
            bevy::ui::FocusPolicy::Pass,
            Node {
                position_type: PositionType::Absolute,
                left: px(24),
                bottom: px(330),
                width: px(520),
                max_width: percent(60),
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                display: Display::None,
                ..default()
            },
        ))
        .with_children(|panel| {
            for row in 0..SHOWN {
                panel
                    .spawn((
                        ChatLine(row),
                        Node { padding: UiRect::axes(px(8), px(2)), column_gap: px(6), display: Display::None, ..default() },
                        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.6)),
                    ))
                    .with_children(|line| {
                        line.spawn((ChatName(row), Text::new(""), TextFont { font_size: 17., ..default() },
                            TextColor(Color::srgb(0.55, 0.81, 1.)), Node { flex_shrink: 0., ..default() }));
                        line.spawn((ChatText(row), Text::new(""), TextFont { font_size: 17., ..default() },
                            TextColor(Color::WHITE), Node { flex_grow: 1., min_width: px(0), ..default() }));
                    });
            }
            panel.spawn((
                ChatDraft,
                Text::new(""),
                TextFont { font_size: 18., ..default() },
                TextColor(Color::WHITE),
                Node { padding: UiRect::axes(px(8), px(4)), margin: UiRect::top(px(4)), display: Display::None, ..default() },
                BackgroundColor(Color::srgba(0.035, 0.065, 0.10, 0.9)),
            ));
        });
}

#[allow(clippy::type_complexity)]
pub(super) fn draw(
    chat: Res<Chat>,
    net: Res<Multiplayer>,
    time: Res<Time<Real>>,
    windows: Query<&Window>,
    mut nodes: Query<(&mut Node, Option<&ChatRoot>, Option<&ChatLine>, Option<&ChatDraft>), Or<(With<ChatRoot>, With<ChatLine>, With<ChatDraft>)>>,
    mut texts: Query<(&mut Text, &mut TextFont, &mut TextColor, Option<&ChatName>, Option<&ChatText>, Option<&ChatDraft>)>,
) {
    let now = time.elapsed_secs_f64();
    let scale = windows.iter().next().map_or(1., |w| (w.height() / 1080.).clamp(0.65, 2.));
    let visible: Vec<&Line> = {
        let recent = chat.lines.iter().filter(|line| chat.typing || now - line.at < LINGER);
        let mut lines: Vec<_> = recent.collect();
        let skip = lines.len().saturating_sub(SHOWN);
        lines.drain(..skip);
        lines
    };
    let shown = net.active() || !visible.is_empty();
    // With others in the session and nothing said yet: how to chat, quietly.
    let hint = net.active() && net.player_ids().len() > 1 && !chat.typing && visible.is_empty();
    for (mut node, root, line, draft) in &mut nodes {
        let mut next = node.clone();
        if root.is_some() {
            next.display = if shown && (chat.typing || hint || !visible.is_empty()) { Display::Flex } else { Display::None };
            next.left = px(24. * scale);
            // Above the trick display (Skate 3's trickdisplay, bottom left).
            next.bottom = px(330. * scale);
            next.width = px(520. * scale);
        } else if let Some(line) = line {
            next.display = if line.0 < visible.len() { Display::Flex } else { Display::None };
        } else if draft.is_some() {
            next.display = if chat.typing || hint { Display::Flex } else { Display::None };
        }
        node.set_if_neq(next);
    }
    for (mut text, mut font, mut colour, name, body, draft) in &mut texts {
        let (wanted, size, tint) = if let Some(name) = name {
            let line = visible.get(name.0);
            let label = line.filter(|l| l.kind != Kind::Notice).map_or(String::new(), |l| format!("{}:", l.name));
            let tint = if line.is_some_and(|l| l.kind == Kind::Own) { Color::srgb(0.98, 0.84, 0.35) } else { Color::srgb(0.55, 0.81, 1.) };
            (label, 17., tint)
        } else if let Some(body) = body {
            let line = visible.get(body.0);
            let tint = if line.is_some_and(|l| l.kind == Kind::Notice) { Color::srgb(1., 0.62, 0.35) } else { Color::WHITE };
            (line.map_or(String::new(), |l| l.text.clone()), 17., tint)
        } else if draft.is_some() && !chat.typing {
            ("Press T or Enter to chat".into(), 15., Color::srgba(0.75, 0.85, 0.95, 0.75))
        } else if draft.is_some() {
            let left = MAX_CHARS - chat.draft.len().min(MAX_CHARS);
            (format!("Say: {}_   ({left})", chat.draft), 18., Color::WHITE)
        } else {
            continue;
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if font.font_size != size * scale {
            font.font_size = size * scale;
        }
        if colour.0 != tint {
            colour.0 = tint;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_messages_in_a_second_send_one() {
        let mut chat = Chat::default();
        let sent = (0..10).filter(|i| chat.submit(&format!("hi {i}"), 1. + f64::from(*i) * 0.1).is_ok()).count();
        assert_eq!(sent, 1);
    }

    #[test]
    fn mashing_send_earns_a_cooldown() {
        let mut chat = Chat::default();
        for i in 0..10 {
            let _ = chat.submit(&format!("spam {i}"), 1. + f64::from(i) * 0.1);
        }
        // Well past the normal limit, but still paused.
        let reason = chat.submit("hello", 6.).unwrap_err();
        assert!(reason.contains("paused"), "{reason}");
        assert!(chat.submit("hello", 1. + COOLDOWN + 1.).is_ok());
    }

    #[test]
    fn steady_chat_goes_through() {
        let mut chat = Chat::default();
        for i in 0..20 {
            assert!(chat.submit(&format!("line {i}"), f64::from(i) * 2.1).is_ok(), "message {i}");
        }
    }

    #[test]
    fn repeats_are_refused() {
        let mut chat = Chat::default();
        assert!(chat.submit("gg", 1.).is_ok());
        assert!(chat.submit("GG", 4.).is_err());
        assert!(chat.submit("gg", 1. + REPEAT_WINDOW + 1.).is_ok());
    }

    #[test]
    fn receivers_hold_a_flooding_player_to_the_limit() {
        // A modified game ignoring its own limit: 200 messages at once.
        let mut sender: VecDeque<(u32, String)> = VecDeque::new();
        let mut chat = Chat::default();
        let mut shown = 0;
        for n in 1..=200u32 {
            sender.push_back((n, format!("flood {n}")));
            if sender.len() > CARRIED {
                sender.pop_front();
            }
            shown += chat.receive(7, &encode(&sender), 1. + f64::from(n) * 0.002).len();
        }
        assert_eq!(shown, 1);
        // Already-seen numbers are never shown again.
        assert!(chat.receive(7, &encode(&sender), 100.).is_empty());
    }

    #[test]
    fn markup_and_control_characters_are_plain_text() {
        let text = clean("<script>alert(1)</script>\u{202e}\u{0}\n\u{200b}héllo");
        // Angle brackets stay visible characters: the game draws text, never HTML.
        assert_eq!(text, "<script>alert(1)</script> hllo");
        assert_eq!(clean("aaaaaaaaaaaa!!!!!!!!"), "aaaa!!!!");
        assert_eq!(clean("   lots     of   space   "), "lots of space");
        assert!(clean(&"x y ".repeat(200)).len() <= MAX_CHARS);
    }

    #[test]
    fn malformed_records_are_cut_short() {
        let mut good = VecDeque::new();
        good.push_back((1, "ok".to_owned()));
        let mut bytes = encode(&good);
        bytes.extend([2, 0, 0, 0, 200, b'x']);
        assert_eq!(decode(&bytes), vec![(1, "ok".to_owned())]);
        assert!(decode(&[0xff; 3]).is_empty());
    }
}
