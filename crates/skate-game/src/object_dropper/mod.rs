//! Skate 3's Object Dropper. The props are the game's own dynamic-object
//! catalog (recipe/dynamic in parkassets.big, exported by
//! tools/asset_pipeline/dropper.py): render meshes, RW ClusteredMesh collision
//! with native edge codes, and textures. A placed prop joins the physics world
//! as an authored ground cluster does, so the board rides it like the map.
//!
//! In a session every player's props are an application record ("mp:props"),
//! so everyone sees and skates everyone's props. The host sets how many each
//! player may have at once (0..=50, default 5) and publishes it ("mp:props-limit");
//! senders and receivers both hold to it. Alone there is no limit.
use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

mod screen;
pub(crate) use screen::{camera_override, ObjectDropperUiPlugin};

pub(crate) const PROPS_KEY: &str = "mp:props";
pub(crate) const LIMIT_KEY: &str = "mp:props-limit";
pub(crate) const DEFAULT_LIMIT: u8 = 5;
pub(crate) const MAX_LIMIT: u8 = 50;
/// Alone there is no limit; this only keeps memory and physics bounded.
const SOLO_CAP: usize = 500;
const RECORD_VERSION: u8 = 1;
const PROP_BYTES: usize = 18;

static OPEN: AtomicBool = AtomicBool::new(false);
static CLOSED: AtomicBool = AtomicBool::new(false);
static OPEN_REQUEST: AtomicBool = AtomicBool::new(false);
static HOST_LIMIT: AtomicU8 = AtomicU8::new(DEFAULT_LIMIT);
/// The verify hook's push (SKATE_VERIFY_DROPPER=.../ride).
static VERIFY_PUSH: AtomicBool = AtomicBool::new(false);

/// Verify only: tap push (A) every half second while the hook asks.
pub(crate) fn verify_push(tick: u64, values: &mut [f32; 18]) {
    if VERIFY_PUSH.load(Ordering::Relaxed) && tick % 30 < 8 {
        values[16] = 1.0;
    }
}

/// The dropper is on screen: gameplay input and the pause menu wait.
pub(crate) fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}
/// Open, or closed this frame (its Escape must not open the pause menu).
pub(crate) fn has_keys() -> bool {
    is_open() || CLOSED.load(Ordering::Relaxed)
}
/// The pause menu's "Object Dropper" row.
pub(crate) fn request_open() {
    OPEN_REQUEST.store(true, Ordering::Relaxed);
}
/// The limit this player sets for the session while hosting.
pub(crate) fn host_limit() -> u8 {
    HOST_LIMIT.load(Ordering::Relaxed)
}
pub(crate) fn set_host_limit(limit: u8) {
    HOST_LIMIT.store(limit.min(MAX_LIMIT), Ordering::Relaxed);
}

// ---- catalog --------------------------------------------------------------

#[derive(serde::Deserialize)]
struct CatalogFile {
    categories: Vec<CategoryFile>,
}
#[derive(serde::Deserialize)]
struct CategoryFile {
    name: String,
    items: Vec<ItemFile>,
}
#[derive(serde::Deserialize)]
struct ItemFile {
    id: String,
    name: String,
}

