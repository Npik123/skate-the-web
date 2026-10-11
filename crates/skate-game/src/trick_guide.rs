//! The Trick Guide, played from Skate 3's own screen (`screens/tricks/
//! trickguide`) over the game's own tables, as `FETrickTutorial` built it.
//!
//! The menu, from the shipped executable and the attribute database
//! (skater-collections):
//! - The top level is the eight categories of class 8BD17173D2F48577, in the
//!   order of the key table at 82FCB738 (Flip Tricks ... Terminology).
//! - A category lists the class 187447EFFFDF8563 entries its field
//!   57035D975F71D506 refers to, in that order (825FDC90). A category with a
//!   single entry opens straight onto that entry's contents (Manuals).
//! - An entry whose field A990472B974734C3 is set is a family: it lists every
//!   trick of class 6918469984A8C596 that refers to it in 06513CC28A532524 and
//!   has a demo animation (3B71EA27D53D7C4F), sorted by F9E5D2F69066E925
//!   (825FDE48 .. 825FE13C).
//! - Titles are 7296FFD6010D73EF (entries) / 843613E915014627 (tricks),
//!   descriptions 273DF7D4EE7A4C1C / 4663ADFED2CD5EF7.
//! - The controls are four tTrickGuideInputSlot (03ACC9F200807B87): gesture,
//!   button, connector, named through the tables at 82FCB650 (gestures) and
//!   82FCB5D8 (buttons); connector 1 Plus, 2 Arrow, 3 Or (825962A0).
use crate::apt_screen::{array, literal, Natives, Screen};
use crate::apt_vm::{Value, Vm};
use serde_json::Value as Json;
use std::collections::HashMap;

const CATEGORY_CLASS: &str = "Hash_8BD17173D2F48577";
const ENTRY_CLASS: &str = "Hash_187447EFFFDF8563";
const TRICK_CLASS: &str = "Hash_6918469984A8C596";
/// 82FCB738: the top level, in order.
const CATEGORIES: [u64; 8] = [
    0x967345FFA6169017, // Flip Tricks
    0x9E2B56D3EAC111E3, // Air Tricks
    0x0AE5C05D32117E6C, // Grinds & Edge Tricks
    0x046ED0CCBDA1091F, // Manuals
    0x7680DF0642C4A9BE, // Hall of Meat
    0x5300D382AC62289A, // Other Tricks
    0xF5C5F450B0C8703E, // Off-Board Actions
    0xD6CA2A68FFE6A743, // Terminology
];
const TITLE: &str = "Hash_7296FFD6010D73EF";
const DESCRIPTION: &str = "Hash_273DF7D4EE7A4C1C";
const ITEMS: &str = "Hash_57035D975F71D506";
const FAMILY: &str = "Hash_A990472B974734C3";
const SLOTS: &str = "Hash_03ACC9F200807B87";
const TRICK_TITLE: &str = "Hash_843613E915014627";
const TRICK_DESCRIPTION: &str = "Hash_4663ADFED2CD5EF7";
const TRICK_FAMILY: &str = "Hash_06513CC28A532524";
const TRICK_ANIMATION: &str = "Hash_3B71EA27D53D7C4F";
const TRICK_ORDER: &str = "Hash_F9E5D2F69066E925";

