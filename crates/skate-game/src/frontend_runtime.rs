//! Skate 3's pause menu (the CrossBar, `core_menu`) run from the original
//! movie. The movie draws and animates itself; this module plays the part of
//! the game's front-end code, which answered the movie's `coremenu.*` queries,
//! owned navigation, and told the screen what changed through `SetState`.
use crate::{
    apt_movie::{Movie, MovieHost},
    apt_vm::{Host, ObjectKind, Value, Vm},
};

/// Screen modes the movie's `SetState` dispatches on. The shipped values
/// were native enums; only their distinctness matters to the script.
pub mod mode {
    pub const REFRESH_MENU: f64 = 1.0;
    pub const REFRESH_OPTION: f64 = 2.0;
    pub const INITIALISE_MENU: f64 = 3.0;
    pub const INITIALISE_SUB_OPTION: f64 = 4.0;
    pub const INIT_SUB_OPTION_ONLINE_TEAM: f64 = 5.0;
    pub const REFRESH_DLC_BANNER: f64 = 6.0;
}
/// Pad codes the movie compares against (`_global.Apt*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Next,
    Previous,
}
impl Key {
    fn code(self) -> f64 {
        match self {
            Key::Up => 1.0,
            Key::Down => 2.0,
            Key::Left => 3.0,
            Key::Right => 4.0,
            Key::Next => 5.0,
            Key::Previous => 6.0,
        }
    }
}
/// A mouse or touch point on the 1280x720 stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    Hover([f32; 2]),
    Press([f32; 2]),
}
enum Target {
    Category(usize),
    Row(usize),
    Value(usize),
}
/// Movie menu levels (core_menu.OPTION_MENULEVEL / SUBOPTION_MENULEVEL).
const OPTION_LEVEL: f64 = 1.0;
const SUB_OPTION_LEVEL: f64 = 2.0;
/// Rows the original column lays out (core_menu MAX_SUBCATS). Skate 3 never
/// had longer columns, so the movie has no scrolling of its own: longer ones
/// are shown as a window of this many rows that follows the highlight.
const ROWS: usize = 8;

/// One row in a category's column. Text starting with `#` is shown literally;
/// anything else is looked up in the game's own string table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub help: String,
    pub enabled: bool,
    /// A non-empty list opens the original picker; the row's value is `value`.
    pub values: Vec<String>,
    pub value: usize,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Category {
    pub name: String,
    pub icon: String,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub title: String,
    pub categories: Vec<Category>,
}
/// What the player did, for the engine to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Accept on a row without a picker.
    Activate { id: u32 },
    /// A picker value was chosen.
    Choose { id: u32, value: usize },
    /// Back on the column: leave the menu.
    Close,
}

pub struct Bindings {
    pub movie: Movie,
    pub model: Model,
    pub menu: usize,
    pub option: usize,
    pub level: f64,
    pub sub: usize,
    /// The first row of the current column the movie shows.
    pub top: usize,
    pub events: Vec<Event>,
    pub shared: crate::apt_screen::Shared,
}
impl Bindings {
    fn category(&self) -> Option<&Category> {
        self.model.categories.get(self.menu)
    }
    fn entry(&self) -> Option<&Entry> {
        self.category().and_then(|c| c.entries.get(self.option))
    }
    fn row(&self, menu: usize, option: usize) -> Option<&Entry> {
        self.model.categories.get(menu).and_then(|c| c.entries.get(option))
    }
    /// A row the movie names by its place on screen, as a row of the column.
    fn shown(&self, menu: usize, place: Option<usize>) -> usize {
        match place {
            Some(place) if menu == self.menu => place + self.top,
            Some(place) => place,
            None => self.option,
        }
    }
    /// Keep the highlighted row inside the window. True when it moved.
    fn follow(&mut self) -> bool {
        let rows = self.category().map_or(0, |c| c.entries.len());
        let top = if rows <= ROWS {
            0
        } else if self.option < self.top {
            self.option
        } else if self.option >= self.top + ROWS {
            self.option + 1 - ROWS
        } else {
            self.top.min(rows - ROWS)
        };
        std::mem::replace(&mut self.top, top) != top
    }
}

use crate::apt_screen::array;
fn literal(text: &str) -> Value {
    Value::Text(text.into())
}