pub(crate) struct Item {
    /// The model's arena ID (16 hex digits), its file name.
    pub key: String,
    /// The arena ID folded to 32 bits: what the network carries. (The low
    /// half alone is shared: it is the asset group, not the model.)
    pub id: u32,
    pub name: String,
}
pub(crate) struct Category {
    /// The recipe component without its DMO_ prefix (Barriers, Ramps...).
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Resource, Default)]
pub(crate) struct Catalog {
    dir: PathBuf,
    pub categories: Vec<Category>,
    keys: HashMap<u32, String>,
}
impl Catalog {
    fn load(asset_root: &std::path::Path) -> Result<Self, String> {
        let dir = asset_root.join("private/dropper");
        let bytes = skate_data::vfs::read(dir.join("catalog.json")).map_err(|e| format!("catalog.json: {e}"))?;
        let file: CatalogFile = serde_json::from_slice(&bytes).map_err(|e| format!("catalog.json: {e}"))?;
        let mut keys = HashMap::new();
        let mut categories = Vec::new();
        for category in file.categories {
            let mut items = Vec::new();
            for item in category.items {
                let Ok(full) = u64::from_str_radix(&item.id, 16) else { continue };
                let id = (full ^ (full >> 32)) as u32;
                if keys.insert(id, item.id.clone()).is_some() {
                    warn!("Object Dropper: two props share network ID {id:08x}; keeping the first");
                    continue;
                }
                items.push(Item { key: item.id, id, name: item.name });
            }
            if !items.is_empty() {
                categories.push(Category { name: category.name, items });
            }
        }
        Ok(Self { dir, categories, keys })
    }
    pub(crate) fn available(&self) -> bool {
        !self.categories.is_empty()
    }
    fn key(&self, id: u32) -> Option<&str> {
        self.keys.get(&id).map(String::as_str)
    }
}

// ---- models ---------------------------------------------------------------

#[derive(serde::Deserialize)]
struct ObjectFile {
    meshes: Vec<MeshFile>,
    collision: Vec<TriangleFile>,
    #[serde(default)]
    grind_splines: Vec<SplineFile>,
}
/// A native grind spline: its header and 120-byte big-endian cubic segments.
#[derive(serde::Deserialize)]
struct SplineFile {
    spline_id: String,
    type_signature: String,
    #[serde(default)]
    flags: u32,
    #[serde(default)]
    trailing_word: u32,
    #[serde(default)]
    closed: bool,
    native_segment_payloads: Vec<String>,
}
#[derive(serde::Deserialize)]
struct MeshFile {
    positions: Vec<f32>,
    #[serde(default)]
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u32>,
    texture: Option<String>,
}
#[derive(serde::Deserialize)]
struct TriangleFile {
    points: [[f32; 3]; 3],
    surface: u32,
    edges: Option<[u8; 3]>,
}

pub(crate) type Triangle = ([[f32; 3]; 3], u16, Option<[u8; 3]>);

/// A prop's grind spline, segments as the 30 words of the native layout:
/// cubic rows t^3, t^2, t, 1 (words 0..16), then bounds min (20) and max (24).
#[derive(Clone)]
pub(crate) struct Spline {
    id: u64,
    kind: u64,
    flags: u32,
    trailing: u32,
    closed: bool,
    segments: Vec<[f32; 30]>,
}
impl Spline {
    fn parse(file: &SplineFile) -> Option<Self> {
        let hex = |t: &str| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok();
        let mut segments = Vec::new();
        for payload in &file.native_segment_payloads {
            if payload.len() != 240 || !payload.is_ascii() {
                return None;
            }
            let bytes: Vec<u8> = (0..120).map(|i| u8::from_str_radix(&payload[i * 2..i * 2 + 2], 16).ok()).collect::<Option<_>>()?;
            let segment: [f32; 30] = std::array::from_fn(|w| f32::from_be_bytes(bytes[w * 4..w * 4 + 4].try_into().unwrap()));
            if !segment[..28].iter().all(|v| v.is_finite()) {
                return None;
            }
            segments.push(segment);
        }
        if segments.is_empty() {
            return None;
        }
        Some(Self {
            id: hex(&file.spline_id)?,
            kind: hex(&file.type_signature)?,
            flags: file.flags,
            trailing: file.trailing_word,
            closed: file.closed,
            segments,
        })
    }
    /// The same curve moved rigidly: the cubic's vector rows turn, its
    /// constant row turns and moves; the bounds enclose the moved box.
    fn moved(&self, turn: Quat, shift: Vec3) -> Self {
        let segments = self
            .segments
            .iter()
            .map(|s| {
                let mut o = *s;
                let v = |i: usize| Vec3::new(s[i], s[i + 1], s[i + 2]);
                for row in [0, 4, 8] {
                    o[row..row + 3].copy_from_slice(&(turn * v(row)).to_array());
                }
                o[12..15].copy_from_slice(&(turn * v(12) + shift).to_array());
                let (low, high) = (v(20), v(24));
                let corners: Vec<Vec3> = (0..8)
                    .map(|c| {
                        let corner = Vec3::new(
                            if c & 1 == 0 { low.x } else { high.x },
                            if c & 2 == 0 { low.y } else { high.y },
                            if c & 4 == 0 { low.z } else { high.z },
                        );
                        turn * corner + shift
                    })
                    .collect();
                let (min, max) = bounds(&corners);
                o[20..23].copy_from_slice(&min.to_array());
                o[24..27].copy_from_slice(&max.to_array());
                o
            })
            .collect();
        Self { segments, ..self.clone() }
    }
    /// As a package rail (.skate layout: little endian, u64 halves swapped).
    fn rail(&self, name: String) -> skate_data::skate_map::Rail {
        let mut raw = Vec::with_capacity(28 + self.segments.len() * 120);
        let header = [self.id as u32, (self.id >> 32) as u32, self.kind as u32, (self.kind >> 32) as u32, self.flags, self.trailing];
        for w in header.into_iter().chain([self.segments.len() as u32]) {
            raw.extend(w.to_le_bytes());
        }
        for s in &self.segments {
            for v in s {
                raw.extend(v.to_bits().to_le_bytes());
            }
        }
        skate_data::skate_map::Rail { name, closed: self.closed, points: Vec::new(), native: Some(raw) }
    }
}

