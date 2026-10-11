//! The Object Dropper on Skate 3's own screen (screens/objectdropper, exported
//! by tools/prepare_frontend.py). The game drove it through SkatePark natives
//! and SetState with FEObjectDropperScreenMode_* constants; this is the
//! world-spawner flavour (free skate), whose catalog is the physics props:
//! one super category, the DMO sub categories, then their props.
//!
//! The flow and controls are the game's own (its tutorial tips spell them
//! out, ID_CHALLENGE_TUT_SKPK_*): LB + B enters the editor on a free camera
//! with a cursor; left stick pans, right stick rotates, D-pad up/down
//! elevates and left/right zooms the camera. Over one of your props the
//! cursor offers A select / Y delete. X opens the catalog; the chosen object
//! appears at the cursor to manipulate: left stick moves it, RT + left stick
//! rotates it, D-pad up/down elevates it, L3 resets it, A places it (props
//! fall and settle onto what is below), Y deletes it, X picks a new one, B
//! backs out. Back resets all your props. B (or LB + B, Start) leaves the
//! editor and drops the skater at the cursor.
//!
//! Keyboard (the browser's keyboard pad, which the desktop build reads the
//! same way): WASD left stick, arrows right stick, Space A, E B, Q X, R Y,
//! Z LB, X RB, Shift LT, C RT, V L3, Tab Back, Escape Start, I/K/J/L D-pad;
//! O also opens and closes the editor.
use super::{Catalog, Model, Models, Prop, Props, CLOSED, OPEN, OPEN_REQUEST};
use crate::apt_scene;
use crate::apt_screen::{array, literal, Natives, Screen};
use crate::apt_vm::{Value, Vm};
use crate::apt_stage::{read_screen, ScreenComposite, ScreenMaterial, Stage};
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::PrimaryWindow;
use skate_core::math::Vector3;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const LAYER: usize = 35;
const DECK: usize = skate_core::physics::board::BodyId::Deck.index();
const FRAME: f32 = 1.0 / 60.0;

/// FEObjectDropperScreenMode_*: the game's own values are native constants
/// the screen only compares, so any distinct numbers serve.
const MODES: [&str; 26] = [
    "FreeCam", "Catalog", "Manipulate", "GroupSelect", "Delete", "Refresh", "RefreshInfo",
    "RefreshFengShui", "HideSelection", "ShowSelection", "RefreshDuplicate", "RefreshFineTune",
    "ShowLockAxis", "ShowRotate", "GotoItems", "GotoSubCategories", "NextCategory",
    "PreviousCategory", "NextSubCategory", "PreviousSubCategory", "NextItem", "PreviousItem",
    "MoveOnLockedAxis", "SnapObject", "RefreshSize", "RefreshStyle",
];
fn mode(name: &str) -> f64 {
    MODES.iter().position(|m| *m == name).map_or(0., |i| (i + 1) as f64)
}

/// The SkatePark natives the screen reads.
#[derive(Default)]
pub(crate) struct Park {
    /// (language ID base, prop names) per sub category.
    categories: Vec<(String, Vec<String>)>,
    language: HashMap<String, String>,
    selecting_sub: bool,
    sub: usize,
    item: usize,
    placing: bool,
    warning: bool,
}
impl Park {
    fn items(&self) -> &[String] {
        self.categories.get(self.sub).map_or(&[], |c| &c.1[..])
    }
    fn localized(&self, id: &str) -> String {
        self.language.get(id).cloned().unwrap_or_else(|| id.trim_start_matches("ID_").to_owned())
    }
}

fn text(value: &str) -> Value {
    if value.starts_with("ID_") { Value::Text(value.into()) } else { literal(value) }
}

impl Natives for Park {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let index = |i: usize| args.get(i).map_or(0., Value::number).max(0.) as usize;
        Ok(Some(match (native, method) {
            ("SkatePark", "IsWorldSpawner") => Value::Bool(true),
            ("SkatePark", "IsUIEnabled") => Value::Bool(true),
            ("SkatePark", "IsCursorVisible") => Value::Bool(false),
            ("SkatePark", "IsSpawnerPresent") => Value::Bool(false),
            ("SkatePark", "IsItemSelected") => Value::Bool(self.placing),
            ("SkatePark", "IsDuplicateModeAllowed" | "IsDuplicateModeEnabled" | "IsFineTuneModeEnabled") => Value::Bool(false),
            ("SkatePark", "IsCurrentItemTerrain") => Value::Bool(false),
            ("SkatePark", "IsFengShuiShowingWarning") => Value::Bool(self.warning),
            ("SkatePark", "IsSelectingSubCategory") => Value::Bool(self.selecting_sub),
            ("SkatePark", "ItemSelectionReady") => Value::Undefined,
            ("SkatePark", "GetSuperCategories") => array(vm, [Value::Text("ID_SKATEPARK_CATEGORY_DMO".into())])?,
            ("SkatePark", "GetCurrentSuperCategoryIndex") => Value::Number(0.),
            // The screen adds these with a "#" (literal) prefix.
            ("SkatePark", "GetSubCategories") => {
                let names: Vec<Value> = self.categories.iter().map(|(id, _)| Value::Text(self.localized(id))).collect();
                array(vm, names)?
            }
            ("SkatePark", "GetCurrentSubCategoryIndex") => Value::Number(self.sub as f64),
            ("SkatePark", "GetSubCategoryName") => {
                self.categories.get(index(0)).map_or_else(|| literal(""), |(id, _)| Value::Text(id.clone()))
            }
            ("SkatePark", "GetSubCategoryDescription") => {
                self.categories.get(index(0)).map_or_else(|| literal(""), |(id, _)| Value::Text(format!("{id}_DESCRIPTION")))
            }
            ("SkatePark", "GetItems") => {
                let names: Vec<Value> = self.items().iter().map(|n| text(n)).collect();
                array(vm, names)?
            }
            ("SkatePark", "GetItemsUnlocked") => {
                let all = vec![Value::Bool(true); self.items().len()];
                array(vm, all)?
            }
            ("SkatePark", "GetItemsNewlyUnlocked") => {
                let none = vec![Value::Bool(false); self.items().len()];
                array(vm, none)?
            }
            ("SkatePark", "GetCurrentItemIndex") => Value::Number(self.item as f64),
            ("SkatePark", "GetSize") => Value::Number(0.),
            ("SkatePark", "GetStyle") => Value::Text("invalid".into()),
            ("SkatePark", "GetColour") => array(vm, [Value::Number(255.), Value::Number(255.), Value::Number(255.)])?,
            ("SkatePark", "GetUnlockString" | "GetCurrentItemSizeInfo") => literal(""),
            // The picker draws a row it is told is disabled dimmed, and then
            // has nothing to highlight: every prop is available.
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game" | "LetterBox", _) => Value::Undefined,
            _ => return Ok(None),
        }))
    }
}

