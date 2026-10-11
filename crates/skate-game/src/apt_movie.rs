//! Original APT movie hierarchy and timeline control, independent of Bevy.
use crate::{
    apt_display::{Control, DisplayList, Placement},
    apt_vm::{Host, Instruction, ObjectKind, Value, Vm},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Deserialize)]
pub struct Frame {
    pub controls: Vec<Control>,
}
#[derive(Clone, Deserialize)]
pub struct Character {
    pub id: i32,
    pub type_name: String,
    #[serde(default)]
    pub frames: Vec<Frame>,
    pub text: Option<serde_json::Value>,
    pub bounds: Option<[f32; 4]>,
}
/// Flash's offset for timeline-placed depths (authored depth 1 is -16383).
const TIMELINE_DEPTH: i32 = 16384;
#[derive(Clone)]
pub struct Instance {
    pub character: i32,
    pub frame: usize,
    pub playing: bool,
    /// Timeline-owned children by depth; rebuilt on every seek.
    pub children: BTreeMap<i32, usize>,
    /// Script-created children (attachMovie, createEmptyMovieClip) by depth.
    /// The timeline never removes these.
    pub attached: BTreeMap<i32, usize>,
    pub placement: Option<Placement>,
    pub parent: Option<usize>,
}
impl Instance {
    /// Children in drawing order. Timeline and script depths share one
    /// stacking order, as in the shipped player: timeline depths sit at
    /// depth - 16384 (Flash's reserved range), so clips a script attaches at
    /// depth 0 and up draw over the authored timeline (the gesture items'
    /// arrow over its dial).
    pub fn ordered_children(&self) -> Vec<usize> {
        let mut all: Vec<(i32, usize)> = self
            .children
            .iter()
            .map(|(d, id)| (d - TIMELINE_DEPTH, *id))
            .chain(self.attached.iter().map(|(d, id)| (*d, *id)))
            .collect();
        all.sort_by_key(|(depth, _)| *depth);
        all.into_iter().map(|(_, id)| id).collect()
    }
}
/// Work the movie queues for the next drain: frame scripts in timeline order,
/// and constructors of clips whose symbol has an AS2 class registered.
#[derive(Clone, Copy, Debug)]
pub enum Pending {
    Action(usize, u32),
    Construct(usize, usize),
}
/// Character id of the synthetic empty clip createEmptyMovieClip makes.
const EMPTY_CLIP: i32 = -1;