/// A prop ready to place: standing on y = 0, centred on its footprint.
pub(crate) struct Model {
    parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    triangles: Vec<Triangle>,
    splines: Vec<Spline>,
}

enum Load {
    Pending,
    Ready(Arc<Model>),
    Failed,
}

type Fetched = (String, Result<(Vec<u8>, Vec<(String, Vec<u8>)>), String>);

#[derive(Resource, Default)]
pub(crate) struct Models {
    loads: HashMap<String, Load>,
    inbox: Arc<Mutex<Vec<Fetched>>>,
    materials: HashMap<String, Handle<StandardMaterial>>,
    plain: Option<Handle<StandardMaterial>>,
}
impl Models {
    /// The model, asking for it the first time.
    pub(crate) fn get(&mut self, catalog: &Catalog, key: &str) -> Option<Arc<Model>> {
        match self.loads.get(key) {
            Some(Load::Ready(model)) => return Some(model.clone()),
            Some(_) => return None,
            None => {}
        }
        self.loads.insert(key.to_owned(), Load::Pending);
        let path = catalog.dir.join("objects").join(format!("{key}.json"));
        let textures = catalog.dir.join("textures");
        fetch(key.to_owned(), path, textures, self.inbox.clone());
        None
    }
    fn failed(&self, key: &str) -> bool {
        matches!(self.loads.get(key), Some(Load::Failed))
    }
}

fn texture_names(json: &[u8]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Only {
        meshes: Vec<OnlyMesh>,
    }
    #[derive(serde::Deserialize)]
    struct OnlyMesh {
        texture: Option<String>,
    }
    let mut names: Vec<String> = serde_json::from_slice::<Only>(json)
        .map(|o| o.meshes.into_iter().filter_map(|m| m.texture).collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(key: String, path: PathBuf, textures: PathBuf, inbox: Arc<Mutex<Vec<Fetched>>>) {
    let result = skate_data::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display())).map(|json| {
        let images = texture_names(&json)
            .into_iter()
            .filter_map(|name| skate_data::vfs::read(textures.join(&name)).ok().map(|bytes| (name, bytes)))
            .collect();
        (json, images)
    });
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((key, result));
}

