//! Skate 3's own Create-a-Skater screen (`cas/createskater`), run by the APT
//! player over the customiser's menu.
//!
//! The screen is a view: the game owns the menu tree and the screen asks it,
//! through `Game.CAC_*` natives, for the current node's title, items, their
//! kinds and values, and thumbnails. `SetState` picks the layout for the node
//! (0 a list, 1 a 3x3 thumbnail browser, 2 style options, 3 sliders, 6 a 5x3
//! graphics browser). Moving the highlight reports `OnMenuHilight` (the
//! customiser previews that choice on the skater); choosing reports
//! `OnMenuSelect` or `CAC_OnThumbnailSelect`.
use crate::apt_screen::{Natives, Screen, array, literal};
use crate::apt_vm::{Value, Vm};

/// The layout for a menu node (createskater's SetState values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    List = 0,
    Thumbs = 1,
    Style = 2,
    Sliders = 3,
    Graphics = 6,
}

/// One row of the current node.
#[derive(Clone, Debug, Default)]
pub(crate) struct Item {
    /// Shown text: a string-table id (ID_...) or literal text.
    pub text: String,
    /// "option", "category", "slider", "selector", "morph" or "color" for
    /// sliders and style rows; thumbnails show it as their corner icon.
    pub kind: &'static str,
    /// Description under the list ("blurb" panel).
    pub description: String,
    /// A slider's position, 0-10.
    pub integer: i32,
    /// A selector's current choice, or a slider's caption.
    pub string: String,
    /// Swatches of a colour row, RGB 0-255.
    pub colours: Vec<[u8; 3]>,
    /// A thumbnail's texture key (an image the stage knows by name).
    pub thumbnail: String,
    pub new: bool,
}

/// What the screen shows now.
#[derive(Clone, Debug, Default)]
pub(crate) struct View {
    pub title: String,
    pub filter: String,
    pub items: Vec<Item>,
    pub index: usize,
    pub window: usize,
    pub help_a: String,
    pub help_b: String,
}

/// The natives' side: the current view plus what the screen reported back.
#[derive(Default)]
pub(crate) struct Cas {
    pub view: View,
    /// Highlight moved to this index (the customiser previews it).
    pub hilight: Option<usize>,
    /// This index was chosen.
    pub chosen: Option<usize>,
    /// Image slots the screen's image renderers reserved, and the image
    /// each shows (`ImageRenderer_*`): drawn over the clip named by the slot.
    pub images: std::collections::BTreeMap<i32, String>,
    reserved: std::collections::BTreeSet<i32>,
    /// Colour swatches (`SwatchRenderer_*`), by slot (from 1000, so their
    /// clip names never meet an image slot's).
    pub swatches: std::collections::BTreeMap<i32, [f32; 3]>,
}

fn text(value: &str) -> Value {
    if value.starts_with("ID_") { Value::Text(value.into()) } else { literal(value) }
}

