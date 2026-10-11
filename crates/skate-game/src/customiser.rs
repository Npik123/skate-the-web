//! Compact character menus over resident retail parts.
use crate::customiser_parts::{Library, Parts};
use bevy::{input::mouse::MouseWheel, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Default, Deserialize)]
struct Entry {
    label: String,
    #[serde(default)]
    children: Vec<Entry>,
    #[serde(default)]
    options: Vec<Entry>,
    patch: Option<Value>,
    scalar: Option<String>,
    note: Option<String>,
    gender: Option<String>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    initial: Option<f64>,
    step: Option<f64>,
    /// A command rather than a choice ("import": load a shared skater).
    #[serde(default)]
    action: Option<String>,
}
#[derive(Resource, Default)]
pub(crate) struct Navigation {
    pub pressed: u16,
    preview_turn: f32,
    previous: u16,
    held_for: f32,
    repeat_at: f32,
}
#[derive(Resource)]
pub(crate) struct Customiser {
    pub open: bool,
    pub enabled: bool,
    just_opened: bool,
    preview_yaw: f32,
    index: Entry,
    path: Vec<usize>,
    selected: usize,
    page_size: usize,
    search: String,
    pub draft: Value,
    settings: PathBuf,
    pub status: String,
    pub redraw: bool,
    /// Waiting for the player to choose (or drop) a .cfss file.
    importing: bool,
    /// The choice highlighted on Skate 3's own screen, shown on the skater
    /// before it is chosen.
    pub(crate) hover: Option<CasRow>,
    /// The original screen's highlighted row on this page.
    pub(crate) cas_index: Option<usize>,
    /// Skate 3's own Create-a-Skater screen (cas_screen) shows the menu; the
    /// built-in panel steps aside.
    pub(crate) original_ui: bool,
}
impl Customiser {
    /// The settings folder (character.json's).
    pub(crate) fn settings_dir(&self) -> Option<std::path::PathBuf> {
        self.settings.parent().map(std::path::Path::to_path_buf)
    }
    /// Back out of a page; from the top, save the skater and close.
    pub(crate) fn go_back(&mut self, pause: &mut crate::graphics_menu::Menu) {
        self.search.clear();
        self.hover = None;
        self.cas_index = None;
        if let Some(i) = self.path.pop() {
            self.selected = self.visible().iter().position(|v| *v == i).unwrap_or(0);
        } else {
            let save = (|| -> Result<(), String> {
                skate_data::vfs::create_dir_all(self.settings.parent().unwrap()).map_err(|e| e.to_string())?;
                let tmp = self.settings.with_extension("pending.json");
                skate_data::vfs::write(&tmp, serde_json::to_vec_pretty(&self.draft).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                skate_data::vfs::rename(tmp, &self.settings).map_err(|e| e.to_string())
            })();
            match save {
                Ok(()) => {
                    self.open = false;
                    pause.open = false;
                }
                Err(e) => self.status = format!("Could not save: {e}"),
            }
        }
        self.redraw = true;
    }

    /// Choose entry `i` of this page (`direction` steps a slider or a list
    /// of options): open a sub-page, run a command, or apply the choice.
    pub(crate) fn activate(&mut self, i: usize, direction: i32, parts: &Parts) {
        let Some(entry) = self.page().children.get(i).cloned() else {
            return;
        };
        self.hover = None;
        if !entry.children.is_empty() {
            self.search.clear();
            self.path.push(i);
            self.selected = 0;
            self.status.clear();
        } else if entry.action.as_deref() == Some("import") {
            crate::platform::pick_file(".cfss");
            self.importing = true;
            self.status = if cfg!(target_arch = "wasm32") {
                "Choose a .cfss file. It is read here and never uploaded.".into()
            } else {
                "Drop a .cfss file on the game window.".into()
            };
        } else if let Some(note) = entry.note {
            self.status = note;
        } else {
            match parts.resolve(&apply(&entry, &self.draft, direction, parts)) {
                Ok(p) => {
                    self.draft = p;
                    self.status.clear();
                }
                Err(e) => self.status = e,
            }
        }
        self.redraw = true;
    }
    /// The open page, as entry indices from the top.
    pub(crate) fn path(&self) -> &[usize] {
        &self.path
    }
    /// Open straight at a page (verification captures).
    pub(crate) fn open_at(&mut self, path: &[usize]) {
        self.begin();
        let mut page = &self.index;
        for &i in path {
            let Some(next) = page.children.get(i) else { break };
            page = next;
            self.path.push(i);
        }
    }
    pub(crate) fn begin(&mut self) {
        self.open = true;
        self.enabled = true;
        self.just_opened = true;
        self.preview_yaw = 0.;
        self.path.clear();
        self.search.clear();
        self.selected = 0;
        self.status.clear();
        self.redraw = true;
    }
    fn page(&self) -> &Entry {
        let mut page = &self.index;
        for &i in &self.path {
            page = &page.children[i];
        }
        page
    }
    fn visible(&self) -> Vec<usize> {
        let gender = self.draft["gender"].as_str().unwrap_or("male");
        self.page()
            .children
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.gender
                    .as_deref()
                    .is_none_or(|g| g == gender || g == "unisex")
            })
            .filter(|(_, e)| {
                self.search.is_empty()
                    || e.label.to_lowercase().contains(&self.search.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn preview_camera(&self) -> (f32, f32, f32) {
        let (height, distance, yaw) = self.preview_framing();
        (height, distance, yaw + self.preview_yaw)
    }
    fn preview_framing(&self) -> (f32, f32, f32) {
        let mut page = &self.index;
        let mut labels = vec![];
        for &i in &self.path {
            page = &page.children[i];
            labels.push(page.label.as_str());
        }
        if labels.contains(&"Upper body") || labels.contains(&"Lower body") {
            let upper = labels.contains(&"Upper body");
            let side = self.draft["tattoos"][if upper { "Arm" } else { "Leg" }]["side"]
                .as_u64()
                .unwrap_or(0);
            let yaw = match side {
                0 => -0.65,
                1 => 0.65,
                3 => std::f32::consts::PI,
                _ => 0.,
            };
            return (if upper { 1.15 } else { 0.5 }, 2.2, yaw);
        }
        if labels.iter().any(|s| {
            [
                "Face",
                "Face presets",
                "Hair",
                "Hair colour",
                "Facial hair",
                "Skin tone",
            ]
            .contains(s)
        }) {
            return (1.48, 1.65, 0.);
        }
        if labels.contains(&"Board") {
            return (0.25, 2.0, 0.6);
        }
        (0.95, 3.2, 0.)
    }
    pub fn preview(&self, parts: &Parts) -> Value {
        let mut profile = self.draft.clone();
        if !self.open {
            return profile;
        }
        // Skate 3's screen tries a highlighted part on before it is bought.
        if let Some(row) = self.hover {
            if let Some(entry) = self.page().children.get(row.entry) {
                let patch = match row.option {
                    Some(k) => entry.options.get(k).and_then(|o| o.patch.as_ref()),
                    None if entry.scalar.is_none() && entry.options.is_empty() => entry.patch.as_ref(),
                    None => None,
                };
                if let Some(patch) = patch.filter(|p| p["gender"].is_null()) {
                    let mut tried = self.draft.clone();
                    merge(&mut tried, patch);
                    if let Ok(tried) = parts.resolve(&tried) {
                        return tried;
                    }
                }
            }
        }
        let mut page = &self.index;
        let mut zone = None;
        let mut tattoos = false;
        for &i in &self.path {
            page = &page.children[i];
            if page.label == "Tattoos" {
                tattoos = true;
            }
            if tattoos && page.label == "Upper body" {
                zone = Some("OuterTorso");
            }
            if tattoos && page.label == "Lower body" {
                zone = Some("Pants");
            }
        }
        if let Some(slot) = zone {
            let gender = profile["gender"].as_str().unwrap_or("male");
            let candidate = parts
                .library
                .models
                .iter()
                .filter(|(_, p)| {
                    p.slot == slot
                        && p.flag("Gender") == gender
                        && (if slot == "OuterTorso" {
                            p.flag("TopType").is_empty()
                        } else {
                            p.flag("IsTattooViewingChoice").eq_ignore_ascii_case("true")
                        })
                })
                .min_by_key(|(id, _)| *id);
            if let Some((id, p)) = candidate {
                profile["selections"][slot] = json!({"asset_id":id,"material_id":p.materials[0]});
                if let Ok(resolved) = parts.resolve(&profile) {
                    return resolved;
                }
            }
        }
        profile
    }
    fn preload_entries(&self) -> Vec<&Entry> {
        if !self.open {
            return vec![];
        }
        let visible = self.visible();
        let start = self.selected / self.page_size * self.page_size;
        visible
            .iter()
            .skip(start)
            .take(self.page_size)
            .flat_map(|&i| {
                let e = &self.page().children[i];
                let current = option_index(e, &self.draft).unwrap_or(0);
                e.options
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| *i < 4 || i.abs_diff(current) <= 2)
                    .map(|(_, v)| v)
                    .chain(std::iter::once(e))
            })
            .collect()
    }
    pub fn preload_outfits(&self, parts: &Parts) -> Vec<Value> {
        self.preload_entries()
            .into_iter()
            .filter_map(|e| e.patch.as_ref())
            .filter_map(|patch| {
                let mut profile = self.draft.clone();
                if let Some(gender) = patch["gender"].as_str() {
                    profile = parts.library.defaults.get(gender)?.clone();
                } else {
                    merge(&mut profile, patch);
                    if let Some(hair) = patch["selections"].get("Hair") {
                        profile["hair_choice"] = hair.clone();
                    }
                }
                parts.resolve(&profile).ok()
            })
            .collect()
    }
}
#[derive(Component)]
struct Root;
#[derive(Component)]
struct Row(usize);
#[derive(Component)]
struct Adjust(usize, i32);
const BACK: usize = usize::MAX;
const NEXT: usize = usize::MAX - 1;
const PREV: usize = usize::MAX - 2;
const RESET: usize = usize::MAX - 3;
pub(crate) struct CustomiserPlugin;
impl Plugin for CustomiserPlugin {
    fn build(&self, app: &mut App) {
        crate::customiser_material::register(app);
        app.init_resource::<Navigation>()
            .add_systems(Startup, crate::customiser_parts::setup)
            // After the input update, so a key is seen on the same frame as
            // the menu's own just_pressed check; otherwise one press moved twice.
            .add_systems(PreUpdate, navigation.after(bevy::input::InputSystems).before(crate::graphics_menu::MenuInput))
            .add_systems(PreUpdate, preferences.after(crate::map_transition::MapTransitionSet)
                .before(crate::input::poll_controllers))
            .add_systems(PostStartup, setup)
            .add_systems(
                Update,
                (crate::platform::dropped_files, verify_import, receive_import, interact, rotate_preview, crate::customiser_parts::update, draw)
                    .chain()
                    .after(crate::graphics_menu::interact)
                    .before(crate::app::FrameSet::Animation),
            );
    }
}
pub(crate) fn navigation(mut nav: ResMut<Navigation>, time: Res<Time<Real>>, keys: Res<ButtonInput<KeyCode>>) {
    let pad = (0..4).find_map(|i| crate::input::platform::poll(i).ok());
    // Remap outside the dead zone so a resting stick cannot drift the preview.
    let axis = pad.as_ref().map_or(0., |p| (p.state.right[0] as f32 / 32767.).clamp(-1., 1.));
    nav.preview_turn = axis.signum() * ((axis.abs() - 0.24) / 0.76).max(0.);
    // Skate 3's Create-a-Skater turns the skater with the triggers.
    let triggers = pad.as_ref().map_or(0., |p| (f32::from(p.state.triggers[1]) - f32::from(p.state.triggers[0])) / 255.);
    let keys_turn = f32::from(u8::from(keys.pressed(KeyCode::KeyE))) - f32::from(u8::from(keys.pressed(KeyCode::KeyQ)));
    if nav.preview_turn == 0. {
        nav.preview_turn = if triggers.abs() > 0.1 { triggers } else { keys_turn };
    }
    let mut current = pad.map_or(0, |p| {
            p.state.buttons
                | if p.state.left[1] > 16000 {
                    1
                } else if p.state.left[1] < -16000 {
                    2
                } else {
                    0
                }
                | if p.state.left[0] > 16000 {
                    8
                } else if p.state.left[0] < -16000 {
                    4
                } else {
                    0
                }
        });
    for (key, bit) in [
        (KeyCode::ArrowUp, 1),
        (KeyCode::ArrowDown, 2),
        (KeyCode::ArrowLeft, 4),
        (KeyCode::ArrowRight, 8),
    ] {
        if keys.pressed(key) {
            current |= bit;
        }
    }
    nav.pressed = current & !nav.previous;
    if current & 15 != 0 && current & 15 == nav.previous & 15 {
        nav.held_for += time.delta_secs();
        if nav.held_for >= nav.repeat_at {
            nav.pressed |= current & 15;
            nav.repeat_at += 0.09;
        }
    } else {
        nav.held_for = 0.;
        nav.repeat_at = 0.35;
    }
    nav.previous = current;
}
fn rotate_preview(mut state: ResMut<Customiser>, nav: Res<Navigation>, time: Res<Time<Real>>) {
    if state.open && nav.preview_turn != 0. {
        // Real time keeps inspection responsive while gameplay is paused.
        state.preview_yaw = (state.preview_yaw + nav.preview_turn * 2.0 * time.delta_secs().min(0.1))
            .rem_euclid(std::f32::consts::TAU);
    }
}

fn page(label: impl Into<String>, children: Vec<Entry>) -> Entry {
    Entry {
        label: label.into(),
        children,
        ..default()
    }
}
fn choice(label: impl Into<String>, patch: Value) -> Entry {
    Entry {
        label: label.into(),
        patch: Some(patch),
        ..default()
    }
}
fn tidy(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|x| x.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn menu(lib: &Library, extras: Vec<Entry>) -> Entry {
    let model_page = |title: &str, slot: &str, types: &[&str]| {
        let mut entries = vec![];
        if [
            "Hat",
            "Hair",
            "Sock",
            "Glasses",
            "Jewellery",
            "WristItem",
            "Accessory",
        ]
        .contains(&slot)
        {
            entries.push(choice("None", json!({"selections":{slot:Value::Null}})));
        }
        if title == "T-shirts" {
            for (id, p) in &lib.models {
                if p.slot == "OuterTorso" && p.flag("TopType").is_empty() {
                    entries.push(Entry { gender:Some(p.flag("Gender").into()),
                        ..choice("No top",json!({"selections":{"OuterTorso":{"asset_id":id,"material_id":p.materials[0]}}})) });
                }
            }
        }
        let mut models: Vec<_> = lib
            .models
            .iter()
            .filter(|(_, p)| {
                p.slot == slot
                    && (slot != "Hair" || p.flag("HairModelType") == "full")
                    && (types.is_empty() || types.contains(&p.flag("TopType")))
            })
            .collect();
        models.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(b.0)));
        for (id, p) in models {
            let options: Vec<_> = p
                .materials
                .iter()
                .flat_map(|mid| {
                    let Some(m)=lib.materials.get(mid) else {return vec![];};
                    let mut options=vec![choice(&m.name,json!({"selections":{slot:{"asset_id":id,"material_id":mid}},"colours":{slot:Value::Null}}))];
                    if m.flag("IsColourizable").eq_ignore_ascii_case("true") {
                        options.extend(lib.colours.iter().map(|c| choice(&c.name,
                            json!({"selections":{slot:{"asset_id":id,"material_id":mid}},"colours":{slot:c.rgb}}))));
                    }
                    options
                })
                .collect();
            if options.is_empty() {
                continue;
            }
            let gender = p.flag("Gender");
            entries.push(Entry {
                label: if slot == "Hair" {
                    tidy(p.flag("HairStyle").replace("moosed", "moussed").as_str())
                } else {
                    p.name.clone()
                },
                options,
                gender: if gender.is_empty() {
                    None
                } else {
                    Some(gender.into())
                },
                ..default()
            });
        }
        if ["SkateBoard", "SkateTruck", "SkateWheel"].contains(&slot) {
            let mut groups: std::collections::BTreeMap<String, Vec<Entry>> = default();
            for e in entries {
                for mut option in e.options {
                    let label = option.label.trim_end_matches(" (Worn)").to_owned();
                    option.label = if option.label.ends_with(" (Worn)") {
                        "Worn"
                    } else {
                        "New"
                    }
                    .into();
                    groups.entry(label).or_default().push(option);
                }
            }
            entries = groups
                .into_iter()
                .map(|(label, options)| Entry {
                    label,
                    options,
                    ..default()
                })
                .collect();
        }
        page(title, entries)
    };
    let morph = |title: &str, name: &str| Entry {
        label: title.into(),
        scalar: Some(format!("morphs.{name}")),
        minimum: Some(0.),
        maximum: Some(0.5),
        initial: Some(if name == "fat" || name == "thin" {
            0.
        } else {
            0.25
        }),
        step: Some(0.025),
        ..default()
    };
    let mut body = page(
        "Body",
        vec![
            page(
                "Gender",
                vec![
                    choice("Male", json!({"gender":"male"})),
                    choice("Female", json!({"gender":"female"})),
                ],
            ),
            page(
                "Skin tone",
                ["light", "dark"]
                    .map(|s| choice(tidy(s), json!({"skin":s})))
                    .to_vec(),
            ),
            model_page("Hair", "Hair", &[]),
            page(
                "Build",
                vec![morph("Weight", "fat"), morph("Definition", "thin")],
            ),
            page(
                "Face",
                vec![
                    page(
                        "Brows",
                        vec![
                            morph("Depth", "local_brows_depth"),
                            morph("Height", "local_brows_height"),
                            morph("Angle", "local_brows_rotation"),
                        ],
                    ),
                    page(
                        "Eyes",
                        vec![
                            morph("Height", "local_eye_height"),
                            morph("Angle", "local_eye_rotation"),
                            morph("Width", "local_eye_width"),
                        ],
                    ),
                    page(
                        "Nose",
                        vec![
                            morph("Curve", "local_nose_curve"),
                            morph("Height", "local_nose_height"),
                            morph("Length", "local_nose_length"),
                            morph("Width", "local_nose_width"),
                        ],
                    ),
                    page(
                        "Mouth",
                        vec![
                            morph("Corners", "local_mouth_corner"),
                            morph("Height", "local_mouth_height"),
                            morph("Fullness", "local_mouth_lipsize"),
                            morph("Width", "local_mouth_width"),
                        ],
                    ),
                    page(
                        "Jaw & chin",
                        vec![
                            morph("Definition", "local_jaw_chiseled"),
                            morph("Depth", "local_jaw_depth"),
                            morph("Chin length", "local_chin_length"),
                        ],
                    ),
                ],
            ),
            page("Facial hair", {
                let mut names: Vec<_> = lib
                    .materials
                    .values()
                    .map(|m| m.flag("FacialHairStyle"))
                    .filter(|s| !s.is_empty())
                    .collect();
                names.sort();
                names.dedup();
                names
                    .into_iter()
                    .map(|s| Entry {
                        gender: Some("male".into()),
                        ..choice(
                            match s {
                                "none" => "Clean shaven".into(),
                                "billygoat" => "Long goatee".into(),
                                "soulpatch" => "Soul patch".into(),
                                "magnum" => "Thick moustache".into(),
                                _ => tidy(s),
                            },
                            json!({"beard":s}),
                        )
                    })
                    .collect()
            }),
        ],
    );
    let mut tattoos: Vec<_> = lib.tattoos.iter().collect();
    tattoos.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(b.0)));
    body.children.push(page(
        "Tattoos",
        [("Upper body", "Arm"), ("Lower body", "Leg")]
            .into_iter()
            .map(|(name, slot)| {
                let mut designs =
                    vec![choice("None", json!({"tattoos":{slot:{"id":Value::Null}}}))];
                designs.extend(
                    tattoos
                        .iter()
                        .map(|(id, t)| choice(&t.name, json!({"tattoos":{slot:{"id":id}}}))),
                );
                page(
                    name,
                    vec![
                        page("Design", designs),
                        page("Placement", {
                            let mut positions = vec![
                                choice(
                                    if slot == "Arm" {
                                        "Left arm"
                                    } else {
                                        "Left leg"
                                    },
                                    json!({"tattoos":{slot:{"side":0}}}),
                                ),
                                choice(
                                    if slot == "Arm" {
                                        "Right arm"
                                    } else {
                                        "Right leg"
                                    },
                                    json!({"tattoos":{slot:{"side":1}}}),
                                ),
                            ];
                            if slot == "Arm" {
                                positions.extend([
                                    choice("Chest", json!({"tattoos":{slot:{"side":2}}})),
                                    choice("Back", json!({"tattoos":{slot:{"side":3}}})),
                                ]);
                            }
                            positions
                        }),
                    ],
                )
            })
            .collect(),
    ));
    let clothes = page(
        "Clothes",
        vec![
            model_page("Hats", "Hat", &[]),
            model_page(
                "T-shirts",
                "OuterTorso",
                &["tshirt", "ls_tshirt", "tanktop"],
            ),
            model_page("Shirts", "OuterTorso", &["buttonshirt"]),
            model_page("Hoodies", "OuterTorso", &["hoody"]),
            model_page("Jackets", "OuterTorso", &["jacket"]),
            model_page("Sweaters", "OuterTorso", &["sweater"]),
            model_page("Pants & shorts", "Pants", &[]),
            model_page("Shoes", "Feet", &[]),
            model_page("Socks", "Sock", &[]),
            page(
                "Accessories",
                vec![
                    model_page("Glasses", "Glasses", &[]),
                    model_page("Necklaces", "Jewellery", &[]),
                    model_page("Wristwear", "WristItem", &[]),
                    model_page("Other", "Accessory", &[]),
                ],
            ),
        ],
    );
    let board = page(
        "Board",
        vec![
            model_page("Deck", "SkateBoard", &[]),
            model_page("Trucks", "SkateTruck", &[]),
            model_page("Wheels", "SkateWheel", &[]),
            Entry {
                label: "Truck tightness".into(),
                scalar: Some("truck".into()),
                minimum: Some(0.),
                maximum: Some(1.),
                initial: Some(0.7),
                step: Some(0.1),
                ..default()
            },
            Entry {
                label: "Wheel hardness".into(),
                scalar: Some("wheel".into()),
                minimum: Some(0.),
                maximum: Some(1.),
                initial: Some(0.7),
                step: Some(0.1),
                ..default()
            },
        ],
    );
    let mut root = page(
        "Your skater",
        vec![
            body,
            clothes,
            board,
            page(
                "Style",
                vec![page(
                    "Posture",
                    ["Default", "Stiff", "Slouch", "Buff"]
                        .iter()
                        .enumerate()
                        .map(|(i, s)| choice(*s, json!({"posture":i})))
                        .collect(),
                )],
            ),
        ],
    );
    // A skater shared as a .cfss file (a Skate 3 Create-a-Skater export).
    root.children.push(Entry {
        label: "Import skater (.cfss)".into(),
        action: Some("import".into()),
        ..default()
    });
    for e in extras {
        if e.label == "Style" {
            root.children[3].children.extend(e.children);
        } else if let Some(i) = root.children[0]
            .children
            .iter()
            .position(|p| p.label == e.label)
        {
            root.children[0].children[i] = e;
        } else {
            root.children[0].children.push(e);
        }
    }
    root
}
pub(crate) fn setup(mut commands: Commands, config: Res<crate::config::Config>, parts: Res<Parts>) {
    let settings = config
        .asset_root
        .parent()
        .unwrap_or(&config.asset_root)
        .join("settings/character.json");
    let mut draft = parts
        .library
        .defaults
        .get("male")
        .cloned()
        .unwrap_or(json!({"selections":{},"morphs":{}}));
    let mut enabled = false;
    let status = String::new();
    if let Ok(b) = skate_data::vfs::read(&settings) {
        if let Ok(saved) = serde_json::from_slice::<Value>(&b) {
            if let Ok(p) = parts.resolve(&saved) {
                draft = p;
                enabled = true;
            }
        }
    }
    let extras = skate_data::vfs::read(
        crate::customiser_parts::asset_directory(&config.asset_root).join("extra-menu.json"),
    )
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default();
    commands.insert_resource(Customiser {
        open: false,
        enabled,
        just_opened: false,
        preview_yaw: 0.,
        index: menu(&parts.library, extras),
        path: vec![],
        selected: 0,
        page_size: 7,
        search: String::new(),
        draft,
        settings,
        status,
        redraw: true,
        importing: false,
        hover: None, cas_index: None, original_ui: false,
    });
    commands.spawn((
        Root,
        GlobalZIndex(20),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(24),
            top: percent(4),
            width: percent(38),
            min_width: px(370),
            max_width: px(520),
            height: percent(92),
            padding: UiRect::all(px(22)),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            border_radius: BorderRadius::all(px(18)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.035, 0.048, 0.98)),
    ));
}
/// Skate 3's own title for a customiser page or category, where it has one.
fn cas_title(label: &str) -> Option<&'static str> {
    Some(match label.to_lowercase().as_str() {
        "your skater" => "ID_CAC_EDITSKATER_TITLE",
        "clothes" | "clothing" => "ID_CAC_CLOTHING_TITLE",
        "body" => "ID_CAC_SKIN_TITLE",
        "board" => "ID_CAC_SKATEBOARDS_TITLE",
        "style" => "ID_CAC_SKATE_STYLE_TITLE",
        "hats" => "ID_CAC_HATS_TITLE",
        "hair" => "ID_CAC_HAIR_TITLE",
        "facial hair" => "ID_CAC_FACIAL_HAIR_TITLE",
        "glasses" => "ID_CAC_GLASSES_TITLE",
        "t-shirts" => "ID_CAC_TSHIRTS_TITLE",
        "shirts" => "ID_CAC_BUTTONSHIRTS_TITLE",
        "hoodies" => "ID_CAC_HOODIES_TITLE",
        "jackets" => "ID_CAC_JACKETS_TITLE",
        "sweaters" => "ID_CAC_SWEATERS_TITLE",
        "pants & shorts" => "ID_CAC_LEGS_TITLE",
        "shoes" => "ID_CAC_SHOES_TITLE",
        "socks" => "ID_CAC_SOCKS_TITLE",
        "necklaces" => "ID_CAC_NECKLACES_TITLE",
        "wristwear" => "ID_CAC_WRIST_TITLE",
        "deck" => "ID_CAC_BOARDS_TITLE",
        "trucks" => "ID_CAC_TRUCKS_TITLE",
        "wheels" => "ID_CAC_WHEELS_TITLE",
        "tattoos" => "ID_CAC_TATTOOS_TITLE",
        "upper body" => "ID_CAC_TATTOOSUPPER_TITLE",
        "lower body" => "ID_CAC_TATTOOSLOWER_TITLE",
        "skin tone" => "ID_CAC_SKIN_COLOUR_TITLE",
        "face presets" => "ID_CAC_PRESETS_TITLE",
        "face" => "ID_CAC_FACE_MODIFICATION_TITLE",
        "eyes" => "ID_CAC_EYES_TITLE",
        "brows" | "eyebrows" => "ID_CAC_BROW_TITLE",
        "nose" => "ID_CAC_NOSE_TITLE",
        "mouth" => "ID_CAC_MOUTH_TITLE",
        "jaw" => "ID_CAC_JAW_TITLE",
        "chin" => "ID_CAC_CHIN_TITLE",
        "body shape" => "ID_CAC_BODY_SHAPE_TITLE",
        "accessories" | "other" => "ID_CAC_ACCESSORIES_TITLE",
        "none" => "ID_CAC_REMOVE_PART",
        _ => return None,
    })
}