/// The page holds the props in their own pack and hands each over on request.
#[cfg(target_arch = "wasm32")]
fn fetch(key: String, path: PathBuf, textures: PathBuf, inbox: Arc<Mutex<Vec<Fetched>>>) {
    let slash = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    let (path, textures) = (slash(&path), slash(&textures));
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let json = crate::web::host_read(&path).await?.ok_or_else(|| format!("{path}: not in the game data"))?;
            let mut images = Vec::new();
            for name in texture_names(&json) {
                if let Ok(Some(bytes)) = crate::web::host_read(&format!("{textures}/{name}")).await {
                    images.push((name, bytes));
                }
            }
            Ok::<_, String>((json, images))
        }
        .await;
        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((key, result));
    });
}

/// Rails ship standing on end (their spline runs along Y); lay those down.
fn upright(points: &[Vec3]) -> Quat {
    let (min, max) = bounds(points);
    let size = max - min;
    if min.y < -0.5 && size.y > 2.5 * size.x.max(size.z) {
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    }
}

fn bounds(points: &[Vec3]) -> (Vec3, Vec3) {
    points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)))
}

fn build_model(
    json: &[u8],
    images: Vec<(String, Vec<u8>)>,
    models: &mut Models,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    textures: &mut Assets<Image>,
) -> Result<Model, String> {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let file: ObjectFile = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let points: Vec<Vec3> = file.collision.iter().flat_map(|t| t.points.map(Vec3::from_array)).collect();
    if points.is_empty() {
        return Err("prop has no collision".into());
    }
    // Stand the prop on y = 0 and centre its footprint, so it turns in place.
    let turn = upright(&points);
    let turned: Vec<Vec3> = points.iter().map(|p| turn * *p).collect();
    let (min, max) = bounds(&turned);
    let offset = Vec3::new(-(min.x + max.x) * 0.5, -min.y, -(min.z + max.z) * 0.5);
    let place = |p: [f32; 3]| (turn * Vec3::from_array(p) + offset).to_array();
    let triangles = file
        .collision
        .iter()
        .map(|t| (t.points.map(place), t.surface.min(u32::from(u16::MAX)) as u16, t.edges))
        .collect();
    for (name, png) in images {
        if models.materials.contains_key(&name) {
            continue;
        }
        if let Some(image) = crate::skater_import::image_from_png(&png) {
            let handle = textures.add(image);
            let material = materials.add(StandardMaterial {
                base_color_texture: Some(handle),
                perceptual_roughness: 0.8,
                alpha_mode: AlphaMode::Mask(0.5),
                ..default()
            });
            models.materials.insert(name, material);
        }
    }
    let plain = models
        .plain
        .get_or_insert_with(|| materials.add(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), perceptual_roughness: 0.8, ..default() }))
        .clone();
    let mut parts = Vec::new();
    for mesh in file.meshes {
        let count = mesh.positions.len() / 3;
        if count == 0 || mesh.uvs.len() / 2 != count || mesh.indices.iter().any(|&i| i as usize >= count) {
            continue;
        }
        let positions: Vec<[f32; 3]> = mesh.positions.chunks_exact(3).map(|p| place([p[0], p[1], p[2]])).collect();
        let uvs: Vec<[f32; 2]> = mesh.uvs.chunks_exact(2).map(|u| [u[0], u[1]]).collect();
        let mut built = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        built.insert_indices(Indices::U32(mesh.indices));
        if mesh.normals.len() == count * 3 {
            let normals: Vec<[f32; 3]> = mesh.normals.chunks_exact(3).map(|n| (turn * Vec3::new(n[0], n[1], n[2])).to_array()).collect();
            built.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        } else {
            built.compute_smooth_normals();
        }
        let material = mesh.texture.as_ref().and_then(|t| models.materials.get(t)).cloned().unwrap_or_else(|| plain.clone());
        parts.push((meshes.add(built), material));
    }
    let splines = file.grind_splines.iter().filter_map(Spline::parse).map(|s| s.moved(turn, offset)).collect();
    Ok(Model { parts, triangles, splines })
}