pub struct Movie {
    pub characters: BTreeMap<i32, Character>,
    pub instances: BTreeMap<usize, Instance>,
    pub actions: BTreeMap<String, Vec<Instruction>>,
    pub pending: VecDeque<Pending>,
    pub root: usize,
    pub root_character: i32,
    pub text_assets: crate::apt_text::TextAssets,
    /// Controller glyphs by name, for clips the native image renderer drew
    /// (`mButtonRender._test = "B"`): texture path and size.
    pub glyphs: BTreeMap<String, (String, [u32; 2])>,
    /// Export names, for attachMovie and Object.registerClass.
    pub linkage: BTreeMap<String, i32>,
    /// AS2 constructor registered for a symbol.
    classes: BTreeMap<i32, usize>,
    unconstructed: BTreeSet<usize>,
    /// Clips whose scale or rotation a script set; the timeline no longer
    /// drives their transform, matching the shipped player.
    pub scripted: BTreeSet<usize>,
    /// Local bounds of each shape, for `_width`/`_height` on clips.
    shape_bounds: BTreeMap<i32, [f32; 4]>,
    states: BTreeMap<i32, Vec<DisplayList>>,
}
impl Movie {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let mut characters: Vec<Character> =
            serde_json::from_value(json["characters"].clone()).map_err(|e| e.to_string())?;
        characters.push(Character {
            id: EMPTY_CLIP,
            type_name: "sprite".into(),
            frames: Vec::new(),
            text: None,
            bounds: None,
        });
        let mut states = BTreeMap::new();
        for c in &characters {
            let mut list = DisplayList::default();
            let mut frames = Vec::new();
            for frame in &c.frames {
                for control in &frame.controls {
                    list.apply(control)?;
                }
                frames.push(list.clone());
            }
            states.insert(c.id, frames);
        }
        let mut shape_bounds = BTreeMap::new();
        if let Some(shapes) = json["shapes"].as_object() {
            for (id, primitives) in shapes {
                let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for p in primitives.as_array().into_iter().flatten() {
                    for v in p["triangles"].as_array().into_iter().flatten().flat_map(|t| t.as_array().into_iter().flatten()) {
                        let (x, y) = (v["position"][0].as_f64().unwrap_or(0.0) as f32, v["position"][1].as_f64().unwrap_or(0.0) as f32);
                        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                    }
                }
                if let (Ok(id), true) = (id.parse::<i32>(), b[0] <= b[2]) {
                    shape_bounds.insert(id, b);
                }
            }
        }
        Ok(Self {
            shape_bounds,
            characters: characters.into_iter().map(|c| (c.id, c)).collect(),
            instances: BTreeMap::new(),
            actions: serde_json::from_value(json["actions"].clone()).map_err(|e| e.to_string())?,
            pending: VecDeque::new(),
            root: usize::MAX,
            root_character: json["root"].as_i64().unwrap_or(0) as i32,
            text_assets: crate::apt_text::TextAssets::load(json)?,
            glyphs: json["glyphs"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, g)| {
                    let size = [g["width"].as_u64()? as u32, g["height"].as_u64()? as u32];
                    Some((name.clone(), (g["texture"].as_str()?.to_string(), size)))
                })
                .collect(),
            linkage: serde_json::from_value(json["linkage"].clone()).unwrap_or_default(),
            classes: BTreeMap::new(),
            unconstructed: BTreeSet::new(),
            scripted: BTreeSet::new(),
            states,
        })
    }
    pub fn initialize(&mut self, vm: &mut Vm) -> Result<(), String> {
        self.root = self.create(vm, self.root_character, None, 0)?;
        vm.set(self.root, "_root", Value::Object(self.root))?;
        // Every clip sees the same authored root, including unnamed children.
        for id in self.instances.keys() {
            vm.set(*id, "_root", Value::Object(self.root))?;
        }
        Ok(())
    }
    /// Object.registerClass: clips of this symbol take the class's prototype
    /// and run its constructor when created.
    pub fn register_class(&mut self, vm: &Vm, linkage: &str, class: Value) -> Result<(), String> {
        let Some(&character) = self.linkage.get(linkage) else {
            return Err(format!("APT registerClass for unknown symbol {linkage}"));
        };
        match class {
            Value::Object(class) if matches!(vm.objects.get(class).map(|o| &o.kind), Some(ObjectKind::Function(_))) => {
                self.classes.insert(character, class);
            }
            _ => {
                self.classes.remove(&character);
            }
        }
        Ok(())
    }
    /// Run queued constructors and frame scripts until nothing is left.
    pub fn drain(vm: &mut Vm, host: &mut impl MovieHost) -> Result<(), String> {
        let mut calls = 0;
        while let Some(work) = host.movie().pending.pop_front() {
            calls += 1;
            if calls > 8192 {
                return Err("APT frame action limit".into());
            }
            match work {
                Pending::Action(object, offset) => {
                    if !host.movie().instances.contains_key(&object) {
                        continue;
                    }
                    let code = host
                        .movie()
                        .actions
                        .get(&offset.to_string())
                        .ok_or("Missing APT action block")?
                        .clone();
                    vm.run_on(object, &code, host)?;
                }
                Pending::Construct(object, class) => {
                    if host.movie().instances.contains_key(&object) {
                        vm.construct(class, object, Vec::new(), host)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn create(
        &mut self,
        vm: &mut Vm,
        character: i32,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<usize, String> {
        // Create-a-Skater's slider rows alone are a few thousand clips.
        if depth > 32 || self.instances.len() > 16_384 {
            return Err("APT movie hierarchy limit".into());
        }
        let c = self
            .characters
            .get(&character)
            .ok_or_else(|| format!("APT unknown character {character}"))?
            .clone();
        let id = vm.object(ObjectKind::Native(format!("movie:{character}")));
        if self.root != usize::MAX {
            vm.set(id, "_root", Value::Object(self.root))?;
        }
        if let Some(parent) = parent {
            vm.set(id, "_parent", Value::Object(parent))?;
        }
        for (key, value) in [
            ("_x", Value::Number(0.0)),
            ("_y", Value::Number(0.0)),
            ("_xscale", Value::Number(100.0)),
            ("_yscale", Value::Number(100.0)),
            ("_rotation", Value::Number(0.0)),
            ("_visible", Value::Bool(true)),
            ("_alpha", Value::Number(100.0)),
        ] {
            vm.set(id, key, value)?;
        }
        if let Some(&class) = self.classes.get(&character) {
            if let Value::Object(prototype) = vm.get(class, "prototype") {
                vm.objects[id].prototype = Some(prototype);
            }
            self.unconstructed.insert(id);
        }
        if let Some(text) = &c.text {
            vm.set(
                id,
                "text",
                Value::Text(text["initial_text"].as_str().unwrap_or("").into()),
            )?;
        }
        self.instances.insert(
            id,
            Instance {
                character,
                frame: 0,
                playing: !c.frames.is_empty(),
                children: BTreeMap::new(),
                attached: BTreeMap::new(),
                placement: None,
                parent,
            },
        );
        self.text_changed(vm, id)?;
        if !c.frames.is_empty() {
            self.seek(vm, id, 0, depth + 1)?;
        } else {
            self.queue_constructor(id);
        }
        Ok(id)
    }
    fn queue_constructor(&mut self, id: usize) {
        if self.unconstructed.remove(&id) {
            let character = self.instances[&id].character;
            if let Some(&class) = self.classes.get(&character) {
                self.pending.push_back(Pending::Construct(id, class));
            }
        }
    }
    pub fn text_changed(&self, vm: &mut Vm, id: usize) -> Result<(), String> {
        let Some(instance) = self.instances.get(&id) else {
            return Ok(());
        };
        let character = &self.characters[&instance.character];
        let Some(text) = &character.text else {
            return Ok(());
        };
        let value = self.text_assets.localize(&vm.get(id, "text").text());
        vm.set(id, "_displayText", Value::Text(value.clone()))?;
        let font = self
            .text_assets
            .fonts
            .get(&(text["font_id"].as_i64().ok_or("Invalid text font id")? as i32))
            .ok_or("Missing original text font")?;
        let height = text["font_height"].as_f64().ok_or("Invalid text size")? as f32;
        let bounds = character.bounds.ok_or("Missing text bounds")?;
        let lines = crate::apt_scene::wrapped(text, &self.text_assets.fonts, &value, bounds);
        let width = lines.iter().map(|line| font.width(line, height)).fold(0.0, f32::max);
        vm.set(id, "textWidth", Value::Number(width as f64))?;
        let rows = lines.len().max(1);
        vm.set(id, "textHeight", Value::Number((height + (rows - 1) as f32 * crate::apt_scene::line_height(height)) as f64))?;
        // Lines laid out; a field that shows them all never scrolls.
        vm.set(id, "maxscroll", Value::Number(rows as f64))?;
        let autosize = vm.get(id, "autoSize").text();
        let scale = vm.get(id, "_xscale").number();
        let scale = if scale.is_finite() { scale / 100.0 } else { 1.0 };
        vm.set(
            id,
            "_width",
            Value::Number(
                if autosize == "left" || autosize == "right" || autosize == "center" {
                    width
                } else {
                    bounds[2] - bounds[0]
                } as f64
                    * scale,
            ),
        )?;
        vm.set(id, "_height", Value::Number((bounds[3] - bounds[1]) as f64))?;
        Ok(())
    }
    /// Keep a clip's scale and rotation fields in step with its placement.
    fn apply_matrix(vm: &mut Vm, id: usize, m: [f32; 6]) -> Result<(), String> {
        vm.set(id, "_x", Value::Number(m[4] as f64))?;
        vm.set(id, "_y", Value::Number(m[5] as f64))?;
        let sx = (m[0] * m[0] + m[1] * m[1]).sqrt();
        // A mirrored placement (the right-hand corners of a 9-slice frame are
        // the left ones flipped) keeps its mirror in a negative y scale, so the
        // fields rebuild the same matrix (local_matrix). Positive scales would
        // turn a horizontal flip into a half turn and drop a vertical one.
        let mirrored = m[0] * m[3] - m[1] * m[2] < 0.;
        let sy = (m[2] * m[2] + m[3] * m[3]).sqrt() * if mirrored { -1. } else { 1. };
        vm.set(id, "_xscale", Value::Number(sx as f64 * 100.0))?;
        vm.set(id, "_yscale", Value::Number(sy as f64 * 100.0))?;
        vm.set(id, "_rotation", Value::Number(m[1].atan2(m[0]).to_degrees() as f64))?;
        Ok(())
    }
    /// A clip's local matrix: its placement, or its fields once a script owns it.
    pub fn local_matrix(&self, vm: &Vm, id: usize) -> [f32; 6] {
        let mut m = self.instances.get(&id).and_then(|i| i.placement.as_ref()).map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
        if self.scripted.contains(&id) {
            let field = |key: &str, fallback: f64| {
                let v = vm.get(id, key).number();
                if v.is_finite() { v } else { fallback }
            };
            let sx = field("_xscale", 100.) as f32 / 100.;
            let sy = field("_yscale", 100.) as f32 / 100.;
            let (sin, cos) = (field("_rotation", 0.) as f32).to_radians().sin_cos();
            m[0] = sx * cos;
            m[1] = sx * sin;
            m[2] = -sy * sin;
            m[3] = sy * cos;
        }
        m[4] = vm.get(id, "_x").number() as f32;
        m[5] = vm.get(id, "_y").number() as f32;
        m
    }
    /// Bounds of a clip's content in its own coordinates (before its own
    /// transform), as the player measures `_width` and `_height`.
    pub fn local_bounds(&self, vm: &Vm, id: usize) -> Option<[f32; 4]> {
        let instance = self.instances.get(&id)?;
        let character = self.characters.get(&instance.character)?;
        if character.type_name == "shape" {
            return self.shape_bounds.get(&character.id).copied();
        }
        if character.text.is_some() {
            return character.bounds;
        }
        let mut out: Option<[f32; 4]> = None;
        for child in instance.ordered_children() {
            if !vm.get(child, "_visible").truth() {
                continue;
            }
            let Some(b) = self.local_bounds(vm, child) else { continue };
            let m = self.local_matrix(vm, child);
            for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let px = m[0] * x + m[2] * y + m[4];
                let py = m[1] * x + m[3] * y + m[5];
                out = Some(match out {
                    None => [px, py, px, py],
                    Some(o) => [o[0].min(px), o[1].min(py), o[2].max(px), o[3].max(py)],
                });
            }
        }
        out
    }
    /// `_width` / `_height` of a clip, measured from its content as the
    /// player does on every read. Text fields keep their laid-out values.
    pub fn measure(&self, vm: &Vm, id: usize, key: &str) -> Option<Value> {
        if key != "_width" && key != "_height" {
            return None;
        }
        let instance = self.instances.get(&id)?;
        if self.characters.get(&instance.character)?.text.is_some() {
            return None;
        }
        let m = self.local_matrix(vm, id);
        let size = self.local_bounds(vm, id).map_or(0.0, |b| {
            if key == "_width" {
                (b[2] - b[0]) * (m[0] * m[0] + m[1] * m[1]).sqrt()
            } else {
                (b[3] - b[1]) * (m[2] * m[2] + m[3] * m[3]).sqrt()
            }
        });
        Some(Value::Number(size as f64))
    }
    /// A named descendant: `child(vm, root, "mColumnAnim.mColumn.mSubcat0")`.
    pub fn child(&self, vm: &Vm, id: usize, path: &str) -> Option<usize> {
        let mut at = id;
        for name in path.split('.') {
            match vm.get(at, name) {
                Value::Object(next) if self.instances.contains_key(&next) => at = next,
                _ => return None,
            }
        }
        Some(at)
    }
    /// Whether a clip and every clip above it is visible.
    pub fn shown(&self, vm: &Vm, id: usize) -> bool {
        let mut at = Some(id);
        while let Some(clip) = at {
            if !vm.get(clip, "_visible").truth() {
                return false;
            }
            at = self.instances.get(&clip).and_then(|i| i.parent);
        }
        true
    }
    /// A clip's transform onto the stage.
    pub fn world_matrix(&self, vm: &Vm, id: usize) -> [f32; 6] {
        let mut m = self.local_matrix(vm, id);
        let mut at = self.instances.get(&id).and_then(|i| i.parent);
        while let Some(clip) = at {
            m = crate::apt_scene::compose(self.local_matrix(vm, clip), m);
            at = self.instances.get(&clip).and_then(|i| i.parent);
        }
        m
    }
    /// The centre of a clip's content on the stage.
    pub fn center(&self, vm: &Vm, id: usize) -> Option<[f32; 2]> {
        let b = self.local_bounds(vm, id)?;
        let m = self.world_matrix(vm, id);
        let (x, y) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
        Some([m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]])
    }
    /// Whether a point on the 1280x720 stage falls on a visible clip's content.
    pub fn hit(&self, vm: &Vm, id: usize, point: [f32; 2]) -> bool {
        if !self.shown(vm, id) {
            return false;
        }
        let Some(b) = self.local_bounds(vm, id) else { return false };
        let m = self.world_matrix(vm, id);
        // Map the point into the clip's own space.
        let det = m[0] * m[3] - m[2] * m[1];
        if det.abs() < 1e-6 {
            return false;
        }
        let (x, y) = (point[0] - m[4], point[1] - m[5]);
        let lx = (m[3] * x - m[2] * y) / det;
        let ly = (-m[1] * x + m[0] * y) / det;
        lx >= b[0] && lx <= b[2] && ly >= b[1] && ly <= b[3]
    }
    /// A script set one of the transform fields; from now on they describe it.
    pub fn transform_changed(&mut self, vm: &mut Vm, id: usize, key: &str) -> Result<(), String> {
        // As in the shipped player, a clip a script has moved, scaled or
        // rotated no longer takes its transform from the timeline.
        if matches!(key, "_x" | "_y" | "_xscale" | "_yscale" | "_rotation") && self.instances.contains_key(&id) {
            self.scripted.insert(id);
        }
        if key == "_width" || key == "_height" {
            // Setting a size scales the clip so its content spans it. Text
            // measures by its laid-out extent; other clips by their content.
            if let Some(instance) = self.instances.get(&id) {
                let base = if self.characters[&instance.character].text.is_some() {
                    Some(if key == "_width" {
                        vm.get(id, "textWidth").number()
                    } else {
                        vm.get(id, "textHeight").number()
                    })
                } else {
                    self.local_bounds(vm, id)
                        .map(|b| if key == "_width" { b[2] - b[0] } else { b[3] - b[1] } as f64)
                };
                if let Some(base) = base {
                    let wanted = vm.get(id, key).number();
                    if base > 0.0 && wanted.is_finite() {
                        let field = if key == "_width" { "_xscale" } else { "_yscale" };
                        // A size never undoes a mirror: the scale keeps its sign.
                        let sign = if vm.get(id, field).number() < 0.0 { -1.0 } else { 1.0 };
                        vm.set(id, field, Value::Number(sign * wanted / base * 100.0))?;
                        self.scripted.insert(id);
                    }
                }
            }
        }
        Ok(())
    }
    fn remove(&mut self, vm: &mut Vm, id: usize) {
        if let Some(instance) = self.instances.remove(&id) {
            for child in instance.children.values().chain(instance.attached.values()) {
                self.remove(vm, *child);
            }
        }
        self.unconstructed.remove(&id);
        self.scripted.remove(&id);
        // Retired handles can remain referenced by ActionScript, but no longer
        // participate in timeline advancement or rendering.
        let _ = vm.set(id, "_visible", Value::Bool(false));
    }
    pub fn seek(
        &mut self,
        vm: &mut Vm,
        id: usize,
        frame: usize,
        nesting: usize,
    ) -> Result<(), String> {
        let instance = self
            .instances
            .get(&id)
            .ok_or("APT absent movie instance")?
            .clone();
        let character = self
            .characters
            .get(&instance.character)
            .ok_or("APT absent movie character")?;
        if frame >= character.frames.len() {
            return Err(format!(
                "APT frame {frame} outside character {}",
                character.id
            ));
        }
        let frame_count = character.frames.len();
        let actions: Vec<_> = character.frames[frame]
            .controls
            .iter()
            .filter(|c| c.type_name == "do_action" && c.actions_offset != 0)
            .map(|c| c.actions_offset)
            .collect();
        let list = self.states[&instance.character][frame].clone();
        let mut children = BTreeMap::new();
        for (depth, placement) in list.depths {
            let previous = instance.children.get(&depth).copied();
            if let Some(name) = previous
                .and_then(|old| self.instances.get(&old))
                .and_then(|i| i.placement.as_ref())
                .map(|p| p.name.clone())
            {
                if !name.is_empty() && name != placement.name {
                    vm.objects[id].fields.remove(&name);
                }
            }
            let child = if let Some(old) = previous.filter(|old| {
                self.instances
                    .get(old)
                    .is_some_and(|i| i.character == placement.character)
            }) {
                old
            } else {
                if let Some(old) = previous {
                    self.remove(vm, old);
                }
                self.create(vm, placement.character, Some(id), nesting + 1)?
            };
            let old = self.instances[&child].placement.as_ref();
            if old.is_none_or(|old| old.matrix != placement.matrix) && !self.scripted.contains(&child) {
                Self::apply_matrix(vm, child, placement.matrix)?;
            }
            if !placement.name.is_empty() {
                vm.set(id, &placement.name, Value::Object(child))?;
                vm.set(child, "_name", Value::Text(placement.name.clone()))?;
            }
            self.instances.get_mut(&child).unwrap().placement = Some(placement);
            children.insert(depth, child);
        }
        for (depth, child) in &instance.children {
            if !children.contains_key(depth) {
                if let Some(name) = self
                    .instances
                    .get(child)
                    .and_then(|i| i.placement.as_ref())
                    .map(|p| p.name.clone())
                {
                    if !name.is_empty() {
                        vm.objects[id].fields.remove(&name);
                    }
                }
                self.remove(vm, *child);
            }
        }
        let current = self.instances.get_mut(&id).unwrap();
        current.frame = frame;
        current.children = children;
        vm.set(id, "_currentframe", Value::Number((frame + 1) as f64))?;
        vm.set(id, "_totalframes", Value::Number(frame_count as f64))?;
        // A registered class constructs after its first frame's children exist
        // and before that frame's scripts, as AS2 does.
        self.queue_constructor(id);
        for offset in actions {
            self.pending.push_back(Pending::Action(id, offset));
        }
        Ok(())
    }
    pub fn advance(&mut self, vm: &mut Vm) -> Result<(), String> {
        let playing: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, i)| i.playing)
            .map(|(id, i)| (*id, i.character, i.frame))
            .collect();
        for (id, character, frame) in playing {
            if !self.instances.contains_key(&id) {
                continue;
            }
            let count = self.characters[&character].frames.len();
            if count > 1 {
                self.seek(vm, id, (frame + 1) % count, 0)?;
            }
        }
        Ok(())
    }
    fn frame_of(&self, id: usize, target: &Value, method: &str) -> Result<usize, String> {
        let c = &self.characters[&self.instances[&id].character];
        // A string names a frame label, even one that looks like a number:
        // the scoring HUD's trick name plays labels "0", "0-1" and "1-2".
        // Only a string matching no label falls back to a frame number.
        let labelled = |label: &str| {
            c.frames.iter().position(|f| {
                f.controls.iter().any(|control| {
                    control.type_name == "frame_label" && control.label.as_deref() == Some(label)
                })
            })
        };
        Ok(match target {
            Value::Text(label) if labelled(label).is_some() => labelled(label).unwrap(),
            Value::Text(label) if label.parse::<f64>().is_err() => {
                return Err(format!("APT character {} lacks label {label}", c.id));
            }
            value => {
                let frame = value.number();
                if !frame.is_finite() || frame < 0.0 {
                    return Err(format!(
                        "Invalid APT frame {frame} in {} on character {}",
                        method, c.id
                    ));
                }
                // The shipped line timer deliberately requests zero at
                // full capacity. Frame zero addresses the first frame.
                ((frame as usize).saturating_sub(1)).min(c.frames.len().saturating_sub(1))
            }
        })
    }
    fn attach(
        &mut self,
        vm: &mut Vm,
        parent: usize,
        character: i32,
        name: &str,
        depth: i32,
        init: Option<usize>,
    ) -> Result<usize, String> {
        if let Some(old) = self.instances[&parent].attached.get(&depth).copied() {
            self.remove(vm, old);
        }
        let child = self.create(vm, character, Some(parent), 1)?;
        if let Some(init) = init {
            for (key, value) in vm.objects[init].fields.clone() {
                vm.set(child, key, value)?;
            }
        }
        vm.set(child, "_name", Value::Text(name.into()))?;
        vm.set(child, "_root", Value::Object(self.root))?;
        vm.set(parent, name, Value::Object(child))?;
        self.instances.get_mut(&parent).unwrap().attached.insert(depth, child);
        Ok(child)
    }
    fn depth_of(&self, id: usize) -> Option<(usize, i32, bool)> {
        let parent = self.instances.get(&id)?.parent?;
        let p = self.instances.get(&parent)?;
        p.attached
            .iter()
            .map(|(d, c)| (*d, *c, true))
            .chain(p.children.iter().map(|(d, c)| (*d, *c, false)))
            .find(|(_, c, _)| *c == id)
            .map(|(d, _, attached)| (parent, d, attached))
    }
    /// MovieClip methods implemented by the player. `None` means the method is
    /// not a clip method and the host should try its own bindings.
    pub fn method(
        &mut self,
        vm: &mut Vm,
        id: usize,
        method: &str,
        args: &[Value],
    ) -> Result<Option<Value>, String> {
        if !self.instances.contains_key(&id) {
            return Ok(None);
        }
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        Ok(Some(match method {
            "stop" => {
                self.instances.get_mut(&id).unwrap().playing = false;
                Value::Undefined
            }
            "play" => {
                self.instances.get_mut(&id).unwrap().playing = true;
                Value::Undefined
            }
            "gotoAndPlay" | "gotoAndStop" => {
                let target = args.first().ok_or("APT goto requires frame")?;
                match self.frame_of(id, target, method) {
                    Ok(frame) => {
                        self.seek(vm, id, frame, 0)?;
                        self.instances.get_mut(&id).unwrap().playing = method == "gotoAndPlay";
                    }
                    // The player ignores a goto to a label the clip lacks.
                    Err(_) if matches!(target, Value::Text(_)) => {}
                    Err(error) => return Err(error),
                }
                Value::Undefined
            }
            "nextFrame" | "prevFrame" => {
                let instance = &self.instances[&id];
                let count = self.characters[&instance.character].frames.len();
                if count > 0 {
                    let frame = if method == "nextFrame" {
                        (instance.frame + 1).min(count - 1)
                    } else {
                        instance.frame.saturating_sub(1)
                    };
                    self.seek(vm, id, frame, 0)?;
                }
                self.instances.get_mut(&id).unwrap().playing = false;
                Value::Undefined
            }
            "attachMovie" => {
                let linkage = arg(0).text();
                // As in the shipped player, an unknown symbol attaches nothing
                // and returns undefined (the Trick Guide's gesture items ask
                // for "g_" + their gesture before one is set).
                // Linkage names are case-insensitive in AS2 (the exporter keeps
                // them lower case; the game asks for "g_grind_5_0_Grind").
                let found = self.linkage.get(&linkage).or_else(|| {
                    self.linkage.iter().find(|(name, _)| name.eq_ignore_ascii_case(&linkage)).map(|(_, c)| c)
                });
                let Some(&character) = found else {
                    return Ok(Some(Value::Undefined));
                };
                let init = match arg(3) {
                    Value::Object(o) => Some(o),
                    _ => None,
                };
                Value::Object(self.attach(vm, id, character, &arg(1).text(), arg(2).number() as i32, init)?)
            }
            "createEmptyMovieClip" => {
                Value::Object(self.attach(vm, id, EMPTY_CLIP, &arg(0).text(), arg(1).number() as i32, None)?)
            }
            // (name, depth, x, y, width, height): a dynamic field in the
            // movie's own text style (its first authored field), sized as asked.
            "createTextField" => {
                let template = self
                    .characters
                    .values()
                    .find(|c| c.text.is_some() && c.id >= 0)
                    .cloned()
                    .ok_or("APT createTextField in a movie without text")?;
                let character = self.characters.keys().next().copied().unwrap_or(0).min(EMPTY_CLIP) - 1;
                let (width, height) = (arg(4).number() as f32, arg(5).number() as f32);
                let mut text = template.text.clone().unwrap_or_default();
                text["initial_text"] = serde_json::Value::String(String::new());
                self.characters.insert(
                    character,
                    Character { id: character, type_name: "text".into(), frames: Vec::new(), text: Some(text), bounds: Some([0., 0., width, height]) },
                );
                self.states.insert(character, Vec::new());
                let field = self.attach(vm, id, character, &arg(0).text(), arg(1).number() as i32, None)?;
                vm.set(field, "_x", Value::Number(arg(2).number()))?;
                vm.set(field, "_y", Value::Number(arg(3).number()))?;
                self.scripted.insert(field);
                Value::Object(field)
            }
            "removeTextField" | "removeMovieClip" => {
                if let Some((parent, depth, true)) = self.depth_of(id) {
                    self.instances.get_mut(&parent).unwrap().attached.remove(&depth);
                    if let Value::Text(name) = vm.get(id, "_name") {
                        vm.objects[parent].fields.remove(&name);
                    }
                    self.remove(vm, id);
                }
                Value::Undefined
            }
            "getDepth" => Value::Number(
                self.depth_of(id).map_or(0, |(_, d, attached)| if attached { d } else { d - TIMELINE_DEPTH }) as f64,
            ),
            "getNextHighestDepth" => {
                let instance = &self.instances[&id];
                let top = instance
                    .attached
                    .keys()
                    .copied()
                    .chain(instance.children.keys().map(|d| d - TIMELINE_DEPTH))
                    .max()
                    .unwrap_or(-1);
                Value::Number((top.max(-1) + 1) as f64)
            }
            "swapDepths" => {
                if let Some((parent, depth, true)) = self.depth_of(id) {
                    let target = arg(0).number() as i32;
                    let p = self.instances.get_mut(&parent).unwrap();
                    let other = p.attached.remove(&target);
                    p.attached.remove(&depth);
                    p.attached.insert(target, id);
                    if let Some(other) = other {
                        p.attached.insert(depth, other);
                    }
                }
                Value::Undefined
            }
            _ => return Ok(None),
        }))
    }
}

/// A host that owns the movie its scripts run against.
pub trait MovieHost: Host {
    fn movie(&mut self) -> &mut Movie;
}
