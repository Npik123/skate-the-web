//! Any original front-end or HUD screen run from its exported movie. The
//! movie's scripts draw and animate; the engine plays the game's native code,
//! answering the calls a screen makes (`HUDComponents.Timer_GetData`,
//! `coremenu.GetOptionName`...) and driving it through `SetState` as the
//! shipped screen manager did.
use crate::{
    apt_movie::{Movie, MovieHost, Pending},
    apt_vm::{Host, ObjectKind, Value, Vm},
};

/// The game's side of one screen: its own native objects and methods.
/// `Ok(None)` falls through to the natives every screen shares.
pub trait Natives {
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String>;
}

/// An AS2 array: a plain object with indexed fields and a length.
pub fn array(vm: &mut Vm, values: impl IntoIterator<Item = Value>) -> Result<Value, String> {
    let object = vm.object(ObjectKind::Plain);
    let mut n = 0;
    for value in values {
        vm.set(object, n.to_string(), value)?;
        n += 1;
    }
    vm.set(object, "length", Value::Number(n as f64))?;
    Ok(Value::Object(object))
}

/// Text the movie shows as written (`#` skips the string table).
pub fn literal(text: impl AsRef<str>) -> Value {
    Value::Text(format!("#{}", text.as_ref()))
}

/// Shared native state of a screen: what the screen manager tracked.
pub struct Shared {
    /// The controller passed to `ScreenManager.OnLoaded`; owns `SetState`.
    pub screen: usize,
    /// `Audio.PlaySound` requests, by the shipped FE sound name.
    pub sounds: Vec<String>,
    /// Pictures image components were asked to show (`LoadRX2Texture`), by
    /// clip; the screen's host draws them over the clip.
    pub images: std::collections::BTreeMap<usize, String>,
    random: u32,
}
impl Default for Shared {
    fn default() -> Self {
        Self { screen: usize::MAX, sounds: Vec::new(), images: Default::default(), random: 0x9e3779b9 }
    }
}

/// Natives every shipped screen leans on: the language table, Math, class
/// registration, arrays, the screen manager and the sound player.
pub fn shared(
    movie: &mut Movie,
    state: &mut Shared,
    vm: &mut Vm,
    object: usize,
    native: &str,
    method: &str,
    args: &[Value],
) -> Result<Option<Value>, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    Ok(Some(match (native, method) {
        // RegisterScreen passes (clip, controller); the controller owns
        // SetState and UpdateInput.
        ("ScreenManager", "OnLoaded") => {
            if let Value::Object(screen) = arg(1) {
                state.screen = screen;
            }
            Value::Undefined
        }
        // Transition notifications the manager used for its own sequencing.
        ("ScreenManager", "IntroComplete" | "OutroComplete") => Value::Undefined,
        ("Audio", "PlaySound") => {
            if let Value::Text(name) = arg(0) {
                state.sounds.push(name);
            }
            Value::Undefined
        }
        ("FELanguage", "GetLanguage") => Value::Text("english".into()),
        ("FELanguage", "GetLocaleText") => Value::Text(format!("#{}", movie.text_assets.localize(&arg(0).text()))),
        ("FELanguage", "GetLocalizedFormatStringCustom") => {
            // (label, value...) with %s/%d placeholders in the table text.
            let mut text = movie.text_assets.localize(&arg(0).text());
            for value in args.iter().skip(1) {
                if let Some(at) = text.find('%') {
                    let end = (at + 2).min(text.len());
                    text.replace_range(at..end, &value.text().trim_start_matches('#').to_string());
                }
            }
            Value::Text(format!("#{text}"))
        }
        ("FELanguage", "GetLocalizedFormatNumberString") => Value::Text(group(arg(0).number().round() as i64)),
        // Seconds as m:ss.
        ("FELanguage", "GetLocalizedTime") => {
            let seconds = arg(0).number().max(0.0);
            Value::Text(format!("{}:{:04.1}", (seconds / 60.0).floor(), seconds % 60.0))
        }
        ("Object", "registerClass") => {
            movie.register_class(vm, &arg(0).text(), arg(1))?;
            Value::Bool(true)
        }
        ("Math", m) => {
            let a = arg(0).number();
            let b = arg(1).number();
            Value::Number(match m {
                "floor" => a.floor(),
                "ceil" => a.ceil(),
                "round" => (a + 0.5).floor(),
                "abs" => a.abs(),
                "min" => a.min(b),
                "max" => a.max(b),
                "sqrt" => a.sqrt(),
                "sin" => a.sin(),
                "cos" => a.cos(),
                "atan2" => a.atan2(b),
                "pow" => a.powf(b),
                "random" => {
                    state.random ^= state.random << 13;
                    state.random ^= state.random >> 17;
                    state.random ^= state.random << 5;
                    state.random as f64 / 4294967296.0
                }
                _ => return Err(format!("Unimplemented Math.{m}")),
            })
        }
        // TextField.setTextFormat: the shipped fonts are fixed per field, so
        // only the colour carries over.
        (_, "setTextFormat") => {
            if let Value::Object(format) = arg(0) {
                let color = vm.get(format, "color");
                if !matches!(color, Value::Undefined) {
                    vm.set(object, "textColor", color)?;
                    movie.text_changed(vm, object)?;
                }
            }
            Value::Undefined
        }
        // Screens never enumerate prototype properties.
        (_, "ASSetPropFlags") => Value::Undefined,
        // Photo and banner loaders: online art is not part of this engine,
        // and the clips that show it stay hidden.
        // An image component asked for a picture by name; the host draws
        // it over the component if it has one by that name.
        (_, "LoadRX2Texture") => {
            let name = arg(0).text().trim_start_matches('#').to_owned();
            if name.is_empty() || name.contains('#') {
                state.images.remove(&object);
            } else {
                state.images.insert(object, name);
            }
            Value::Undefined
        }
        (_, "Unload" | "Destroy" | "Init") if state.images.contains_key(&object) => {
            state.images.remove(&object);
            Value::Undefined
        }
        (_, m) if m.starts_with("ImageRenderer_") || m == "LoadOnlineTexture" => Value::Undefined,
        // Arrays are plain objects with a length.
        (_, "push") => {
            let length = vm.get(object, "length").number();
            let mut n = if length.is_finite() { length as usize } else { 0 };
            for value in args {
                vm.set(object, n.to_string(), value.clone())?;
                n += 1;
            }
            vm.set(object, "length", Value::Number(n as f64))?;
            Value::Number(n as f64)
        }
        (_, "pop") => {
            let length = vm.get(object, "length").number();
            if !length.is_finite() || length < 1.0 {
                return Ok(Some(Value::Undefined));
            }
            let last = (length as usize - 1).to_string();
            let value = vm.get(object, &last);
            vm.objects[object].fields.remove(&last);
            vm.set(object, "length", Value::Number(length - 1.0))?;
            value
        }
        _ => return Ok(None),
    }))
}