fn receive_models(
    mut models: ResMut<Models>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let arrived = std::mem::take(&mut *models.inbox.lock().unwrap_or_else(|e| e.into_inner()));
    for (key, result) in arrived {
        let load = match result.and_then(|(json, textures)| build_model(&json, textures, &mut models, &mut meshes, &mut materials, &mut images)) {
            Ok(model) => Load::Ready(Arc::new(model)),
            Err(error) => {
                warn!("Object Dropper: prop {key} unavailable ({error})");
                Load::Failed
            }
        };
        models.loads.insert(key, load);
    }
}

// ---- placed props ---------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Prop {
    pub id: u32,
    pub pos: Vec3,
    pub yaw: f32,
}
impl Prop {
    fn transform(&self) -> Transform {
        Transform::from_translation(self.pos).with_rotation(Quat::from_rotation_y(self.yaw))
    }
}

/// [version][count] then per prop: [u32 id][f32 x, y, z][u16 yaw], little endian.
pub(crate) fn encode(props: &[Prop]) -> Vec<u8> {
    let props = &props[..props.len().min(usize::from(MAX_LIMIT))];
    let mut out = Vec::with_capacity(2 + props.len() * PROP_BYTES);
    out.extend([RECORD_VERSION, props.len() as u8]);
    for prop in props {
        out.extend(prop.id.to_le_bytes());
        for v in prop.pos.to_array() {
            out.extend(v.to_le_bytes());
        }
        let turn = prop.yaw.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        out.extend(((turn * 65536.).round() as u32 as u16).to_le_bytes());
    }
    out
}

/// A player's props, or nothing if the record is not a valid one.
pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<Prop>> {
    let (&version, rest) = bytes.split_first()?;
    let (&count, rest) = rest.split_first()?;
    if version != RECORD_VERSION || count > MAX_LIMIT || rest.len() != usize::from(count) * PROP_BYTES {
        return None;
    }
    let f = |b: &[u8]| f32::from_le_bytes(b.try_into().unwrap());
    rest.chunks_exact(PROP_BYTES)
        .map(|c| {
            let pos = Vec3::new(f(&c[4..8]), f(&c[8..12]), f(&c[12..16]));
            let yaw = f32::from(u16::from_le_bytes([c[16], c[17]])) / 65536. * std::f32::consts::TAU;
            (pos.is_finite() && pos.abs().max_element() < 100_000.)
                .then_some(Prop { id: u32::from_le_bytes(c[..4].try_into().unwrap()), pos, yaw })
        })
        .collect()
}

#[derive(Component)]
struct PropRoot;

#[derive(Resource, Default)]
pub(crate) struct Props {
    /// This player's props, oldest first.
    pub mine: Vec<Prop>,
    /// Everyone else's, as their records last said.
    remote: Vec<(u64, Vec<Prop>)>,
    /// How many this player may have (None: alone, no limit).
    pub limit: Option<usize>,
    /// Seconds left on the "limit reached" notice.
    pub warning: f32,
    signature: u64,
    built_triangles: usize,
    entities: Vec<Entity>,
    map: Option<PathBuf>,
}
impl Props {
    /// Add a prop; at the limit the oldest makes way, as in Skate 3.
    pub(crate) fn add(&mut self, prop: Prop) -> bool {
        let cap = self.limit.unwrap_or(SOLO_CAP);
        if cap == 0 {
            self.warning = 3.;
            return false;
        }
        if self.mine.len() >= cap {
            let excess = self.mine.len() + 1 - cap;
            self.mine.drain(..excess);
            self.warning = 3.;
        }
        self.mine.push(prop);
        true
    }
    /// Remove this player's prop nearest `at`, within `reach` metres.
    pub(crate) fn remove_near(&mut self, at: Vec3, reach: f32) -> bool {
        let nearest = self
            .mine
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.pos.distance(at)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = nearest {
            self.mine.remove(i);
        }
        nearest.is_some()
    }
}