/// 82FCB650: the stick gestures the screen's gesture items play, by id.
const GESTURES: [&str; 58] = [
    "", "grind_5_0_Grind", "grind_fs_5_0", "grind_fs_bluntslide", "grind_fs_boardslide", "grind_fs_crooks",
    "grind_fs_lipslide", "grind_fs_noseblunt", "grind_fs_overcrooks", "grind_fs_salad", "grind_fs_noseslide",
    "grind_fs_tailslide", "grind_nose_grind", "grind_fs_feeble", "grind_fs_smith", "grind_fs_willie",
    "grind_fs_overwillie", "nollie", "nollie_360_flip", "nollie_360_hardflip", "nollie_360_heelflip",
    "nollie_360_inward_heel", "nollie_360popshuvit", "nollie_fs_360popshuvit", "nollie_fs_popshuvit",
    "nollie_hardflip", "nollie_heelflip", "nollie_inward_heelflip", "nollie_kickflip", "nollie_popshuvit",
    "nollie_varial_flip", "nollie_varial_heelflip", "nollie_fs_popshuvit_late", "nollie_popshuvit_late",
    "nollie_underflip_shuv", "nollie_underflip_heelflip", "nollie_underflip_kickflip", "ollie",
    "ollie_360_flip", "ollie_360_hardflip", "ollie_360_heelflip", "ollie_360_inward_heelflip",
    "ollie_360popshuvit", "ollie_fs_360popshuvit", "ollie_fs_popshuvit", "ollie_hardflip", "ollie_heelflip",
    "ollie_inward_heelflip", "ollie_kickflip", "ollie_popshuvit", "ollie_varial_heelflip",
    "ollie_varial_kickflip", "ollie_underflip_kickflip", "ollie_underflip_shuv", "ollie_fs_popshuvit_late",
    "ollie_popshuvit_late", "ollie_underflip_heelflip", "revert",
];
/// 82FCB5D8: the buttons the screen's button items draw, by id.
const BUTTONS: [&str; 28] = [
    "", "Back", "Start", "A", "B", "Y", "X", "R_Trigger", "L_Trigger", "L_Bumper", "R_Bumper", "L3_Click",
    "R3_Click", "Stick_Right_Up", "Stick_Right_Down", "Stick_Right_L", "Stick_Right_R", "Stick_Right",
    "DPad_Up", "DPad_Down", "DPad_Left", "DPad_Right", "Stick_Left_Up", "Stick_Left_Down", "Stick_Left_L",
    "Stick_Left_R", "Stick_Left", "DPad",
];

/// One control step: gesture id, button id, connector to the next step.
pub(crate) type Slot = [u32; 3];

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Node {
    pub title: String,
    pub description: String,
    pub children: Vec<Node>,
    pub slots: Vec<Slot>,
}

/// The guide's menu from the converted attribute database.
pub(crate) fn tree(collections: &Json) -> Result<Vec<Node>, String> {
    let rows = collections["collections"].as_array().ok_or("skater collections: no collections")?;
    let id = |text: &str| skate_data::attrib_hash::numeric_name(text);
    let mut by: HashMap<(String, String), &Json> = HashMap::new();
    for row in rows {
        let (Some(class), Some(key)) = (row["class"].as_str(), row["key"].as_str()) else { continue };
        by.entry((id(class), id(key))).or_insert(row);
    }
    // A field, inherited through the row's parents as AttribSys reads it.
    let field = |class: &str, key: &str, name: &str| -> Option<&Json> {
        let mut at = by.get(&(id(class), id(key)))?;
        for _ in 0..16 {
            if let Some(value) = at["fields"].get(name) {
                return Some(value);
            }
            let parent = at["parent"].as_str().filter(|p| !p.is_empty())?;
            at = by.get(&(id(class), id(parent)))?;
        }
        None
    };
    let text = |class: &str, key: &str, name: &str| -> String {
        field(class, key, name).and_then(|f| f["data"].as_str()).filter(|t| !t.starts_with("#ATTRIB_MISSING")).unwrap_or("").to_owned()
    };
    let slots = |class: &str, key: &str| -> Vec<Slot> {
        let Some(items) = field(class, key, SLOTS).and_then(|f| f["array"]["items"].as_array()) else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| item.as_str())
            .filter_map(|hex| {
                let word = |i: usize| u32::from_str_radix(hex.get(i * 8..i * 8 + 8)?, 16).ok();
                Some([word(0)?, word(1)?, word(2)?])
            })
            .filter(|s| s[0] != 0 || s[1] != 0)
            .collect()
    };
    // A reference ("<class 16 hex><key 16 hex>...") to an entry key.
    let reference = |hex: &str| hex.get(16..32).map(|k| format!("Hash_{}", k.to_ascii_uppercase()));
    // Tricks by the family they belong to, in collection order.
    let mut families: HashMap<String, Vec<(i32, Node)>> = HashMap::new();
    for row in rows.iter().filter(|r| r["class"].as_str().map(id).as_deref() == Some(TRICK_CLASS)) {
        let key = row["key"].as_str().unwrap_or_default();
        if text(TRICK_CLASS, key, TRICK_ANIMATION).is_empty() {
            continue;
        }
        let Some(family) = field(TRICK_CLASS, key, TRICK_FAMILY).and_then(|f| f["data"].as_str()).and_then(reference) else { continue };
        let order = field(TRICK_CLASS, key, TRICK_ORDER)
            .and_then(|f| f["data"].as_str())
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .map_or(0, |v| v as i32);
        let node = Node {
            title: text(TRICK_CLASS, key, TRICK_TITLE),
            description: text(TRICK_CLASS, key, TRICK_DESCRIPTION),
            children: Vec::new(),
            slots: slots(TRICK_CLASS, key),
        };
        families.entry(id(&family)).or_default().push((order, node));
    }
    for list in families.values_mut() {
        list.sort_by_key(|(order, _)| *order);
    }
    let entry = |key: &str| -> Option<Node> {
        by.get(&(id(ENTRY_CLASS), id(key)))?;
        let family = field(ENTRY_CLASS, key, FAMILY).and_then(|f| f["data"].as_str()).is_some_and(|d| d.starts_with("01"));
        let children = if family {
            families.get(&id(key)).map(|l| l.iter().map(|(_, n)| n.clone()).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };
        Some(Node { title: text(ENTRY_CLASS, key, TITLE), description: text(ENTRY_CLASS, key, DESCRIPTION), children, slots: slots(ENTRY_CLASS, key) })
    };
    let mut top = Vec::new();
    for category in CATEGORIES {
        let key = format!("Hash_{category:016X}");
        let refs = field(CATEGORY_CLASS, &key, ITEMS).and_then(|f| f["array"]["items"].as_array()).cloned().unwrap_or_default();
        // An entry the database lacks (a later download's) is left out.
        let mut children: Vec<Node> = refs.iter().filter_map(|r| r.as_str()).filter_map(reference).filter_map(|k| entry(&k)).collect();
        // One entry: the category opens straight onto its contents.
        if children.len() == 1 && !children[0].children.is_empty() {
            children = std::mem::take(&mut children[0].children);
        }
        top.push(Node { title: text(CATEGORY_CLASS, &key, TITLE), description: text(CATEGORY_CLASS, &key, DESCRIPTION), children, slots: Vec::new() });
    }
    Ok(top)
}

