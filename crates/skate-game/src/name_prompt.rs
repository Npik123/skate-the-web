//! Choosing the name other skaters see, on Skate 3's own popup (`sk8popup`).
//!
//! Skate 3 took the console's gamertag and had no name screen, so this is the
//! game's general popup: its body shows the name being typed and its picker
//! offers to go on or cancel. It opens when someone goes online still called
//! "Player" (`graphics_menu::ask_name`), and from the pause menu's Player Name.
//! The keyboard types; Enter or the pad's A picks; Escape or B cancels; the
//! mouse and touch use the picker as on the main menu. On the web, the page
//! stops turning keys into a controller while the popup has them.
use crate::{
    apt_scene,
    apt_stage::{ScreenComposite, ScreenMaterial, Stage},
    graphics_menu::{Menu, Online},
    main_menu::Shown,
};
use bevy::{input::keyboard::KeyboardInput, prelude::*, window::PrimaryWindow};

/// Above the pause menu's layers.
const LAYER: usize = 33;
const FRAME: f32 = 1.0 / 60.0;
/// The same characters and length the game keeps in a name.
const MAX_NAME: usize = 16;

/// The popup loaded: the pause menu's own name field is not needed.
#[derive(Resource)]
pub(crate) struct Available;

#[derive(Resource)]
struct NamePrompt {
    popup: Shown,
    stage: Stage,
    open: bool,
    draft: String,
    clock: f32,
    cursor: Option<Vec2>,
    /// Why the name was refused, shown under it.
    hint: Option<&'static str>,
    failed: bool,
    verify_clock: f32,
    verified: bool,
}

pub(crate) struct NamePromptPlugin;
impl Plugin for NamePromptPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup))
            .add_systems(PreUpdate, run.after(crate::graphics_menu::MenuInput))
            .add_systems(Update, present);
    }
}

fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    cameras: Query<Entity, With<IsDefaultUiCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<ScreenComposite>>,
) {
    let Ok(output) = cameras.single() else { return };
    let root = config.asset_root.join("private/frontend");
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 13, -5);
    match crate::main_menu::load(&root, "sk8popup.json", &mut stage, &mut images) {
        Ok(popup) => {
            commands.insert_resource(Available);
            commands.insert_resource(NamePrompt {
            popup,
            stage,
            open: false,
            draft: String::new(),
            clock: 0.,
            cursor: None,
            hint: None,
            failed: false,
            verify_clock: 0.,
            verified: false,
            });
        }
        Err(error) => warn!("Name popup unavailable ({error}); names are typed in the pause menu's panel"),
    }
}

/// The popup's body (one line): the name being typed, after any reason it
/// was refused.
fn body(draft: &str, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("{hint} Your name: {draft}_"),
        None => format!("Your name: {draft}_"),
    }
}

