//! The game modes' HUD, played from Skate 3's own challenge screens: the
//! 3-2-1 countdown (`hudcountdown`), the clock (`hudtimer`), the objective
//! panel (`hudobjective2`), the standings (`leaderboardhud`), Hall of Meat's
//! slam scoring (`homscoring`), S.K.A.T.E.'s trick panel (`hudabd`), the
//! letters (`hudthrowmes`), 1-Up's score to beat (`dpo`),
//! central messages (`hudcentral`), the
//! "Killed It" / "Owned It" stingers and the round results
//! (`endchallenge_online`). `game_modes` describes the round as a `View`; this
//! module answers each screen's `HUDComponents` / `Mission` queries from it and
//! steps the screens through their states as the game's HUD manager did.
use crate::{
    apt_scene,
    apt_screen::{array, group, literal, Natives, Screen},
    apt_stage::{read_screen, ScreenComposite, ScreenMaterial, Stage},
    apt_vm::{Value, Vm},
};
use bevy::{prelude::*, window::PrimaryWindow};

/// The stage's render layer (the pause menu uses 30, the scoring HUD its own).
const LAYER: usize = 31;
const FRAME: f32 = 1.0 / 60.0;

/// One objective row: hudobjective2's states and its title/value text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Objective {
    pub state: Goal,
    pub title: String,
    pub value: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Goal {
    Fail = 0,
    Success = 1,
    Active = 3,
}
/// A standings or results row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub name: String,
    pub score: f64,
    pub local: bool,
    /// Shown in place of the score (Freeskate shows each player's ping).
    pub label: Option<String>,
}
/// Hall of Meat's slam panel: total, then speed, drop, air time, bail time
/// and rotation, each with its score (a metric below the screen's own
/// threshold stays hidden).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Slam {
    pub score: f64,
    pub metrics: [f64; 5],
    pub scores: [f64; 5],
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Results {
    pub title: String,
    /// Subheader icon: 0 none, 1 owned it, 2 killed it.
    pub icon: u32,
    pub result: String,
    pub detail: String,
    pub rows: Vec<Row>,
}
/// Which stinger plays, once per change of `seq`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Stinger {
    pub seq: u32,
    pub owned: bool,
}
/// Everything the round shows. Text starting with `ID_` is looked up in the
/// game's string table; anything else is shown as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct View {
    pub title: String,
    /// 3, 2, 1, then 0 for GO.
    pub countdown: Option<u32>,
    /// Clock text, and whether it is in its final seconds.
    pub clock: Option<(String, bool)>,
    pub objectives: Vec<Objective>,
    pub players: Vec<Row>,
    /// 1-Up: the board shows points as the shipped 1-Up pips.
    pub one_up: bool,
    pub slam: Option<Slam>,
    /// Text and colour (0 white, 1 red, 2 celebratory).
    pub message: Option<(String, u32)>,
    pub stinger: Stinger,
    pub results: Option<Results>,
    /// S.K.A.T.E.: the trick panel.
    pub abd: Option<Abd>,
    /// S.K.A.T.E.'s standings: letters, not scores (1-Up's is `one_up`).
    pub skate: bool,
    /// The rider's stance: the trick gestures mirror for goofy.
    pub regular: bool,
    /// A letter taken: hudthrowmes' state (1-5 S.K.A.T.E., 6-8 1-U-P) and
    /// a count, so the same letter twice plays twice.
    pub letter: Option<(u32, u32)>,
    /// 1-Up's score panel.
    pub dpo: Option<Dpo>,
}

/// The DPO panel: a title, a value over a target, and whether the value
/// is winning (2) or losing (0).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Dpo {
    pub title: String,
    pub value: f64,
    pub target: String,
    pub highlight: u32,
}

/// hudabd's figures: the tricks already done (newest first), and the trick
/// set and the copy attempt, each as (name, stick gesture).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Abd {
    pub list: Vec<String>,
    pub set: Option<(String, String)>,
    pub copy: Option<(String, String)>,
}
impl View {
    pub fn is_empty(&self) -> bool {
        self.countdown.is_none()
            && self.clock.is_none()
            && self.objectives.is_empty()
            && self.players.is_empty()
            && self.slam.is_none()
            && self.message.is_none()
            && self.results.is_none()
            && self.abd.is_none()
            && self.dpo.is_none()
    }
}

fn text(value: &str) -> Value {
    if value.starts_with("ID_") { Value::Text(value.into()) } else { literal(value) }
}

