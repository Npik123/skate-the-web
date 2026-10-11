//! Skate 3's front end, played from its own screens: the title's "Press
//! START" (`actionoverlay2`), the first-run difficulty popup (`sk8popup`), the
//! CrossBar as the main menu (`frontend_menu`, with the main menu's columns),
//! and the loading screen with Coach Frank's tips (`tiploader`) while the
//! chosen world loads in place. Controller, keyboard, mouse and touch all work.
//!
//! The engine starts here with no world (`Config::front_end`); gameplay stays
//! paused behind an opaque backdrop until the world has loaded.
use crate::{
    apt_scene,
    apt_screen::{array, literal, Natives, Screen},
    apt_stage::{read_screen, ScreenComposite, ScreenMaterial, Stage},
    apt_vm::{Value, Vm},
    graphics_menu::Menu,
};
use bevy::{prelude::*, window::PrimaryWindow};

const LAYER: usize = 32;
const FRAME: f32 = 1.0 / 60.0;
/// Written once the first-run difficulty has been chosen.
const SETUP: &str = "settings/front-end.json";
/// Apt pad codes the screens compare against.
pub(crate) const APT: [(&str, f64); 6] =
    [("AptUp", 1.0), ("AptDown", 2.0), ("AptLeft", 3.0), ("AptRight", 4.0), ("AptNext", 5.0), ("AptPrevious", 6.0)];
pub(crate) const APT_DOWN: f64 = 2.0;
pub(crate) const APT_RIGHT: f64 = 4.0;
const DIFFICULTIES: [&str; 3] = ["EASY", "NORMAL", "HARDCORE"];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Step {
    #[default]
    Off,
    Title,
    Difficulty,
    Menu,
    /// Mobile mode, a big city chosen: Skate 3's popup asks first.
    Confirm,
    Loading,
}

/// Cities of several hundred MB: more memory than most phone browsers allow.
const HEAVY_WORLDS: [&str; 3] = ["downtown", "university", "industrial"];
fn heavy(world: &crate::map_library::Entry) -> bool {
    world.path.as_ref().and_then(|p| p.file_stem()).is_some_and(|s| HEAVY_WORLDS.contains(&s.to_string_lossy().to_lowercase().as_str()))
}

/// Where the front end is; `Off` once a world is playing.
#[derive(Resource, Default)]
pub(crate) struct MainMenu {
    pub step: Step,
    entered: bool,
    first_run: bool,
    generation: u64,
    /// The world last handed to the page to download ahead of time.
    prefetched: Option<std::path::PathBuf>,
    /// The world the menu points at, and since when: it is only fetched
    /// ahead once the player stops on it, not while scrolling past.
    pointed: Option<(std::path::PathBuf, f32)>,
    /// Every other world has been queued for download in the background.
    queued_all: bool,
}
impl MainMenu {
    pub(crate) fn active(&self) -> bool {
        self.step != Step::Off
    }
}

/// The game's side of the front-end screens.
#[derive(Default)]
pub(crate) struct Front {
    pub(crate) description: String,
    pub(crate) options: Vec<String>,
    pub(crate) hilight: usize,
    pub(crate) chosen: Option<usize>,
    /// The popup shows its B Back button.
    pub(crate) cancelable: bool,
    tip: u32,
}
fn text(value: &str) -> Value {
    if value.starts_with("ID_") { Value::Text(value.into()) } else { literal(value) }
}
impl Natives for Front {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let index = |i: usize| args.get(i).map_or(0.0, Value::number);
        Ok(Some(match (native, method) {
            ("Popup", "GetDescription") => text(&self.description),
            ("Popup", "GetNumOptions") => Value::Number(self.options.len() as f64),
            ("Popup", "GetOption") => self.options.get(index(0) as usize).map_or(literal(""), |o| text(o)),
            ("Popup", "IsPopupCancelable") => Value::Bool(self.cancelable),
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game", "OnMenuHilight") => {
                if index(1) >= 0.0 {
                    self.hilight = index(1) as usize;
                }
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                if index(1) >= 0.0 {
                    self.chosen = Some(index(1) as usize);
                }
                Value::Undefined
            }
            ("HUDComponents", "SignUp_GetChallengeTitle") => literal(""),
            // One panel per load, taking turns: they share the screen's middle.
            ("TipsLoader", "IsCoachFrankEnabled") => Value::Bool(self.tip % 2 == 0),
            ("TipsLoader", "IsHintEnabled") => Value::Bool(self.tip % 2 == 1),
            ("TipsLoader", "IsStatsEnabled" | "IsLeaderboardEnabled" | "IsFeedEnabled" | "IsReelEnabled") => Value::Bool(false),
            ("TipsLoader", "GetCoachFrankText") => Value::Text(format!("ID_TIPS_COACHFRANK_{}", 1 + self.tip % 20)),
            ("TipsLoader", "GetHint") => {
                let n = 1 + (self.tip / 20 + self.tip) % 31;
                array(vm, [Value::Text(format!("ID_HINTS_TITLE_{n}")), Value::Text(format!("ID_HINTS_TEXT_{n}"))])?
            }
            ("TipsLoader", "GetStats" | "GetFeedItems" | "GetLeaderboard" | "GetReelItem") => array(vm, [])?,
            _ => return Ok(None),
        }))
    }
}