/// The screen's natives: where the menu is, and what the picker reports.
#[derive(Default)]
pub(crate) struct Guide {
    pub root: Vec<Node>,
    /// Chosen item per level above the current one.
    pub path: Vec<usize>,
    /// The highlight on the current level.
    pub index: usize,
    pub goofy: bool,
    /// The picker's accept, taken by the driver.
    pub selected: Option<usize>,
}
impl Guide {
    pub(crate) fn level(&self) -> &[Node] {
        let mut list = &self.root[..];
        for &i in &self.path {
            match list.get(i) {
                Some(node) => list = &node.children,
                None => return &[],
            }
        }
        list
    }
    pub(crate) fn current(&self) -> Option<&Node> {
        self.level().get(self.index)
    }
    /// Into the highlighted item, if it has a list of its own.
    pub(crate) fn enter(&mut self) -> bool {
        if self.current().is_some_and(|n| !n.children.is_empty()) {
            self.path.push(self.index);
            self.index = 0;
            true
        } else {
            false
        }
    }
    /// Back a level, to the item that was open; false at the top.
    pub(crate) fn back(&mut self) -> bool {
        match self.path.pop() {
            Some(i) => {
                self.index = i;
                true
            }
            None => false,
        }
    }
}

fn text(value: &str) -> Value {
    if value.starts_with("ID_") { Value::Text(value.into()) } else { literal(value) }
}

impl Natives for Guide {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let number = |i: usize| args.get(i).map_or(0.0, Value::number);
        Ok(Some(match (native, method) {
            ("FETrickTutorial", "IsGoofy") => Value::Bool(self.goofy),
            ("HUDComponents", "PlayerStance_IsRegular") => Value::Bool(!self.goofy),
            ("FETrickTutorial", "GetCurrentMenuItem") => Value::Number(self.index as f64),
            // [level, has a list of its own, description]
            ("FETrickTutorial", "GetMenuItemData") => {
                let node = self.current();
                let open = node.is_some_and(|n| !n.children.is_empty());
                let description = node.map_or("", |n| n.description.as_str()).to_owned();
                let level = self.path.len() as f64;
                array(vm, [Value::Number(level), Value::Bool(open), if description.is_empty() { literal("") } else { text(&description) }])?
            }
            // [steps, then gesture, button, connector for each of four]
            ("FETrickTutorial", "GetMenuItemGesturesButtons") => {
                let slots = self.current().map(|n| n.slots.clone()).unwrap_or_default();
                let mut out = vec![Value::Number(slots.len().min(4) as f64)];
                for i in 0..4 {
                    let [gesture, button, connector] = slots.get(i).copied().unwrap_or_default();
                    let gesture = GESTURES.get(gesture as usize).copied().unwrap_or("");
                    let button = BUTTONS.get(button as usize).copied().unwrap_or("");
                    let connector = match connector { 1 => "Plus", 2 => "Arrow", 3 => "Or", _ => "" };
                    out.extend([Value::Text(gesture.into()), Value::Text(button.into()), Value::Text(connector.into())]);
                }
                array(vm, out)?
            }
            ("Game", "GetMenuItems") => {
                let titles: Vec<Value> = self.level().iter().map(|n| text(&n.title)).collect();
                array(vm, titles)?
            }
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game", "OnMenuHilight") => {
                self.index = number(1).max(0.) as usize;
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                self.index = number(1).max(0.) as usize;
                self.selected = Some(self.index);
                Value::Undefined
            }
            ("LetterBox", _) => Value::Undefined,
            _ => return Ok(None),
        }))
    }
}