/// The session's limit for this player: the host's own setting while hosting,
/// else what the host published (the default until it arrives).
fn session_limit(net: &crate::multiplayer::Multiplayer) -> Option<usize> {
    let (_, hosting, _) = net.session_clock()?;
    if hosting {
        return Some(usize::from(host_limit()));
    }
    let host = net.host_actor();
    let published = net
        .application_values(LIMIT_KEY)
        .into_iter()
        .find(|(id, _)| *id == host)
        .and_then(|(_, value)| value.first().copied());
    Some(usize::from(published.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)))
}

fn sync(
    mut props: ResMut<Props>,
    net: Option<ResMut<crate::multiplayer::Multiplayer>>,
    config: Res<crate::config::Config>,
    mut changed: MessageReader<crate::map_transition::WorldChanged>,
    time: Res<Time<Real>>,
) {
    props.warning = (props.warning - time.delta_secs()).max(0.);
    if changed.read().count() > 0 {
        // Props belong to the map they were placed on.
        if props.map != config.map_path {
            props.mine.clear();
        }
        props.signature = 0;
    }
    props.map.clone_from(&config.map_path);
    let Some(mut net) = net else {
        props.limit = None;
        props.remote.clear();
        return;
    };
    let limit = session_limit(&net);
    if limit != props.limit {
        info!("OBJECT_DROPPER_LIMIT {limit:?} (players' props at once; None alone)");
    }
    props.limit = limit;
    if let Some(limit) = props.limit {
        if props.mine.len() > limit {
            let excess = props.mine.len() - limit;
            props.mine.drain(..excess);
        }
        let record = encode(&props.mine);
        net.publish_application(PROPS_KEY, record);
        if net.session_clock().is_some_and(|c| c.1) {
            net.publish_application(LIMIT_KEY, vec![host_limit()]);
        }
        let cap = limit.max(1);
        props.remote = net
            .application_values(PROPS_KEY)
            .into_iter()
            .filter_map(|(id, value)| decode(&value).map(|mut p| {
                p.truncate(cap);
                (id, p)
            }))
            .collect();
        props.remote.sort_by_key(|(id, _)| *id);
    } else {
        props.remote.clear();
    }
}

/// Respawn the props and rebuild their collision when the set changes, a
/// model arrives, or the physics world was replaced (a new map).
#[allow(clippy::too_many_arguments)]
fn build(
    mut commands: Commands,
    mut props: ResMut<Props>,
    catalog: Res<Catalog>,
    mut models: ResMut<Models>,
    physics: Option<ResMut<crate::physics::GamePhysics>>,
    skater: Option<ResMut<crate::physics::SkaterRuntime>>,
) {
    let Some(mut physics) = physics else { return };
    let all: Vec<Prop> = props.mine.iter().chain(props.remote.iter().flat_map(|(_, p)| p.iter())).copied().collect();
    let ready: Vec<(Prop, Option<Arc<Model>>)> = all
        .iter()
        .map(|p| (*p, catalog.key(p.id).map(str::to_owned).and_then(|k| models.get(&catalog, &k))))
        .collect();
    let signature = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (p, m) in &ready {
            (p.id, p.pos.to_array().map(f32::to_bits), p.yaw.to_bits(), m.is_some()).hash(&mut h);
        }
        h.finish() | 1
    };
    let world_replaced = physics.world_triangles().len() != props.built_triangles;
    if signature == props.signature && !world_replaced {
        return;
    }
    for entity in props.entities.drain(..) {
        commands.entity(entity).despawn();
    }
    let mut collision = Vec::new();
    let mut rails = Vec::new();
    for (prop, model) in &ready {
        let Some(model) = model else { continue };
        let transform = prop.transform();
        let root = commands.spawn((transform, Visibility::default(), PropRoot)).id();
        for (mesh, material) in &model.parts {
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::default(), ChildOf(root)));
        }
        props.entities.push(root);
        for spline in &model.splines {
            let name = format!("dropper_{}_{}", props.entities.len(), rails.len());
            rails.push(spline.moved(transform.rotation, transform.translation).rail(name));
        }
        collision.push(
            model
                .triangles
                .iter()
                .map(|(points, surface, edges)| (points.map(|p| transform.transform_point(Vec3::from_array(p)).to_array()), *surface, *edges))
                .collect::<Vec<_>>(),
        );
    }
    physics.set_placed(&collision);
    match physics.set_placed_rails(&rails) {
        // The air trajectory keeps its own handle on the grind paths.
        Ok(grind) => {
            if let Some(mut skater) = skater {
                skater.trajectory.bind_grind_world(grind);
            }
        }
        Err(error) => warn!("Object Dropper: props cannot be ground ({error})"),
    }
    props.built_triangles = physics.world_triangles().len();
    props.signature = signature;
    let failed = all.iter().filter(|p| catalog.key(p.id).is_none_or(|k| models.failed(k))).count();
    info!(
        "OBJECT_DROPPER_BUILT props={} ready={} unavailable={failed} triangles={} grind_splines={}",
        all.len(),
        collision.len(),
        collision.iter().map(Vec::len).sum::<usize>(),
        rails.len()
    );
}