/// The first RGB triple in a patch (a colour choice's swatch).
fn patch_colour(patch: &Value) -> Option<[u8; 3]> {
    match patch {
        Value::Array(a) if a.len() == 3 && a.iter().all(Value::is_number) => {
            let c = |i: usize| (a[i].as_f64().unwrap_or(0.).clamp(0., 1.) * 255.).round() as u8;
            Some([c(0), c(1), c(2)])
        }
        Value::Object(o) => o.values().find_map(patch_colour),
        _ => None,
    }
}

/// A row of the original screen: a page entry, or one option of it (each
/// colourway of a garment is its own thumbnail, as in Skate 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CasRow {
    pub entry: usize,
    pub option: Option<usize>,
}

impl Customiser {
    /// This page as Skate 3's Create-a-Skater screen shows it, with the row
    /// each item stands for.
    pub(crate) fn cas_page(&self, parts: &Parts) -> (crate::cas_screen::Layout, crate::cas_screen::View, Vec<CasRow>) {
        use crate::cas_screen::{Item, Layout, View};
        let page = self.page();
        let visible = self.visible();
        let entries: Vec<(usize, &Entry)> = visible.iter().map(|&i| (i, &page.children[i])).collect();
        let text = |label: &str| cas_title(label).map(str::to_owned).unwrap_or_else(|| label.to_owned());
        let leaf = |e: &Entry| e.children.is_empty() && e.action.is_none();
        let part_of = |patch: &Value| -> Option<(String, String)> {
            let selections = patch.get("selections")?.as_object()?;
            selections.values().find_map(|v| Some((v["asset_id"].as_str()?.to_owned(), v["material_id"].as_str()?.to_owned())))
        };
        let tattoo_of = |patch: &Value| -> Option<String> {
            patch.get("tattoos")?.as_object()?.values().find_map(|v| v["id"].as_str().map(str::to_owned))
        };
        let removal = |p: &Value| p["selections"].as_object().is_some_and(|o| !o.is_empty() && o.values().all(Value::is_null));
        let wearable = |e: &Entry| {
            leaf(e) && e.scalar.is_none()
                && (e.patch.as_ref().is_some_and(|p| part_of(p).is_some() || removal(p))
                    || (!e.options.is_empty() && e.options.iter().all(|o| o.patch.as_ref().and_then(|p| part_of(p)).is_some())))
        };
        let parts_page = !entries.is_empty()
            && entries.iter().all(|(_, e)| wearable(e))
            && entries.iter().any(|(_, e)| e.patch.as_ref().and_then(|p| part_of(p)).is_some() || !e.options.is_empty());
        let graphics_page = !entries.is_empty()
            && entries.iter().all(|(_, e)| leaf(e) && e.patch.as_ref().and_then(|p| tattoo_of(p)).is_some());
        let sliders = !parts_page && !graphics_page && !entries.is_empty()
            && entries.iter().all(|(_, e)| leaf(e))
            && entries.iter().any(|(_, e)| e.scalar.is_some() || !e.options.is_empty());
        let layout = if parts_page {
            Layout::Thumbs
        } else if graphics_page {
            Layout::Graphics
        } else if sliders {
            Layout::Sliders
        } else {
            Layout::List
        };
        // Parts are photographed in the game (cas_thumbnails): "part:<model>:<material>".
        let part_thumb = |(asset, mid): (String, String)| format!("part:{asset}:{mid}");
        let mut rows = Vec::new();
        let mut items = Vec::new();
        for &(i, e) in &entries {
            match layout {
                Layout::Thumbs if !e.options.is_empty() => {
                    for (k, o) in e.options.iter().enumerate() {
                        let thumbnail = o.patch.as_ref().and_then(|p| part_of(p)).map(part_thumb).unwrap_or_default();
                        let label = if e.options.len() > 1 { format!("{} - {}", e.label, o.label) } else { e.label.clone() };
                        let worn = o.patch.as_ref().is_some_and(|p| matches_patch(&self.draft, p));
                        items.push(Item { text: label.clone(), kind: if worn { "owned" } else { "none" }, description: label, thumbnail, ..Item::default() });
                        rows.push(CasRow { entry: i, option: Some(k) });
                    }
                }
                Layout::Thumbs | Layout::Graphics => {
                    let thumbnail = e
                        .patch
                        .as_ref()
                        .and_then(|p| {
                            part_of(p).map(part_thumb)
                                .or_else(|| tattoo_of(p).and_then(|t| parts.library.tattoos.get(&t).map(|t| t.texture.clone())))
                        })
                        .unwrap_or_default();
                    let worn = e.patch.as_ref().is_some_and(|p| matches_patch(&self.draft, p));
                    items.push(Item { text: text(&e.label), kind: if worn { "owned" } else { "none" }, description: text(&e.label), thumbnail, ..Item::default() });
                    rows.push(CasRow { entry: i, option: None });
                }
                _ => {
                    // Every row explains itself in the panel under the list.
                    let mut item = Item { text: text(&e.label), kind: "option", description: text(&e.label), ..Default::default() };
                    if let Some(key) = &e.scalar {
                        let (min, max) = (e.minimum.unwrap_or(0.), e.maximum.unwrap_or(1.));
                        let value = scalar(&self.draft, key, e.initial.unwrap_or(min));
                        item.kind = "slider";
                        item.integer = (((value - min) / (max - min).max(1e-9)) * 10.).round() as i32;
                        item.string = "#".into();
                    } else if !e.options.is_empty() {
                        let current = option_index(e, &self.draft);
                        let colours: Vec<[u8; 3]> = e.options.iter().filter_map(|o| o.patch.as_ref().and_then(patch_colour)).collect();
                        if colours.len() == e.options.len() && colours.len() <= 16 {
                            item.kind = "color";
                            item.colours = colours;
                            item.integer = current.unwrap_or(0) as i32;
                        } else {
                            item.kind = "selector";
                            item.string = current.and_then(|i| e.options.get(i)).map_or_else(|| "#".into(), |o| text(&o.label));
                        }
                    }
                    items.push(item);
                    rows.push(CasRow { entry: i, option: None });
                }
            }
        }
        // The highlight starts on what the skater wears.
        let worn = rows.iter().position(|r| {
            let e = &page.children[r.entry];
            let patch = match r.option {
                Some(k) => e.options[k].patch.as_ref(),
                None => e.patch.as_ref(),
            };
            matches!(layout, Layout::Thumbs | Layout::Graphics) && patch.is_some_and(|p| matches_patch(&self.draft, p))
        });
        let index = self.cas_index.or(worn).unwrap_or(0).min(rows.len().saturating_sub(1));
        let columns = if layout == Layout::Graphics { 5 } else { 3 };
        let view = View {
            title: text(&page.label),
            filter: String::new(),
            items,
            index,
            window: if matches!(layout, Layout::Thumbs | Layout::Graphics) { index - index % columns } else { 0 },
            help_a: "ID_COMMON_SELECT".into(),
            help_b: if self.path.is_empty() { "ID_CAC_FINISHED_TITLE".into() } else { "ID_COMMON_BACK".into() },
        };
        (layout, view, rows)
    }

