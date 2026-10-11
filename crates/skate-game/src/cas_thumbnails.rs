//! Item thumbnails for the Create-a-Skater screen, rendered in the game as
//! Skate 3 renders them: each part model (in its bind pose, with its own
//! material) photographed offscreen, one at a time, into a small image that
//! the screen then shows. Images are kept for the session.
use crate::customiser_material::SkaterMaterial;
use crate::customiser_parts::Parts;
use bevy::{
    camera::{RenderTarget, primitives::Aabb, visibility::RenderLayers},
    prelude::*,
    render::render_resource::TextureFormat,
};
use std::collections::{HashMap, VecDeque};

/// The studio's own render layer.
const LAYER: usize = 26;
const SIZE: u32 = 160;
/// Far from any world, so nothing else shares the shot.
const SET: Vec3 = Vec3::new(0., -5000., 0.);
/// Items photographed at once, each on its own spot far from the others.
const PARALLEL: usize = 6;
const SPACING: f32 = 400.;

#[derive(Resource, Default)]
pub(crate) struct Thumbnails {
    done: HashMap<(String, String), Handle<Image>>,
    queue: VecDeque<(String, String)>,
    shoots: Vec<Shoot>,
    /// The studio's one light, while anything is being photographed.
    light: Option<Entity>,
    /// Frames the studio has been running: its first shots wait for the
    /// renderer to build its pipelines for the studio camera.
    age: u32,
}

struct Shoot {
    key: (String, String),
    image: Handle<Image>,
    scene: Entity,
    camera: Entity,
    spot: usize,
    /// Frames since the scene appeared (it is framed, then rendered twice).
    frames: u32,
    framed: bool,
}

impl Thumbnails {
    /// The thumbnail of a part in a material, once photographed; asks for it
    /// otherwise.
    pub(crate) fn get(&mut self, asset: &str, material: &str) -> Option<Handle<Image>> {
        let key = (asset.to_owned(), material.to_owned());
        if let Some(image) = self.done.get(&key) {
            return Some(image.clone());
        }
        if !self.shoots.iter().any(|s| s.key == key) && !self.queue.contains(&key) {
            // Newest first: the browser asks for what is on screen now.
            self.queue.push_front(key);
            self.queue.truncate(30);
        }
        None
    }
}

pub(crate) struct ThumbnailsPlugin;
impl Plugin for ThumbnailsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Thumbnails>().add_systems(Update, shoot);
    }
}

