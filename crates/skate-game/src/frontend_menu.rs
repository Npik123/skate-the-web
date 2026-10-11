//! The pause menu presented through Skate 3's original CrossBar movie
//! (`core_menu`), when the user's installation includes it. The movie draws and
//! animates; `graphics_menu` still owns every setting and action, and its panel
//! returns for pages the CrossBar cannot show (text entry, lobby browser).
use crate::{
    apt_scene,
    frontend_runtime::{Event, Key, Pointer, Runtime},
    graphics_menu::{CrossBarAction, Menu, Page},
    player_settings::PlayerSettings,
};
use crate::apt_stage::{ScreenComposite, ScreenMaterial, Stage};
use bevy::{prelude::*, window::PrimaryWindow};

/// The menu's own render layer, kept apart from the scoring HUD's.
const LAYER: usize = 30;
/// The front end's authored frame rate (core_menu: 16 ms per frame).
const FRAME: f32 = 1.0 / 60.0;

#[derive(Resource)]
struct FrontEnd {
    runtime: Runtime,
    shapes: apt_scene::Shapes,
    stage: Stage,
    dimmer: Entity,
    actions: Vec<CrossBarAction>,
    showing: bool,
    /// The last pointer position seen, so only movement hovers.
    cursor: Option<Vec2>,
    /// When a finger last lifted: browsers follow a tap with a synthetic
    /// click, which must not act a second time.
    touched_at: f32,
    /// When Enter last went down: the page also sends it as the pad's
    /// Start, which must not act a second time.
    enter_at: f32,
    clock: f32,
    failed: bool,
}
#[derive(Component)]
struct MenuOverlay;

pub(crate) struct FrontEndPlugin;
impl Plugin for FrontEndPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup))
        .add_systems(PreUpdate, control.after(crate::graphics_menu::MenuInput).after(crate::main_menu::FrontEndFlow))
        .add_systems(Update, (verify, advance, render).chain());
    }
}