    /// The highlight on the original screen moved to `row`: try it on.
    pub(crate) fn cas_hover(&mut self, row: CasRow, index: usize) {
        self.cas_index = Some(index);
        self.selected = self.visible().iter().position(|v| *v == row.entry).unwrap_or(0);
        self.hover = Some(row);
    }

    /// `row` chosen on the original screen (`direction` steps a slider or
    /// selector row).
    pub(crate) fn cas_choose(&mut self, row: CasRow, direction: i32, parts: &Parts) {
        self.selected = self.visible().iter().position(|v| *v == row.entry).unwrap_or(0);
        match row.option {
            Some(k) => {
                let Some(entry) = self.page().children.get(row.entry) else { return };
                let Some(patch) = entry.options.get(k).and_then(|o| o.patch.clone()) else { return };
                let mut profile = self.draft.clone();
                merge(&mut profile, &patch);
                if let Some(hair) = patch["selections"].get("Hair") {
                    profile["hair_choice"] = hair.clone();
                }
                match parts.resolve(&profile) {
                    Ok(p) => {
                        self.draft = p;
                        self.status.clear();
                    }
                    Err(e) => self.status = e,
                }
                self.redraw = true;
            }
            None => {
                let opens = self.page().children.get(row.entry).is_some_and(|e| !e.children.is_empty());
                self.activate(row.entry, direction, parts);
                if opens {
                    self.cas_index = None;
                }
            }
        }
    }
}