/// A score as the HUD prints it: digits grouped by thousands.
pub fn group(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::from("#");
    if value < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub struct Bindings<N> {
    pub movie: Movie,
    pub natives: N,
    pub shared: Shared,
}
impl<N: Natives> Host for Bindings<N> {
    fn property(&self, vm: &Vm, object: usize, key: &str) -> Option<Value> {
        self.movie.measure(vm, object, key)
    }
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
        }
        self.movie.transform_changed(vm, object, key)
    }
    fn call(&mut self, vm: &mut Vm, object: usize, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let queued = self.movie.pending.len();
        if let Some(value) = self.movie.method(vm, object, method, &args)? {
            if method == "attachMovie" || method == "duplicateMovieClip" {
                // As in the shipped player, an attached clip's class
                // constructors run before attachMovie returns, so the caller
                // can set it up straight away.
                let mut constructors = Vec::new();
                let mut rest = std::collections::VecDeque::new();
                for (i, work) in std::mem::take(&mut self.movie.pending).into_iter().enumerate() {
                    match work {
                        Pending::Construct(clip, class) if i >= queued => constructors.push((clip, class)),
                        other => rest.push_back(other),
                    }
                }
                self.movie.pending = rest;
                for (clip, class) in constructors {
                    if self.movie.instances.contains_key(&clip) {
                        vm.construct(class, clip, Vec::new(), self)?;
                    }
                }
            }
            return Ok(value);
        }
        let native = match vm.objects.get(object).map(|o| &o.kind) {
            Some(ObjectKind::Native(name)) => name.clone(),
            _ => "object".into(),
        };
        if let Some(value) = self.natives.call(vm, &native, method, &args)? {
            return Ok(value);
        }
        if let Some(value) = shared(&mut self.movie, &mut self.shared, vm, object, &native, method, &args)? {
            return Ok(value);
        }
        // A method a clip's class does not have does nothing in AS2 (the
        // Trick Guide calls its picker's SetSkin, which this version of the
        // control lacks). Game natives stay strict: those are ours to answer.
        if native.starts_with("movie:") {
            return Ok(Value::Undefined);
        }
        Err(format!("Unimplemented screen binding {native}.{method}"))
    }
}
impl<N: Natives> MovieHost for Bindings<N> {
    fn movie(&mut self) -> &mut Movie {
        &mut self.movie
    }
}