/// Opt-in coverage (--verify with SKATE_VERIFY_CROSSBAR=<keys>): once the
/// skater is ready, open the pause menu and press the comma-separated keys
/// (L R U D for the pad's directions, A accept, B back; N names the player
/// "Tester" and E presses Enter on the name panel), a quarter second apart,
/// logging the highlighted row after each.
fn verify(
    fe: Option<ResMut<FrontEnd>>,
    mut menu: ResMut<Menu>,
    mut net: ResMut<crate::multiplayer::Multiplayer>,
    animation: Res<crate::animation::AnimationStatus>,
    time: Res<Time<Real>>,
    mut state: Local<(f32, usize)>,
) {
    let Ok(keys) = std::env::var("SKATE_VERIFY_CROSSBAR") else { return };
    let Some(mut fe) = fe else { return };
    if !animation.ready {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < 3. {
        return;
    }
    if !menu.open {
        if state.1 == 0 {
            menu.open = true;
        }
        return;
    }
    let steps: Vec<&str> = keys.split(',').filter(|k| !k.is_empty()).collect();
    let due = ((state.0 - 4.) / 0.25).max(0.) as usize;
    while state.1 < steps.len() && state.1 < due && (fe.showing || matches!(steps[state.1], "N" | "E")) {
        match steps[state.1] {
            "N" => {
                state.1 += 1;
                net.set_player_name("Tester".into());
                info!("CROSSBAR_VERIFY step={} named the player", state.1);
                continue;
            }
            "E" => {
                state.1 += 1;
                menu.injected.push((crate::graphics_menu::Page::Multiplayer(2), 7, 1));
                info!("CROSSBAR_VERIFY step={} enter on the name", state.1);
                continue;
            }
            _ => {}
        }
        let key = match steps[state.1] {
            "L" => Key::Left,
            "R" => Key::Right,
            "U" => Key::Up,
            "D" => Key::Down,
            "A" => Key::Next,
            _ => Key::Previous,
        };
        state.1 += 1;
        if let Err(error) = fe.runtime.input(key) {
            error!("CROSSBAR_VERIFY {error}");
        }
        let b = &fe.runtime.bindings;
        let row = b.model.categories.get(b.menu).and_then(|c| c.entries.get(b.option)).map(|e| e.name.clone());
        info!("CROSSBAR_VERIFY step={} menu={} option={} top={} row={row:?}", state.1, b.menu, b.option, b.top);
    }
}

fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    menu: Res<Menu>,
    net: Res<crate::multiplayer::Multiplayer>,
    player: Res<PlayerSettings>,
    cameras: Query<Entity, With<IsDefaultUiCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<ScreenComposite>>,
) {
    let Ok(output) = cameras.single() else {
        return;
    };
    let root = config.asset_root.join("private/frontend");
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 11, -2);
    let loaded = (|| -> Result<_, String> {
        let (source, shapes) = crate::apt_stage::read_screen(&root, "core_menu.json")?;
        let (model, _) = menu.crossbar(&net, config.map_path.as_deref(), &player, &[], false, None);
        let runtime = Runtime::load(&source, model)?;
        stage.load_textures(&mut images, &root, &shapes, &runtime.bindings.movie)?;
        Ok((runtime, shapes))
    })();
    let (runtime, shapes) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            // Without the original front end the panel menu is the pause menu.
            info!("Original pause menu unavailable ({error}); using the built-in menu");
            return;
        }
    };
    // The shipped pause screen sits over the frozen world, darkened.
    let dimmer = commands
        .spawn((
            MenuOverlay,
            UiTargetCamera(output),
            GlobalZIndex(10),
            Pickable::IGNORE,
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Visibility::Hidden,
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        ))
        .id();
    commands.entity(stage.overlay).insert(MenuOverlay);
    commands.insert_resource(FrontEnd {
        runtime,
        shapes,
        stage,
        dimmer,
        actions: Vec::new(),
        showing: false,
        cursor: None,
        touched_at: f32::NEG_INFINITY,
        enter_at: f32::NEG_INFINITY,
        clock: 0.0,
        failed: false,
    });
    info!("Original pause menu (core_menu) loaded from {}", root.display());
}

/// One navigation key per frame. Keyboard and pad both drive the CrossBar;
/// the web build maps Enter and Esc onto the pad's Start, so the keyboard's
/// own meaning wins when both arrive together.
fn key(keys: &ButtonInput<KeyCode>, pressed: u16) -> Option<Key> {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || pressed & 0x1000 != 0 {
        return Some(Key::Next);
    }
    if keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Backspace)
        || pressed & 0x2000 != 0
        || (pressed & 0x10 != 0 && !keys.pressed(KeyCode::Enter))
    {
        return Some(Key::Previous);
    }
    [(1, Key::Up), (2, Key::Down), (4, Key::Left), (8, Key::Right)]
        .into_iter()
        .find(|(bit, _)| pressed & bit != 0)
        .map(|(_, key)| key)
}