pub(crate) struct Shown {
    pub(crate) screen: Screen<Front>,
    pub(crate) shapes: apt_scene::Shapes,
    /// Frames to keep running (outros play after a screen is dismissed).
    pub(crate) running: u32,
}
impl Shown {
    pub(crate) fn state(&mut self, state: f64) -> Result<(), String> {
        self.running = u32::MAX;
        self.screen.set_state(state)
    }
    /// Show the screen as the native screen manager did: visible, intro playing.
    pub(crate) fn show(&mut self) -> Result<(), String> {
        let root = self.screen.bindings.movie.root;
        self.screen.vm.set(root, "_visible", Value::Bool(true))?;
        self.screen.call("OnLoaded", vec![]).map(|_| ())
    }
    /// Set a text field's text, as a script assignment would.
    pub(crate) fn set_text(&mut self, path: &str, value: &str) -> Result<(), String> {
        let movie = &self.screen.bindings.movie;
        let Some(field) = movie.child(&self.screen.vm, movie.root, path) else { return Ok(()) };
        self.screen.vm.set(field, "text", text(value))?;
        self.screen.bindings.movie.text_changed(&mut self.screen.vm, field)
    }
    pub(crate) fn input(&mut self, code: f64) -> Result<(), String> {
        self.screen.call("UpdateInput", vec![Value::Number(code), Value::Number(0.0)]).map(|_| ())
    }
    /// The picker item under a stage point, by its row (sk8popup's picker).
    pub(crate) fn item_at(&self, point: [f32; 2]) -> Option<usize> {
        let (vm, movie) = (&self.screen.vm, &self.screen.bindings.movie);
        (0..8)
            .filter_map(|k| movie.child(vm, movie.root, &format!("mcPanel.mMenuPicker.mMenuPicker.mItemGroup.Item{k}")).map(|c| (k, c)))
            .filter(|&(_, clip)| movie.hit(vm, clip, point))
            .filter_map(|(k, clip)| {
                let label = movie.child(vm, clip, "mText.mTitle.TEXTLABEL").unwrap_or(clip);
                movie.center(vm, label).map(|c| (k, (c[1] - point[1]).abs()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }
}

#[derive(Resource)]
struct FrontScreens {
    title: Shown,
    popup: Shown,
    loader: Shown,
    stage: Stage,
    backdrop: Entity,
    clock: f32,
    cursor: Option<Vec2>,
    touched_at: f32,
    /// When the current step began: the tap (and the browser's synthetic
    /// click after it) that changed step must not also act on the next one.
    step_at: f32,
    failed: bool,
}

/// The front end's step for this frame; the CrossBar runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FrontEndFlow;

pub(crate) struct MainMenuPlugin;
impl Plugin for MainMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MainMenu>()
            .add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup))
            .add_systems(PreUpdate, flow.in_set(FrontEndFlow).after(crate::graphics_menu::MenuInput))
            .add_systems(Update, present)
            .add_systems(Update, loader_status.before(present));
    }
}