fn usable(name: &str) -> bool {
    let name = crate::multiplayer::sanitize_name(name);
    !name.trim().is_empty() && !name.eq_ignore_ascii_case("Player")
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    prompt: Option<ResMut<NamePrompt>>,
    mut menu: ResMut<Menu>,
    mut net: ResMut<crate::multiplayer::Multiplayer>,
    keys: Res<ButtonInput<KeyCode>>,
    mut typing: MessageReader<KeyboardInput>,
    nav: Res<crate::customiser::Navigation>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    (mouse, touches, window, time): (Res<ButtonInput<MouseButton>>, Res<Touches>, Single<&Window, With<PrimaryWindow>>, Res<Time<Real>>),
) {
    let Some(mut prompt) = prompt else {
        typing.clear();
        return;
    };
    let prompt = &mut *prompt;
    let wanted = menu.pending_online.is_some() && !prompt.failed;
    if !wanted {
        typing.clear();
        if prompt.open {
            close(prompt, &mut menu);
        }
        return;
    }
    let result = (|| -> Result<(), String> {
        if !prompt.open {
            prompt.open = true;
            prompt.hint = None;
            prompt.draft = if usable(&net.player_name) { net.player_name.clone() } else { String::new() };
            menu.hidden = true;
            #[cfg(target_arch = "wasm32")]
            crate::web::set_typing(true);
            let popup = &mut prompt.popup;
            let natives = &mut popup.screen.bindings.natives;
            natives.description = body(&prompt.draft, None);
            natives.options = vec![going(&menu).into(), "Cancel".into()];
            natives.hilight = 0;
            natives.chosen = None;
            natives.cancelable = true;
            popup.show()?;
            popup.state(0.)?;
            popup.screen.take_sounds();
            // The key that opened the popup is not a letter for it.
            typing.clear();
            return Ok(());
        }
        // Typing.
        let before = prompt.draft.clone();
        for event in typing.read() {
            if !event.state.is_pressed() {
                continue;
            }
            if event.key_code == KeyCode::Backspace {
                prompt.draft.pop();
                continue;
            }
            if let Some(text) = &event.text {
                for ch in text.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
                    if prompt.draft.chars().count() < MAX_NAME && !(ch == ' ' && prompt.draft.is_empty()) {
                        prompt.draft.push(ch);
                    }
                }
            }
        }
        // Test hook (SKATE_VERIFY_NAME=<name>): type it, then pick the first
        // item a few seconds later.
        if let Ok(name) = std::env::var("SKATE_VERIFY_NAME") {
            prompt.verify_clock += time.delta_secs();
            if prompt.verify_clock > 0.5 && prompt.draft != name && prompt.verify_clock < 1.0 {
                prompt.draft = name;
            }
            if prompt.verify_clock > 6.0 && !prompt.verified {
                prompt.verified = true;
                prompt.popup.screen.bindings.natives.chosen = Some(0);
            }
        }
        if prompt.draft != before {
            prompt.hint = None;
            refresh(prompt)?;
        }
        // Choosing: Enter / A pick the highlighted item, Escape / B cancel.
        let popup = &mut prompt.popup;
        let stage = |p: Vec2| [p.x / window.width().max(1.) * 1280., p.y / window.height().max(1.) * 720.];
        let mut press = touches.iter_just_released().last().map(|t| stage(t.position()));
        if let Some(cursor) = window.cursor_position() {
            if mouse.just_pressed(MouseButton::Left) {
                press = Some(stage(cursor));
            } else if prompt.cursor.is_some_and(|last| last.distance(cursor) > 0.5) {
                if let Some(item) = popup.item_at(stage(cursor)) {
                    while popup.screen.bindings.natives.hilight != item {
                        let at = popup.screen.bindings.natives.hilight;
                        popup.input(if item > at { 2. } else { 1. })?;
                    }
                }
            }
            prompt.cursor = Some(cursor);
        }
        if let Some(point) = press {
            if let Some(item) = popup.item_at(point) {
                while popup.screen.bindings.natives.hilight != item {
                    let at = popup.screen.bindings.natives.hilight;
                    popup.input(if item > at { 2. } else { 1. })?;
                }
                popup.input(5.)?;
            }
        }
        if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
            popup.input(1.)?;
        }
        if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
            popup.input(2.)?;
        }
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) || nav.pressed & 0x1000 != 0 {
            popup.input(5.)?;
        }
        let cancel = keys.just_pressed(KeyCode::Escape) || nav.pressed & 0x2000 != 0;
        for name in popup.screen.take_sounds() {
            sounds.push(name);
        }
        match popup.screen.bindings.natives.chosen.take().or(cancel.then_some(1)) {
            Some(0) if !usable(&prompt.draft) => {
                prompt.hint = Some(if prompt.draft.trim().is_empty() { "Type a name first." } else { "Pick a name other than Player." });
                refresh(prompt)?;
            }
            Some(0) => {
                net.set_player_name(crate::multiplayer::sanitize_name(&prompt.draft));
                let then = menu.pending_online.take();
                crate::graphics_menu::clear_name_prompt();
                close(prompt, &mut menu);
                match then {
                    Some(Online::Row(row)) => {
                        menu.injected.push((crate::graphics_menu::Page::Multiplayer(0), row, 1));
                        menu.open = false;
                    }
                    Some(Online::Browser(index)) => {
                        net.join_row(index);
                        menu.open = false;
                    }
                    Some(Online::Rename) | None => {}
                }
            }
            Some(_) => {
                // Cancelled: back to the menu; a mode waiting to be hosted is dropped.
                menu.pending_online = None;
                crate::graphics_menu::cancel_name_prompt();
                close(prompt, &mut menu);
            }
            None => {}
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("Name popup stopped: {error}");
        prompt.failed = true;
        close(prompt, &mut menu);
        commands.remove_resource::<Available>();
    }
}

/// The popup's first item: what choosing the name leads to.
fn going(menu: &Menu) -> &'static str {
    match menu.pending_online {
        Some(Online::Rename) | None => "Save name",
        Some(Online::Row(row)) if row == 2 || row == 0 => "Host session",
        _ => "Join session",
    }
}

/// New text in the popup's body, without rebuilding its picker.
fn refresh(prompt: &mut NamePrompt) -> Result<(), String> {
    let text = body(&prompt.draft, prompt.hint);
    prompt.popup.screen.bindings.natives.description = text.clone();
    prompt.popup.set_text("mcPanel.body", &text)
}

fn close(prompt: &mut NamePrompt, menu: &mut Menu) {
    prompt.open = false;
    prompt.popup.running = 30;
    let _ = prompt.popup.screen.call("Show", vec![crate::apt_vm::Value::Bool(false)]);
    menu.hidden = false;
    #[cfg(target_arch = "wasm32")]
    crate::web::set_typing(false);
}

fn present(
    mut commands: Commands,
    prompt: Option<ResMut<NamePrompt>>,
    time: Res<Time<Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
) {
    let Some(mut prompt) = prompt else { return };
    let prompt = &mut *prompt;
    prompt.clock = (prompt.clock + time.delta_secs()).min(FRAME * 4.);
    let mut frames = 0;
    while prompt.clock >= FRAME {
        prompt.clock -= FRAME;
        frames += 1;
    }
    let mut draws = Vec::new();
    if !prompt.failed && (prompt.open || prompt.popup.running > 0) {
        let result = (|| -> Result<(), String> {
            let popup = &mut prompt.popup;
            for _ in 0..frames {
                popup.screen.advance()?;
            }
            if !prompt.open {
                popup.running = popup.running.saturating_sub(frames);
            }
            draws.extend(apt_scene::draw(&popup.screen.bindings.movie, &popup.screen.vm, &popup.shapes)?);
            Ok(())
        })();
        if let Err(error) = result {
            error!("Name popup stopped: {error}");
            prompt.failed = true;
            draws.clear();
        }
    }
    prompt.stage.render(&mut commands, &mut meshes, &mut materials, &draws);
}