/// The game's HUD natives, answered from the current view.
#[derive(Default)]
struct Hud {
    view: View,
}
impl Hud {
    fn standings(&self) -> &[Row] {
        match &self.view.results {
            Some(results) => &results.rows,
            None => &self.view.players,
        }
    }
}
impl Natives for Hud {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let arg = |i: usize| args.get(i).map_or(0.0, Value::number);
        let index = |i: usize| {
            let v = arg(i);
            if v.is_finite() && v >= 0.0 { v as usize } else { usize::MAX }
        };
        let view = &self.view;
        Ok(Some(match (native, method) {
            // --- countdown and clock ------------------------------------------
            ("HUDComponents", "Countdown_GetCountdownTime") => {
                Value::Text(format!("#{}", view.countdown.unwrap_or(0)))
            }
            ("HUDComponents", "Countdown_IsInStadium") => Value::Bool(false),
            ("HUDComponents", "Timer_GetData") => {
                let (clock, last) = view.clock.clone().unwrap_or_default();
                // The second value is 0 for the red final-seconds colour.
                array(vm, [literal(clock), Value::Number(if last { 0.0 } else { 1.0 })])?
            }
            // --- objective panel ----------------------------------------------
            ("HUDComponents", "Objective_GetMaxNumObjectives") => Value::Number(4.0),
            ("HUDComponents", "Objective_GetNumObjectives") => Value::Number(view.objectives.len().min(4) as f64),
            ("HUDComponents", "Objective_GetActivityTitle") => text(&view.title),
            ("HUDComponents", "Objective_GetActivityNameVisibility") => Value::Bool(true),
            ("HUDComponents", "Objective_GetActivityType") => literal(""),
            ("HUDComponents", "Objective_GetActivityTypeVisibility") => Value::Bool(false),
            ("HUDComponents", "Objective_GetObjectiveState") => {
                Value::Number(view.objectives.get(index(0)).map_or(5.0, |o| o.state as u32 as f64))
            }
            ("HUDComponents", "Objective_GetObjectiveTitle") => {
                view.objectives.get(index(0)).map_or(literal(""), |o| text(&o.title))
            }
            ("HUDComponents", "Objective_GetObjectiveValue") => {
                view.objectives.get(index(0)).map_or(literal(""), |o| text(&o.value))
            }
            // 0: the row shows its value beside the title ("fraction").
            ("HUDComponents", "Objective_GetObjectiveType") => Value::Number(0.0),
            ("Mission", "IsInSkateParkTutorial") => Value::Bool(false),
            // --- standings ----------------------------------------------------
            ("Mission", "GetTitle") | ("Mission", "GetResultsTitle") => match &view.results {
                Some(results) if method == "GetResultsTitle" => text(&results.title),
                _ => text(&view.title),
            },
            ("HUDComponents", "Leaderboard_GetSetupDataBlock") => {
                // players, players on the top team, layout, variant text,
                // teams, online mission, show players.
                // eLeaderBoardHUD_1up / _Skate draw each player's letters.
                let layout = if view.skate { 7.0 } else if view.one_up { 1.0 } else { 3.0 };
                let n = view.players.len().min(8) as f64;
                array(
                    vm,
                    [
                        Value::Number(n),
                        Value::Number(0.0),
                        Value::Number(layout),
                        Value::Text("#Unknown".into()),
                        Value::Bool(false),
                        Value::Bool(false),
                        Value::Bool(true),
                    ],
                )?
            }
            ("HUDComponents", "Leaderboard_GetLeaderboardNameBlock") => {
                let names: Vec<Value> = view.players.iter().take(8).map(|r| literal(&r.name)).collect();
                array(vm, names)?
            }
            ("HUDComponents", "Leaderboard_GetLeaderboardDataBlock") => {
                // Per player: rank, score, score text, highlight, turn, team,
                // display row, id.
                let mut block = Vec::new();
                for (i, row) in view.players.iter().take(8).enumerate() {
                    let score = if view.one_up || view.skate { row.score } else { row.score.round() };
                    block.extend([
                        Value::Number((i + 1) as f64),
                        Value::Number(score),
                        row.label.as_ref().map_or_else(|| Value::Text(group(score as i64)), |l| literal(l)),
                        Value::Bool(row.local),
                        Value::Bool(false),
                        Value::Number(0.0),
                        Value::Number(i as f64),
                        Value::Number(i as f64),
                    ]);
                }
                array(vm, block)?
            }
            ("FELeaderboard", "GetVoipStatus") => array(vm, (0..8).map(|_| Value::Number(0.0)))?,
            ("FELeaderboard", "EndChallengeOnline_GetLeaderboardNavData") => {
                let n = self.standings().len() as f64;
                array(vm, [Value::Number(n), Value::Number(0.0), Value::Number(0.0)])?
            }
            // --- Hall of Meat -------------------------------------------------
            ("HUDComponents", "HOM_GetScore") => Value::Number(view.slam.as_ref().map_or(0.0, |s| s.score.round())),
            ("HUDComponents", "HOM_GetBonusData") => {
                // Metric values, then which are shown.
                let slam = view.slam.clone().unwrap_or_default();
                let values = slam.metrics.map(|m| Value::Number((m * 10.0).round() / 10.0));
                let shown = slam.scores.map(|s| Value::Bool(s > 0.0));
                array(vm, values.into_iter().chain(shown))?
            }
            ("HUDComponents", "HOM_GetBonusScores") => {
                let slam = view.slam.clone().unwrap_or_default();
                array(vm, slam.scores.map(|s| Value::Number(s.round())))?
            }
            // Collision bonuses (cars, props, peds...): none in this world yet.
            ("HUDComponents", "HOM_GetBonusCollisionData") => array(
                vm,
                [Value::Number(0.0), Value::Number(0.0), Value::Bool(false), Value::Number(0.0), Value::Number(0.0)],
            )?,
            ("HUDComponents", "HOM_GetBonusCollisionScore") => {
                let empty: Vec<Value> = (0..5).map(|_| Value::Undefined).collect();
                array(vm, empty)?
            }
            // --- S.K.A.T.E. ---------------------------------------------------
            ("HUDComponents", "ABD_GetNumTricksInList") => {
                Value::Number(view.abd.as_ref().map_or(0, |a| a.list.len().min(5)) as f64)
            }
            ("HUDComponents", "ABD_GetTrickHalNames") => {
                // Names, then each one's spin (0: the name says it all).
                let list = view.abd.as_ref().map_or(&[][..], |a| &a.list[..a.list.len().min(5)]);
                let names = list.iter().map(|n| literal(n));
                array(vm, names.chain(list.iter().map(|_| Value::Number(0.0))).collect::<Vec<_>>())?
            }
            ("HUDComponents", "ABD_GetTrickSetData") | ("HUDComponents", "ABD_GetTricCopyData") => {
                // Name, gesture ("none": nothing set or tried), spin.
                let abd = view.abd.clone().unwrap_or_default();
                let trick = if method == "ABD_GetTrickSetData" { abd.set } else { abd.copy };
                let (name, gesture) = trick.unwrap_or_else(|| (String::new(), "none".into()));
                // The gesture is a clip id (g_<gesture>), not display text.
                array(vm, [literal(&name), Value::Text(gesture), Value::Number(0.0)])?
            }
            ("HUDComponents", "ABD_GetMoveToUpperPosition") => Value::Bool(false),
            // --- 1-Up's score panel -------------------------------------------
            ("HUDComponents", "DPO_GetParams") => {
                // Value, target, title, (unused), highlight.
                let dpo = view.dpo.clone().unwrap_or_default();
                array(vm, [Value::Number(dpo.value.round()), literal(&dpo.target), text(&dpo.title), Value::Number(0.0), Value::Number(dpo.highlight as f64)])?
            }
            ("HUDComponents", "DPO_IsUsingTwoDPOs") => Value::Bool(false),
            ("HUDComponents", "PlayerStance_IsRegular") => Value::Bool(view.regular),
            ("Tricks", "IsStanceTrickGesturesFlipped") => Value::Bool(false),
            // --- central messages ---------------------------------------------
            ("HUDComponents", "Central_GetMessageData") => {
                let (message, colour) = view.message.clone().unwrap_or_default();
                array(vm, [text(&message), Value::Number(colour as f64)])?
            }
            ("HUDComponents", "Central_GetModifiedMessageData") | ("HUDComponents", "Central_GetTeamMessageData") => {
                array(vm, [literal(""), Value::Number(0.0)])?
            }
            // kAUDIOHOOK_NONE: game_modes plays the round's own stingers.
            ("HUDComponents", "Central_GetAudioHook") => Value::Number(-1.0),
            // The central message's voice-over cue.
            ("Audio", "PlayGenCentralMsge") => Value::Undefined,
            ("Mission", "GetTeamInfo") => array(vm, [literal(""), Value::Number(0.0)])?,
            // --- stingers -----------------------------------------------------
            ("Mission", "SetKilledItShakeActivation") | ("Render", "SetEffect") => Value::Undefined,
            // --- results ------------------------------------------------------
            ("HUDComponents", "Results_GetNumPlayers") => Value::Number(self.standings().len().min(8) as f64),
            // eResultsLBType_PlaceNameInt: place, name, score.
            ("HUDComponents", "Results_GetLayoutType") => Value::Number(0.0),
            ("HUDComponents", "Results_GetLeaderboardDataBlock") => {
                // place, int, float, string, time, choice, row, name.
                let mut block = Vec::new();
                for (i, row) in self.standings().iter().take(8).enumerate() {
                    block.extend([
                        Value::Number((i + 1) as f64),
                        Value::Text(group(row.score.round() as i64)),
                        Value::Number(row.score),
                        literal(""),
                        literal(""),
                        Value::Bool(false),
                        Value::Number(i as f64),
                        literal(&row.name),
                    ]);
                }
                array(vm, block)?
            }
            ("HUDComponents", "Results_GetLeaderboardSubHeaderDataBlock") => {
                let results = view.results.clone();
                match results {
                    Some(r) => array(
                        vm,
                        [Value::Bool(true), Value::Number(r.icon as f64), text(&r.result), text(&r.detail)],
                    )?,
                    None => array(vm, [Value::Bool(false)])?,
                }
            }
            ("HUDComponents", "Results_GetLeaderboardColTitle") => match index(0) {
                0 => literal("#"),
                1 => Value::Text("ID_COMMON_NAME".into()),
                _ => Value::Text("ID_COMMON_SCORE".into()),
            },
            ("HUDComponents", "Results_GetThrowdownType") => Value::Number(0.0),
            ("HUDComponents", "NumberToDistance") => literal(format!("{:.1}m", arg(0))),
            ("Mission", "GetCurrentPlayerPosition") => {
                Value::Number(self.standings().iter().position(|r| r.local).map_or(1, |i| i + 1) as f64)
            }
            ("Mission", "GetResultsStatusText") => literal(""),
            ("Mission", "GetResultsStatusTimer") => literal(""),
            // A platform that is neither console: only the quit prompt shows.
            ("object", "GetPlatform") => Value::Number(0.0),
            _ => return Ok(None),
        }))
    }
}