fn setup(mut commands: Commands, config: Res<crate::config::Config>) {
    // Verify only: the limit this player sets if it hosts.
    if let Some(limit) = std::env::var("SKATE_VERIFY_DROPPER_LIMIT").ok().and_then(|v| v.parse().ok()) {
        set_host_limit(limit);
    }
    let catalog = Catalog::load(&config.asset_root).unwrap_or_else(|error| {
        info!("Object Dropper unavailable ({error}); run tools/asset_pipeline/dropper.py");
        Catalog::default()
    });
    if catalog.available() {
        info!(
            "OBJECT_DROPPER_READY categories={} props={}",
            catalog.categories.len(),
            catalog.categories.iter().map(|c| c.items.len()).sum::<usize>()
        );
    }
    commands.insert_resource(catalog);
}

pub(crate) struct ObjectDropperPlugin;
impl Plugin for ObjectDropperPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Models>()
            .init_resource::<Props>()
            .add_systems(Startup, setup)
            .add_systems(Update, (receive_models, sync, build).chain())
            .add_plugins(ObjectDropperUiPlugin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_reject_garbage() {
        let props: Vec<Prop> = (0..50)
            .map(|i| Prop { id: 0x5c4e_4000 + i, pos: Vec3::new(i as f32, 1.5, -3.25), yaw: 1.25 })
            .collect();
        let record = encode(&props);
        assert!(record.len() <= skate_net::lobby::MAX_APP_VALUE, "{}", record.len());
        let back = decode(&record).unwrap();
        assert_eq!(back.len(), 50);
        assert_eq!(back[7].id, props[7].id);
        assert_eq!(back[7].pos, props[7].pos);
        assert!((back[7].yaw - 1.25).abs() < 1e-3);
        // Too many, wrong length, wrong version, non-finite.
        let mut many = record.clone();
        many[1] = 51;
        assert!(decode(&many).is_none());
        assert!(decode(&record[..record.len() - 1]).is_none());
        assert!(decode(&[2, 0]).is_none());
        let mut nan = encode(&props[..1]);
        nan[6..10].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&nan).is_none());
        assert_eq!(decode(&[1, 0]).unwrap(), vec![]);
    }

    #[test]
    fn the_limit_keeps_the_newest() {
        let mut props = Props { limit: Some(2), ..default() };
        for i in 0..4 {
            assert!(props.add(Prop { id: i, pos: Vec3::ZERO, yaw: 0. }));
        }
        assert_eq!(props.mine.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 3]);
        props.limit = Some(0);
        assert!(!props.add(Prop { id: 9, pos: Vec3::ZERO, yaw: 0. }));
        props.limit = None;
        for i in 0..10 {
            props.add(Prop { id: 10 + i, pos: Vec3::X * i as f32, yaw: 0. });
        }
        assert_eq!(props.mine.len(), 12);
        assert!(props.remove_near(Vec3::X * 4.2, 1.));
        assert_eq!(props.mine.len(), 11);
        assert!(!props.remove_near(Vec3::X * 100., 1.));
    }
}