pub(crate) fn load(root: &std::path::Path, file: &str, stage: &mut Stage, images: &mut Assets<Image>) -> Result<Shown, String> {
    let (json, shapes) = read_screen(root, file)?;
    let globals: Vec<(&str, Value)> = APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
    let objects: [(&str, &[(&str, Value)]); 5] =
        [("Popup", &[]), ("Game", &[]), ("HUDComponents", &[]), ("TipsLoader", &[]), ("LetterBox", &[])];
    let screen = Screen::load(&json, Front::default(), &objects, &globals)?;
    stage.load_textures(images, root, &shapes, &screen.bindings.movie)?;
    Ok(Shown { screen, shapes, running: 0 })
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    mut front: ResMut<MainMenu>,
    mut menu: ResMut<Menu>,
    cameras: Query<Entity, With<IsDefaultUiCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<ScreenComposite>>,
) {
    if !config.front_end {
        return;
    }
    let Ok(output) = cameras.single() else {
        return;
    };
    let root = config.asset_root.join("private/frontend");
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 12, -4);
    let loaded = (|| -> Result<_, String> {
        Ok((
            load(&root, "actionoverlay2.json", &mut stage, &mut images)?,
            load(&root, "sk8popup.json", &mut stage, &mut images)?,
            load(&root, "tiploader.json", &mut stage, &mut images)?,
        ))
    })();
    let (title, popup, loader) = match loaded {
        Ok(screens) => screens,
        Err(error) => {
            // Without the original screens the main menu is still the CrossBar.
            warn!("Original title screens unavailable ({error}); opening the main menu directly");
            front.step = Step::Menu;
            menu.front_end = true;
            menu.hidden = true;
            menu.open = true;
            return;
        }
    };
    // Gameplay stays paused under the backdrop until a world is loaded.
    let backdrop = commands
        .spawn((
            UiTargetCamera(output),
            GlobalZIndex(9),
            Pickable::IGNORE,
            BackgroundColor(Color::srgb(0.02, 0.03, 0.04)),
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        ))
        .id();
    front.first_run = !skate_data::vfs::exists(config.asset_root.parent().unwrap_or(&config.asset_root).join(SETUP));
    front.step = Step::Title;
    menu.hidden = true;
    menu.open = true;
    // The world the main menu starts on: the last one played.
    if let Ok(Some(path)) = crate::map_library::default_map(&config.asset_root) {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase());
        if let Some(index) = menu.worlds().iter().position(|w| {
            w.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_lowercase()) == stem
        }) {
            menu.front_world = index;
        }
    }
    commands.insert_resource(FrontScreens {
        title,
        popup,
        loader,
        stage,
        backdrop,
        clock: 0.0,
        cursor: None,
        touched_at: f32::NEG_INFINITY,
        step_at: 0.0,
        failed: false,
    });
    info!("Original front end (title, popup, loading screen) loaded from {}", root.display());
}

/// What the loading screen is waiting for, in its own LOADING line, so a long
/// download never looks frozen.
fn loader_status(
    screens: Option<ResMut<FrontScreens>>,
    front: Res<MainMenu>,
    transition: Res<crate::map_transition::MapTransition>,
    mut shown: Local<(String, u32)>,
) {
    let Some(mut screens) = screens else { return };
    if front.step != Step::Loading || screens.failed {
        shown.0.clear();
        return;
    }
    let status = transition.status();
    // Re-set now and then too, in case the movie wrote its own label back.
    shown.1 = shown.1.wrapping_add(1);
    if status.is_empty() || (status == shown.0 && shown.1 % 30 != 0) {
        return;
    }
    if let Err(error) = screens.loader.set_text("mLoading.labelText", &status) {
        warn!("loading screen status: {error}");
    }
    shown.0 = status;
}