/// The screens, in drawing order (later ones draw on top).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Objective,
    Leaderboard,
    Timer,
    Slam,
    Abd,
    Dpo,
    Letters,
    Central,
    Countdown,
    KilledIt,
    OwnedIt,
    Results,
}
const SCREENS: [(Kind, &str); 12] = [
    (Kind::Objective, "hudobjective2.json"),
    (Kind::Leaderboard, "leaderboardhud.json"),
    (Kind::Timer, "hudtimer.json"),
    (Kind::Slam, "homscoring.json"),
    (Kind::Abd, "hudabd.json"),
    (Kind::Dpo, "dpo.json"),
    (Kind::Letters, "hudthrowmes.json"),
    (Kind::Central, "hudcentral.json"),
    (Kind::Countdown, "hudcountdown.json"),
    (Kind::KilledIt, "killedit.json"),
    (Kind::OwnedIt, "ownthespot.json"),
    (Kind::Results, "endchallenge_online.json"),
];

struct Layer {
    kind: Kind,
    json: serde_json::Value,
    shapes: apt_scene::Shapes,
    screen: Option<Screen<Hud>>,
    /// Frames this screen keeps running after it was told to hide (outros).
    running: u32,
}
impl Layer {
    fn load(&mut self, view: &View) -> Result<(), String> {
        let globals: Vec<(&str, Value)> = Vec::new();
        let red = Value::Number(16736595.0);
        let white = Value::Number(15790320.0);
        let manager: &[(&str, Value)] = &[("mColour_Red", red), ("mColour_OffWhite", white)];
        let objects: [(&str, &[(&str, Value)]); 6] = [
            ("HUDComponents", &[]),
            ("Tricks", &[]),
            ("Mission", &[]),
            ("FELeaderboard", &[]),
            ("Render", &[]),
            ("ScreenManager", manager),
        ];
        let screen = Screen::load(&self.json, Hud { view: view.clone() }, &objects, &globals)?;
        self.screen = Some(screen);
        Ok(())
    }
    fn screen(&mut self) -> &mut Screen<Hud> {
        self.screen.as_mut().expect("loaded")
    }
    fn state(&mut self, state: f64) -> Result<(), String> {
        self.running = 120;
        self.screen().set_state(state)
    }
}

