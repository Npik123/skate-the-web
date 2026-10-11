//! Native HUD bindings and the original ActionScript-driven movie lifecycle.
use crate::{
    apt_movie::{Movie, MovieHost},
    apt_vm::{Host, ObjectKind, Value, Vm},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub sequence_score: i32,
    pub line_score: i32,
    pub sequence_timer: i32,
    pub line_time: f32,
    pub line_capacity: f32,
    pub multiplier: f32,
    pub clean: bool,
    pub sketchy: bool,
    pub stance: [bool; 4],
    pub trick_name: String,
    pub trick_metrics: [Value; 5],
    pub context_tricks: Vec<Value>,
}
pub struct Bindings {
    pub movie: Movie,
    pub input: Input,
    random: u32,
}
pub struct Runtime {
    pub vm: Vm,
    pub bindings: Bindings,
    pub controller: usize,
}

fn array(vm: &mut Vm, values: impl IntoIterator<Item = Value>) -> Result<Value, String> {
    let object = vm.object(ObjectKind::Plain);
    let mut n = 0;
    for value in values {
        vm.set(object, n.to_string(), value)?;
        n += 1;
    }
    vm.set(object, "length", Value::Number(n as f64))?;
    Ok(Value::Object(object))
}
fn score(value: i32) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
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
impl Host for Bindings {
    fn property(&self, vm: &Vm, object: usize, key: &str) -> Option<Value> {
        self.movie.measure(vm, object, key)
    }
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
        }
        self.movie.transform_changed(vm, object, key)?;
        Ok(())
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
            Some(ObjectKind::Native(name)) => name.as_str(),
            _ => "global",
        };
        match (native, method) {
            ("Math", "floor") => Ok(Value::Number(
                args.first().ok_or("Math.floor argument")?.number().floor(),
            )),
            ("Math", "random") => {
                // Presentation-only random source; never influences scoring.
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                Ok(Value::Number(self.random as f64 / 4294967296.0))
            }
            // The HUD never enumerates prototype properties. Marking their
            // enumeration flags therefore leaves its observable fields intact.
            ("global", "ASSetPropFlags") => Ok(Value::Undefined),
            ("FELanguage", "GetLanguage") => Ok(Value::Text("english".into())),
            ("Tricks", "GetLineTimerMaxPoints") => {
                Ok(Value::Number(self.input.line_capacity as f64))
            }
            //825C2910 truncates manager+32 to integer;825C28F8 returns+192.
            ("Tricks", "GetLineTimeRemaining") => {
                Ok(Value::Number(self.input.line_time.trunc() as f64))
            }
            ("Tricks", "GetLineScore") => Ok(Value::Number(self.input.line_score as f64)),
            ("HUDComponents", "TrickDisplay_GetSequenceMultiplier") => {
                Ok(Value::Number(self.input.multiplier as f64))
            }
            ("Tricks", "GetGeneralInfo") => array(
                vm,
                [
                    Value::Number(self.input.sequence_score as f64),
                    Value::Text(score(self.input.sequence_score)),
                    // 825C2D90 exposes backend152/153; the shipped
                    // UpdateTrickScoring reads [2] as sketchy, [3] as clean.
                    Value::Bool(self.input.sketchy),
                    Value::Bool(self.input.clean),
                    Value::Number(self.input.sequence_timer as f64),
                    Value::Number(self.input.line_score as f64),
                    Value::Text(score(self.input.line_score)),
                ],
            ),
            ("Tricks", "GetCurrentTrickStance") => array(vm, self.input.stance.map(Value::Bool)),
            ("Tricks", "GetCurrentTrickName") => Ok(Value::Text(format!(
                "#{}",
                crate::scoring_hud::localize_trick(
                    &self.input.trick_name,
                    Some(&self.movie.text_assets)
                )
            ))),
            ("Tricks", "GetCurrentTrickMetrics") => {
                let mut metrics = self.input.trick_metrics.clone();
                metrics[0] = Value::Text(format!(
                    "#{}",
                    crate::scoring_hud::localize_trick(
                        &self.input.trick_name,
                        Some(&self.movie.text_assets)
                    )
                ));
                array(vm, metrics)
            }
            ("Tricks", "TrickDisplay_GetAllTrickData") => {
                array(vm, self.input.context_tricks.clone())
            }
            _ => Err(format!(
                "Unimplemented original HUD binding {native}.{method}"
            )),
        }
    }
}
impl MovieHost for Bindings {
    fn movie(&mut self) -> &mut Movie {
        &mut self.movie
    }
}
impl Runtime {
    pub fn load(json: &serde_json::Value, input: Input) -> Result<Self, String> {
        let mut vm = Vm::new();
        for name in ["MovieClip", "Tricks", "FELanguage", "HUDComponents", "Math"] {
            let object = vm.object(ObjectKind::Native(name.into()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            vm.set(vm.global, name, Value::Object(object))?;
        }
        vm.set(vm.global, "Screen_EdgeOffset", Value::Number(0.0))?;
        let mut bindings = Bindings {
            movie: Movie::load(json)?,
            input,
            random: 0x9e3779b9,
        };
        let initial: Vec<_> = bindings
            .movie
            .characters
            .values()
            .flat_map(|c| &c.frames)
            .flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action")
            .map(|c| c.actions_offset)
            .collect();
        for offset in initial {
            let code = bindings.movie.actions[&offset.to_string()].clone();
            vm.run(&code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut runtime = Self {
            vm,
            bindings,
            controller: usize::MAX,
        };
        runtime.drain()?;
        let Value::Object(controller) = runtime.vm.get(runtime.bindings.movie.root, "screen")
        else {
            return Err("Original HUD controller was not constructed".into());
        };
        runtime.controller = controller;
        runtime.vm.call_method(
            controller,
            "ScreenShow",
            vec![Value::Bool(true)],
            &mut runtime.bindings,
        )?;
        runtime.drain()?;
        Ok(runtime)
    }
    fn drain(&mut self) -> Result<(), String> {
        Movie::drain(&mut self.vm, &mut self.bindings)
    }
    pub fn update(
        &mut self,
        input: Input,
        new_trick: bool,
        modified_trick: bool,
        close_tricks: bool,
    ) -> Result<(), String> {
        self.vm.begin_update();
        let previous = self.bindings.input.clone();
        self.bindings.input = input;
        let mut methods = Vec::new();
        if previous.multiplier != self.bindings.input.multiplier {
            methods.push("UpdateMultiplier");
        }
        if previous.sequence_timer != self.bindings.input.sequence_timer
            || previous.sequence_score != self.bindings.input.sequence_score
            || previous.clean != self.bindings.input.clean
            || previous.sketchy != self.bindings.input.sketchy
            || previous.line_score != self.bindings.input.line_score
        {
            methods.push("UpdateTrickScoring");
        }
        if previous.stance != self.bindings.input.stance {
            methods.push("UpdateStance");
        }
        if new_trick {
            methods.push("RefreshTrickName");
        } else if modified_trick {
            methods.push("CurrentTrickModified");
        }
        if close_tricks {
            methods.push("CloseTrickDisplay");
        }
        for method in methods {
            self.vm
                .call_method(self.controller, method, vec![], &mut self.bindings)?;
            self.drain()?;
        }
        // The movie handles this as event 11 (0196CC), not onEnterFrame.
        // Re-seeking the 501-frame multiTimer_mc on every tick pins it to
        // the same whole-second sample instead of letting gotoAndPlay run.
        if previous.line_time.trunc() != self.bindings.input.line_time.trunc()
            || previous.line_score != self.bindings.input.line_score
        {
            self.vm.call_method(
                self.controller,
                "UpdateLineDisplay",
                vec![],
                &mut self.bindings,
            )?;
            self.drain()?;
        }
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.vm
            .collect(self.bindings.movie.instances.keys().copied())?;
        Ok(())
    }
}

#[cfg(test)]
mod trick_display_tests {
    use super::*;

    fn input(trick: &str, score: i32) -> Input {
        Input {
            sequence_score: score,
            line_score: score,
            sequence_timer: 5,
            line_time: 5.0,
            line_capacity: 10.0,
            multiplier: 1.0,
            clean: false,
            sketchy: false,
            stance: [false; 4],
            trick_name: trick.into(),
            trick_metrics: [Value::Text(trick.into()), Value::Bool(false), Value::Bool(false), Value::Bool(false), Value::Bool(true)],
            context_tricks: Vec::new(),
        }
    }

    /// Plays two tricks through the shipped HUD movie and optionally dumps the
    /// frame for inspection (SKATE_HUD_DUMP). Diagnostic for the trick name.
    #[test]
    #[ignore = "Requires private installed HUD assets"]
    fn trick_name_shows_after_repeated_tricks() {
        let path = std::env::var("SKATE_HUD_JSON").expect("SKATE_HUD_JSON");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut runtime = Runtime::load(&json, input("", 0)).unwrap();
        let settle: usize = std::env::var("SKATE_HUD_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(45);
        for (trick, score) in [("Kickflip", 500), ("Heelflip", 1200)] {
            runtime.update(input(trick, score), true, false, false).unwrap();
            for _ in 0..settle {
                runtime.update(input(trick, score), false, false, false).unwrap();
            }
        }
        let shapes: crate::apt_scene::Shapes = serde_json::from_value(json["shapes"].clone()).unwrap();
        let draws = crate::apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &shapes).unwrap();
        if let Ok(out) = std::env::var("SKATE_HUD_DUMP") {
            let dump: Vec<_> = draws.iter().map(|d| serde_json::json!({
                "texture": d.texture, "multiply": d.multiply, "add": d.add,
                "vertices": d.vertices.iter().map(|v| [v.position[0], v.position[1], v.uv[0], v.uv[1]]).collect::<Vec<_>>(),
            })).collect();
            std::fs::write(out, serde_json::to_vec(&dump).unwrap()).unwrap();
        }
        // Shown means every clip up to the root is visible, not merely that
        // the text field holds the name (it did while the animation never ran).
        let movie = &runtime.bindings.movie;
        let on_screen = |id: usize| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if !runtime.vm.get(c, "_visible").truth() || runtime.vm.get(c, "_alpha").number() <= 0.0 {
                    return false;
                }
                cur = movie.instances[&c].parent;
            }
            true
        };
        let shown: Vec<String> = movie.instances.keys()
            .filter(|id| on_screen(**id))
            .map(|id| runtime.vm.get(*id, "_displayText").text())
            .filter(|t| t != "undefined" && !t.is_empty())
            .collect();
        assert!(shown.iter().any(|t| t.to_lowercase().contains("heelflip")), "trick name not on screen: {shown:?}");
    }
}