impl Host for Bindings {
    fn property(&self, vm: &Vm, object: usize, key: &str) -> Option<Value> {
        self.movie.measure(vm, object, key)
    }
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
        }
        self.movie.transform_changed(vm, object, key)
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        object: usize,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        if let Some(value) = self.movie.method(vm, object, method, &args)? {
            return Ok(value);
        }
        let native = match vm.objects.get(object).map(|o| &o.kind) {
            Some(ObjectKind::Native(name)) => name.clone(),
            _ => "object".into(),
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        let index = |i: usize, current: usize| {
            let v = arg(i).number();
            if v.is_finite() && v >= 0.0 { v as usize } else { current }
        };
        let given = |i: usize| {
            let v = arg(i).number();
            (v.is_finite() && v >= 0.0).then_some(v as usize)
        };
        // (menu, row on screen) -> the row in the column.
        let place = |b: &Self| {
            let menu = index(0, b.menu);
            (menu, b.shown(menu, given(1)))
        };
        Ok(match (native.as_str(), method) {
            // --- the game's CrossBar front end -------------------------------
            ("coremenu", "GetIsDemo") => Value::Bool(false),
            ("coremenu", "IsDLCEnabled") => Value::Bool(false),
            ("coremenu", "GetCurrentDLCBannerIndex") => Value::Number(-1.0),
            ("coremenu", "GetMaxSubOptions") => Value::Number(
                self.model
                    .categories
                    .iter()
                    .flat_map(|c| &c.entries)
                    .map(|e| e.values.len())
                    .max()
                    .unwrap_or(0)
                    .clamp(1, 8) as f64,
            ),
            // Career progress drives the progression panel; a negative step
            // selects its "welcome" state, which carries no career data.
            ("coremenu", "GetProgressionData") => array(vm, [-1.0, 0.0, 0.0, 0.0, 0.0].map(Value::Number))?,
            ("coremenu", "GetCurrentModeTitle") => literal(&self.model.title),
            ("coremenu", "GetNumMenus") => Value::Number(self.model.categories.len() as f64),
            ("coremenu", "GetCurrentMenuIndex") => Value::Number(self.menu as f64),
            ("coremenu", "GetMenuIconName") => Value::Text(
                self.model.categories.get(index(0, self.menu)).map_or(String::new(), |c| c.icon.clone()),
            ),
            ("coremenu", "GetMenuName") => literal(
                &self.model.categories.get(index(0, self.menu)).map_or(String::new(), |c| c.name.clone()),
            ),
            // Row queries name their category and row explicitly:
            // (menu), (menu, option) and (menu, option, sub).
            ("coremenu", "GetNumOptions") => Value::Number(
                self.model.categories.get(index(0, self.menu)).map_or(0, |c| c.entries.len().min(ROWS)) as f64,
            ),
            ("coremenu", "GetCurrentOptionIndex") => Value::Number(self.option.saturating_sub(self.top) as f64),
            ("coremenu", "GetOptionName") => {
                let (menu, row) = place(self);
                literal(&self.row(menu, row).map_or(String::new(), |e| e.name.clone()))
            }
            ("coremenu", "GetOptionIconName") => {
                let (menu, row) = place(self);
                Value::Text(self.row(menu, row).map_or(String::new(), |e| e.icon.clone()))
            }
            ("coremenu", "IsItemEnabled") => {
                let (menu, row) = place(self);
                Value::Bool(self.row(menu, row).is_some_and(|e| e.enabled))
            }
            ("coremenu", "GetOptionHelperDescription") => {
                literal(&self.entry().map_or(String::new(), |e| e.help.clone()))
            }
            ("coremenu", "GetNumSubOptions") => {
                let (menu, row) = place(self);
                Value::Number(self.row(menu, row).map_or(0, |e| e.values.len()) as f64)
            }
            ("coremenu", "GetSubOptionName") => {
                let (menu, row) = place(self);
                literal(&self.row(menu, row).and_then(|e| e.values.get(index(2, self.sub))).cloned().unwrap_or_default())
            }
            ("coremenu", "GetCurrentSubOptionIndex") => Value::Number(self.sub as f64),
            ("coremenu", "GetCurrentMenuLevel") => Value::Number(self.level),
            // --- picker callbacks -------------------------------------------
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game", "OnMenuHilight") => {
                self.sub = index(1, self.sub);
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                let value = index(1, self.sub);
                if let Some(entry) = self.entry() {
                    self.events.push(Event::Choose { id: entry.id, value });
                }
                Value::Undefined
            }
            ("Game", "GetDLCPackBannerURI") => Value::Text(String::new()),
            (native, method) => {
                if let Some(value) =
                    crate::apt_screen::shared(&mut self.movie, &mut self.shared, vm, object, native, method, &args)?
                {
                    return Ok(value);
                }
                return Err(format!("Unimplemented front-end binding {native}.{method}"));
            }
        })
    }
}
impl MovieHost for Bindings {
    fn movie(&mut self) -> &mut Movie {
        &mut self.movie
    }
}