const OBJECTS: [(&str, &[(&str, Value)]); 3] = [("Game", &[]), ("LetterBox", &[]), ("SkatePark", &[])];

struct Loaded {
    screen: Screen<Park>,
    shapes: apt_scene::Shapes,
    stage: Stage,
}


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The free camera and its cursor.
    Free,
    Categories,
    Items,
    /// Manipulating an object.
    Place,
}

struct Ghost {
    id: u32,
    model: Option<Arc<Model>>,
    pos: Vec3,
    yaw: f32,
    lift: f32,
    entity: Option<Entity>,
    /// A placed prop picked up to move: put back if this is backed out of.
    origin: Option<Prop>,
    /// Where it appeared, for L3 (reset object).
    start: (Vec3, f32),
}

/// The editor camera: it orbits the cursor point, which is the screen centre.
#[derive(Clone, Copy)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    height: f32,
    distance: f32,
}
impl Default for Orbit {
    fn default() -> Self {
        Self { focus: Vec3::ZERO, yaw: 0., height: 5., distance: 8. }
    }
}
impl Orbit {
    fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0., self.yaw.cos())
    }
    fn right(&self) -> Vec3 {
        let f = self.forward();
        Vec3::new(-f.z, 0., f.x)
    }
    fn transform(&self) -> Transform {
        let eye = self.focus - self.forward() * self.distance + Vec3::Y * self.height;
        Transform::from_translation(eye).looking_at(self.focus, Vec3::Y)
    }
    /// Right stick turns, D-pad up/down elevates, D-pad left/right zooms.
    fn steer(&mut self, input: &Input, dt: f32) {
        self.yaw -= input.rs.x * 1.8 * dt;
        let elevate = f32::from(u8::from(input.held & 1 != 0)) - f32::from(u8::from(input.held & 2 != 0)) + input.rs.y;
        self.height = (self.height + elevate * 6. * dt).clamp(1.5, 40.);
        let zoom = f32::from(u8::from(input.held & 8 != 0)) - f32::from(u8::from(input.held & 4 != 0));
        self.distance = (self.distance - zoom * 10. * dt).clamp(3., 45.);
    }
    /// Pan speed grows with the view, as a map camera's does.
    fn pan_speed(&self) -> f32 {
        4. + (self.distance + self.height) * 0.45
    }
}

#[derive(Resource)]
pub(crate) struct DropperUi {
    loaded: Option<Loaded>,
    /// The screen could not be loaded or stopped: the dropper still works,
    /// without its pictures.
    failed: bool,
    tried: bool,
    phase: Phase,
    ghost: Option<Ghost>,
    clock: f32,
    previous: (u16, i8),
    natives: Park,
    orbit: Orbit,
    /// Which of this player's props the cursor is over.
    hover: Option<usize>,
}
impl Default for DropperUi {
    fn default() -> Self {
        Self {
            loaded: None, failed: false, tried: false, phase: Phase::Free, ghost: None, clock: 0.,
            previous: (0, 0), natives: Park::default(), orbit: Orbit::default(), hover: None,
        }
    }
}
impl DropperUi {
    fn park(&mut self) -> &mut Park {
        match &mut self.loaded {
            Some(l) => &mut l.screen.bindings.natives,
            None => &mut self.natives,
        }
    }
    fn set_state(&mut self, name: &str) {
        let Some(loaded) = &mut self.loaded else { return };
        if let Err(error) = loaded.screen.set_state(mode(name)) {
            error!("Original Object Dropper screen stopped: {error}");
            self.loaded = None;
            self.failed = true;
        }
    }
    fn show(&mut self, visible: bool) {
        let Some(loaded) = &mut self.loaded else { return };
        let root = loaded.screen.bindings.movie.root;
        let _ = loaded.screen.vm.set(root, "_visible", Value::Bool(visible));
        if visible {
            let _ = loaded.screen.bindings.movie.method(&mut loaded.screen.vm, root, "gotoAndPlay", &[Value::Number(1.)]);
        }
    }
    /// Back to the free camera, the cursor over nothing yet.
    fn free(&mut self) {
        self.phase = Phase::Free;
        self.hover = None;
        let park = self.park();
        park.placing = false;
        park.selecting_sub = true;
        self.set_state("FreeCam");
        self.set_state("HideSelection");
    }
    /// Drop the object being manipulated; a picked-up prop goes back.
    fn discard(&mut self, commands: &mut Commands, props: &mut Props) {
        if let Some(ghost) = self.ghost.take() {
            if let Some(entity) = ghost.entity {
                commands.entity(entity).despawn();
            }
            if let Some(origin) = ghost.origin {
                props.mine.push(origin);
            }
        }
    }
}