#[derive(Resource)]
struct ModeHud {
    layers: Vec<Layer>,
    stage: Stage,
    shown: View,
    clock: f32,
    stinger_left: f32,
    failed: bool,
}

pub(crate) struct ModeHudPlugin;
impl Plugin for ModeHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup.after(crate::graphics_menu::PresentationSetup))
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
    let Ok(output) = cameras.single() else {
        return;
    };
    let root = config.asset_root.join("private/frontend");
    let mut stage = Stage::spawn(&mut commands, &mut images, &mut composites, &window, output, LAYER, 5, -3);
    let loaded = (|| -> Result<Vec<Layer>, String> {
        let mut layers = Vec::new();
        for (kind, file) in SCREENS {
            let (json, shapes) = read_screen(&root, file)?;
            let mut layer = Layer { kind, json, shapes, screen: None, running: 0 };
            layer.load(&View::default())?;
            let movie = &layer.screen.as_ref().expect("loaded").bindings.movie;
            stage.load_textures(&mut images, &root, &layer.shapes, movie)?;
            layers.push(layer);
        }
        Ok(layers)
    })();
    match loaded {
        Ok(layers) => {
            commands.insert_resource(ModeHud {
                layers,
                stage,
                shown: View::default(),
                clock: 0.0,
                stinger_left: 0.0,
                failed: false,
            });
            info!("Original challenge HUD loaded from {}", root.display());
        }
        Err(error) => warn!("Original challenge HUD unavailable ({error})"),
    }
}