impl Natives for Cas {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let number = |i: usize| args.get(i).map_or(0.0, Value::number);
        let index = |i: usize| {
            let n = number(i);
            if n.is_finite() && n >= 0.0 { n as usize } else { usize::MAX }
        };
        let item = |i: usize| self.view.items.get(i);
        Ok(Some(match (native, method) {
            ("Game", "CAC_GetNumItems") => Value::Number(self.view.items.len() as f64),
            ("Game", "CAC_GetItemText") => item(index(0)).map_or(literal(""), |i| text(&i.text)),
            ("Game", "CAC_GetCurrentMenuIndex") => Value::Number(self.view.index as f64),
            ("Game", "CAC_GetCurrentMenuMinWindowIndex") => Value::Number(self.view.window as f64),
            ("Game", "CAC_UpdateMenuIndices") => {
                if index(0) < self.view.items.len() {
                    self.view.index = index(0);
                    self.view.window = index(1).min(self.view.items.len());
                }
                Value::Undefined
            }
            // Kinds and panel states are compared and used as frame labels by
            // the script, so they go back as plain strings.
            ("Game", "CAC_GetOptionType") => Value::Text(item(index(0)).map_or("option", |i| i.kind).into()),
            // "#" alone is the script's empty text.
            ("Game", "CAC_GetStringValue") => match item(index(0)) {
                Some(i) if !i.string.is_empty() && i.string != "#" => text(&i.string),
                _ => Value::Text("#".into()),
            },
            ("Game", "CAC_GetIntegerValue") => Value::Number(item(index(0)).map_or(0, |i| i.integer) as f64),
            ("Game", "CAC_GetDescriptionText") => item(index(0)).map_or(Value::Text("#".into()), |i| {
                if i.description.is_empty() { Value::Text("#".into()) } else { text(&i.description) }
            }),
            // A thumbnail's panel shows its name under its icon state; list
            // rows show their description, if any.
            ("Game", "CAC_GetInfoPanelType") => Value::Text(match item(index(0)) {
                Some(i) if matches!(i.kind, "owned" | "none") => i.kind,
                Some(i) if !i.description.is_empty() => "blurb",
                _ => "noblurb",
            }.into()),
            ("Game", "CAC_GetItemUnlockHALID") => Value::Text("#".into()),
            ("Game", "CAC_GetTitleText") => text(&self.view.title),
            // "#" alone hides the filter tab.
            ("Game", "CAC_GetFilterText") => {
                if self.view.filter.is_empty() { Value::Text("#".into()) } else { text(&self.view.filter) }
            }
            ("Game", "CAC_GetButtonHelpTextA") => if self.view.help_a.is_empty() { Value::Text("#".into()) } else { text(&self.view.help_a) },
            ("Game", "CAC_GetButtonHelpTextB") => if self.view.help_b.is_empty() { Value::Text("#".into()) } else { text(&self.view.help_b) },
            // No store: every part is free, so the wallet shows nothing.
            ("Game", "CAC_GetReceiptBalance") => literal(""),
            ("Game", "CAC_GetThumbnailFileName") => item(index(0)).map_or(literal(""), |i| literal(&i.thumbnail)),
            // The panel's picture: the highlighted item, larger.
            ("Game", "CAC_GetSponsorLogoFileName") => item(index(0)).map_or(Value::Text("#".into()), |i| literal(&i.thumbnail)),
            ("Game", "CAC_IsPartNew") => Value::Bool(item(index(0)).is_some_and(|i| i.new)),
            ("Game", "CAC_GetColours") => {
                let colours = item(index(0)).map(|i| i.colours.clone()).unwrap_or_default();
                array(vm, colours.iter().flat_map(|c| c.iter().map(|v| Value::Number(f64::from(*v)))).collect::<Vec<_>>())?
            }
            ("Game", "CAC_OnThumbnailSelect") => {
                if index(0) < self.view.items.len() {
                    self.chosen = Some(index(0));
                }
                Value::Undefined
            }
            ("Game", "OnMenuHilight") => {
                if index(1) < self.view.items.len() {
                    self.view.index = index(1);
                    self.hilight = Some(index(1));
                }
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                if index(1) < self.view.items.len() {
                    self.chosen = Some(index(1));
                }
                Value::Undefined
            }
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game", "ImageRenderer_ReserveNextFreeIndex") => {
                let slot = (0..).find(|i| !self.reserved.contains(i)).unwrap_or(0);
                self.reserved.insert(slot);
                Value::Number(f64::from(slot))
            }
            ("Game", "ImageRenderer_Load" | "ImageRenderer_LoadAsset" | "ImageRenderer_LoadAssetByName") => {
                let slot = number(1) as i32;
                // Names come back as literal text ("#" marks it as not a
                // string-table id).
                let name = args.first().map(Value::text).unwrap_or_default();
                let name = name.trim_start_matches('#').to_owned();
                if name.is_empty() || name.contains('#') {
                    self.images.remove(&slot);
                } else {
                    self.images.insert(slot, name);
                }
                Value::Undefined
            }
            ("Game", "ImageRenderer_Unload") => {
                self.images.remove(&(number(0) as i32));
                Value::Undefined
            }
            ("Game", "ImageRenderer_FreeImageIndex") => {
                self.images.remove(&(number(0) as i32));
                self.reserved.remove(&(number(0) as i32));
                Value::Undefined
            }
            ("Game", m) if m.starts_with("ImageRenderer_") => Value::Undefined,
            ("Game", "SwatchRenderer_ReserveIndex") => {
                let slot = (1000..).find(|i| !self.swatches.contains_key(i)).unwrap_or(1000);
                self.swatches.insert(slot, [1.; 3]);
                Value::Number(f64::from(slot))
            }
            ("Game", "SwatchRenderer_SetColour") => {
                let c = |i: usize| (number(i) / 255.).clamp(0., 1.) as f32;
                self.swatches.insert(number(0) as i32, [c(1), c(2), c(3)]);
                Value::Undefined
            }
            ("Game", "SwatchRenderer_FreeIndex") => {
                self.swatches.remove(&(number(0) as i32));
                Value::Undefined
            }
            ("LetterBox", "EnableLetterBox") => Value::Undefined,
            _ => return Ok(None),
        }))
    }
}