pub struct Runtime {
    pub vm: Vm,
    pub bindings: Bindings,
}
impl Runtime {
    pub fn load(json: &serde_json::Value, model: Model) -> Result<Self, String> {
        let mut vm = Vm::new();
        for name in [
            "MovieClip", "Math", "Object", "FELanguage", "Game", "Audio", "ScreenManager", "coremenu",
        ] {
            let object = vm.object(ObjectKind::Native(name.into()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            vm.set(vm.global, name, Value::Object(object))?;
        }
        for (name, value) in [
            ("FECrossBarScreenMode_RefreshMenu", mode::REFRESH_MENU),
            ("FECrossBarScreenMode_RefreshOption", mode::REFRESH_OPTION),
            ("FECrossBarScreenMode_InitialiseMenu", mode::INITIALISE_MENU),
            ("FECrossBarScreenMode_InitialiseSubOption", mode::INITIALISE_SUB_OPTION),
            ("FECrossBarScreenMode_InitSubOptionOnlineTeam", mode::INIT_SUB_OPTION_ONLINE_TEAM),
            ("FECrossBarScreenMode_RefreshDLCBanner", mode::REFRESH_DLC_BANNER),
            ("AptUp", Key::Up.code()),
            ("AptDown", Key::Down.code()),
            ("AptLeft", Key::Left.code()),
            ("AptRight", Key::Right.code()),
            ("AptNext", Key::Next.code()),
            ("AptPrevious", Key::Previous.code()),
            ("Screen_EdgeOffset", 0.0),
        ] {
            vm.set(vm.global, name, Value::Number(value))?;
        }
        crate::apt_screen::define_sounds(&mut vm)?;
        let mut bindings = Bindings {
            movie: Movie::load(json)?,
            model,
            menu: 0,
            option: 0,
            level: OPTION_LEVEL,
            sub: 0,
            top: 0,
            events: Vec::new(),
            shared: Default::default(),
        };
        // Class definitions (DoInitAction) run before any clip exists.
        let initial: Vec<u32> = json["init_actions"]
            .as_array()
            .map(|a| a.iter().filter_map(|i| i["actions"].as_u64()).map(|o| o as u32).collect())
            .unwrap_or_default();
        vm.begin_update();
        for offset in initial {
            let code = bindings
                .movie
                .actions
                .get(&offset.to_string())
                .ok_or("Missing front-end class definition")?
                .clone();
            vm.run_on(vm.global, &code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut runtime = Self { vm, bindings };
        runtime.drain()?;
        if runtime.bindings.shared.screen == usize::MAX {
            if let Value::Object(screen) = runtime.vm.get(runtime.bindings.movie.root, "screen") {
                runtime.bindings.shared.screen = screen;
            } else {
                return Err("Original pause menu did not construct its screen".into());
            }
        }
        Ok(runtime)
    }
    fn drain(&mut self) -> Result<(), String> {
        Movie::drain(&mut self.vm, &mut self.bindings)
    }
    fn screen_call(&mut self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.vm.begin_update();
        let value = self
            .vm
            .call_method(self.bindings.shared.screen, method, args, &mut self.bindings)?;
        self.drain()?;
        Ok(value)
    }
    fn set_state(&mut self, mode: f64) -> Result<(), String> {
        self.screen_call("SetState", vec![Value::Number(mode)]).map(|_| ())
    }
    /// Show the menu: replay the intro and build the column for the model.
    pub fn open(&mut self, model: Model) -> Result<(), String> {
        self.bindings.model = model;
        self.bindings.menu = self.bindings.menu.min(self.bindings.model.categories.len().saturating_sub(1));
        self.bindings.option = 0;
        self.bindings.top = 0;
        self.bindings.level = OPTION_LEVEL;
        let root = self.bindings.movie.root;
        self.vm.begin_update();
        self.bindings.movie.method(&mut self.vm, root, "gotoAndPlay", &[Value::Text("intro".into())])?;
        self.drain()?;
        self.set_state(mode::INITIALISE_MENU)
    }
    /// New rows for the current column, highlighted from the top.
    pub fn replace(&mut self, model: Model) -> Result<(), String> {
        self.bindings.model = model;
        self.bindings.menu = self.bindings.menu.min(self.bindings.model.categories.len().saturating_sub(1));
        self.bindings.option = 0;
        self.bindings.top = 0;
        self.bindings.level = OPTION_LEVEL;
        self.set_state(mode::REFRESH_MENU)
    }
    pub fn input(&mut self, key: Key) -> Result<(), String> {
        let b = &mut self.bindings;
        let categories = b.model.categories.len();
        let options = b.category().map_or(0, |c| c.entries.len());
        let mut state = None;
        if b.level == SUB_OPTION_LEVEL {
            // The picker owns navigation; the screen forwards keys to it and
            // tracks the highlighted value through Game.OnMenuHilight.
            //
            // Choosing is answered here rather than by the movie: once its
            // picker reports a selection it ignores further input until the
            // game closes it, and nothing in the screen's states does that,
            // so the values stayed up and the row names stayed hidden. Report
            // the highlighted value, then leave exactly as backing out does,
            // which closes the picker cleanly.
            if matches!(key, Key::Next | Key::Previous) {
                if key == Key::Next {
                    if let Some(entry) = b.entry() {
                        let event = Event::Choose { id: entry.id, value: b.sub };
                        b.events.push(event);
                    }
                }
                b.level = OPTION_LEVEL;
                self.screen_call("UpdateInput", vec![Value::Number(Key::Previous.code()), Value::Number(0.0)])?;
                self.set_state(mode::REFRESH_MENU)?;
            } else {
                self.screen_call("UpdateInput", vec![Value::Number(key.code()), Value::Number(0.0)])?;
            }
            return Ok(());
        }
        match key {
            Key::Left | Key::Right if categories > 0 => {
                b.menu = if key == Key::Right { (b.menu + 1) % categories } else { (b.menu + categories - 1) % categories };
                b.option = 0;
                b.top = 0;
                state = Some(mode::REFRESH_MENU);
            }
            Key::Up | Key::Down if options > 0 => {
                b.option = if key == Key::Down { (b.option + 1) % options } else { (b.option + options - 1) % options };
                // Past the shown rows: the window moves and the rows are relabelled.
                state = Some(if b.follow() { mode::REFRESH_MENU } else { mode::REFRESH_OPTION });
            }
            Key::Next => {
                if let Some(entry) = b.entry().cloned() {
                    if !entry.enabled {
                    } else if entry.values.is_empty() {
                        b.events.push(Event::Activate { id: entry.id });
                    } else {
                        b.level = SUB_OPTION_LEVEL;
                        b.sub = entry.value.min(entry.values.len() - 1);
                        state = Some(mode::INITIALISE_SUB_OPTION);
                    }
                }
            }
            Key::Previous => b.events.push(Event::Close),
            _ => {}
        }
        self.screen_call("UpdateInput", vec![Value::Number(key.code()), Value::Number(0.0)])?;
        if let Some(state) = state {
            self.set_state(state)?;
        }
        Ok(())
    }
    /// One movie frame (the shipped front end ran at the movie's 60 Hz rate).
    pub fn advance(&mut self) -> Result<(), String> {
        self.vm.begin_update();
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.vm.collect(self.bindings.movie.instances.keys().copied())
    }
    /// What a point on the stage is over: a tab, a row, or a picker value.
    fn target(&self, point: [f32; 2]) -> Option<Target> {
        let (vm, movie) = (&self.vm, &self.bindings.movie);
        let root = movie.root;
        let b = &self.bindings;
        // Rows and items are measured with their (invisible) glow pieces, so
        // their boxes overlap: of those under the point, the nearest label wins.
        let nearest = |clips: &mut dyn Iterator<Item = (usize, usize)>| {
            clips
                .filter(|&(_, clip)| movie.hit(vm, clip, point))
                .filter_map(|(index, clip)| {
                    let label = movie.child(vm, clip, "mText.mTitle.TEXTLABEL").unwrap_or(clip);
                    movie.center(vm, label).map(|c| (index, (c[1] - point[1]).abs()))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        };
        if b.level == SUB_OPTION_LEVEL {
            let values = &b.entry()?.values;
            let mut items = (0..8).filter_map(|k| {
                movie.child(vm, root, &format!("mColumnAnim.mSubColumn.mMenuPicker.mItemGroup.Item{k}")).map(|c| (k, c))
            });
            if let Some(k) = nearest(&mut items) {
                let item = movie.child(vm, root, &format!("mColumnAnim.mSubColumn.mMenuPicker.mItemGroup.Item{k}"))?;
                let label = movie.child(vm, item, "mText.mTitle.TEXTLABEL").map(|t| vm.get(t, "text").text());
                return label.and_then(|l| values.iter().position(|v| *v == l)).map(Target::Value);
            }
        }
        for i in 0..b.model.categories.len() {
            if movie.child(vm, root, &format!("cat{i}")).is_some_and(|tab| movie.hit(vm, tab, point)) {
                return Some(Target::Category(i));
            }
        }
        let rows = b.category().map_or(0, |c| c.entries.len().min(ROWS));
        let mut rows = (0..rows).filter_map(|j| movie.child(vm, root, &format!("mColumnAnim.mColumn.mSubcat{j}")).map(|c| (j, c)));
        nearest(&mut rows).map(|j| Target::Row(b.top + j))
    }
    /// Mouse and touch: hovering moves the highlight, pressing chooses, as
    /// the pad's directions and A would. Pressing off the picker closes it.
    pub fn pointer(&mut self, pointer: Pointer) -> Result<(), String> {
        let (point, press) = match pointer {
            Pointer::Hover(point) => (point, false),
            Pointer::Press(point) => (point, true),
        };
        let target = self.target(point);
        let sub = self.bindings.level == SUB_OPTION_LEVEL;
        match target {
            Some(Target::Value(value)) if sub => {
                for _ in 0..16 {
                    if self.bindings.sub == value {
                        break;
                    }
                    self.input(if value > self.bindings.sub { Key::Down } else { Key::Up })?;
                }
                if press {
                    self.input(Key::Next)?;
                }
            }
            Some(Target::Category(i)) if press => {
                if sub {
                    self.input(Key::Previous)?;
                }
                let n = self.bindings.model.categories.len();
                let right = (i + n - self.bindings.menu) % n;
                let key = if right <= n / 2 { Key::Right } else { Key::Left };
                for _ in 0..n {
                    if self.bindings.menu == i {
                        break;
                    }
                    self.input(key)?;
                }
            }
            Some(Target::Row(j)) if !sub => {
                for _ in 0..16 {
                    if self.bindings.option == j {
                        break;
                    }
                    self.input(if j > self.bindings.option { Key::Down } else { Key::Up })?;
                }
                if press {
                    self.input(Key::Next)?;
                }
            }
            None if press && sub => self.input(Key::Previous)?,
            _ => {}
        }
        Ok(())
    }
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.bindings.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Model {
        let entry = |id, name: &str, values: &[&str]| Entry {
            id,
            name: format!("#{name}"),
            icon: "map".into(),
            help: format!("#{name} help"),
            enabled: true,
            values: values.iter().map(|v| format!("#{v}")).collect(),
            value: 0,
        };
        Model {
            title: "#FREESKATE".into(),
            categories: vec![
                Category { name: "#MAPS".into(), icon: "singleplayer".into(), entries: vec![entry(1, "University", &[]), entry(2, "Downtown", &[])] },
                Category { name: "#OPTIONS".into(), icon: "options".into(), entries: vec![entry(3, "Difficulty", &["Easy", "Normal", "Hard"])] },
            ],
        }
    }

    /// Runs the shipped movie end to end when the private front-end export is
    /// installed. Diagnostic: prints the first failure with its call path.
    #[test]
    #[ignore = "Requires private installed front-end assets"]
    fn original_pause_menu_runs() {
        let path = std::env::var("SKATE_FRONTEND_JSON").expect("SKATE_FRONTEND_JSON");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut runtime = Runtime::load(&json, sample()).unwrap();
        runtime.open(sample()).unwrap();
        for _ in 0..40 {
            runtime.advance().unwrap();
        }
        if let Ok(out) = std::env::var("SKATE_FRONTEND_DUMP") {
            let shapes: crate::apt_scene::Shapes = serde_json::from_value(json["shapes"].clone()).unwrap();
            let draws = crate::apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &shapes).unwrap();
            let dump: Vec<_> = draws
                .iter()
                .map(|d| serde_json::json!({
                    "texture": d.texture,
                    "multiply": d.multiply,
                    "add": d.add,
                    "vertices": d.vertices.iter().map(|v| [v.position[0], v.position[1], v.uv[0], v.uv[1]]).collect::<Vec<_>>(),
                }))
                .collect();
            std::fs::write(out, serde_json::to_vec(&dump).unwrap()).unwrap();
        }
        for (step, key) in [Key::Down, Key::Right, Key::Next, Key::Down, Key::Next].into_iter().enumerate() {
            runtime.input(key).unwrap();
            for _ in 0..10 {
                runtime.advance().unwrap();
            }
            if step == 3 && std::env::var_os("SKATE_MENU_TREE").is_some() {
                println!("--- picker open");
                print_tree(&runtime, runtime.bindings.movie.root, 0);
                println!("--- end picker");
            }
        }
        assert!(runtime.take_events().iter().any(|e| matches!(e, Event::Choose { id: 3, value: 1 })));
        if std::env::var_os("SKATE_MENU_TREE").is_some() {
            print_tree(&runtime, runtime.bindings.movie.root, 0);
        }
    }

    /// Mouse and touch reach every part of the original menu.
    #[test]
    #[ignore = "Requires private installed front-end assets"]
    fn original_pause_menu_takes_the_pointer() {
        let path = std::env::var("SKATE_FRONTEND_JSON").expect("SKATE_FRONTEND_JSON");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut runtime = Runtime::load(&json, sample()).unwrap();
        runtime.open(sample()).unwrap();
        let settle = |runtime: &mut Runtime| (0..20).for_each(|_| runtime.advance().unwrap());
        settle(&mut runtime);
        // A point over each thing, found the way a click would find it.
        let find = |runtime: &Runtime, wanted: &dyn Fn(&Target) -> bool| -> [f32; 2] {
            for y in (0..720).step_by(4) {
                for x in (0..1280).step_by(4) {
                    let point = [x as f32, y as f32];
                    if runtime.target(point).as_ref().is_some_and(wanted) {
                        return point;
                    }
                }
            }
            panic!("nothing on the stage matched");
        };
        let row1 = find(&runtime, &|t| matches!(t, Target::Row(1)));
        runtime.pointer(Pointer::Hover(row1)).unwrap();
        assert_eq!(runtime.bindings.option, 1, "hover highlights the row");
        let tab = find(&runtime, &|t| matches!(t, Target::Category(1)));
        runtime.pointer(Pointer::Press(tab)).unwrap();
        assert_eq!(runtime.bindings.menu, 1, "a tab press switches column");
        settle(&mut runtime);
        let row0 = find(&runtime, &|t| matches!(t, Target::Row(0)));
        runtime.pointer(Pointer::Press(row0)).unwrap();
        assert_eq!(runtime.bindings.level, SUB_OPTION_LEVEL, "pressing a picker row opens it");
        settle(&mut runtime);
        let normal = find(&runtime, &|t| matches!(t, Target::Value(1)));
        runtime.pointer(Pointer::Press(normal)).unwrap();
        assert!(runtime.take_events().iter().any(|e| matches!(e, Event::Choose { id: 3, value: 1 })), "pressing a value chooses it");
    }

    fn print_tree(runtime: &Runtime, id: usize, depth: usize) {
        {
            fn tree(runtime: &Runtime, id: usize, depth: usize) {
                let (vm, movie) = (&runtime.vm, &runtime.bindings.movie);
                let Some(instance) = movie.instances.get(&id) else { return };
                if !vm.get(id, "_visible").truth() {
                    return;
                }
                println!("{}{} #{} x={} y={} text={:?}", "  ".repeat(depth), vm.get(id, "_name").text(), instance.character,
                    vm.get(id, "_x").text(), vm.get(id, "_y").text(), vm.get(id, "text").text());
                for child in instance.ordered_children() {
                    tree(runtime, child, depth + 1);
                }
            }
            tree(runtime, id, depth);
        }
    }
}