/// Step every screen to match the view, as the game's HUD manager did.
fn transition(hud: &mut ModeHud, view: &View, dt: f32) -> Result<(), String> {
    let old = hud.shown.clone();
    for layer in &mut hud.layers {
        if let Some(screen) = layer.screen.as_mut() {
            screen.bindings.natives.view = view.clone();
        }
    }
    let layer = |hud: &mut ModeHud, kind: Kind| hud.layers.iter_mut().position(|l| l.kind == kind).unwrap();
    // Countdown: ShowCountdown on each new number, HideCountdown after GO.
    let i = layer(hud, Kind::Countdown);
    if view.countdown != old.countdown {
        match view.countdown {
            Some(_) => hud.layers[i].state(2.)?,
            None => hud.layers[i].state(1.)?,
        }
    }
    // Clock: intro, then a refresh per change; the final seconds flash.
    let i = layer(hud, Kind::Timer);
    match (&old.clock, &view.clock) {
        (None, Some(_)) => {
            hud.layers[i].state(1.)?;
            hud.layers[i].state(3.)?;
        }
        (Some(_), None) => hud.layers[i].state(2.)?,
        (Some((a, was_last)), Some((b, last))) => {
            if a != b {
                hud.layers[i].state(3.)?;
            }
            if *last && !was_last {
                hud.layers[i].state(4.)?;
            }
        }
        (None, None) => {}
    }
    // Objectives: intro and a full setup, then forced refreshes.
    let i = layer(hud, Kind::Objective);
    if view.objectives != old.objectives || view.title != old.title {
        if view.objectives.is_empty() {
            hud.layers[i].state(2.)?;
        } else {
            if old.objectives.is_empty() {
                hud.layers[i].state(1.)?;
            }
            hud.layers[i].state(100.)?;
        }
    }
    // Standings: set up when the table's shape changes, refresh on scores.
    let i = layer(hud, Kind::Leaderboard);
    let names = |v: &View| v.players.iter().map(|r| (r.name.clone(), r.local)).collect::<Vec<_>>();
    if view.players.is_empty() {
        if !old.players.is_empty() {
            hud.layers[i].state(2.)?;
        }
    } else if old.players.is_empty() || names(view) != names(&old) || view.one_up != old.one_up || view.skate != old.skate || view.title != old.title {
        hud.layers[i].state(3.)?;
        hud.layers[i].state(1.)?;
    } else if view.players != old.players {
        hud.layers[i].state(5.)?;
    }
    // Hall of Meat: the slam panel follows the bail.
    let i = layer(hud, Kind::Slam);
    match (&old.slam, &view.slam) {
        (None, Some(_)) => {
            hud.layers[i].state(1.)?;
            hud.layers[i].state(3.)?;
        }
        (Some(a), Some(b)) if a != b => hud.layers[i].state(3.)?,
        (Some(_), None) => hud.layers[i].state(2.)?,
        _ => {}
    }
    // S.K.A.T.E.: show and fill the trick panel, refresh it on changes.
    let i = layer(hud, Kind::Abd);
    match (&old.abd, &view.abd) {
        (None, Some(_)) => hud.layers[i].state(1.)?,
        (Some(a), Some(b)) if a != b => hud.layers[i].state(3.)?,
        (Some(_), None) => hud.layers[i].state(2.)?,
        _ => {}
    }
    // The score panel: set up and shown, refreshed, hidden. It sits above
    // the standings by their height (the game's _global LeaderboardHUD_Height,
    // shared here between the two movies).
    let lb = layer(hud, Kind::Leaderboard);
    let height = hud.layers[lb].screen.as_ref().map(|s| s.vm.get(s.vm.global, "LeaderboardHUD_Height"));
    let i = layer(hud, Kind::Dpo);
    if let (Some(height), Some(screen)) = (height, hud.layers[i].screen.as_mut()) {
        let global = screen.vm.global;
        screen.vm.set(global, "LeaderboardHUD_Height", height)?;
    }
    // SetupDPO (1) places and shows it; Update (3) fills its text.
    match (&old.dpo, &view.dpo) {
        (None, Some(_)) => {
            hud.layers[i].state(1.)?;
            hud.layers[i].state(3.)?;
        }
        (Some(a), Some(b)) if a != b => hud.layers[i].state(3.)?,
        (Some(_), None) => hud.layers[i].state(2.)?,
        _ => {}
    }
    // A letter: the screen plays it and its outro by itself.
    if view.letter != old.letter {
        if let Some((state, _)) = view.letter {
            let i = layer(hud, Kind::Letters);
            hud.layers[i].state(state as f64)?;
            hud.layers[i].running = 240;
        }
    }
    // Central message: ShowMessage / HideMessage.
    let i = layer(hud, Kind::Central);
    if view.message != old.message {
        match view.message {
            Some(_) => hud.layers[i].state(4.)?,
            None => hud.layers[i].state(5.)?,
        }
    }
    // Stingers play once, then hide.
    if view.stinger.seq != old.stinger.seq && view.stinger.seq != 0 {
        let i = layer(hud, if view.stinger.owned { Kind::OwnedIt } else { Kind::KilledIt });
        hud.layers[i].state(1.)?;
        hud.stinger_left = 3.5;
    }
    if hud.stinger_left > 0.0 {
        hud.stinger_left -= dt;
        if hud.stinger_left <= 0.0 {
            for kind in [Kind::KilledIt, Kind::OwnedIt] {
                let i = layer(hud, kind);
                hud.layers[i].state(0.)?;
            }
        }
    }
    // Results: the screen builds its table when constructed, so it is
    // rebuilt for each round's results.
    let i = layer(hud, Kind::Results);
    if view.results != old.results {
        let layer = &mut hud.layers[i];
        layer.load(view)?;
        if view.results.is_some() {
            // Fill the table again once its controls have run a frame.
            layer.screen().advance()?;
            layer.state(5.)?;
        }
        layer.running = if view.results.is_some() { u32::MAX } else { 0 };
    }
    hud.shown = view.clone();
    Ok(())
}