pub(crate) const OBJECTS: [(&str, &[(&str, Value)]); 4] =
    [("Game", &[]), ("LetterBox", &[]), ("FETrickTutorial", &[]), ("HUDComponents", &[])];

pub(crate) fn load(json: &serde_json::Value, globals: &[(&str, Value)]) -> Result<Screen<Guide>, String> {
    Screen::load(json, Guide::default(), &OBJECTS, globals)
}

/// Bring the screen in as the screen manager did: visible, its intro, then
/// the list and the highlighted item's controls.
pub(crate) fn open(screen: &mut Screen<Guide>) -> Result<(), String> {
    let root = screen.bindings.movie.root;
    screen.vm.set(root, "_visible", Value::Bool(true))?;
    screen.bindings.movie.method(&mut screen.vm, root, "play", &[])?;
    refresh(screen)
}

/// The level changed: rebuild the list, then show the item's controls.
pub(crate) fn refresh(screen: &mut Screen<Guide>) -> Result<(), String> {
    screen.set_state(1.)?;
    screen.set_state(11.)
}

// ---- in game -------------------------------------------------------------

use crate::apt_scene;
use crate::apt_stage::{ScreenComposite, ScreenMaterial, Stage, read_screen};
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::PrimaryWindow;

const LAYER: usize = 34;
const FRAME: f32 = 1.0 / 60.0;
const UP: f64 = 1.0;
const DOWN: f64 = 2.0;
const NEXT: f64 = 5.0;
/// The picker shows this many rows (trickguide's NumOptions).
const ROWS: usize = 8;

#[derive(Resource)]
pub(crate) struct GuideUi {
    screen: Screen<Guide>,
    shapes: apt_scene::Shapes,
    stage: Stage,
    shown: bool,
    clock: f32,
    failed: bool,
    cursor: Option<Vec2>,
    touched_at: f32,
}

pub(crate) struct TrickGuidePlugin;
impl Plugin for TrickGuidePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup))
            .add_systems(Update, (verify_open, drive, present).chain().after(crate::graphics_menu::interact));
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
    let dir = config.asset_root.join("private/frontend");
    if !skate_data::vfs::is_file(dir.join("trickguide.json")) {
        return;
    }
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 22, -1);
    let loaded = (|| -> Result<_, String> {
        let (json, shapes) = read_screen(&dir, "trickguide.json")?;
        let globals: Vec<(&str, Value)> = crate::main_menu::APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
        let mut screen = load(&json, &globals)?;
        let root = screen.bindings.movie.root;
        screen.vm.set(root, "_visible", Value::Bool(false))?;
        stage.load_textures(&mut images, &dir, &shapes, &screen.bindings.movie)?;
        Ok((screen, shapes))
    })();
    match loaded {
        Ok((screen, shapes)) => {
            commands.insert_resource(GuideUi { screen, shapes, stage, shown: false, clock: 0., failed: false, cursor: None, touched_at: -1. });
        }
        Err(error) => warn!("Original Trick Guide unavailable ({error})"),
    }
}

/// The menu, built the first time the guide opens (the collections are big).
fn build_tree(config: &crate::config::Config) -> Result<Vec<Node>, String> {
    let path = config.asset_root.join("private/stock/skater-collections.json");
    let bytes = skate_data::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: Json = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    tree(&json)
}