/// The natives every screen can reach, besides its own.
/// The FE sound names the original screens pass to `Audio.PlaySound`: globals
/// the game defines, each holding its own name here, which `game_audio` plays
/// as the rendered sk8_menu patch of that name.
pub const FE_SOUNDS: [&str; 24] = [
    "FE_SOUND_CORE_A_BUTTON",
    "FE_SOUND_CORE_B_BUTTON",
    "FE_SOUND_CORE_NAV_UP",
    "FE_SOUND_CORE_NAV_DOWN",
    "FE_SOUND_CHALLENGE_COUNT_3",
    "FE_SOUND_CHALLENGE_COUNT_2",
    "FE_SOUND_CHALLENGE_COUNT_1",
    "FE_SOUND_CHALLENGE_COUNT_GO",
    "FE_SOUND_CHALLENGE_STAD_3",
    "FE_SOUND_CHALLENGE_STAD_2",
    "FE_SOUND_CHALLENGE_STAD_1",
    "FE_SOUND_CHALLENGE_STAD_GO",
    "FE_SOUND_CHALLENGE_FAILED",
    "FE_SOUND_CHALLENGE_SUCCESS",
    "FE_SOUND_CHALLENGE_OTS",
    "FE_SOUND_HOM_TRANSITION",
    "FE_SOUND_HOM_SCORE_RED",
    "FE_SOUND_KILLEDIT_INTRO",
    "FE_SOUND_TRICK_SUCCESSFUL",
    "FE_SOUND_TRICK_UNSUCCESSFUL",
    "FE_SOUND_CENTRAL_MESSAGE_DEFAULT",
    "FE_SOUND_OWNED_DOM_MESSAGE",
    "FE_SOUND_TITLESCREEN_START",
    "FE_SOUND_CORE_X_BUTTON",
];

/// Define the FE sound globals on a fresh VM.
pub fn define_sounds(vm: &mut Vm) -> Result<(), String> {
    for name in FE_SOUNDS {
        vm.set(vm.global, name, Value::Text(name.into()))?;
    }
    Ok(())
}

const SHARED_OBJECTS: [&str; 7] = ["MovieClip", "Math", "Object", "FELanguage", "Audio", "ScreenManager", "TextFormat"];

pub struct Screen<N> {
    pub vm: Vm,
    pub bindings: Bindings<N>,
}
impl<N: Natives> Screen<N> {
    /// Build the screen: native objects (with any fields they carry), global
    /// constants, class definitions, then the root clip and its constructors.
    pub fn load(
        json: &serde_json::Value,
        natives: N,
        objects: &[(&str, &[(&str, Value)])],
        globals: &[(&str, Value)],
    ) -> Result<Self, String> {
        let mut vm = Vm::new();
        let shared = SHARED_OBJECTS.iter().map(|name| (*name, &[][..]));
        for (name, fields) in shared.chain(objects.iter().copied()) {
            let object = vm.object(ObjectKind::Native(name.into()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            for (key, value) in fields {
                vm.set(object, *key, value.clone())?;
            }
            vm.set(vm.global, name, Value::Object(object))?;
        }
        vm.set(vm.global, "Screen_EdgeOffset", Value::Number(0.0))?;
        define_sounds(&mut vm)?;
        for (name, value) in globals {
            vm.set(vm.global, *name, value.clone())?;
        }
        let mut bindings = Bindings { movie: Movie::load(json)?, natives, shared: Shared::default() };
        // Class definitions (DoInitAction) run before any clip exists.
        let initial: Vec<u32> = json["init_actions"]
            .as_array()
            .map(|a| a.iter().filter_map(|i| i["actions"].as_u64()).map(|o| o as u32).collect())
            .unwrap_or_default();
        vm.begin_update();
        for offset in initial {
            let code = bindings.movie.actions.get(&offset.to_string()).ok_or("Missing screen class definition")?.clone();
            vm.run_on(vm.global, &code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut screen = Self { vm, bindings };
        screen.drain()?;
        if screen.bindings.shared.screen == usize::MAX {
            match screen.vm.get(screen.bindings.movie.root, "screen") {
                Value::Object(controller) => screen.bindings.shared.screen = controller,
                _ => return Err("Original screen did not construct its controller".into()),
            }
        }
        Ok(screen)
    }
    fn drain(&mut self) -> Result<(), String> {
        Movie::drain(&mut self.vm, &mut self.bindings)
    }
    /// Call a method on the screen's controller (`SetState`, `UpdateInput`...).
    pub fn call(&mut self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.vm.begin_update();
        let controller = self.bindings.shared.screen;
        let value = self.vm.call_method(controller, method, args, &mut self.bindings)?;
        self.drain()?;
        Ok(value)
    }
    pub fn set_state(&mut self, state: f64) -> Result<(), String> {
        self.call("SetState", vec![Value::Number(state)]).map(|_| ())
    }
    /// One movie frame (the shipped front end and HUD ran at 60 Hz).
    pub fn advance(&mut self) -> Result<(), String> {
        self.vm.begin_update();
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.vm.collect(self.bindings.movie.instances.keys().copied())
    }
    pub fn take_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.bindings.shared.sounds)
    }
}