fn categories(catalog: &Catalog) -> Vec<(String, Vec<String>)> {
    catalog
        .categories
        .iter()
        .map(|c| (format!("ID_SKATEPARK_CATEGORY_DMO_{}", c.name.to_uppercase().replace(' ', "_")), c.items.iter().map(|i| i.name.clone()).collect()))
        .collect()
}

/// The screen loads the first time the dropper opens (it is not small).
fn load_screen(
    ui: &mut DropperUi,
    config: &crate::config::Config,
    catalog: &Catalog,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    composites: &mut Assets<ScreenComposite>,
    window: &Window,
    output: Option<Entity>,
) {
    ui.natives.categories = categories(catalog);
    if ui.tried {
        return;
    }
    ui.tried = true;
    let dir = config.asset_root.join("private/frontend");
    let Some(output) = output else { return };
    if !skate_data::vfs::is_file(dir.join("objectdropper.json")) {
        warn!("Original Object Dropper screen not installed; run tools/prepare_frontend.py");
        return;
    }
    let mut stage = Stage::spawn(commands, images, composites, window, output, LAYER, 20, -1);
    let loaded = (|| -> Result<_, String> {
        let (json, shapes) = read_screen(&dir, "objectdropper.json")?;
        let mut park = Park { categories: ui.natives.categories.clone(), selecting_sub: true, ..Park::default() };
        if let Some(language) = json["language"].as_object() {
            park.language = language.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect();
        }
        let mut globals: Vec<(String, Value)> = crate::main_menu::APT.iter().map(|(n, v)| (n.to_string(), Value::Number(*v))).collect();
        globals.extend(MODES.iter().map(|m| (format!("FEObjectDropperScreenMode_{m}"), Value::Number(mode(m)))));
        let globals: Vec<(&str, Value)> = globals.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        let mut screen = Screen::load(&json, park, &OBJECTS, &globals)?;
        let root = screen.bindings.movie.root;
        screen.vm.set(root, "_visible", Value::Bool(false))?;
        stage.load_textures(images, &dir, &shapes, &screen.bindings.movie)?;
        Ok((screen, shapes))
    })();
    match loaded {
        Ok((screen, shapes)) => ui.loaded = Some(Loaded { screen, shapes, stage }),
        Err(error) => {
            warn!("Original Object Dropper screen unavailable ({error})");
            ui.failed = true;
        }
    }
}


/// One frame of pad and keyboard, as presses and held axes.
#[derive(Default)]
struct Input {
    /// List steps: D-pad or either stick.
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    accept: bool,
    back: bool,
    catalog: bool,
    delete: bool,
    /// Back: reset all props.
    clear: bool,
    /// Start or O: leave the editor.
    start: bool,
    /// LB held, B pressed: Skate 3's shortcut into the editor (and out).
    combo: bool,
    /// L3: reset object.
    reset: bool,
    previous_category: bool,
    next_category: bool,
    held: u16,
    ls: Vec2,
    rs: Vec2,
    rt: f32,
}

fn read_input(keys: &ButtonInput<KeyCode>, previous: &mut (u16, i8)) -> Input {
    let pad = (0..4).find_map(|i| crate::input::platform::poll(i).ok());
    let stick = |v: i16| {
        let v = f32::from(v) / 32767.;
        v.signum() * ((v.abs() - 0.24) / 0.76).max(0.)
    };
    let (mut buttons, mut left, mut right, mut triggers) = pad.as_ref().map_or((0, [0.; 2], [0.; 2], [0.; 2]), |p| {
        (
            p.state.buttons,
            [stick(p.state.left[0]), stick(p.state.left[1])],
            [stick(p.state.right[0]), stick(p.state.right[1])],
            [f32::from(p.state.triggers[0]) / 255., f32::from(p.state.triggers[1]) / 255.],
        )
    });
    // The browser's keyboard already is a pad (web/main.js KEY_BUTTONS); the
    // desktop build reads the same layout from the keys themselves.
    if !cfg!(target_arch = "wasm32") {
        for (key, bit) in [
            (KeyCode::Space, 0x1000), (KeyCode::KeyE, 0x2000), (KeyCode::KeyQ, 0x4000), (KeyCode::KeyR, 0x8000),
            (KeyCode::KeyZ, 0x0100), (KeyCode::KeyX, 0x0200), (KeyCode::Escape, 0x0010), (KeyCode::Tab, 0x0020),
            (KeyCode::KeyV, 0x0040), (KeyCode::KeyB, 0x0080),
            (KeyCode::KeyI, 0x0001), (KeyCode::KeyK, 0x0002), (KeyCode::KeyJ, 0x0004), (KeyCode::KeyL, 0x0008),
        ] {
            if keys.pressed(key) {
                buttons |= bit;
            }
        }
        let axis = |neg, pos| f32::from(u8::from(keys.pressed(pos))) - f32::from(u8::from(keys.pressed(neg)));
        left = [left[0] + axis(KeyCode::KeyA, KeyCode::KeyD), left[1] + axis(KeyCode::KeyS, KeyCode::KeyW)];
        right = [right[0] + axis(KeyCode::ArrowLeft, KeyCode::ArrowRight), right[1] + axis(KeyCode::ArrowDown, KeyCode::ArrowUp)];
        triggers = [
            triggers[0].max(f32::from(u8::from(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)))),
            triggers[1].max(f32::from(u8::from(keys.pressed(KeyCode::KeyC)))),
        ];
    }
    let pressed = buttons & !previous.0;
    previous.0 = buttons;
    // Either stick steps through a list as the D-pad does.
    let vertical = left[1] + right[1];
    let step = if vertical > 0.5 { 1 } else if vertical < -0.5 { -1 } else { 0 };
    let (stick_up, stick_down) = (step == 1 && previous.1 != 1, step == -1 && previous.1 != -1);
    previous.1 = step;
    let key = |k| keys.just_pressed(k);
    let combo = buttons & 0x100 != 0 && pressed & 0x2000 != 0;
    Input {
        up: pressed & 1 != 0 || stick_up,
        down: pressed & 2 != 0 || stick_down,
        left: pressed & 4 != 0,
        right: pressed & 8 != 0,
        accept: pressed & 0x1000 != 0 || key(KeyCode::Enter) || key(KeyCode::NumpadEnter),
        back: (pressed & 0x2000 != 0 && !combo) || key(KeyCode::Backspace),
        catalog: pressed & 0x4000 != 0,
        delete: pressed & 0x8000 != 0 || key(KeyCode::Delete),
        clear: pressed & 0x20 != 0,
        start: pressed & 0x10 != 0 || key(KeyCode::KeyO),
        combo,
        reset: pressed & 0x40 != 0,
        previous_category: pressed & 0x100 != 0,
        next_category: pressed & 0x200 != 0,
        held: buttons,
        ls: Vec2::new(left[0], left[1]).clamp_length_max(1.),
        rs: Vec2::new(right[0], right[1]).clamp_length_max(1.),
        rt: triggers[1],
    }
}