/// What the player pressed this frame, from any device.
#[derive(Default)]
struct Pressed {
    accept: bool,
    back: bool,
    up: bool,
    down: bool,
    /// A click or tap on the stage, and pointer movement.
    press: Option<[f32; 2]>,
    hover: Option<[f32; 2]>,
}
fn pressed(
    keys: &ButtonInput<KeyCode>,
    nav: &crate::customiser::Navigation,
    mouse: &ButtonInput<MouseButton>,
    touches: &Touches,
    window: &Window,
    screens: &mut FrontScreens,
    now: f32,
) -> Pressed {
    let stage = |p: Vec2| [p.x / window.width().max(1.) * 1280., p.y / window.height().max(1.) * 720.];
    let mut out = Pressed {
        accept: keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || nav.pressed & (0x1000 | 0x10) != 0,
        back: keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Backspace) || nav.pressed & 0x2000 != 0,
        up: nav.pressed & 1 != 0,
        down: nav.pressed & 2 != 0,
        ..default()
    };
    if let Some(touch) = touches.iter_just_released().last() {
        screens.touched_at = now;
        out.press = Some(stage(touch.position()));
    } else if let Some(cursor) = window.cursor_position() {
        if mouse.just_pressed(MouseButton::Left) && now - screens.touched_at > 0.6 {
            out.press = Some(stage(cursor));
        } else if screens.cursor.is_some_and(|last| last.distance(cursor) > 0.5) {
            out.hover = Some(stage(cursor));
        }
        screens.cursor = Some(cursor);
    }
    out.back |= mouse.just_pressed(MouseButton::Right);
    out
}