#[allow(clippy::too_many_arguments)]
fn shoot(
    mut commands: Commands,
    mut studio: ResMut<Thumbnails>,
    mut parts: ResMut<Parts>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SkaterMaterial>>,
    children: Query<&Children>,
    meshes: Query<(Entity, &Aabb, &GlobalTransform), With<Mesh3d>>,
    mut cameras: Query<(&mut Camera, &mut Transform)>,
) {
    let studio = &mut *studio;
    if !studio.shoots.is_empty() {
        studio.age = studio.age.saturating_add(1);
    }
    // Frames each shot keeps rendering: long while the studio is new (its
    // pipelines compile in the background), then a few.
    let hold = if studio.age < 900 { 240 } else { 8 };
    let mut finished = Vec::new();
    for (n, shot) in studio.shoots.iter_mut().enumerate() {
        // Every mesh of the part: the studio's layer and the part's material.
        let mut found = Vec::new();
        let mut stack = vec![shot.scene];
        while let Some(e) = stack.pop() {
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
            if let Ok(mesh) = meshes.get(e) {
                found.push(mesh);
            }
        }
        if found.is_empty() {
            shot.frames += 1;
            // A model that never appears is given up on.
            if shot.frames > 600 {
                finished.push((n, false));
            }
            continue;
        }
        if !shot.framed {
            // The material as an outfit wears it (profile_material sets the
            // skin, opacity and tint the raw material lacks).
            let material = parts
                .profile_material(&shot.key.0, &shot.key.1, &serde_json::json!({}), &materials)
                .map(|m| materials.add(m))
                .or_else(|| parts.material_handle(&shot.key.1));
            let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for (e, aabb, transform) in &found {
                let mut entity = commands.entity(*e);
                entity.insert(RenderLayers::layer(LAYER));
                if let Some(material) = &material {
                    entity.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(material.clone()));
                }
                let centre = transform.transform_point(Vec3::from(aabb.center));
                let half = Vec3::from(aabb.half_extents) * transform.compute_transform().scale;
                low = low.min(centre - half);
                high = high.max(centre + half);
            }
            let slot = parts.library.models.get(&shot.key.0).map(|p| p.slot.clone()).unwrap_or_default();
            // A wheel set is four wheels at the board's corners and trucks
            // come in pairs: frame one of them.
            let (low, high) = {
                let extent = high - low;
                let long = if extent.z >= extent.x { Vec3::Z } else { Vec3::X };
                match slot.as_str() {
                    "SkateWheel" => (low, low + Vec3::splat(extent.y.max(0.02))),
                    "SkateTruck" => (low, high - long * (extent.dot(long) * 0.5)),
                    _ => (low, high),
                }
            };
            let centre = (low + high) * 0.5;
            let radius = ((high - low).length() * 0.5).max(0.02);
            if let Ok((mut camera, mut transform)) = cameras.get_mut(shot.camera) {
                let extent = high - low;
                let (direction, up, fill) = match slot.as_str() {
                    // A deck's graphic is underneath: seen from below, the
                    // board standing upright in the frame.
                    "SkateBoard" => {
                        let long = if extent.z >= extent.x { Vec3::Z } else { Vec3::X };
                        (Vec3::new(0.05, -1., 0.02).normalize(), long, 0.62)
                    }
                    "SkateWheel" => (Vec3::new(0.6, 0.45, 1.).normalize(), Vec3::Y, 0.9),
                    "SkateTruck" => (Vec3::new(0.3, 0.5, 1.).normalize(), Vec3::Y, 0.5),
                    // Front on, a little from the side and above.
                    _ => (Vec3::new(0.35, 0.2, 1.).normalize(), Vec3::Y, 1.05),
                };
                let distance = radius / (22.5f32.to_radians()).tan() * fill;
                *transform = Transform::from_translation(centre + direction * distance).looking_at(centre, up);
                camera.is_active = true;
            }
            shot.framed = true;
            shot.frames = 0;
            // Shown at once; the picture fills in as soon as it renders.
            studio.done.insert(shot.key.clone(), shot.image.clone());
            continue;
        }
        shot.frames += 1;
        if shot.frames >= hold {
            finished.push((n, true));
        }
    }
    for (n, ok) in finished.into_iter().rev() {
        let shot = studio.shoots.remove(n);
        debug!("Thumbnail {}:{} {}", shot.key.0, shot.key.1, if ok { "taken" } else { "gave up" });
        commands.entity(shot.scene).despawn();
        commands.entity(shot.camera).despawn();
        if ok {
            studio.done.insert(shot.key, shot.image);
        }
    }
    // Start the next items whose model and material are loaded.
    let mut waiting = VecDeque::new();
    while studio.shoots.len() < PARALLEL {
        let Some(key) = studio.queue.pop_front() else { break };
        parts.warm(&key.1, &server, &mut materials);
        let Some(scene) = parts.scene_path(&key.0) else { continue };
        let gltf = server.load::<Gltf>(scene.clone());
        if !server.is_loaded_with_dependencies(gltf.id()) || !parts.material_ready(&key.1, &server) {
            waiting.push_back(key);
            continue;
        }
        let spot = (0..PARALLEL).find(|i| !studio.shoots.iter().any(|s| s.spot == *i)).unwrap_or(0);
        let place = SET + Vec3::X * SPACING * spot as f32;
        let mut target = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
        target.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
        let image = images.add(target);
        let scene = commands
            .spawn((SceneRoot(server.load(GltfAssetLabel::Scene(0).from_asset(scene))), Transform::from_translation(place), RenderLayers::layer(LAYER)))
            .id();
        let camera = commands
            .spawn((
                Camera3d::default(),
                Camera { is_active: false, order: -20, clear_color: ClearColorConfig::Custom(Color::srgba(0.16, 0.18, 0.2, 1.)), ..default() },
                RenderTarget::Image(image.clone().into()),
                Projection::Perspective(PerspectiveProjection { fov: 45f32.to_radians(), near: 0.05, far: 50., ..default() }),
                Transform::from_translation(place + Vec3::new(0., 1., 3.)),
                RenderLayers::layer(LAYER),
            ))
            .id();
        debug!("Thumbnail {}:{} started at spot {spot}", key.0, key.1);
        studio.shoots.push(Shoot { key, image, scene, camera, spot, frames: 0, framed: false });
    }
    // Items still loading keep their place at the front.
    while let Some(key) = waiting.pop_back() {
        studio.queue.push_front(key);
    }
    match (studio.shoots.is_empty(), studio.light) {
        (false, None) => {
            studio.light = Some(
                commands
                    .spawn((
                        DirectionalLight { illuminance: 9000., shadows_enabled: false, ..default() },
                        Transform::from_translation(SET).looking_to(Vec3::new(-0.4, -0.6, -1.), Vec3::Y),
                        RenderLayers::layer(LAYER),
                    ))
                    .id(),
            );
        }
        (true, Some(light)) => {
            commands.entity(light).despawn();
            studio.light = None;
        }
        _ => {}
    }
}