/// The world height under `at`: the map or a prop already placed.
fn ground(physics: &crate::physics::GamePhysics, at: Vec3) -> Option<f32> {
    let start = Vector3::new(at.x, at.y + 6., at.z);
    let end = Vector3::new(at.x, at.y - 60., at.z);
    physics.world().query_thin_line(start, end).ok().flatten().map(|hit| hit.geometry.position.y)
}

/// Where the skater's board is, and the way the camera looks along the ground.
fn view(physics: &crate::physics::GamePhysics, camera: &crate::camera::CameraRuntime) -> (Vec3, Vec3) {
    let deck = physics.board.part_transforms()[DECK].translation;
    let board = Vec3::new(deck.x, deck.y, deck.z);
    let forward = camera
        .frame
        .as_ref()
        .map(|f| board - Vec3::new(f.position[0], f.position[1], f.position[2]))
        .map(|d| Vec3::new(d.x, 0., d.z))
        .filter(|d| d.length_squared() > 1e-4)
        .map_or(Vec3::Z, Vec3::normalize);
    (board, forward)
}


#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut ui: ResMut<DropperUi>,
    catalog: Res<Catalog>,
    mut models: ResMut<Models>,
    mut props: ResMut<Props>,
    config: Res<crate::config::Config>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    (physics, camera, menu, chat, mut skater): (
        Option<Res<crate::physics::GamePhysics>>,
        Option<Res<crate::camera::CameraRuntime>>,
        Option<Res<crate::graphics_menu::Menu>>,
        Option<Res<crate::multiplayer::chat::Chat>>,
        Option<ResMut<crate::physics::SkaterRuntime>>,
    ),
    (cameras, window, mut images, mut composites, mut sounds): (
        Query<Entity, With<IsDefaultUiCamera>>,
        Single<&Window, With<PrimaryWindow>>,
        ResMut<Assets<Image>>,
        ResMut<Assets<ScreenComposite>>,
        ResMut<crate::game_audio::SoundQueue>,
    ),
) {
    CLOSED.store(false, Ordering::Relaxed);
    let ui = &mut *ui;
    // The screenshot hook drives the dropper itself; a pad in use elsewhere
    // on the machine (XInput needs no focus) must not.
    let input = if std::env::var_os("SKATE_VERIFY_DROPPER").is_some() {
        Input::default()
    } else {
        read_input(&keys, &mut ui.previous)
    };
    let (Some(physics), Some(camera)) = (physics, camera) else { return };
    let open = OPEN.load(Ordering::Relaxed);
    let skating = menu.as_ref().is_none_or(|m| !m.open && !m.front_end) && chat.as_ref().is_none_or(|c| !c.has_keys());
    let requested = OPEN_REQUEST.swap(false, Ordering::Relaxed);
    if !open {
        set_camera(None);
        if catalog.available() && (requested || (skating && (keys.just_pressed(KeyCode::KeyO) || input.combo))) {
            // The camera rises from behind the skater, the cursor at the board.
            let (board, forward) = view(&physics, &camera);
            ui.orbit = Orbit {
                focus: Vec3::new(board.x, ground(&physics, board).unwrap_or(board.y), board.z),
                yaw: forward.x.atan2(forward.z),
                ..Orbit::default()
            };
            load_screen(ui, &config, &catalog, &mut commands, &mut images, &mut composites, &window, cameras.iter().next());
            OPEN.store(true, Ordering::Relaxed);
            let park = ui.park();
            park.sub = park.sub.min(park.categories.len().saturating_sub(1));
            ui.show(true);
            ui.free();
            sounds.push("FE_SOUND_CORE_A_BUTTON");
            info!("OBJECT_DROPPER_OPEN");
        }
        return;
    }
    if menu.as_ref().is_some_and(|m| m.open) {
        close(ui, &mut commands, &mut props);
        return;
    }
    let warning = props.warning > 0.;
    if ui.park().warning != warning {
        ui.park().warning = warning;
        ui.set_state("RefreshFengShui");
    }
    let dt = time.delta_secs().min(0.1);
    let (board, _) = view(&physics, &camera);
    if input.start || input.combo {
        leave(ui, &mut commands, &mut props, skater.as_deref_mut());
        sounds.push("FE_SOUND_CORE_B_BUTTON");
        return;
    }
    let count = |park: &Park| park.items().len();
    match ui.phase {
        Phase::Free => {
            let orbit = &mut ui.orbit;
            orbit.steer(&input, dt);
            orbit.focus += (orbit.forward() * input.ls.y - orbit.right() * input.ls.x) * orbit.pan_speed() * dt;
            keep_near(&mut orbit.focus, board);
            if let Some(y) = ground(&physics, orbit.focus) {
                orbit.focus.y = y;
            }
            // The cursor over one of this player's props selects it.
            let focus = orbit.focus;
            let hover = props
                .mine
                .iter()
                .enumerate()
                .map(|(i, p)| (i, Vec2::new(p.pos.x - focus.x, p.pos.z - focus.z).length()))
                .filter(|(_, d)| *d < 2.5)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            if hover != ui.hover {
                ui.hover = hover;
                ui.park().placing = hover.is_some();
                ui.set_state(if hover.is_some() { "ShowSelection" } else { "HideSelection" });
            }
            if input.back {
                leave(ui, &mut commands, &mut props, skater.as_deref_mut());
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            } else if input.catalog {
                let park = ui.park();
                (park.selecting_sub, park.placing) = (true, false);
                ui.phase = Phase::Categories;
                ui.set_state("Catalog");
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if input.clear {
                props.mine.clear();
                ui.hover = None;
                ui.set_state("HideSelection");
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            } else if let (Some(i), true) = (hover, input.accept) {
                // Pick it up to move it.
                let prop = props.mine.remove(i);
                let model = catalog.key(prop.id).map(str::to_owned).and_then(|k| models.get(&catalog, &k));
                ui.ghost = Some(Ghost { id: prop.id, model, pos: prop.pos, yaw: prop.yaw, lift: 0., entity: None, origin: Some(prop), start: (prop.pos, prop.yaw) });
                ui.hover = None;
                ui.park().placing = true;
                ui.phase = Phase::Place;
                ui.set_state("Manipulate");
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if let (Some(i), true) = (hover, input.delete) {
                props.mine.remove(i);
                ui.hover = None;
                ui.set_state("Delete");
                ui.set_state("HideSelection");
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            }
        }
        Phase::Categories => {
            let park = ui.park();
            let last = park.categories.len().saturating_sub(1);
            if input.up && park.sub > 0 {
                park.sub -= 1;
                ui.set_state("PreviousSubCategory");
                sounds.push("FE_SOUND_CORE_SCROLL");
            } else if input.down && park.sub < last {
                park.sub += 1;
                ui.set_state("NextSubCategory");
                sounds.push("FE_SOUND_CORE_SCROLL");
            } else if input.accept {
                (park.selecting_sub, park.item) = (false, 0);
                ui.phase = Phase::Items;
                ui.set_state("GotoItems");
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if input.back {
                ui.discard(&mut commands, &mut props);
                ui.free();
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            }
        }
        Phase::Items => {
            // Chosen while still downloading, it appears in hand once it is in.
            let previewed = ui.ghost.is_some();
            let park = ui.park();
            let last = count(park).saturating_sub(1);
            if input.up && park.item > 0 {
                park.item -= 1;
                ui.set_state("PreviousItem");
                sounds.push("FE_SOUND_CORE_SCROLL");
            } else if input.down && park.item < last {
                park.item += 1;
                ui.set_state("NextItem");
                sounds.push("FE_SOUND_CORE_SCROLL");
            } else if (input.previous_category || input.left) && park.sub > 0 {
                (park.sub, park.item) = (park.sub - 1, 0);
                ui.set_state("GotoItems");
            } else if (input.next_category || input.right) && park.sub + 1 < park.categories.len() {
                (park.sub, park.item) = (park.sub + 1, 0);
                ui.set_state("GotoItems");
            } else if input.accept && previewed {
                // The previewed object, at the cursor, is the one to place.
                if let Some(ghost) = ui.ghost.as_mut() {
                    ghost.start = (ghost.pos, ghost.yaw);
                }
                ui.park().placing = true;
                ui.phase = Phase::Place;
                ui.set_state("Manipulate");
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if input.back {
                park.selecting_sub = true;
                ui.phase = Phase::Categories;
                ui.discard(&mut commands, &mut props);
                ui.set_state("GotoSubCategories");
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            }
        }
        Phase::Place => {
            let orbit = ui.orbit;
            let Some(ghost) = ui.ghost.as_mut() else {
                ui.free();
                return;
            };
            if ghost.model.is_none() {
                if let Some(key) = catalog.key(ghost.id).map(str::to_owned) {
                    ghost.model = models.get(&catalog, &key);
                }
            }
            // RT + left stick rotates; otherwise the left stick moves it.
            if input.rt > 0.3 {
                ghost.yaw -= input.ls.x * 2.2 * dt;
            } else {
                ghost.pos += (orbit.forward() * input.ls.y - orbit.right() * input.ls.x) * orbit.pan_speed() * 0.6 * dt;
                keep_near(&mut ghost.pos, board);
            }
            let elevate = f32::from(u8::from(input.held & 1 != 0)) - f32::from(u8::from(input.held & 2 != 0));
            ghost.lift = (ghost.lift + elevate * 1.5 * dt).clamp(0., 8.);
            if let Some(y) = ground(&physics, Vec3::new(ghost.pos.x, ghost.pos.y - ghost.lift, ghost.pos.z)) {
                ghost.pos.y = y + ghost.lift;
            }
            show(&mut commands, ghost);
            let (pos, lift, yaw, id, ready) = (ghost.pos, ghost.lift, ghost.yaw, ghost.id, ghost.model.is_some());
            // The camera turns about the object; up/down elevates the object,
            // so only the right stick moves the camera here.
            let orbit = &mut ui.orbit;
            orbit.yaw -= input.rs.x * 1.8 * dt;
            orbit.height = (orbit.height + input.rs.y * 6. * dt).clamp(1.5, 40.);
            let zoom = f32::from(u8::from(input.held & 8 != 0)) - f32::from(u8::from(input.held & 4 != 0));
            orbit.distance = (orbit.distance - zoom * 10. * dt).clamp(3., 45.);
            orbit.focus = pos - Vec3::Y * lift;
            if input.accept && ready {
                // Props fall and settle onto whatever is under them.
                let settled = ground(&physics, pos).map_or(pos, |y| Vec3::new(pos.x, y, pos.z));
                let prop = Prop { id, pos: settled, yaw };
                if let Some(entity) = ui.ghost.take().and_then(|g| g.entity) {
                    commands.entity(entity).despawn();
                }
                if props.add(prop) {
                    info!("OBJECT_DROPPER_PLACED id={:08x} pos={:?} yaw={:.2} mine={}", prop.id, prop.pos, prop.yaw, props.mine.len());
                }
                ui.free();
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if input.delete {
                if let Some(entity) = ui.ghost.take().and_then(|g| g.entity) {
                    commands.entity(entity).despawn();
                }
                ui.free();
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            } else if input.reset {
                if let Some(ghost) = ui.ghost.as_mut() {
                    (ghost.pos, ghost.yaw) = ghost.start;
                    ghost.lift = 0.;
                }
                sounds.push("FE_SOUND_CORE_SCROLL");
            } else if input.catalog {
                ui.discard(&mut commands, &mut props);
                let park = ui.park();
                (park.selecting_sub, park.placing) = (true, false);
                ui.phase = Phase::Categories;
                ui.set_state("Catalog");
                sounds.push("FE_SOUND_CORE_A_BUTTON");
            } else if input.back {
                ui.discard(&mut commands, &mut props);
                ui.free();
                sounds.push("FE_SOUND_CORE_B_BUTTON");
            }
        }
    }
    if ui.phase == Phase::Items {
        preview(&mut commands, ui, &catalog, &mut models, &physics);
    }
    aim(&ui.orbit, dt);
}

/// The editor stays within reach of the skater, as the game's did.
fn keep_near(point: &mut Vec3, board: Vec3) {
    let flat = Vec3::new(point.x - board.x, 0., point.z - board.z);
    if flat.length() > 400. {
        let kept = board + flat.normalize() * 400.;
        (point.x, point.z) = (kept.x, kept.z);
    }
}

/// The highlighted catalog object stands at the cursor while you browse.
fn preview(commands: &mut Commands, ui: &mut DropperUi, catalog: &Catalog, models: &mut Models, physics: &crate::physics::GamePhysics) {
    let park = ui.park();
    let Some(chosen) = catalog.categories.get(park.sub).and_then(|c| c.items.get(park.item)) else { return };
    let (focus, yaw) = (ui.orbit.focus, ui.orbit.yaw);
    if ui.ghost.as_ref().is_none_or(|g| g.id != chosen.id) {
        if let Some(entity) = ui.ghost.take().and_then(|g| g.entity) {
            commands.entity(entity).despawn();
        }
        ui.ghost = Some(Ghost { id: chosen.id, model: None, pos: focus, yaw, lift: 0., entity: None, origin: None, start: (focus, yaw) });
    }
    let Some(ghost) = ui.ghost.as_mut() else { return };
    if ghost.model.is_none() {
        ghost.model = models.get(catalog, &chosen.key);
    }
    if let Some(y) = ground(physics, ghost.pos) {
        ghost.pos.y = y;
    }
    show(commands, ghost);
}

/// Spawn the ghost's meshes once its model is in, then keep them on it.
fn show(commands: &mut Commands, ghost: &mut Ghost) {
    let transform = Transform::from_translation(ghost.pos).with_rotation(Quat::from_rotation_y(ghost.yaw));
    match (ghost.entity, &ghost.model) {
        (Some(entity), _) => {
            commands.entity(entity).insert(transform);
        }
        (None, Some(model)) => {
            let root = commands.spawn((transform, Visibility::default(), GhostRoot)).id();
            for (mesh, material) in &model.parts {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::default(), ChildOf(root)));
            }
            ghost.entity = Some(root);
            info!("OBJECT_DROPPER_GHOST id={:08x} pos={:?}", ghost.id, ghost.pos);
        }
        (None, None) => {}
    }
}

#[derive(Component)]
struct GhostRoot;

/// Where the editor camera is, read by camera.rs after the game camera.
static CAMERA: std::sync::Mutex<Option<Transform>> = std::sync::Mutex::new(None);
pub(crate) fn camera_override() -> Option<Transform> {
    *CAMERA.lock().unwrap_or_else(|e| e.into_inner())
}
fn set_camera(transform: Option<Transform>) {
    *CAMERA.lock().unwrap_or_else(|e| e.into_inner()) = transform;
}
/// Ease the camera to the orbit (it flies up from the skater on entry).
fn aim(orbit: &Orbit, dt: f32) {
    let wanted = orbit.transform();
    let mut camera = CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    let blend = 1. - (-8. * dt).exp();
    *camera = Some(match *camera {
        Some(from) => Transform {
            translation: from.translation.lerp(wanted.translation, blend),
            rotation: from.rotation.slerp(wanted.rotation, blend),
            ..wanted
        },
        None => wanted,
    });
}

/// Leave the editor: the skater is dropped at the cursor.
fn leave(ui: &mut DropperUi, commands: &mut Commands, props: &mut Props, skater: Option<&mut crate::physics::SkaterRuntime>) {
    let (at, yaw) = (ui.orbit.focus, ui.orbit.yaw);
    close(ui, commands, props);
    if let Some(skater) = skater {
        let mut matrix = [[0.; 4]; 4];
        matrix[0] = [yaw.cos(), 0., -yaw.sin(), 0.];
        matrix[1] = [0., 1., 0., 0.];
        matrix[2] = [yaw.sin(), 0., yaw.cos(), 0.];
        matrix[3] = [at.x, at.y + 0.05, at.z, 0.];
        if let Err(error) = skater.travel_to(matrix) {
            warn!("Object Dropper: skater not moved to the cursor ({error})");
        }
    }
}

fn close(ui: &mut DropperUi, commands: &mut Commands, props: &mut Props) {
    OPEN.store(false, Ordering::Relaxed);
    set_camera(None);
    CLOSED.store(true, Ordering::Relaxed);
    ui.discard(commands, props);
    ui.hover = None;
    ui.phase = Phase::Free;
    ui.park().placing = false;
    ui.show(false);
    info!("OBJECT_DROPPER_CLOSED");
}

fn present(
    mut commands: Commands,
    mut ui: ResMut<DropperUi>,
    time: Res<Time<Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
) {
    let ui = &mut *ui;
    let phase = ui.phase;
    let Some(loaded) = &mut ui.loaded else { return };
    if !OPEN.load(Ordering::Relaxed) {
        loaded.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
        return;
    }
    ui.clock = (ui.clock + time.delta_secs()).min(FRAME * 4.0);
    let mut frames = 0;
    while ui.clock >= FRAME {
        ui.clock -= FRAME;
        frames += 1;
    }
    let result = (|| -> Result<Vec<apt_scene::Draw>, String> {
        for _ in 0..frames {
            loaded.screen.advance()?;
        }
        // The cursor marks the screen centre (the orbit's focus) on the free
        // camera; with an object in hand, the object shows where it goes.
        let (vm, movie) = (&mut loaded.screen.vm, &loaded.screen.bindings.movie);
        if let Some(cursor) = movie.child(vm, movie.root, "mObjectDropper.mCursor") {
            vm.set(cursor, "_visible", Value::Bool(phase == Phase::Free))?;
        }
        for name in loaded.screen.take_sounds() {
            sounds.push(name);
        }
        apt_scene::draw(&loaded.screen.bindings.movie, &loaded.screen.vm, &loaded.shapes)
    })();
    match result {
        Ok(draws) => loaded.stage.render(&mut commands, &mut meshes, &mut materials, &draws),
        Err(error) => {
            error!("Original Object Dropper screen stopped: {error}");
            loaded.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
            ui.loaded = None;
            ui.failed = true;
        }
    }
}


/// Opt-in coverage (--verify with SKATE_VERIFY_DROPPER=<category>/<item>[/ride]):
/// once the skater is ready, open the dropper, pick that prop on the original
/// screen, drop it ahead of the skater and close. With "ride", the skater then
/// pushes at it, and the board's path is logged; "catalog" and "place" leave
/// the screen up on the prop list or the placing view instead.
#[allow(clippy::too_many_arguments)]
fn verify(
    mut commands: Commands,
    mut ui: ResMut<DropperUi>,
    animation: Res<crate::animation::AnimationStatus>,
    mut props: ResMut<Props>,
    catalog: Res<Catalog>,
    mut models: ResMut<Models>,
    physics: Option<Res<crate::physics::GamePhysics>>,
    skater: Option<ResMut<crate::physics::SkaterRuntime>>,
    time: Res<Time<Real>>,
    mut state: Local<(u8, f32, f32)>,
) {
    let Ok(spec) = std::env::var("SKATE_VERIFY_DROPPER") else { return };
    let (Some(physics), Some(mut skater)) = (physics, skater) else { return };
    let parts: Vec<&str> = spec.split('/').collect();
    let sub: usize = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let item: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let ride = parts.get(2) == Some(&"ride");
    let grind = parts.get(2) == Some(&"grind");
    // "catalog" or "place": stop there with the screen up, for a picture.
    let stop = match parts.get(2) {
        Some(&"free") => 1,
        Some(&"catalog") => 2,
        Some(&"place") => 4,
        _ => u8::MAX,
    };
    if state.0 == stop {
        return;
    }
    let now = time.elapsed_secs();
    match state.0 {
        0 if animation.ready && catalog.available() => {
            super::request_open();
            *state = (1, now, 0.);
        }
        // Walk the screen to the prop, as the pad would.
        1 if OPEN.load(Ordering::Relaxed) && now - state.1 > 0.3 => {
            ui.phase = Phase::Categories;
            ui.set_state("Catalog");
            let park = ui.park();
            park.sub = sub.min(park.categories.len().saturating_sub(1));
            ui.set_state("NextSubCategory");
            let park = ui.park();
            (park.selecting_sub, park.item) = (false, 0);
            ui.phase = Phase::Items;
            ui.set_state("GotoItems");
            let park = ui.park();
            park.item = item.min(park.items().len().saturating_sub(1));
            ui.set_state("GotoItems");
            *state = (2, now, 0.);
        }
        // Choose it (the drive system spawns the ghost on accept).
        2 if now - state.1 > 0.3 => {
            if let Some(chosen) = catalog.categories.get(sub).and_then(|c| c.items.get(item)) {
                let model = models.get(&catalog, &chosen.key);
                if model.is_some() {
                    let deck = physics.board.part_transforms()[DECK].translation;
                    let heading = skater_heading(&physics);
                    let forward = Vec3::new(heading.sin(), 0., heading.cos());
                    let pos = Vec3::new(deck.x, deck.y, deck.z) + forward * 6.;
                    let pos = Vec3::new(pos.x, ground(&physics, pos).unwrap_or(deck.y), pos.z);
                    if let Some(entity) = ui.ghost.take().and_then(|g| g.entity) {
                        commands.entity(entity).despawn();
                    }
                    ui.ghost = Some(Ghost { id: chosen.id, model, pos, yaw: heading, lift: 0., entity: None, origin: None, start: (pos, heading) });
                    ui.orbit.focus = pos;
                    ui.park().placing = true;
                    ui.phase = Phase::Place;
                    ui.set_state("Manipulate");
                    *state = (3, now, 0.);
                }
            }
        }
        3 if now - state.1 > 0.3 => {
            *state = (4, now, 0.);
        }
        4 => {
            // The drive system drops on accept; do it here for the hook.
            // SKATE_VERIFY_DROPPER_COUNT drops that many, side by side.
            let count: usize = std::env::var("SKATE_VERIFY_DROPPER_COUNT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
            if let Some(g) = &ui.ghost {
                let side = Vec3::new(g.yaw.cos(), 0., -g.yaw.sin());
                for i in 0..count {
                    let prop = Prop { id: g.id, pos: g.pos + side * (4. * i as f32), yaw: g.yaw };
                    info!("OBJECT_DROPPER_VERIFY dropping {:08x} at {:?}", prop.id, prop.pos);
                    props.add(prop);
                }
                info!("OBJECT_DROPPER_VERIFY mine={} limit={:?}", props.mine.len(), props.limit);
            }
            *state = (5, now, 0.);
        }
        5 if !props.mine.is_empty() && now - state.1 > 0.3 => {
            close(&mut ui, &mut commands, &mut props);
            // Push (A) as a player would, straight at the prop.
            super::VERIFY_PUSH.store(ride, Ordering::Relaxed);
            // Drop the skater onto the prop's first grind spline, rolling along it.
            if grind {
                let first = props.mine.first().copied();
                let model = first.and_then(|p| catalog.key(p.id).map(str::to_owned)).and_then(|k| models.get(&catalog, &k));
                if let (Some(prop), Some(model)) = (first, model) {
                    if let Some(spline) = model.splines.first() {
                        let s = spline.moved(Quat::from_rotation_y(prop.yaw), prop.pos).segments[0];
                        let start = Vec3::new(s[12], s[13], s[14]);
                        let end = start + Vec3::new(s[0] + s[4] + s[8], s[1] + s[5] + s[9], s[2] + s[6] + s[10]);
                        let along = (end - start).normalize_or_zero();
                        let heading = along.x.atan2(along.z);
                        let at = start + (end - start) * 0.2 + Vec3::Y * 0.45;
                        let mut matrix = [[0.; 4]; 4];
                        matrix[0] = [heading.cos(), 0., -heading.sin(), 0.];
                        matrix[1] = [0., 1., 0., 0.];
                        matrix[2] = [heading.sin(), 0., heading.cos(), 0.];
                        matrix[3] = [at.x, at.y, at.z, 0.];
                        info!("OBJECT_DROPPER_VERIFY grind rail start={start:?} end={end:?} drop={at:?}");
                        if let Err(error) = skater.travel(matrix, Some((along * 4.5).to_array())) {
                            error!("Object Dropper verify grind: {error}");
                        }
                    }
                }
            }
            *state = (6, now, now);
        }
        6 => {
            if now - state.2 >= 0.25 {
                state.2 = now;
                let deck = physics.board.part_transforms()[DECK].translation;
                let top = props.mine.first().map_or(0., |p| p.pos.y);
                let v = physics.board.bodies()[DECK].rates.linear_velocity;
                let physical = skater.player_input.physical.state.state_16;
                info!("OBJECT_DROPPER_VERIFY t={:.2} board=({:.2},{:.2},{:.2}) speed={:.2} prop_base_y={top:.2} state={physical}{}", now - state.1, deck.x, deck.y, deck.z,
                    (v.x * v.x + v.z * v.z).sqrt(), if (400..=405).contains(&physical) { " GRINDING" } else { "" });
            }
        }
        _ => {}
    }
}

fn skater_heading(physics: &crate::physics::GamePhysics) -> f32 {
    let basis = physics.board.part_transforms()[DECK].basis.columns;
    // The deck's forward axis (+Z) on the ground.
    let z = basis[2];
    z[0].atan2(z[2])
}

/// The skater leaves the world while the editor is up and is spawned at the
/// cursor on the way out (ID_CHALLENGE_TUT_SKPK_*_SPAWN_SKATER).
fn hide_skater(mut players: Query<&mut Visibility, With<crate::world::PlayerRoot>>, mut hidden: Local<bool>) {
    let open = OPEN.load(Ordering::Relaxed);
    if open == *hidden {
        return;
    }
    *hidden = open;
    for mut visibility in &mut players {
        *visibility = if open { Visibility::Hidden } else { Visibility::Inherited };
    }
}

pub(crate) struct ObjectDropperUiPlugin;
impl Plugin for ObjectDropperUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DropperUi>()
            .add_systems(PreUpdate, drive.after(bevy::input::InputSystems).before(crate::graphics_menu::MenuInput))
            .add_systems(Update, (verify, present, hide_skater).chain());
    }
}