impl GuideUi {
    /// The picker row (0..ROWS, top first) under a stage point.
    fn row_at(&self, point: [f32; 2]) -> Option<usize> {
        let (vm, movie) = (&self.screen.vm, &self.screen.bindings.movie);
        (0..ROWS).find(|&k| row_clip(vm, movie, k).is_some_and(|c| movie.shown(vm, c) && movie.hit(vm, c, point)))
    }
    /// The first item the picker shows (it scrolls when a level is long).
    fn window_start(&self) -> usize {
        let (vm, movie) = (&self.screen.vm, &self.screen.bindings.movie);
        movie.child(vm, movie.root, PICKER)
            .map(|p| vm.get(p, "mWindowIndexMin").number())
            .filter(|v| v.is_finite() && *v >= 0.)
            .map_or(0, |v| v as usize)
    }
    fn press(&mut self, code: f64) -> Result<(), String> {
        self.screen.call("UpdateInput", vec![Value::Number(code), Value::Number(0.)]).map(|_| ())
    }
    /// Move the highlight to `target` the way the pad does.
    fn move_to(&mut self, target: usize) -> Result<(), String> {
        for _ in 0..64 {
            let at = self.screen.bindings.natives.index;
            if at == target {
                break;
            }
            self.press(if target > at { DOWN } else { UP })?;
            if self.screen.bindings.natives.index == at {
                break;
            }
        }
        self.screen.set_state(11.)
    }
}

const PICKER: &str = "mTrickGuide.mContent.mTopInner.mMenuPicker";

/// The clip of the picker's `k`th visible row (its item prefix + k).
fn row_clip(vm: &Vm, movie: &crate::apt_movie::Movie, k: usize) -> Option<usize> {
    let picker = movie.child(vm, movie.root, PICKER)?;
    let prefix = vm.get(picker, "mStandardItemPrefix").text();
    movie.child(vm, picker, &format!("mMenuPicker.mItemGroup.{prefix}{k}"))
}

#[allow(clippy::too_many_arguments)]
fn drive(
    ui: Option<ResMut<GuideUi>>,
    config: Res<crate::config::Config>,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    nav: Res<crate::customiser::Navigation>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    (touches, window, time): (Res<Touches>, Single<&Window, With<PrimaryWindow>>, Res<Time<Real>>),
    skater: Option<Res<crate::physics::SkaterRuntime>>,
) {
    let Some(mut ui) = ui else {
        // Without the screen there is nothing to show: back to the menu.
        menu.trick_guide = false;
        return;
    };
    let ui = &mut *ui;
    if ui.failed {
        menu.trick_guide = false;
        return;
    }
    if !menu.trick_guide {
        if ui.shown {
            ui.shown = false;
            let root = ui.screen.bindings.movie.root;
            let _ = ui.screen.vm.set(root, "_visible", Value::Bool(false));
        }
        return;
    }
    let result = (|| -> Result<(), String> {
        if !ui.shown {
            if ui.screen.bindings.natives.root.is_empty() {
                ui.screen.bindings.natives.root = build_tree(&config)?;
            }
            let natives = &mut ui.screen.bindings.natives;
            (natives.path, natives.index, natives.selected) = (Vec::new(), 0, None);
            // The gestures mirror for a goofy rider, as in the game.
            natives.goofy = skater.as_ref().is_some_and(|s| !s.animation.packet.regular_stance);
            ui.shown = true;
            return open(&mut ui.screen);
        }
        let up = keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) || nav.pressed & 1 != 0;
        let down = keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) || nav.pressed & 2 != 0;
        let accept = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || nav.pressed & 0x1000 != 0;
        let back = keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Backspace) || nav.pressed & 0x2000 != 0
            || mouse.just_pressed(MouseButton::Right);
        // Mouse and touch: the stage is 1280x720 stretched over the window.
        let stage = |p: Vec2| [p.x / window.width().max(1.) * 1280., p.y / window.height().max(1.) * 720.];
        let now = time.elapsed_secs();
        let (mut click, mut hover) = (None, None);
        // The screenshot hook walks the guide itself; a stray pointer must not.
        let scripted = std::env::var_os("SKATE_VERIFY_TRICK_GUIDE").is_some();
        if scripted {
            // (nothing: the hook drives)
        } else if let Some(touch) = touches.iter_just_released().last() {
            ui.touched_at = now;
            click = Some(stage(touch.position()));
        } else if let Some(cursor) = window.cursor_position() {
            if mouse.just_pressed(MouseButton::Left) && now - ui.touched_at > 0.6 {
                click = Some(stage(cursor));
            } else if ui.cursor.is_some_and(|last| last.distance(cursor) > 0.5) {
                hover = Some(stage(cursor));
            }
            ui.cursor = Some(cursor);
        }
        if up || down {
            ui.press(if up { UP } else { DOWN })?;
            ui.screen.set_state(11.)?;
        }
        if let Some(point) = hover {
            if let Some(row) = ui.row_at(point) {
                let target = ui.window_start() + row;
                if target != ui.screen.bindings.natives.index {
                    ui.move_to(target)?;
                }
            }
        }
        let mut chosen = accept;
        if let Some(row) = click.and_then(|point| ui.row_at(point)) {
            ui.move_to(ui.window_start() + row)?;
            chosen = true;
        }
        if chosen {
            // The picker plays its select sound and reports through OnMenuSelect.
            ui.press(NEXT)?;
            ui.screen.bindings.natives.selected = None;
            if ui.screen.bindings.natives.enter() {
                refresh(&mut ui.screen)?;
            }
        }
        if back {
            sounds.push("FE_SOUND_CORE_B_BUTTON");
            if ui.screen.bindings.natives.back() {
                refresh(&mut ui.screen)?;
            } else {
                // Out of the top level: the guide closes onto the menu.
                menu.trick_guide = false;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("Original Trick Guide stopped: {error}");
        ui.failed = true;
        menu.trick_guide = false;
    }
}