#[allow(clippy::too_many_arguments)]
fn flow(
    screens: Option<ResMut<FrontScreens>>,
    mut front: ResMut<MainMenu>,
    mut menu: ResMut<Menu>,
    config: Res<crate::config::Config>,
    mut transition: ResMut<crate::map_transition::MapTransition>,
    current: Res<crate::map_transition::CurrentMap>,
    (keys, nav, mouse, touches, window, time): (
        Res<ButtonInput<KeyCode>>,
        Res<crate::customiser::Navigation>,
        Res<ButtonInput<MouseButton>>,
        Res<Touches>,
        Single<&Window, With<PrimaryWindow>>,
        Res<Time<Real>>,
    ),
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    mut commands: Commands,
) {
    if front.step == Step::Off {
        // Skating: the other worlds download one at a time, smallest first.
        if !front.queued_all && config.front_end {
            front.queued_all = true;
            let mut worlds = menu.worlds();
            worlds.sort_by_key(|w| w.path.as_ref().map_or(usize::MAX, |p| world_order(p)));
            for path in worlds.iter().filter_map(|w| w.path.as_ref()) {
                crate::platform::prefetch(path, true);
            }
        }
        return;
    }
    // The world the menu points at starts downloading before Freeskate.
    let worlds = menu.worlds();
    let pointed = worlds.get(menu.front_world.min(worlds.len().saturating_sub(1))).and_then(|w| w.path.clone());
    let clock = time.elapsed_secs();
    if pointed != front.pointed.as_ref().map(|(p, _)| p.clone()) {
        front.pointed = pointed.clone().map(|p| (p, clock));
    }
    // Scrolling past a 400 MB city must not start downloading it.
    let rested = front.pointed.as_ref().is_some_and(|(_, since)| clock - since >= 1.5);
    if rested && pointed.is_some() && pointed != front.prefetched {
        crate::platform::prefetch(pointed.as_ref().unwrap(), false);
        front.prefetched = pointed;
    }
    // Without the original screens only the CrossBar step runs.
    let Some(mut screens) = screens else {
        if !menu.open && !menu.launch {
            menu.open = true;
        }
        launch(&mut front, &mut menu, &mut transition, &current);
        return;
    };
    if screens.failed {
        return;
    }
    let now = time.elapsed_secs();
    let mut input = pressed(&keys, &nav, &mouse, &touches, &window, &mut screens, now);
    // "Press Start" takes any key or pad button: players press whatever is
    // under their hand, and nothing else on the title would use it.
    let any_key = keys.get_just_pressed().next().is_some() || nav.pressed != 0;
    let result = (|| -> Result<(), String> {
        let entering = !front.entered;
        front.entered = true;
        if entering {
            screens.step_at = now;
        }
        if now - screens.step_at < 0.4 {
            input.press = None;
            input.accept = false;
        }
        match front.step {
            Step::Title => {
                if entering {
                    let title = &mut screens.title;
                    title.state(3.)?;
                    // The press-start symbol keeps the overlay's default label;
                    // the game set its own prompt and Start glyph.
                    title.set_text("mSignup.mOverlaySymbol.signup", "ID_ACTION_OVERLAY_PRESS_START")?;
                    title.set_text("mSignup.mOverlaySymbol.signuplabel", "ID_ACTION_OVERLAY_PRESS_START")?;
                    let movie = &title.screen.bindings.movie;
                    if let Some(button) = movie.child(&title.screen.vm, movie.root, "mSignup.mButtonItem") {
                        title.screen.vm.call_method(button, "SetButtonType", vec![Value::Text("Start_hud".into())], &mut title.screen.bindings)?;
                    }
                    title.screen.call("SetSymbol", vec![Value::Text("pressstart".into())])?;
                }
                if input.accept || input.press.is_some() || (any_key && now - screens.step_at >= 0.4) {
                    sounds.push("FE_SOUND_TITLESCREEN_START");
                    screens.title.screen.call("Show", vec![Value::Bool(false)])?;
                    screens.title.running = 60;
                    let next = if front.first_run { Step::Difficulty } else { Step::Menu };
                    go(&mut front, next);
                }
            }
            Step::Difficulty => {
                let popup = &mut screens.popup;
                if entering {
                    let natives = &mut popup.screen.bindings.natives;
                    natives.options = DIFFICULTIES.iter().map(|d| format!("ID_GAMESETTINGS_DIFFICULTY_{d}")).collect();
                    natives.hilight = 1;
                    natives.description = format!("ID_GAMESETTINGS_DIFFICULTY_DESCRIPTION_{}", DIFFICULTIES[1]);
                    popup.show()?;
                    popup.state(0.)?;
                    // Start on Normal, the game's standard (quietly: no click).
                    popup.input(2.)?;
                    popup.screen.take_sounds();
                }
                let before = popup.screen.bindings.natives.hilight;
                if let Some(point) = input.hover.or(input.press) {
                    if let Some(item) = popup.item_at(point) {
                        for _ in 0..4 {
                            let at = popup.screen.bindings.natives.hilight;
                            if at == item {
                                break;
                            }
                            popup.input(if item > at { 2. } else { 1. })?;
                        }
                        if input.press.is_some() {
                            popup.input(5.)?;
                        }
                    }
                }
                if input.up {
                    popup.input(1.)?;
                }
                if input.down {
                    popup.input(2.)?;
                }
                if input.accept {
                    popup.input(5.)?;
                }
                // The popup's own clicks (FE_SOUND_CORE_NAV_*, _A_BUTTON).
                let requested = popup.screen.take_sounds();
                let silent = requested.is_empty();
                for name in requested {
                    sounds.push(name);
                }
                let natives = &mut popup.screen.bindings.natives;
                if natives.hilight != before {
                    if silent {
                        sounds.push("menu_move");
                    }
                    let name = DIFFICULTIES.get(natives.hilight).copied().unwrap_or("NORMAL");
                    let description = format!("ID_GAMESETTINGS_DIFFICULTY_DESCRIPTION_{name}");
                    natives.description = description.clone();
                    // Straight into the body: a Refresh would rebuild the picker.
                    popup.set_text("mcPanel.body", &description)?;
                }
                if let Some(chosen) = popup.screen.bindings.natives.chosen.take() {
                    if silent {
                        sounds.push("menu_select");
                    }
                    menu.choose_difficulty(chosen.min(2));
                    let root = config.asset_root.parent().unwrap_or(&config.asset_root).to_path_buf();
                    let _ = skate_data::vfs::create_dir_all(root.join("settings"));
                    if let Err(error) = skate_data::vfs::write(root.join(SETUP), b"{\"setup\": true}\n") {
                        warn!("{SETUP}: {error}");
                    }
                    front.first_run = false;
                    popup.running = 0;
                    go(&mut front, Step::Menu);
                }
            }
            Step::Menu => {
                if entering {
                    menu.front_end = true;
                    menu.open = true;
                    menu.launch = false;
                }
                if menu.launch {
                    let worlds = menu.worlds();
                    let chosen = worlds.get(menu.front_world.min(worlds.len().saturating_sub(1)));
                    if menu.mobile() && chosen.is_some_and(heavy) {
                        menu.launch = false;
                        go(&mut front, Step::Confirm);
                    } else {
                        launch(&mut front, &mut menu, &mut transition, &current);
                    }
                } else if !menu.open {
                    // Back out of the main menu to the title.
                    menu.front_end = false;
                    menu.open = true;
                    go(&mut front, Step::Title);
                }
            }
            Step::Confirm => {
                let popup = &mut screens.popup;
                if entering {
                    let natives = &mut popup.screen.bindings.natives;
                    natives.options = vec!["Load anyway".into(), "Back".into()];
                    natives.hilight = 1;
                    // One line in this popup: short enough to fit.
                    natives.description = "Big city: a phone may run out of memory here.".into();
                    popup.show()?;
                    popup.state(0.)?;
                    popup.input(2.)?;
                    popup.screen.take_sounds();
                }
                if let Some(point) = input.hover.or(input.press) {
                    if let Some(item) = popup.item_at(point) {
                        for _ in 0..2 {
                            let at = popup.screen.bindings.natives.hilight;
                            if at == item {
                                break;
                            }
                            popup.input(if item > at { 2. } else { 1. })?;
                        }
                        if input.press.is_some() {
                            popup.input(5.)?;
                        }
                    }
                }
                if input.up {
                    popup.input(1.)?;
                }
                if input.down {
                    popup.input(2.)?;
                }
                if input.accept {
                    popup.input(5.)?;
                }
                for name in popup.screen.take_sounds() {
                    sounds.push(name);
                }
                let chosen = popup.screen.bindings.natives.chosen.take().or(input.back.then_some(1));
                if let Some(chosen) = chosen {
                    popup.running = 0;
                    if chosen == 0 {
                        menu.launch = true;
                        launch(&mut front, &mut menu, &mut transition, &current);
                    } else {
                        menu.open = true;
                        menu.launch = false;
                        go(&mut front, Step::Menu);
                    }
                }
            }
            Step::Loading => {
                if entering {
                    screens.loader.screen.bindings.natives.tip = (time.elapsed_secs() * 1000.) as u32;
                    screens.loader.state(1.)?;
                }
                if !transition.busy() {
                    if current.generation != front.generation {
                        // The world is in: the front end steps aside.
                        screens.loader.state(2.)?;
                        screens.loader.running = 30;
                        front.step = Step::Off;
                        menu.hidden = false;
                        menu.front_end = false;
                        commands.entity(screens.backdrop).insert(Visibility::Hidden);
                    } else {
                        // It failed: back to the main menu with the reason.
                        screens.loader.state(0.)?;
                        go(&mut front, Step::Menu);
                    }
                }
            }
            Step::Off => {}
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("Original front end stopped: {error}");
        screens.failed = true;
        menu.hidden = false;
        menu.front_end = true;
        menu.open = true;
        front.step = Step::Menu;
    }
}

/// Background download order: the parks before the cities.
fn world_order(path: &std::path::Path) -> usize {
    const ORDER: [&str; 9] = [
        "startpark", "blackboxpark", "industrialskatepark", "downtownskatepark", "maloofmoneycup",
        "skateschool", "industrial", "university", "downtown",
    ];
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    ORDER.iter().position(|o| *o == stem).unwrap_or(ORDER.len())
}

fn go(front: &mut MainMenu, step: Step) {
    info!("FRONT_END {:?} -> {step:?}", front.step);
    front.step = step;
    front.entered = false;
}

/// Load the world chosen on the main menu.
fn launch(
    front: &mut MainMenu,
    menu: &mut Menu,
    transition: &mut crate::map_transition::MapTransition,
    current: &crate::map_transition::CurrentMap,
) {
    if !menu.launch {
        return;
    }
    menu.launch = false;
    let worlds = menu.worlds();
    let Some(world) = worlds.get(menu.front_world.min(worlds.len().saturating_sub(1))).cloned() else {
        menu.open = true;
        return;
    };
    front.generation = current.generation;
    transition.request(world);
    menu.front_end = false;
    go(front, Step::Loading);
}

fn present(
    mut commands: Commands,
    screens: Option<ResMut<FrontScreens>>,
    time: Res<Time<Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
) {
    let Some(mut screens) = screens else {
        return;
    };
    let screens = &mut *screens;
    screens.clock = (screens.clock + time.delta_secs()).min(FRAME * 4.0);
    let mut frames = 0;
    while screens.clock >= FRAME {
        screens.clock -= FRAME;
        frames += 1;
    }
    let mut draws = Vec::new();
    if !screens.failed {
        let result = (|| -> Result<(), String> {
            for shown in [&mut screens.title, &mut screens.popup, &mut screens.loader] {
                if shown.running == 0 {
                    continue;
                }
                for _ in 0..frames {
                    shown.screen.advance()?;
                }
                shown.screen.take_sounds();
                shown.running = shown.running.saturating_sub(frames);
                draws.extend(apt_scene::draw(&shown.screen.bindings.movie, &shown.screen.vm, &shown.shapes)?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            error!("Original front end stopped: {error}");
            screens.failed = true;
            draws.clear();
        }
    }
    screens.stage.render(&mut commands, &mut meshes, &mut materials, &draws);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(root: &std::path::Path, file: &str) -> Shown {
        let (json, shapes) = read_screen(root, file).unwrap();
        let globals: Vec<(&str, Value)> = APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
        let objects: [(&str, &[(&str, Value)]); 5] =
            [("Popup", &[]), ("Game", &[]), ("HUDComponents", &[]), ("TipsLoader", &[]), ("LetterBox", &[])];
        let screen = Screen::load(&json, Front::default(), &objects, &globals).unwrap_or_else(|e| panic!("{file}: {e}"));
        Shown { screen, shapes, running: 0 }
    }
    fn settle(shown: &mut Shown) -> usize {
        for _ in 0..40 {
            shown.screen.advance().unwrap();
        }
        apt_scene::draw(&shown.screen.bindings.movie, &shown.screen.vm, &shown.shapes).unwrap().len()
    }

    /// The title prompt, the difficulty popup (by pad and by pointer) and the
    /// loading screen, when the private front-end export is installed.
    #[test]
    #[ignore = "Requires private installed front-end assets"]
    fn original_front_end_screens_run() {
        let root = std::path::PathBuf::from(std::env::var("SKATE_FRONTEND_DIR").expect("SKATE_FRONTEND_DIR"));
        let mut title = open(&root, "actionoverlay2.json");
        title.state(3.).unwrap();
        assert!(settle(&mut title) > 0, "Press START draws");

        let mut popup = open(&root, "sk8popup.json");
        let natives = &mut popup.screen.bindings.natives;
        natives.options = DIFFICULTIES.iter().map(|d| format!("ID_GAMESETTINGS_DIFFICULTY_{d}")).collect();
        natives.description = "ID_GAMESETTINGS_DIFFICULTY_DESCRIPTION_NORMAL".into();
        popup.show().unwrap();
        popup.state(0.).unwrap();
        assert!(settle(&mut popup) > 0, "the popup draws");
        popup.screen.take_sounds();
        popup.input(2.).unwrap();
        assert_eq!(popup.screen.bindings.natives.hilight, 1, "down moves the picker");
        assert_eq!(popup.screen.take_sounds(), ["FE_SOUND_CORE_NAV_DOWN"], "the picker asks for its own click");
        // Hover finds the third item and pressing chooses it.
        let mut hard = None;
        'scan: for y in (0..720).step_by(3) {
            for x in (0..1280).step_by(6) {
                if popup.item_at([x as f32, y as f32]) == Some(2) {
                    hard = Some([x as f32, y as f32]);
                    break 'scan;
                }
            }
        }
        assert!(hard.is_some(), "Hardcore is on the stage");
        popup.input(2.).unwrap();
        popup.screen.take_sounds();
        popup.input(5.).unwrap();
        assert_eq!(popup.screen.bindings.natives.chosen, Some(2), "A chooses the highlighted item");
        assert!(popup.screen.take_sounds().iter().any(|s| s == "FE_SOUND_CORE_A_BUTTON"), "A asks for the select click");

        let mut loader = open(&root, "tiploader.json");
        loader.state(1.).unwrap();
        assert!(settle(&mut loader) > 0, "the loading screen draws");
        // Its LOADING line carries the status (loader_status).
        let movie = &loader.screen.bindings.movie;
        let field = movie.child(&loader.screen.vm, movie.root, "mLoading.labelText").expect("the loader's status line");
        loader.set_text("mLoading.labelText", "DOWNLOADING 120 / 360 MB").unwrap();
        settle(&mut loader);
        assert_eq!(loader.screen.vm.get(field, "text").text().trim_start_matches('#'), "DOWNLOADING 120 / 360 MB");
    }
}