/// The profile with entry's choice applied (`direction` steps a slider or an
/// option list; 0 picks the next option).
fn apply(entry: &Entry, draft: &Value, direction: i32, parts: &Parts) -> Value {
    let mut profile = draft.clone();
    if let Some(key) = &entry.scalar {
        let amount = if direction < 0 { -1. } else { 1. };
        let val = (scalar(&profile, key, entry.initial.unwrap_or(0.)) + amount * entry.step.unwrap_or(0.1))
            .clamp(entry.minimum.unwrap_or(0.), entry.maximum.unwrap_or(1.));
        if let Some((p, c)) = key.split_once('.') {
            profile[p][c] = json!(val);
        } else {
            profile[key.as_str()] = json!(val);
        }
        return profile;
    }
    let patch = if entry.options.is_empty() {
        entry.patch.clone()
    } else {
        let current = option_index(entry, &profile);
        let index = match current {
            Some(i) => (i as i32 + if direction == 0 { 1 } else { direction }).rem_euclid(entry.options.len() as i32) as usize,
            None => 0,
        };
        entry.options[index].patch.clone()
    };
    if let Some(patch) = patch {
        if let Some(gender) = patch["gender"].as_str() {
            if let Some(default) = parts.library.defaults.get(gender) {
                profile = default.clone();
            }
        } else {
            merge(&mut profile, &patch);
            if let Some(hair) = patch["selections"].get("Hair") {
                profile["hair_choice"] = hair.clone();
            }
        }
    }
    profile
}
fn merge(target: &mut Value, patch: &Value) {
    if let (Some(a), Some(b)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in b {
            merge(a.entry(k.clone()).or_insert(Value::Null), v);
        }
    } else {
        *target = patch.clone();
    }
}
fn scalar(profile: &Value, key: &str, initial: f64) -> f64 {
    profile
        .pointer(&format!("/{}", key.replace('.', "/")))
        .and_then(Value::as_f64)
        .unwrap_or(initial)
}
fn option_index(entry: &Entry, profile: &Value) -> Option<usize> {
    entry.options.iter().position(|o| {
        o.patch.as_ref().is_some_and(|p| {
            p.as_object().is_some_and(|fields| {
                fields.iter().all(|(key, value)| {
                    if key == "selections" {
                        value.as_object().is_some_and(|selections| {
                            selections.iter().all(|(slot, v)| {
                                (if slot == "Hair" {
                                    profile
                                        .get("hair_choice")
                                        .unwrap_or(&profile["selections"][slot])
                                } else {
                                    &profile["selections"][slot]
                                }) == v
                            })
                        })
                    } else {
                        matches_patch(&profile[key], value)
                    }
                })
            })
        })
    })
}