pub(crate) const OBJECTS: [(&str, &[(&str, Value)]); 2] = [("Game", &[]), ("LetterBox", &[])];

pub(crate) fn load(json: &serde_json::Value, globals: &[(&str, Value)]) -> Result<Screen<Cas>, String> {
    Screen::load(json, Cas::default(), &OBJECTS, globals)
}

/// Bring the screen in, as the screen manager does: visible, then its intro
/// (frames 1-19, which end by reporting IntroComplete).
pub(crate) fn show(screen: &mut Screen<Cas>) -> Result<(), String> {
    let root = screen.bindings.movie.root;
    screen.vm.set(root, "_visible", Value::Bool(true))?;
    screen.bindings.movie.method(&mut screen.vm, root, "play", &[])?;
    screen.call("OnLoaded", vec![]).map(|_| ())
}


// ---- in game -------------------------------------------------------------

use crate::apt_scene;
use crate::apt_stage::{ScreenComposite, ScreenMaterial, Stage, read_screen};
use crate::customiser::{CasRow, Customiser};
use crate::customiser_parts::Parts;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::PrimaryWindow;

const LAYER: usize = 33;
const FRAME: f32 = 1.0 / 60.0;
const UP: f64 = 1.0;
const DOWN: f64 = 2.0;
const LEFT: f64 = 3.0;
const RIGHT: f64 = 4.0;
const NEXT: f64 = 5.0;

#[derive(Resource)]
pub(crate) struct CasUi {
    screen: Screen<Cas>,
    shapes: apt_scene::Shapes,
    stage: Stage,
    rows: Vec<CasRow>,
    layout: Layout,
    /// The page shown (customiser path), to notice when it changes.
    page: Option<Vec<usize>>,
    shown: bool,
    clock: f32,
    failed: bool,
    /// Thumbnails requested from the asset server, by image name.
    thumbnails: std::collections::BTreeMap<String, Handle<Image>>,
    /// The pointer's last stage position, and when a touch last ended (the
    /// browser's synthetic click after a tap must not act twice).
    cursor: Option<Vec2>,
    touched_at: f32,
}

pub(crate) struct CasScreenPlugin;
impl Plugin for CasScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup).after(crate::customiser::setup))
            .add_systems(Update, (verify_open, drive, present).chain()
                .before(crate::customiser_parts::update)
                .after(crate::graphics_menu::interact));
    }
}

fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    mut customiser: ResMut<Customiser>,
    cameras: Query<Entity, With<IsDefaultUiCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<ScreenComposite>>,
) {
    let Ok(output) = cameras.single() else { return };
    let root = config.asset_root.join("private/frontend");
    if !skate_data::vfs::is_file(root.join("createskater.json")) {
        return;
    }
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 21, -1);
    let loaded = (|| -> Result<_, String> {
        let (json, shapes) = read_screen(&root, "createskater.json")?;
        let globals: Vec<(&str, Value)> = crate::main_menu::APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
        let screen = load(&json, &globals)?;
        stage.load_textures(&mut images, &root, &shapes, &screen.bindings.movie)?;
        Ok((screen, shapes))
    })();
    match loaded {
        Ok((screen, shapes)) => {
            customiser.original_ui = true;
            commands.insert_resource(CasUi {
                screen,
                shapes,
                stage,
                rows: Vec::new(),
                layout: Layout::List,
                page: None,
                shown: false,
                clock: 0.,
                failed: false,
                thumbnails: Default::default(),
                cursor: None,
                touched_at: -1.,
            });
        }
        Err(error) => warn!("Original Create-a-Skater screen unavailable ({error}); using the built-in menus"),
    }
}