fn control(
    fe: Option<ResMut<FrontEnd>>,
    mut menu: ResMut<Menu>,
    net: Res<crate::multiplayer::Multiplayer>,
    config: Res<crate::config::Config>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Res<crate::customiser::Navigation>,
    mut player: ResMut<PlayerSettings>,
    (mut debug, camera, mut skater): (
        ResMut<crate::debug_cam::DebugCam>,
        Res<crate::camera::CameraRuntime>,
        ResMut<crate::physics::SkaterRuntime>,
    ),
    overlays: (
        Res<crate::customiser::Customiser>,
        Res<crate::custom_models::CustomModels>,
        Res<crate::modding::ModMenu>,
        Res<crate::teleport_menu::Travel>,
        Res<crate::map_transition::MapTransition>,
    ),
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    mut modes: ResMut<crate::game_modes::GameModes>,
    (mouse, touches, mut wheel, window, time): (
        Res<ButtonInput<MouseButton>>,
        Res<Touches>,
        MessageReader<bevy::input::mouse::MouseWheel>,
        Single<&Window, With<PrimaryWindow>>,
        Res<Time<Real>>,
    ),
) {
    let Some(mut fe) = fe else {
        return;
    };
    let (customiser, custom_models, mods, travel, transition) = overlays;
    let other_screen = customiser.open || custom_models.open || mods.open || travel.open || transition.busy() || menu.controls_help || menu.trick_guide;
    // While the front end shows its own screens, the CrossBar waits for the
    // main menu step.
    let want = !fe.failed && menu.open && !other_screen && !menu.panel_page() && (!menu.hidden || menu.front_end);
    menu.crossbar = want;
    let pois = travel.names();
    let debug_active = camera.manual_cam.manual_cam_active();
    if !want {
        fe.showing = false;
        if !menu.open {
            menu.mode_pick = None;
        }
        return;
    }
    if !fe.showing {
        let (model, actions) = menu.crossbar(&net, config.map_path.as_deref(), &player, &pois, debug_active, Some(&modes));
        fe.actions = actions;
        if let Err(error) = fe.runtime.open(model) {
            error!("Original pause menu stopped: {error}");
            fe.failed = true;
            menu.crossbar = false;
            return;
        }
        fe.showing = true;
        fe.clock = 0.0;
        // A tap that opened the menu is followed by the browser's own click.
        fe.touched_at = time.elapsed_secs();
        // The key that opened the menu must not also act on it.
        return;
    }
    // Mouse and touch on the original movie: the stage is 1280x720 stretched
    // over the window.
    let stage = |p: Vec2| [p.x / window.width().max(1.) * 1280., p.y / window.height().max(1.) * 720.];
    let now = time.elapsed_secs();
    let mut pointer = None;
    if let Some(touch) = touches.iter_just_released().last() {
        fe.touched_at = now;
        pointer = Some(Pointer::Press(stage(touch.position())));
    } else if let Some(cursor) = window.cursor_position() {
        if mouse.just_pressed(MouseButton::Left) && now - fe.touched_at > 0.6 {
            pointer = Some(Pointer::Press(stage(cursor)));
        } else if fe.cursor.is_some_and(|last| last.distance(cursor) > 0.5) {
            pointer = Some(Pointer::Hover(stage(cursor)));
        }
        fe.cursor = Some(cursor);
    }
    let scroll: f32 = wheel.read().map(|w| w.y).sum();
    if keys.just_pressed(KeyCode::Enter) {
        fe.enter_at = now;
    }
    let start = nav.pressed & 0x10 != 0 && !keys.pressed(KeyCode::Enter) && now - fe.enter_at > 0.5;
    let mut pressed_key = if nav.pressed & 0x10 != 0 && !start {
        // The echo of Enter: drop it, keeping any other button this frame.
        key(&keys, nav.pressed & !0x10)
    } else if start && menu.front_end {
        // On Skate 3's main menu Start selects, as A does.
        Some(Key::Next)
    } else {
        key(&keys, nav.pressed)
    };
    if pressed_key.is_none() && mouse.just_pressed(MouseButton::Right) {
        pressed_key = Some(Key::Previous);
    }
    if pressed_key.is_none() && scroll != 0. {
        pressed_key = Some(if scroll > 0. { Key::Up } else { Key::Down });
    }
    let mut fallback = None;
    if let Some(pointer) = pointer {
        let before = (fe.runtime.bindings.menu, fe.runtime.bindings.option, fe.runtime.bindings.sub, fe.runtime.bindings.level);
        if let Err(error) = fe.runtime.pointer(pointer) {
            error!("Original pause menu stopped: {error}");
            fe.failed = true;
            menu.crossbar = false;
            return;
        }
        let after = (fe.runtime.bindings.menu, fe.runtime.bindings.option, fe.runtime.bindings.sub, fe.runtime.bindings.level);
        if matches!(pointer, Pointer::Press(_)) {
            fallback = Some("menu_select");
        } else if before != after {
            fallback = Some("menu_move");
        }
    }
    if let Some(key) = pressed_key {
        fallback = Some(match key {
            Key::Next => "menu_select",
            Key::Previous => "menu_back",
            _ => "menu_move",
        });
        if let Err(error) = fe.runtime.input(key) {
            error!("Original pause menu stopped: {error}");
            fe.failed = true;
            menu.crossbar = false;
            return;
        }
    }
    // The menu's own requests (FE_SOUND_CORE_NAV_UP...) when it made any;
    // moves it handles in native code (the crossbar) get the engine's event.
    let requested = std::mem::take(&mut fe.runtime.bindings.shared.sounds);
    if requested.is_empty() {
        if let Some(event) = fallback {
            sounds.push(event);
        }
    }
    for name in requested {
        sounds.push(name);
    }
    let mut changed = false;
    // The column changes its rows (a mode's spots): shown afresh from the top.
    let mut restart = false;
    for event in fe.runtime.take_events() {
        let action = |id: u32| fe.actions.get(id as usize).cloned();
        match event {
            Event::Close => menu.open = false,
            Event::Activate { id } => match action(id) {
                Some(CrossBarAction::Resume) => menu.open = false,
                Some(CrossBarAction::Skate) => {
                    menu.launch = true;
                    menu.open = false;
                }
                Some(CrossBarAction::Row(page, row)) => menu.injected.push((page, row, 1)),
                Some(CrossBarAction::Panel(page, selected)) => menu.open_panel(page, selected),
                Some(CrossBarAction::EndMode) => {
                    modes.request(None);
                    menu.open = false;
                }
                Some(CrossBarAction::DebugCam) => {
                    debug.request_toggle();
                    menu.open = false;
                }
                Some(CrossBarAction::Controls) => menu.controls_help = true,
                Some(CrossBarAction::TrickGuide) => menu.trick_guide = true,
                Some(CrossBarAction::ObjectDropper) => {
                    crate::object_dropper::request_open();
                    menu.open = false;
                }
                Some(CrossBarAction::Discord) => crate::graphics_menu::open_link(crate::graphics_menu::DISCORD_INVITE),
                Some(CrossBarAction::Donate) => crate::graphics_menu::open_link(crate::graphics_menu::DONATE_LINK),
                Some(CrossBarAction::Coffee) => crate::graphics_menu::open_link(crate::graphics_menu::COFFEE_LINK),
                Some(CrossBarAction::CameraTeleport) => {
                    debug.request_spawn();
                    menu.open = false;
                }
                Some(CrossBarAction::Rename) => {
                    crate::graphics_menu::ask_name(&mut menu, crate::graphics_menu::Online::Rename);
                }
                Some(CrossBarAction::ModeBack) => {
                    menu.mode_pick = None;
                    restart = true;
                }
                _ => {}
            },
            Event::Choose { id, value } => {
                match action(id) {
                    Some(CrossBarAction::Pick { page, row, current }) => {
                        menu.injected.push((page, row, value as i32 - current as i32));
                    }
                    Some(CrossBarAction::Travel) => menu.injected.push((Page::Map(value), 51, 1)),
                    Some(CrossBarAction::World) => menu.front_world = value,
                    Some(CrossBarAction::PropLimit) => crate::object_dropper::set_host_limit(value as u8),
                    Some(CrossBarAction::MaxPlayers) => crate::multiplayer::request_capacity(value + 2),
                    Some(CrossBarAction::Kick) => {
                        if value > 0 {
                            crate::multiplayer::kick::menu_pick(value, net.is_host());
                            menu.open = false;
                        }
                    }
                    Some(CrossBarAction::KickBallot) => {
                        crate::multiplayer::kick::request(crate::multiplayer::kick::Request::Ballot(value == 0));
                        menu.open = false;
                    }
                    // Host a Session: free skate, or a mode (its spot next).
                    Some(CrossBarAction::HostMode) => match value.checked_sub(1) {
                        None if net.needs_name() => crate::graphics_menu::ask_name(&mut menu, crate::graphics_menu::Online::Row(2)),
                        None => menu.injected.push((Page::Multiplayer(0), 2, 1)),
                        Some(i) => {
                            if let Some(&kind) = crate::graphics_menu::host_modes(Some(&modes)).get(i) {
                                menu.mode_pick = Some((kind, true));
                                restart = true;
                            }
                        }
                    },
                    Some(CrossBarAction::GameMode) => {
                        if let Some(kind) = value.checked_sub(1).and_then(|i| crate::graphics_menu::host_modes(Some(&modes)).get(i).copied()) {
                            menu.mode_pick = Some((kind, false));
                            restart = true;
                        }
                    }
                    Some(CrossBarAction::Mode(kind)) => {
                        if !modes.spots(kind).is_empty() {
                            let start = crate::game_modes::Start { kind, challenge: value.checked_sub(1) };
                            if menu.mode_pick.is_some_and(|(_, host)| host) {
                                // Host the session, then start the mode in it
                                // (a name first, if it is still the default).
                                modes.request_hosted(start);
                                if net.needs_name() {
                                    menu.mode_pick = None;
                                    crate::graphics_menu::ask_name(&mut menu, crate::graphics_menu::Online::Row(2));
                                    continue;
                                }
                                menu.injected.push((Page::Multiplayer(0), 2, 1));
                            } else {
                                modes.request(Some(start));
                            }
                            menu.mode_pick = None;
                            menu.open = false;
                        }
                    }
                    Some(CrossBarAction::Poi) => {
                        if let Some(matrix) = travel.matrix(value) {
                            match skater.travel_to(matrix) {
                                Ok(()) => menu.open = false,
                                Err(error) => warn!("Teleport: {error}"),
                            }
                        }
                    }
                    Some(CrossBarAction::Graphics(field)) => menu.set_graphics(field, value),
                    Some(CrossBarAction::Player(field)) => {
                        field.set(&mut player, value);
                        if let Err(error) = player.save(&config.asset_root) {
                            warn!("Player settings not saved: {error}");
                        }
                    }
                    _ => {}
                }
                changed = true;
            }
        }
    }
    if restart {
        let (model, actions) = menu.crossbar(&net, config.map_path.as_deref(), &player, &pois, debug_active, Some(&modes));
        fe.actions = actions;
        if let Err(error) = fe.runtime.replace(model) {
            error!("Original pause menu stopped: {error}");
            fe.failed = true;
            menu.crossbar = false;
        }
        return;
    }
    if changed {
        // Picker values live in the model; refresh them without replaying the
        // column, which still shows the same rows.
        let (model, actions) = menu.crossbar(&net, config.map_path.as_deref(), &player, &pois, debug_active, Some(&modes));
        fe.actions = actions;
        fe.runtime.bindings.model = model;
    }
}

fn advance(fe: Option<ResMut<FrontEnd>>, time: Res<Time<Real>>) {
    let Some(mut fe) = fe else {
        return;
    };
    if !fe.showing || fe.failed {
        return;
    }
    fe.clock = (fe.clock + time.delta_secs()).min(FRAME * 4.0);
    while fe.clock >= FRAME {
        fe.clock -= FRAME;
        if let Err(error) = fe.runtime.advance() {
            error!("Original pause menu stopped: {error}");
            fe.failed = true;
            return;
        }
    }
}

fn render(
    mut commands: Commands,
    fe: Option<ResMut<FrontEnd>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
) {
    let Some(mut fe) = fe else {
        return;
    };
    let showing = fe.showing && !fe.failed;
    commands.entity(fe.dimmer).insert(if showing { Visibility::Visible } else { Visibility::Hidden });
    let draws = if showing {
        match apt_scene::draw(&fe.runtime.bindings.movie, &fe.runtime.vm, &fe.shapes) {
            Ok(draws) => draws,
            Err(error) => {
                error!("Original pause menu geometry: {error}");
                fe.failed = true;
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let fe = &mut *fe;
    fe.stage.render(&mut commands, &mut meshes, &mut materials, &draws);
}