/// Opt-in screenshot coverage (--verify with SKATE_VERIFY_TRICK_GUIDE=<path of
/// item indices, e.g. 0/0/1 for Flip Tricks > Ollie Flips > Kickflip>): open
/// the guide once the skater is ready and walk to that item.
fn verify_open(
    animation: Res<crate::animation::AnimationStatus>,
    ui: Option<ResMut<GuideUi>>,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    mut state: Local<u8>,
) {
    let Ok(path) = std::env::var("SKATE_VERIFY_TRICK_GUIDE") else { return };
    let Some(mut ui) = ui else { return };
    match *state {
        0 if animation.ready => {
            menu.open = true;
            menu.trick_guide = true;
            *state = 1;
        }
        1 if ui.shown => {
            let steps: Vec<usize> = path.split('/').filter_map(|s| s.parse().ok()).collect();
            let result = (|| -> Result<(), String> {
                for (i, &step) in steps.iter().enumerate() {
                    ui.move_to(step)?;
                    if i + 1 < steps.len() && ui.screen.bindings.natives.enter() {
                        refresh(&mut ui.screen)?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                error!("Trick Guide verify: {error}");
            }
            *state = 2;
        }
        _ => {}
    }
}

fn present(
    mut commands: Commands,
    ui: Option<ResMut<GuideUi>>,
    time: Res<Time<Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
) {
    let Some(mut ui) = ui else { return };
    let ui = &mut *ui;
    if ui.failed || !ui.shown {
        ui.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
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
            ui.screen.advance()?;
        }
        for name in ui.screen.take_sounds() {
            sounds.push(name);
        }
        apt_scene::draw(&ui.screen.bindings.movie, &ui.screen.vm, &ui.shapes)
    })();
    match result {
        Ok(draws) => ui.stage.render(&mut commands, &mut meshes, &mut materials, &draws),
        Err(error) => {
            error!("Original Trick Guide stopped: {error}");
            ui.failed = true;
            ui.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires the private front-end export and collections (SKATE_FRONTEND_DIR, SKATE_COLLECTIONS)"]
    fn original_trick_guide_runs() {
        let dir = std::path::PathBuf::from(std::env::var("SKATE_FRONTEND_DIR").expect("SKATE_FRONTEND_DIR"));
        let (json, _) = read_screen(&dir, "trickguide.json").unwrap();
        let globals: Vec<(&str, Value)> = crate::main_menu::APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
        let mut screen = load(&json, &globals).unwrap();
        screen.bindings.natives.root = guide().expect("SKATE_COLLECTIONS");
        open(&mut screen).unwrap();
        for _ in 0..30 {
            screen.advance().unwrap();
        }
        let (vm, movie) = (&screen.vm, &screen.bindings.movie);
        let rows: Vec<String> = (0..ROWS)
            .filter_map(|k| row_clip(vm, movie, k))
            .filter(|&c| movie.shown(vm, c))
            .map(|c| {
                let label = ["mText.mTitle.TEXTLABEL", "mText.mTitle", "mText.TEXTLABEL", "mText"].iter().find_map(|p| movie.child(vm, c, p));
                label.map(|t| vm.get(t, "text").text()).unwrap_or_default()
            })
            .collect();
        eprintln!("rows: {rows:?}");
        assert!(rows.len() >= 8, "the picker shows its rows");
        // Into Flip Tricks > Ollie Flips, onto Kickflip: its flick shows.
        for _ in 0..2 {
            screen.call("UpdateInput", vec![Value::Number(NEXT), Value::Number(0.)]).unwrap();
            assert!(screen.bindings.natives.enter());
            refresh(&mut screen).unwrap();
        }
        screen.call("UpdateInput", vec![Value::Number(DOWN), Value::Number(0.)]).unwrap();
        screen.set_state(11.).unwrap();
        assert_eq!(screen.bindings.natives.current().unwrap().title, "ID_TRICK_FLIP_KICKFLIP");
        for _ in 0..30 {
            screen.advance().unwrap();
        }
        let (vm, movie) = (&screen.vm, &screen.bindings.movie);
        let item = movie
            .child(vm, movie.root, "mTrickGuide.mContent.mBottomInner.mcGestureButtonGroup.mcGestureItem0")
            .expect("gesture item");
        assert!(vm.get(item, "_visible").truth(), "Kickflip shows its gesture");
    }

    fn guide() -> Option<Vec<Node>> {
        let path = std::env::var("SKATE_COLLECTIONS").ok()?;
        let json: Json = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        Some(tree(&json).unwrap())
    }

    #[test]
    #[ignore = "Requires the private skater collections (SKATE_COLLECTIONS)"]
    fn the_guide_is_the_games_own() {
        let top = guide().expect("SKATE_COLLECTIONS");
        let titles: Vec<&str> = top.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, [
            "ID_TRICK_TYPE_FLIP_TRICKS", "ID_TRICK_TYPE_AIR_TRICKS", "ID_TRICK_TYPE_GRINDS_EDGE_TRICKS",
            "ID_TRICK_TYPE_MANUALS", "ID_TRICK_TYPE_HOM", "ID_TRICK_TYPE_OTHER_TRICKS",
            "ID_TRICK_TYPE_OFFBOARD_ACTIONS", "ID_TRICK_TYPE_TERMINOLOGY",
        ]);
        // Flip Tricks > Ollie Flips starts Ollie, Kickflip, Heelflip.
        let ollie_flips = &top[0].children[0];
        assert_eq!(ollie_flips.title, "ID_TRICK_TYPE_OLLIE_FLIPS");
        let first: Vec<&str> = ollie_flips.children.iter().take(3).map(|n| n.title.as_str()).collect();
        assert_eq!(first, ["ID_TRICK_FLIP_OLLIE", "ID_TRICK_FLIP_KICKFLIP", "ID_TRICK_FLIP_HEELFLIP"]);
        // Kickflip is flicked: the ollie_kickflip gesture.
        assert_eq!(GESTURES[ollie_flips.children[1].slots[0][0] as usize], "ollie_kickflip");
        // Manuals has one entry, so it opens on the manuals themselves.
        assert!(top[3].children.iter().all(|n| n.children.is_empty()) && !top[3].children.is_empty());
        // Off-Board Actions: Sprint is a stick and a button.
        let sprint = top[6].children.iter().find(|n| n.title == "ID_TRICK_TYPE_SPRINT").unwrap();
        assert_eq!(sprint.slots.len(), 2);
        let count: usize = top.iter().map(|c| c.children.iter().map(|e| e.children.len().max(1)).sum::<usize>()).sum();
        eprintln!("trick guide: {} categories, {count} pages", top.len());
    }

    #[test]
    fn navigation_keeps_the_way_back() {
        let leaf = |t: &str| Node { title: t.into(), ..Default::default() };
        let mut guide = Guide {
            root: vec![Node { title: "a".into(), children: vec![leaf("a1"), leaf("a2")], ..Default::default() }, leaf("b")],
            ..Default::default()
        };
        guide.index = 0;
        assert!(guide.enter());
        guide.index = 1;
        assert_eq!(guide.current().unwrap().title, "a2");
        assert!(!guide.enter(), "a leaf has nothing to open");
        assert!(guide.back());
        assert_eq!((guide.index, guide.path.len()), (0, 0));
        assert!(!guide.back(), "the top level closes the guide instead");
    }
}