impl CasUi {
    /// Show the customiser's current page.
    fn refresh(&mut self, state: &Customiser, parts: &Parts) -> Result<(), String> {
        let (layout, mut view, rows) = state.cas_page(parts);
        if !state.status.is_empty() {
            view.filter = state.status.clone();
        }
        self.screen.bindings.natives.view = view;
        self.rows = rows;
        self.layout = layout;
        self.page = Some(state.path().to_vec());
        self.screen.set_state(layout as i32 as f64)
    }
    /// Values changed on the same page (a slider moved): redraw them.
    fn update_values(&mut self, state: &Customiser, parts: &Parts) -> Result<(), String> {
        let (_, view, _) = state.cas_page(parts);
        let index = self.screen.bindings.natives.view.index;
        let natives = &mut self.screen.bindings.natives;
        natives.view.items = view.items;
        natives.view.filter = if state.status.is_empty() { String::new() } else { state.status.clone() };
        natives.view.index = index;
        self.screen.set_state(50.)
    }
}

/// Opt-in screenshot coverage (--verify with SKATE_VERIFY_CAS=<page path,
/// e.g. 1/0>): open Create-a-Skater at that page once the skater is ready.
fn verify_open(
    config: Res<crate::config::Config>,
    animation: Res<crate::animation::AnimationStatus>,
    mut state: ResMut<Customiser>,
    mut done: Local<bool>,
) {
    if *done || config.verification_capture.is_none() || !animation.ready {
        return;
    }
    let Ok(path) = std::env::var("SKATE_VERIFY_CAS") else { return };
    *done = true;
    let path: Vec<usize> = path.split('/').filter_map(|p| p.trim().parse().ok()).collect();
    state.open_at(&path);
}

impl CasUi {
    /// The row under a stage point, and the clip it was found in.
    fn row_at(&self, point: [f32; 2]) -> Option<(usize, usize)> {
        let (vm, movie) = (&self.screen.vm, &self.screen.bindings.movie);
        let controller = self.screen.bindings.shared.screen;
        let hit = |path: &str| movie.child(vm, movie.root, path).filter(|&c| movie.hit(vm, c, point));
        let view = &self.screen.bindings.natives.view;
        match self.layout {
            Layout::Thumbs | Layout::Graphics => {
                let menu = if self.layout == Layout::Thumbs { "mThumbnails" } else { "mThumbs2" };
                let window = vm.get(controller, "mMinWindowIndex").number();
                let window = if window.is_finite() && window >= 0. { window as usize } else { view.window };
                (0..15).find_map(|k| hit(&format!("mCreateSkater.mSelection.{menu}.mThumb{k}")).map(|c| (window + k, c)))
                    .filter(|(i, _)| *i < view.items.len())
            }
            Layout::Sliders | Layout::Style => (0..view.items.len().min(16))
                .find_map(|k| {
                    let menu = if self.layout == Layout::Sliders { "mMorphmenu" } else { "mStylemenu" };
                    hit(&format!("mCreateSkater.mSelection.{menu}.mItem{k}")).map(|c| (k, c))
                }),
            _ => (0..11).find_map(|k| {
                let clip = hit(&format!("mCreateSkater.mSelection.mMenuPicker.mItemGroup.Item{k}"))?;
                // The picker scrolls; its rows show the item text.
                let label = movie.child(vm, clip, "mText.mTitle.TEXTLABEL")
                    .or_else(|| movie.child(vm, clip, "mText.mTitle"))
                    .map(|l| vm.get(l, "text").text())
                    .unwrap_or_default();
                let wanted = label.trim_start_matches('#');
                view.items.iter().position(|i| {
                    let t = i.text.trim_start_matches('#');
                    t == wanted || movie.text_assets.localize(t) == wanted
                })
                .map(|i| (i, clip))
            }),
        }
    }