fn present(
    mut commands: Commands,
    hud: Option<ResMut<ModeHud>>,
    modes: Res<crate::game_modes::GameModes>,
    time: Res<Time<Real>>,
    mut sounds: ResMut<crate::game_audio::SoundQueue>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
    (net, mut original): (Option<Res<crate::multiplayer::Multiplayer>>, Option<ResMut<crate::multiplayer::hud::OriginalRoster>>),
) {
    let Some(mut hud) = hud else {
        return;
    };
    if hud.failed {
        return;
    }
    let mut view = modes.view();
    // Freeskate with others: the session's players in the same standings
    // screen the challenges use, each with their ping where a score would be.
    let freeskate = view.is_empty() && modes.round_kind().is_none();
    if let Some(net) = net.as_ref().filter(|n| freeskate && n.active()) {
        let roster = crate::multiplayer::hud::roster(net);
        if roster.len() > 1 {
            view.title = "ID_MISSION_TEMPLATE_ONLINE_FREESKATE_TITLE".into();
            view.players = roster.into_iter().map(|(name, ping, local)| Row { name, score: 0., local, label: Some(ping) }).collect();
        }
    }
    if let Some(original) = original.as_mut() {
        if !original.0 {
            original.0 = true;
        }
    }
    let dt = time.delta_secs();
    let result = (|| -> Result<Vec<apt_scene::Draw>, String> {
        if view != hud.shown || hud.stinger_left > 0.0 {
            transition(&mut hud, &view, dt)?;
        }
        // Keep running while anything shows, and for outros after.
        let busy = !view.is_empty() || hud.stinger_left > 0.0;
        hud.clock = (hud.clock + dt).min(FRAME * 4.0);
        let mut frames = 0;
        while hud.clock >= FRAME {
            hud.clock -= FRAME;
            frames += 1;
        }
        let mut draws = Vec::new();
        for layer in &mut hud.layers {
            // The results screen shows itself on load; it exists only at the end.
            if layer.kind == Kind::Results && view.results.is_none() {
                continue;
            }
            let active = busy || layer.running > 0;
            if !active {
                continue;
            }
            let screen = layer.screen();
            for _ in 0..frames {
                screen.advance()?;
            }
            for name in screen.take_sounds() {
                sounds.push(name);
            }
            layer.running = layer.running.saturating_sub(frames);
            let screen = layer.screen.as_ref().expect("loaded");
            draws.extend(apt_scene::draw(&screen.bindings.movie, &screen.vm, &layer.shapes)?);
        }
        Ok(draws)
    })();
    let hud = &mut *hud;
    match result {
        Ok(draws) => hud.stage.render(&mut commands, &mut meshes, &mut materials, &draws),
        Err(error) => {
            error!("Original challenge HUD stopped: {error}");
            hud.failed = true;
            hud.stage.render(&mut commands, &mut meshes, &mut materials, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays every original challenge screen through a round when the
    /// private front-end export is installed (SKATE_FRONTEND_DIR).
    #[test]
    #[ignore = "Requires private installed front-end assets"]
    fn original_challenge_screens_run() {
        let root = std::path::PathBuf::from(std::env::var("SKATE_FRONTEND_DIR").expect("SKATE_FRONTEND_DIR"));
        let row = |name: &str, score: f64, local: bool| Row { name: name.into(), score, local, label: None };
        let objective = |title: &str, value: &str| Objective { state: Goal::Active, title: title.into(), value: value.into() };
        let playing = View {
            title: "Spot Battle".into(),
            clock: Some(("2:41".into(), false)),
            objectives: vec![objective("ID_COMMON_SCORE", "1,234")],
            players: vec![row("You", 1234., true), row("Skater 2", 800., false)],
            message: Some(("ID_CHALLENGE_CENTRAL_MESSAGE_ROUNDOVER".into(), 0)),
            slam: Some(Slam { score: 2400., metrics: [41., 3.2, 0.8, 2.5, 0.], scores: [164., 192., 120., 100., 0.] }),
            ..default()
        };
        let views = [
            View { title: "Spot Battle".into(), countdown: Some(3), ..default() },
            View { title: "Spot Battle".into(), countdown: Some(1), ..default() },
            View { countdown: Some(0), ..playing.clone() },
            playing.clone(),
            View { clock: Some(("0:09".into(), true)), stinger: Stinger { seq: 1, owned: false }, ..playing.clone() },
            View {
                results: Some(Results {
                    title: "ID_CHALLENGE_COMMON_ROUND_RESULTS".into(),
                    icon: 2,
                    result: "ID_CHALLENGE_COMMON_KILLED_IT".into(),
                    detail: "You 1234".into(),
                    rows: playing.players.clone(),
                }),
                ..default()
            },
            // 1-Up: the score panel and a letter.
            View {
                dpo: Some(Dpo { title: "#Skater 2".into(), value: 300., target: "1,200".into(), highlight: 0 }),
                letter: Some((7, 1)),
                one_up: true,
                ..playing.clone()
            },
            // S.K.A.T.E.: a trick to set, then one set and copied.
            View { abd: Some(Abd::default()), letter: Some((3, 2)), skate: true, ..playing.clone() },
            View {
                abd: Some(Abd {
                    list: vec!["Kickflip".into()],
                    set: Some(("Kickflip".into(), "ollie_kickflip".into())),
                    copy: None,
                }),
                ..playing.clone()
            },
            View {
                abd: Some(Abd {
                    list: vec!["Kickflip".into()],
                    set: Some(("Kickflip".into(), "ollie_kickflip".into())),
                    copy: Some(("Kickflip".into(), "ollie_kickflip".into())),
                }),
                ..playing.clone()
            },
        ];
        for (kind, file) in SCREENS {
            let (json, shapes) = read_screen(&root, file).unwrap();
            let mut layer = Layer { kind, json, shapes, screen: None, running: 0 };
            layer.load(&View::default()).unwrap_or_else(|e| panic!("{file}: {e}"));
            let mut hud_layers = vec![layer];
            let mut drawn = 0;
            let mut shown = View::default();
            for view in &views {
                let mut hud = Probe { layers: std::mem::take(&mut hud_layers), shown };
                hud.step(view).unwrap_or_else(|e| panic!("{file} {view:?}: {e}"));
                for _ in 0..30 {
                    hud.layers[0].screen().advance().unwrap_or_else(|e| panic!("{file}: {e}"));
                }
                let screen = hud.layers[0].screen.as_ref().unwrap();
                let draws = apt_scene::draw(&screen.bindings.movie, &screen.vm, &hud.layers[0].shapes).unwrap();
                if std::env::var("SKATE_HUD_DRAWS").is_ok_and(|f| file.starts_with(&f)) {
                    for d in &draws {
                        let xs: Vec<f32> = d.vertices.iter().map(|v| v.position[0]).collect();
                        println!("DRAW {:?} n={} x0={:?} mul={:?}", d.texture, d.vertices.len(), xs.first(), d.multiply);
                    }
                    println!("----");
                }
                drawn = drawn.max(draws.len());
                shown = hud.shown;
                hud_layers = hud.layers;
            }
            println!("{file}: up to {drawn} draws");
            if std::env::var("SKATE_HUD_TREE").is_ok_and(|f| file.starts_with(&f)) {
                let screen = hud_layers[0].screen.as_ref().unwrap();
                fn tree(screen: &Screen<Hud>, id: usize, depth: usize) {
                    let vm = &screen.vm;
                    let movie = &screen.bindings.movie;
                    let Some(instance) = movie.instances.get(&id) else { return };
                    println!(
                        "{}{} #{} vis={} x={} y={} text={:?}",
                        "  ".repeat(depth),
                        vm.get(id, "_name").text(),
                        instance.character,
                        vm.get(id, "_visible").truth(),
                        vm.get(id, "_x").text(),
                        vm.get(id, "_y").text(),
                        vm.get(id, "text").text()
                    );
                    for child in instance.ordered_children() {
                        tree(screen, child, depth + 1);
                    }
                }
                tree(screen, screen.bindings.movie.root, 0);
            }
            assert!(drawn > 0, "{file} never drew");
        }
    }

    /// `transition` for a single screen, without the Bevy stage.
    struct Probe {
        layers: Vec<Layer>,
        shown: View,
    }
    impl Probe {
        fn step(&mut self, view: &View) -> Result<(), String> {
            let old = std::mem::replace(&mut self.shown, view.clone());
            let layer = &mut self.layers[0];
            layer.screen().bindings.natives.view = view.clone();
            match layer.kind {
                Kind::Countdown if view.countdown != old.countdown => layer.state(if view.countdown.is_some() { 2. } else { 1. }),
                Kind::Timer if view.clock.is_some() => {
                    layer.state(1.)?;
                    layer.state(3.)
                }
                Kind::Objective if !view.objectives.is_empty() => {
                    layer.state(1.)?;
                    layer.state(100.)
                }
                Kind::Leaderboard if !view.players.is_empty() => {
                    layer.state(3.)?;
                    layer.state(1.)?;
                    layer.state(5.)
                }
                Kind::Slam if view.slam.is_some() => {
                    layer.state(1.)?;
                    layer.state(3.)
                }
                Kind::Dpo if view.dpo.is_some() => {
                    layer.state(1.)?;
                    layer.state(3.)
                }
                Kind::Letters => match view.letter {
                    Some((state, _)) if view.letter != old.letter => layer.state(state as f64),
                    _ => Ok(()),
                },
                Kind::Abd => match (&old.abd, &view.abd) {
                    (None, Some(_)) => layer.state(1.),
                    (Some(a), Some(b)) if a != b => layer.state(3.),
                    _ => Ok(()),
                },
                Kind::Central if view.message.is_some() => layer.state(4.),
                Kind::KilledIt | Kind::OwnedIt if view.stinger.seq != 0 => layer.state(1.),
                Kind::Results if view.results.is_some() => {
                    layer.load(view)?;
                    layer.screen().advance()?;
                    layer.state(5.)
                }
                _ => Ok(()),
            }
        }
    }
}