fn interact(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    nav: Res<Navigation>,
    mut wheel: MessageReader<MouseWheel>,
    mut typing: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut state: ResMut<Customiser>,
    parts: Res<Parts>,
    mut pause: ResMut<crate::graphics_menu::Menu>,
    buttons: Query<(&Interaction, &Row)>,
    adjustments: Query<(&Interaction, &Adjust)>,
) {
    if !state.open || state.original_ui {
        wheel.clear();
        typing.clear();
        return;
    }
    if state.just_opened {
        state.just_opened = false;
        wheel.clear();
        typing.clear();
        return;
    }
    if state.page().children.iter().all(|e| e.children.is_empty()) {
        for event in typing.read() {
            if event.state == bevy::input::ButtonState::Pressed
                && !keys.pressed(KeyCode::ControlLeft)
                && !keys.pressed(KeyCode::ControlRight)
            {
                if let Some(text) = &event.text {
                    for c in text
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                    {
                        if state.search.chars().count() < 32 {
                            state.search.push(c);
                            state.selected = 0;
                            state.redraw = true;
                        }
                    }
                }
            }
        }
    } else {
        typing.clear();
    }
    if !state.search.is_empty() {
        if keys.just_pressed(KeyCode::Backspace) {
            state.search.pop();
            state.selected = 0;
            state.redraw = true;
            return;
        }
        if keys.just_pressed(KeyCode::Escape) || nav.pressed & 0x2000 != 0 {
            state.search.clear();
            state.selected = 0;
            state.redraw = true;
            return;
        }
    }
    let visible = state.visible();
    let count = visible.len();
    let mut action = None;
    let mut back = keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Backspace)
        || nav.pressed & 0x2000 != 0;
    let mut movement = 0;
    if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
        movement -= 1;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
        movement += 1;
    }
    for scroll in wheel.read() {
        if scroll.y > 0. {
            movement -= 1;
        } else if scroll.y < 0. {
            movement += 1;
        }
    }
    if keys.just_pressed(KeyCode::PageDown) {
        movement += state.page_size as i32;
    }
    if keys.just_pressed(KeyCode::PageUp) {
        movement -= state.page_size as i32;
    }
    if count > 0 {
        if movement != 0 {
            state.selected = (state.selected as i32 + movement).rem_euclid(count as i32) as usize;
            state.redraw = true;
        }
        if keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0 {
            action = Some((visible[state.selected], 0));
        }
        if keys.just_pressed(KeyCode::ArrowRight) || nav.pressed & 8 != 0 {
            action = Some((visible[state.selected], 1));
        }
        if keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0 {
            action = Some((visible[state.selected], -1));
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        for (interaction, row) in &buttons {
            if *interaction == Interaction::Pressed {
                match row.0 {
                    BACK => back = true,
                    NEXT => {
                        if count > 0 {
                            state.selected = (state.selected + state.page_size).min(count - 1);
                            state.redraw = true;
                        }
                    }
                    PREV => {
                        state.selected = state.selected.saturating_sub(state.page_size);
                        state.redraw = true;
                    }
                    RESET => {
                        let g = state.draft["gender"].as_str().unwrap_or("male");
                        if let Some(p) = parts.library.defaults.get(g) {
                            state.draft = p.clone();
                            state.redraw = true;
                        }
                    }
                    i => {
                        state.selected = visible.iter().position(|v| *v == i).unwrap_or(0);
                        action = Some((i, 0));
                    }
                }
            }
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        for (interaction, adjust) in &adjustments {
            if *interaction == Interaction::Pressed {
                state.selected = visible.iter().position(|i| *i == adjust.0).unwrap_or(0);
                action = Some((adjust.0, adjust.1));
            }
        }
    }
    if back {
        state.go_back(&mut pause);
        return;
    }
    if let Some((i, direction)) = action {
        state.activate(i, direction, &parts);
    }
}
/// Opt-in screenshot coverage of the importer (--verify with
/// SKATE_VERIFY_IMPORT=<file.cfss>); never changes an ordinary session.
fn verify_import(config: Res<crate::config::Config>, mut state: ResMut<Customiser>, mut done: Local<bool>) {
    if *done || config.verification_capture.is_none() {
        return;
    }
    *done = true;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(bytes) = std::env::var("SKATE_VERIFY_IMPORT").ok().and_then(|path| std::fs::read(path).ok()) {
        crate::platform::offer_file(bytes);
        state.importing = true;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = &mut state;
}

/// A .cfss file the player chose or dropped: its skater becomes the draft,
/// shown on the skater at once; leaving the menu keeps it, as with any change.
/// Its graphics are saved under settings/imported, named by their contents.
fn receive_import(mut state: ResMut<Customiser>, parts: Res<Parts>) {
    let Some(bytes) = crate::platform::take_picked() else { return };
    if !state.importing && !state.open {
        return;
    }
    state.importing = false;
    state.redraw = true;
    let imported = match crate::skater_import::import(&bytes, &parts.library) {
        Ok(imported) => imported,
        Err(e) => {
            state.status = format!("Could not import: {e}");
            return;
        }
    };
    if let Some(settings) = state.settings_dir() {
        let folder = settings.join("imported");
        let _ = skate_data::vfs::create_dir_all(&folder);
        for (hash, png) in &imported.textures {
            if let Err(e) = skate_data::vfs::write(folder.join(format!("{hash}.png")), png) {
                state.status = format!("Could not keep the imported graphics: {e}");
                return;
            }
        }
    }
    match parts.resolve(&imported.profile) {
        Ok(profile) => {
            state.draft = profile;
            state.enabled = true;
            let skipped = if imported.missing > 0 {
                format!(" ({} item{} this copy does not have were left as they were)", imported.missing, if imported.missing == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            state.status = format!("Imported \"{}\"{skipped}. Leave the menu to keep it.", imported.title);
        }
        Err(e) => state.status = format!("Could not import: {e}"),
    }
}

fn preferences(
    state: Res<Customiser>,
    models: Res<crate::custom_models::CustomModels>,
    mut physics: ResMut<crate::physics::GamePhysics>,
    mut skater: ResMut<crate::physics::SkaterRuntime>,
) {
    apply_preferences(&state.draft, &mut physics, &mut skater.animation);
    if let Some(style) = models.native_style() {
        skater.animation.motion.playback_context.pro_skater = skate_core::animation::skeleton_input::name::encode(style.as_bytes());
        skater.animation.motion.animation.posture.set_profile(0);
        physics.set_gesture_preferences(None);
    }
}
pub(crate) fn apply_preferences(
    profile: &Value,
    physics: &mut crate::physics::GamePhysics,
    animation: &mut crate::skater_animation::SkaterAnimation,
) {
    physics.set_equipment_preferences(
        scalar(profile, "truck", 0.7) as f32,
        scalar(profile, "wheel", 0.7) as f32,
    );
    physics.set_gesture_preferences(profile["gestures"].as_object().map(|g| {
        // ResetGestureSet824FA730 marks all 37 entries available and selects
        // the first four in table order for Up,Down,Left,Right.
        std::array::from_fn(|i| {
            g.get(&i.to_string())
                .and_then(Value::as_u64)
                .unwrap_or(i as u64) as u32
        })
    }));
    animation.set_customisation(
        scalar(profile, "stance", 1.) as u32,
        scalar(profile, "style", 0.) as u32,
    );
    animation
        .motion
        .animation
        .posture
        .set_profile(scalar(profile, "posture", 0.) as u32);
}
fn draw(
    mut commands: Commands,
    mut state: ResMut<Customiser>,
    window: Single<&Window>,
    mut root: Single<(Entity, &mut Node), With<Root>>,
) {
    // Only on change: any Node write re-runs UI layout that frame.
    let display = if state.open && !state.original_ui { Display::Flex } else { Display::None };
    if root.1.display != display {
        root.1.display = display;
    }
    if !state.open || state.original_ui {
        return;
    }
    let size = (((window.height() * 0.92 - 255.) / 90.).floor() as usize).clamp(1, 10);
    if size != state.page_size {
        state.page_size = size;
        state.redraw = true;
    }
    if !state.redraw {
        return;
    }
    state.redraw = false;
    commands.entity(root.0).despawn_children();
    let page = state.page();
    let visible = state.visible();
    let start = state.selected / state.page_size * state.page_size;
    let subtitle = if !state.search.is_empty() {
        format!("Search: {}", state.search)
    } else if state.path.is_empty() {
        "Make it yours".to_owned()
    } else {
        let mut p = &state.index;
        let mut crumbs = vec![];
        for &i in &state.path {
            crumbs.push(p.label.clone());
            p = &p.children[i];
        }
        crumbs.join(" / ")
    };
    commands.entity(root.0).with_children(|p| {
        p.spawn((
            Text::new(subtitle),
            TextFont {
                font_size: 13.,
                ..default()
            },
            TextColor(Color::srgb(0.48, 0.62, 0.68)),
        ));
        p.spawn((
            Text::new(&page.label),
            TextFont {
                font_size: 29.,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
        p.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            flex_grow: 1.,
            min_height: px(0),
            ..default()
        })
        .with_children(|list| {
            if visible.is_empty() {
                list.spawn((
                    Text::new("No matches. Esc clears your search."),
                    TextFont {
                        font_size: 16.,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            }
            for (pos, &i) in visible.iter().enumerate().skip(start).take(state.page_size) {
                let e = &page.children[i];
                let active = option_index(e, &state.draft);
                let detail = if let Some(key) = &e.scalar {
                    let v = scalar(&state.draft, key, e.initial.unwrap_or(0.));
                    format!(
                        "{}%   - / +",
                        ((v - e.minimum.unwrap_or(0.))
                            / (e.maximum.unwrap_or(1.) - e.minimum.unwrap_or(0.))
                            * 100.)
                            .round() as i32
                    )
                } else if !e.options.is_empty() {
                    let index = active.unwrap_or(0);
                    format!(
                        "{}  -  {}/{}",
                        e.options[index].label,
                        index + 1,
                        e.options.len()
                    )
                } else if !e.children.is_empty() {
                    String::new()
                } else if e.patch.is_some() {
                    if e.patch
                        .as_ref()
                        .is_some_and(|p| matches_patch(&state.draft, p))
                    {
                        "Selected".into()
                    } else {
                        "Select".into()
                    }
                } else {
                    String::new()
                };
                let adjustable = e.scalar.is_some() || e.options.len() > 1;
                let width = (window.width() * 0.38).clamp(370., 520.)
                    - 44.
                    - 24.
                    - if adjustable { 68. } else { 0. };
                let label = format!(
                    "{}{}",
                    e.label,
                    if !e.children.is_empty() { "  >" } else { "" }
                );
                let label_size = (width / (label.chars().count().max(1) as f32 * 0.64)).clamp(16., 18.);
                let detail_size = (width / (detail.chars().count().max(1) as f32 * 0.64)).clamp(10., 12.);
                list.spawn((
                    Node {
                        width: percent(100),
                        height: px(84),
                        flex_shrink: 0.,
                        align_items: AlignItems::Stretch,
                        border_radius: BorderRadius::all(px(9)),
                        ..default()
                    },
                    BackgroundColor(if pos == state.selected {
                        Color::srgb(0.08, 0.27, 0.29)
                    } else {
                        Color::srgb(0.045, 0.065, 0.085)
                    }),
                ))
                .with_children(|r| {
                    if adjustable {
                        adjust_button(r, i, -1, "-");
                    }
                    r.spawn((
                        Button,
                        Row(i),
                        Node {
                            flex_grow: 1.,
                            min_width: px(0),
                            padding: UiRect::axes(px(12), px(7)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_children(|text| {
                        text.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: label_size,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                        if !detail.is_empty() {
                            text.spawn((
                                Text::new(detail),
                                TextFont {
                                    font_size: detail_size,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.60, 0.76, 0.78)),
                            ));
                        }
                    });
                    if adjustable {
                        adjust_button(r, i, 1, "+");
                    }
                });
            }
        });
        if !state.status.is_empty() {
            p.spawn((
                Text::new(&state.status),
                TextFont {
                    font_size: 13.,
                    ..default()
                },
                TextColor(Color::srgb(1., 0.72, 0.4)),
            ));
        }
        p.spawn(Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|footer| {
            for (id, title) in [
                (
                    BACK,
                    if state.path.is_empty() {
                        "Done"
                    } else {
                        "< Back"
                    },
                ),
                (PREV, "<"),
                (NEXT, ">"),
            ] {
                if id != BACK && visible.len() <= state.page_size {
                    continue;
                }
                footer
                    .spawn((
                        Button,
                        Row(id),
                        Node {
                            padding: UiRect::axes(px(14), px(8)),
                            border_radius: BorderRadius::all(px(8)),
                            ..default()
                        },
                        BackgroundColor(if id == BACK {
                            Color::srgb(0.12, 0.39, 0.38)
                        } else {
                            Color::srgb(0.07, 0.10, 0.13)
                        }),
                    ))
                    .with_children(|r| {
                        r.spawn((
                            Text::new(title),
                            TextFont {
                                font_size: 17.,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                    });
            }
            if visible.len() > state.page_size {
                footer.spawn((
                    Text::new(format!(
                        "{} / {}",
                        start / state.page_size + 1,
                        visible.len().div_ceil(state.page_size)
                    )),
                    TextFont {
                        font_size: 13.,
                        ..default()
                    },
                    TextColor(Color::srgb(0.6, 0.7, 0.75)),
                ));
            }
        });
        p.spawn((
            Text::new(if state.path.is_empty() {
                "Right stick: Rotate   |   Done saves and resumes."
            } else {
                "Up/Down Browse   Left/Right Change   Right stick: Rotate
Type to search"
            }),
            TextFont {
                font_size: 12.,
                ..default()
            },
            TextColor(Color::srgb(0.48, 0.60, 0.66)),
        ));
    });
}

fn adjust_button(parent: &mut ChildSpawnerCommands, index: usize, direction: i32, label: &str) {
    parent
        .spawn((
            Button,
            Adjust(index, direction),
            Node {
                width: px(34),
                flex_shrink: 0.,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(label),
                TextFont {
                    font_size: 22.,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.9, 0.9)),
            ));
        });
}
fn matches_patch(profile: &Value, patch: &Value) -> bool {
    if let Some(object) = patch.as_object() {
        object.iter().all(|(k, v)| matches_patch(&profile[k], v))
    } else {
        profile == patch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn customiser_missing_library_keeps_stock_scene_visible() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_resource::<crate::customiser_parts::Parts>()
            .init_resource::<crate::custom_models::CustomModels>()
            .init_resource::<crate::animation::AnimationStatus>()
            .init_resource::<Assets<crate::customiser_material::SkaterMaterial>>()
            .init_resource::<Assets<Image>>();
        let world = app.world_mut();
        world.insert_resource(Customiser {
            open: true, enabled: true, just_opened: false, preview_yaw: 0.,
            index: Entry::default(), path: vec![], selected: 0, page_size: 6,
            search: String::new(), draft: json!({"selections":{},"morphs":{}}),
            settings: PathBuf::new(), status: String::new(), redraw: false, importing: false, hover: None, cas_index: None, original_ui: false,
        });
        let player = world.spawn(crate::world::PlayerRoot).id();
        let stock = world.spawn((SceneRoot(default()), Visibility::Inherited, ChildOf(player))).id();
        world.run_system_once(crate::customiser_parts::update).unwrap();
        assert_eq!(*world.get::<Visibility>(stock).unwrap(), Visibility::Inherited);
        assert!(world.resource::<Customiser>().status.contains("unavailable"));
        assert!(world.resource::<Parts>().applied.is_null());
    }
    #[test]
    fn customiser_colour_choice_is_distinct_from_the_original() {
        let entry = Entry {
            options: vec![
                choice(
                    "Original",
                    json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}},"colours":{"Feet":null}}),
                ),
                choice(
                    "Red",
                    json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}},"colours":{"Feet":[1.,0.,0.]}}),
                ),
            ],
            ..default()
        };
        let mut p = json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}}});
        assert_eq!(option_index(&entry, &p), Some(0));
        merge(&mut p, entry.options[1].patch.as_ref().unwrap());
        assert_eq!(option_index(&entry, &p), Some(1));
        let material = json!({"name":"original","flags":{},"diffuse":"",
            "alpha":false,"tint":[0.4,0.5,0.6],"metallic":0.,"roughness":0.7});
        let library: Library = serde_json::from_value(json!({
            "models":{
                "shoe":{"slot":"Feet","name":"shoe","flags":{},"materials":["new"],"scene":""},
                "head":{"slot":"Rostral","name":"head","flags":{},"materials":["head"],"scene":""}},
            "materials":{"new":material.clone(),"head":material},"defaults":{},"morphs":[]
        })).unwrap();
        let parts = Parts::for_test(library);
        p["selections"]["Rostral"] = json!({"asset_id":"head","material_id":"head"});
        assert!(parts.resolve(&p).is_ok());
        merge(&mut p, entry.options[0].patch.as_ref().unwrap());
        assert_eq!(option_index(&entry, &p), Some(0));
        assert!(parts.resolve(&p).is_ok(), "original material tint must be selectable");
        let restored: Value = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(parts.resolve(&restored).is_ok(), "saved/online null reset must remain valid");
        for invalid in [json!([2.,0.,0.]),json!([-0.1,0.,0.]),json!([0.,1.]),json!("red")] {
            p["colours"]["Feet"] = invalid;
            assert_eq!(parts.resolve(&p).unwrap_err(), "Invalid clothing colour");
        }
    }
    #[test]
    fn customiser_owned_menu_presets_tattoos_and_search() {
        let Ok(path) = std::env::var("SKATE_CAC_TEST_LIBRARY") else {
            return;
        };
        let lib: Library = serde_json::from_slice(&skate_data::vfs::read(&path).unwrap()).unwrap();
        let extras: Vec<Entry> = serde_json::from_slice(
            &skate_data::vfs::read(std::path::Path::new(&path).with_file_name("extra-menu.json")).unwrap(),
        )
        .unwrap();
        let parts = Parts::for_test(lib);
        for gender in ["male", "female"] {
            let mut state = Customiser {
                open: true,
                enabled: true,
                just_opened: false,
                preview_yaw: 0.,
                index: menu(&parts.library, extras.clone()),
                path: vec![],
                selected: 0,
                page_size: 6,
                search: String::new(),
                draft: parts.library.defaults[gender].clone(),
                settings: PathBuf::new(),
                status: String::new(),
                redraw: true,
                importing: false,
                hover: None, cas_index: None, original_ui: false,
            };
            assert_eq!(state.visible().len(), 4);
            let body = 0;
            let tattoos = state.index.children[body]
                .children
                .iter()
                .position(|e| e.label == "Tattoos")
                .unwrap();
            let original = state.draft.clone();
            for (i, slot) in [(0, "OuterTorso"), (1, "Pants")] {
                state.path = vec![body, tattoos, i];
                let preview = state.preview(&parts);
                assert_ne!(preview["selections"][slot], original["selections"][slot]);
                assert_eq!(state.draft, original);
            }
            state.open = false;
            assert_eq!(state.preview(&parts), original);
            state.open = true;
            let faces = state.index.children[body]
                .children
                .iter()
                .position(|e| e.label == "Face presets")
                .unwrap();
            state.path = vec![body, faces];
            assert_eq!(state.visible().len(), 10);
            for i in state.visible() {
                let mut profile = state.draft.clone();
                merge(
                    &mut profile,
                    state.page().children[i].patch.as_ref().unwrap(),
                );
                assert!(parts.resolve(&profile).is_ok());
            }
            state.search = "Face 10".into();
            assert_eq!(state.visible().len(), 1);
            state.search = "No such item".into();
            assert_eq!(state.visible().len(), 0);
            // Selecting a new face/body profile must survive JSON save/reload.
            let encoded = serde_json::to_vec(&state.draft).unwrap();
            let restored: Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                parts.resolve(&restored).unwrap(),
                parts.resolve(&state.draft).unwrap()
            );
        }
    }
    /// Every page of the real menu as the original screen shows it
    /// (SKATE_LIBRARY=<library-v3.json>, SKATE_EXTRA_MENU=<extra-menu.json>).
    #[test]
    #[ignore = "Requires the private character library"]
    fn every_page_maps_onto_the_original_screen() {
        let library: Library = serde_json::from_slice(&std::fs::read(std::env::var("SKATE_LIBRARY").unwrap()).unwrap()).unwrap();
        let extras: Vec<Entry> = std::env::var("SKATE_EXTRA_MENU").ok()
            .and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut parts = Parts::default();
        let draft = library.defaults["male"].clone();
        let index = menu(&library, extras);
        parts.library = library;
        let mut state = Customiser {
            open: true, enabled: true, just_opened: false, preview_yaw: 0., index, path: vec![], selected: 0, page_size: 7,
            search: String::new(), draft, settings: PathBuf::new(), status: String::new(), redraw: false, importing: false, hover: None, cas_index: None, original_ui: false,
        };
        fn walk(state: &mut Customiser, parts: &Parts, depth: usize, pages: &mut usize) {
            let (layout, view, visible) = state.cas_page(parts);
            *pages += 1;
            let kinds: std::collections::BTreeMap<&str, usize> = view.items.iter().fold(Default::default(), |mut m, i| { *m.entry(i.kind).or_default() += 1; m });
            let thumbs = view.items.iter().filter(|i| !i.thumbnail.is_empty()).count();
            println!("{}{} [{layout:?}] {} items {kinds:?} thumbs={thumbs}", "  ".repeat(depth), view.title, view.items.len());
            for (row, r) in visible.iter().enumerate() {
                let i = r.entry;
                if r.option.is_none() && !state.page().children[i].children.is_empty() {
                    state.path.push(i);
                    state.selected = 0;
                    walk(state, parts, depth + 1, pages);
                    state.path.pop();
                    state.selected = row;
                }
            }
        }
        let mut pages = 0;
        walk(&mut state, &parts, 0, &mut pages);
        assert!(pages > 10);
    }

}