    /// Move the highlight to `target` the way the pad would.
    fn move_to(&mut self, target: usize) -> Result<(), String> {
        for _ in 0..64 {
            let current = self.screen.bindings.natives.view.index;
            if current == target {
                break;
            }
            let code = match self.layout {
                Layout::Thumbs | Layout::Graphics => {
                    let columns = if self.layout == Layout::Thumbs { 3 } else { 5 };
                    let (row, col) = (current / columns, current % columns);
                    let (trow, tcol) = (target / columns, target % columns);
                    if trow > row { DOWN } else if trow < row { UP } else if tcol > col { RIGHT } else { LEFT }
                }
                _ => if target > current { DOWN } else { UP },
            };
            press(&mut self.screen, code)?;
            if self.screen.bindings.natives.view.index == current {
                break;
            }
        }
        Ok(())
    }
}

/// Input for the original screen, and what the screen reports back.
#[allow(clippy::too_many_arguments)]
fn drive(
    ui: Option<ResMut<CasUi>>,
    mut state: ResMut<Customiser>,
    parts: Res<Parts>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    nav: Res<crate::customiser::Navigation>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut pause: ResMut<crate::graphics_menu::Menu>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    (touches, window, time): (Res<Touches>, Single<&Window, With<PrimaryWindow>>, Res<Time<Real>>),
) {
    let Some(mut ui) = ui else { return };
    let ui = &mut *ui;
    if ui.failed {
        return;
    }
    if !state.open {
        if ui.shown {
            ui.shown = false;
            ui.page = None;
            let root = ui.screen.bindings.movie.root;
            let _ = ui.screen.vm.set(root, "_visible", Value::Bool(false));
        }
        wheel.clear();
        return;
    }
    let result = (|| -> Result<(), String> {
        if !ui.shown {
            ui.shown = true;
            show(&mut ui.screen)?;
            ui.refresh(&state, &parts)?;
            return Ok(());
        }
        if ui.page.as_deref() != Some(state.path()) {
            ui.refresh(&state, &parts)?;
        }
        let up = keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0;
        let down = keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0;
        let left = keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0;
        let right = keys.just_pressed(KeyCode::ArrowRight) || nav.pressed & 8 != 0;
        let accept = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) || nav.pressed & 0x1000 != 0;
        // Mouse and touch: the stage is 1280x720 stretched over the window.
        let stage = |p: Vec2| [p.x / window.width().max(1.) * 1280., p.y / window.height().max(1.) * 720.];
        let now = time.elapsed_secs();
        let mut click = None;
        let mut hover = None;
        if let Some(touch) = touches.iter_just_released().last() {
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
        if let Some(point) = hover {
            if let Some((index, _)) = ui.row_at(point) {
                ui.move_to(index)?;
            }
        }
        if let Some(point) = click {
            if let Some((index, clip)) = ui.row_at(point) {
                ui.move_to(index)?;
                let row = ui.rows.get(index).copied();
                let slider = ui.layout == Layout::Sliders
                    && ui.screen.bindings.natives.view.items.get(index).is_some_and(|i| i.kind != "option");
                if slider {
                    // A slider row: its left half steps down, the right up.
                    let movie = &ui.screen.bindings.movie;
                    let centre = movie.center(&ui.screen.vm, clip).map_or(640., |c| c[0]);
                    if let Some(row) = row {
                        let direction = if point[0] < centre { -1 } else { 1 };
                        sounds.push(if direction > 0 { "FE_SOUND_CORE_NAV_DOWN" } else { "FE_SOUND_CORE_NAV_UP" });
                        state.cas_choose(row, direction, &parts);
                        ui.update_values(&state, &parts)?;
                    }
                } else {
                    press(&mut ui.screen, NEXT)?;
                }
            }
        }
        let back = keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Backspace) || nav.pressed & 0x2000 != 0
            || mouse.just_pressed(MouseButton::Right);
        let mut scroll = 0;
        for event in wheel.read() {
            scroll += if event.y > 0. { -1 } else if event.y < 0. { 1 } else { 0 };
        }
        if back {
            sounds.push("FE_SOUND_CORE_B_BUTTON");
            state.go_back(&mut pause);
            if state.open {
                ui.refresh(&state, &parts)?;
            }
            return Ok(());
        }
        let vertical = [(up || scroll < 0, UP), (down || scroll > 0, DOWN)];
        match ui.layout {
            Layout::Sliders => {
                for (pressed, code) in vertical {
                    if pressed {
                        press(&mut ui.screen, code)?;
                    }
                }
                if left || right {
                    let index = ui.screen.bindings.natives.view.index;
                    if let Some(row) = ui.rows.get(index).copied() {
                        sounds.push(if right { "FE_SOUND_CORE_NAV_DOWN" } else { "FE_SOUND_CORE_NAV_UP" });
                        state.cas_choose(row, if right { 1 } else { -1 }, &parts);
                        ui.update_values(&state, &parts)?;
                    }
                }
                if accept {
                    press(&mut ui.screen, NEXT)?;
                }
            }
            Layout::Thumbs | Layout::Graphics => {
                for (pressed, code) in vertical.into_iter().chain([(left, LEFT), (right, RIGHT)]) {
                    if pressed {
                        press(&mut ui.screen, code)?;
                    }
                }
                if accept {
                    press(&mut ui.screen, NEXT)?;
                }
            }
            _ => {
                for (pressed, code) in vertical {
                    if pressed {
                        press(&mut ui.screen, code)?;
                    }
                }
                if accept {
                    press(&mut ui.screen, NEXT)?;
                }
            }
        }
        // What the screen reported.
        let natives = &mut ui.screen.bindings.natives;
        if let Some(index) = natives.hilight.take() {
            if let Some(row) = ui.rows.get(index).copied() {
                state.cas_hover(row, index);
            }
        }
        if let Some(index) = natives.chosen.take() {
            if let Some(row) = ui.rows.get(index).copied() {
                let before = state.path().to_vec();
                state.cas_choose(row, 0, &parts);
                if state.path() == before.as_slice() && state.open {
                    ui.update_values(&state, &parts)?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("Original Create-a-Skater screen stopped: {error}");
        ui.failed = true;
        state.original_ui = false;
        state.redraw = true;
    }
}

/// A pad or key press, as the screen's UpdateInput takes it.
fn press(screen: &mut Screen<Cas>, code: f64) -> Result<(), String> {
    screen.call("UpdateInput", vec![Value::Number(code), Value::Number(0.)]).map(|_| ())
}

/// Run and draw the screen; thumbnails go over their image slots.
fn present(
    mut commands: Commands,
    ui: Option<ResMut<CasUi>>,
    time: Res<Time<Real>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    mut studio: ResMut<crate::cas_thumbnails::Thumbnails>,
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
        let mut draws = apt_scene::draw(&ui.screen.bindings.movie, &ui.screen.vm, &ui.shapes)?;
        // Pictures go over their image components: the clip an image
        // renderer renamed to its slot number, or the component itself.
        let movie = &ui.screen.bindings.movie;
        let vm = &ui.screen.vm;
        let mut wanted: Vec<(usize, String)> = ui.screen.bindings.shared.images.iter().map(|(c, n)| (*c, n.clone())).collect();
        for (slot, name) in &ui.screen.bindings.natives.images {
            let slot_name = slot.to_string();
            for &clip in movie.instances.keys() {
                if vm.get(clip, "_name").text() == slot_name {
                    wanted.push((clip, name.clone()));
                }
            }
        }
        // Swatches: a solid colour over their renamed clip.
        for (slot, rgb) in &ui.screen.bindings.natives.swatches {
            let slot_name = slot.to_string();
            for &clip in movie.instances.keys() {
                if vm.get(clip, "_name").text() != slot_name || !movie.shown(vm, clip) {
                    continue;
                }
                let Some(b) = movie.local_bounds(vm, clip) else { continue };
                let m = movie.world_matrix(vm, clip);
                let point = |x: f32, y: f32| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                let corners = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[1]), (b[2], b[3]), (b[0], b[3])];
                let vertices = corners.iter().map(|&(x, y)| apt_scene::Vertex { position: point(x, y), uv: [0., 0.] }).collect();
                draws.push(apt_scene::Draw { texture: None, vertices, multiply: [rgb[0], rgb[1], rgb[2], 1.], add: [0.; 4] });
            }
        }
        for (clip, name) in wanted {
            if !movie.instances.contains_key(&clip) || !movie.shown(vm, clip) {
                continue;
            }
            // Parts come from the thumbnail studio once photographed; other
            // pictures (tattoo designs) are files.
            let handle = if let Some(part) = name.strip_prefix("part:") {
                let Some((asset, material)) = part.split_once(':') else { continue };
                match studio.get(asset, material) {
                    Some(image) => image,
                    None => continue,
                }
            } else {
                ui.thumbnails.entry(name.clone()).or_insert_with(|| server.load(name.clone())).clone()
            };
            let key = format!("image:{name}");
            ui.stage.set_texture(&key, handle);
            {
                let Some(b) = movie.local_bounds(vm, clip) else { continue };
                let m = movie.world_matrix(vm, clip);
                let point = |x: f32, y: f32| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                let corners = [(b[0], b[1], 0., 0.), (b[2], b[1], 1., 0.), (b[2], b[3], 1., 1.), (b[0], b[1], 0., 0.), (b[2], b[3], 1., 1.), (b[0], b[3], 0., 1.)];
                let vertices = corners
                    .iter()
                    .map(|&(x, y, u, v)| apt_scene::Vertex { position: point(x, y), uv: [u, v] })
                    .collect();
                // Opaque: texture alpha carries other data (masks, specular).
                draws.push(apt_scene::Draw { texture: Some(key.clone()), vertices, multiply: [1., 1., 1., 0.], add: [0., 0., 0., 1.] });
            }
        }
        Ok(draws)
    })();
    match result {
        Ok(draws) => ui.stage.render(&mut commands, &mut meshes, &mut materials, &draws),
        Err(error) => {
            error!("Original Create-a-Skater screen stopped: {error}");
            ui.failed = true;
            ui.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apt_scene, apt_stage::read_screen};

    fn item(text: &str, kind: &'static str) -> Item {
        Item { text: text.into(), kind, ..Default::default() }
    }

    /// The original screen in each layout, when the private front-end export
    /// is installed (SKATE_FRONTEND_DIR).
    #[test]
    #[ignore = "Requires private installed front-end assets"]
    fn original_create_a_skater_runs() {
        let root = std::path::PathBuf::from(std::env::var("SKATE_FRONTEND_DIR").expect("SKATE_FRONTEND_DIR"));
        let (json, shapes) = read_screen(&root, "createskater.json").unwrap();
        let globals: Vec<(&str, Value)> = crate::main_menu::APT.iter().map(|(n, v)| (*n, Value::Number(*v))).collect();
        let mut screen = load(&json, &globals).unwrap_or_else(|e| panic!("load: {e}"));
        let settle = |screen: &mut Screen<Cas>| {
            for _ in 0..30 {
                screen.advance().unwrap_or_else(|e| panic!("advance: {e}"));
            }
            apt_scene::draw(&screen.bindings.movie, &screen.vm, &shapes).unwrap_or_else(|e| panic!("draw: {e}")).len()
        };
        let natives = &mut screen.bindings.natives;
        natives.view = View {
            title: "ID_CAC_EDITSKATER_TITLE".into(),
            items: vec![item("ID_CAC_SKIN_TITLE", "option"), item("ID_CAC_HAIR_TITLE", "option"), item("Merchandise", "option")],
            help_a: "ID_COMMON_SELECT".into(),
            help_b: "ID_COMMON_BACK".into(),
            ..Default::default()
        };
        show(&mut screen).unwrap_or_else(|e| panic!("show: {e}"));
        screen.set_state(0.).unwrap_or_else(|e| panic!("list: {e}"));
        let drawn = settle(&mut screen);
        assert!(drawn > 0, "the list draws");
        screen.call("UpdateInput", vec![Value::Number(crate::main_menu::APT_DOWN), Value::Number(0.)]).unwrap_or_else(|e| panic!("down: {e}"));
        assert_eq!(screen.bindings.natives.view.index, 1, "down moves the list");

        let natives = &mut screen.bindings.natives;
        natives.view.title = "ID_CAC_THUMBNAILS_TITLE".into();
        natives.view.items = (0..14).map(|i| Item { thumbnail: format!("thumb{i}"), ..item(&format!("Tee {i}"), "option") }).collect();
        natives.view.index = 0;
        screen.set_state(1.).unwrap_or_else(|e| panic!("thumbs: {e}"));
        settle(&mut screen);
        screen.call("UpdateInput", vec![Value::Number(crate::main_menu::APT_RIGHT), Value::Number(0.)]).unwrap();
        assert_eq!(screen.bindings.natives.view.index, 1, "right moves the thumbnails");

        let natives = &mut screen.bindings.natives;
        natives.view.title = "ID_CAC_BODY_SHAPE_TITLE".into();
        natives.view.items = vec![
            Item { integer: 4, string: "#".into(), ..item("Weight", "slider") },
            Item { colours: vec![[200, 160, 130], [120, 80, 60]], integer: 1, ..item("Skin tone", "color") },
            Item { string: "Default".into(), ..item("Posture", "selector") },
        ];
        natives.view.index = 0;
        screen.set_state(3.).unwrap_or_else(|e| panic!("sliders: {e}"));
        {
            let (vm, movie) = (&screen.vm, &screen.bindings.movie);
            println!("right after SetState(3): menu={:?} item0={:?}", movie.child(vm, movie.root, "mCreateSkater.mSelection.mMorphmenu"),
                movie.child(vm, movie.root, "mCreateSkater.mSelection.mMorphmenu.mItem0"));
        }
        for frame in 0..30 {
            screen.advance().unwrap();
            let (vm, movie) = (&screen.vm, &screen.bindings.movie);
            let menu = movie.child(vm, movie.root, "mCreateSkater.mSelection.mMorphmenu");
            let item = movie.child(vm, movie.root, "mCreateSkater.mSelection.mMorphmenu.mItem0");
            if frame < 3 || item.is_none() { println!("  frame {frame}: menu={menu:?} item0={item:?}"); }
            if item.is_none() { break; }
        }
        {
            let (vm, movie) = (&screen.vm, &screen.bindings.movie);
            let menu = movie.child(vm, movie.root, "mCreateSkater.mSelection.mMorphmenu");
            println!("morph menu: {menu:?}");
            for k in 0..3 {
                if let Some(c) = movie.child(vm, movie.root, &format!("mCreateSkater.mSelection.mMorphmenu.mItem{k}")) {
                    println!("  mItem{k}: x={:?} y={:?} visible={:?} shown={} bounds={:?}", vm.get(c, "_x"), vm.get(c, "_y"), vm.get(c, "_visible"), movie.shown(vm, c), movie.local_bounds(vm, c));
                } else {
                    println!("  mItem{k}: missing");
                }
            }
            let thumb = movie.child(vm, movie.root, "mCreateSkater.mSelection.mThumbnails.mThumb0.mIcon");
            println!("thumb icon: {thumb:?}");
        }
        screen.call("UpdateInput", vec![Value::Number(crate::main_menu::APT_DOWN), Value::Number(0.)]).unwrap();
        assert_eq!(screen.bindings.natives.view.index, 1, "down moves the sliders");
        if let Ok(dump) = std::env::var("SKATE_CAS_TREE") {
            std::fs::write(dump, format!("{:#?}", apt_scene::draw(&screen.bindings.movie, &screen.vm, &shapes).unwrap().len())).unwrap();
        }
    }
}
