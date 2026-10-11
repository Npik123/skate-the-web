//! Where original screens are drawn: an offscreen 1280x720 movie stage (the
//! authored front-end resolution) composited over the window. One stage can
//! carry several movies, drawn in order; the pause menu and the HUD each have
//! their own, so a menu darkens the HUD rather than the other way round.
use crate::{apt_movie::Movie, apt_scene};
use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendState, Extent3d, PrimitiveTopology, RenderPipelineDescriptor, ShaderType, TextureDimension,
        TextureFormat,
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d},
    ui_render::UiMaterialPlugin,
    window::PrimaryWindow,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, ShaderType)]
struct ColorTransform {
    multiply: Vec4,
    add: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct ScreenMaterial {
    #[uniform(0)]
    color: ColorTransform,
    #[texture(1)]
    #[sampler(2)]
    atlas: Handle<Image>,
}
impl Material2d for ScreenMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://skate3rust/hud_render.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
/// Premultiplied composite of the offscreen stage, as for the scoring HUD.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct ScreenComposite {
    #[texture(0)]
    #[sampler(1)]
    image: Handle<Image>,
}
impl UiMaterial for ScreenComposite {
    fn fragment_shader() -> ShaderRef {
        "embedded://skate3rust/hud_composite.wgsl".into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _: UiMaterialKey<Self>) {
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
            }
        }
    }
}

pub(crate) struct StagePlugin;
impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((Material2dPlugin::<ScreenMaterial>::default(), UiMaterialPlugin::<ScreenComposite>::default()))
            .add_systems(Update, resize);
    }
}

pub(crate) fn texture(images: &mut Assets<Image>, width: u32, height: u32, bytes: Vec<u8>) -> Handle<Image> {
    images.add(Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// Read a screen export. Screens share the game's string table
/// (`language.json`) unless they carry their own.
pub(crate) fn read_screen(root: &Path, file: &str) -> Result<(serde_json::Value, apt_scene::Shapes), String> {
    let read = |file: &str| -> Result<serde_json::Value, String> {
        serde_json::from_slice(&skate_data::vfs::read(root.join(file)).map_err(|e| format!("{file}: {e}"))?)
            .map_err(|e| format!("{file}: {e}"))
    };
    let mut json = read(file)?;
    if json.get("language").is_none() {
        static LANGUAGE: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        let language = match LANGUAGE.get() {
            Some(language) => language.clone(),
            None => {
                let language = read("language.json")?;
                LANGUAGE.get_or_init(|| language).clone()
            }
        };
        json["language"] = language;
    }
    // Controller glyphs are optional: older installs draw the placeholders.
    if json.get("glyphs").is_none() {
        if let Ok(glyphs) = read("buttons.json") {
            json["glyphs"] = glyphs;
        }
    }
    let shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| format!("{file}: {e}"))?;
    Ok((json, shapes))
}

struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<ScreenMaterial>,
}
#[derive(Component)]
struct StageTarget(Handle<Image>, Handle<ScreenComposite>);

pub(crate) struct Stage {
    textures: BTreeMap<String, Handle<Image>>,
    white: Handle<Image>,
    /// The UI node showing the stage over the window.
    pub overlay: Entity,
    layer: usize,
    slots: Vec<Slot>,
}
impl Stage {
    /// A stage on its own render layer, shown by a full-window UI node at
    /// `z` over the `output` UI camera. Hidden until something is drawn.
    pub fn spawn(
        commands: &mut Commands,
        images: &mut Assets<Image>,
        composites: &mut Assets<ScreenComposite>,
        window: &Window,
        output: Entity,
        layer: usize,
        z: i32,
        order: isize,
    ) -> Self {
        let target = images.add(Image::new_target_texture(
            window.physical_width().max(1),
            window.physical_height().max(1),
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        commands.spawn((
            Camera2d,
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::Fixed { width: 1280., height: 720. },
                ..OrthographicProjection::default_2d()
            }),
            Camera { order, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
            RenderTarget::Image(target.clone().into()),
            RenderLayers::layer(layer),
            Msaa::Off,
        ));
        let composite = composites.add(ScreenComposite { image: target.clone() });
        let overlay = commands
            .spawn((
                MaterialNode(composite.clone()),
                StageTarget(target, composite),
                UiTargetCamera(output),
                GlobalZIndex(z),
                Pickable::IGNORE,
                Visibility::Hidden,
                Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
            ))
            .id();
        Self { textures: BTreeMap::new(), white: texture(images, 1, 1, vec![255; 4]), overlay, layer, slots: Vec::new() }
    }

    /// Draw `key` (a draw's texture name) with an image made elsewhere.
    pub fn set_texture(&mut self, key: &str, image: Handle<Image>) {
        if self.textures.get(key) != Some(&image) {
            self.textures.insert(key.to_owned(), image);
        }
    }

    /// Upload the textures and font pages a screen draws with. Paths are
    /// relative to the export directory and shared between its screens.
    pub fn load_textures(
        &mut self,
        images: &mut Assets<Image>,
        root: &Path,
        shapes: &apt_scene::Shapes,
        movie: &Movie,
    ) -> Result<(), String> {
        let mut files = BTreeMap::new();
        for texture in shapes.values().flatten().filter_map(|s| s.texture.as_ref()) {
            files.insert(texture.rgba.clone(), [texture.width, texture.height]);
        }
        for font in movie.text_assets.fonts.values() {
            files.insert(font.texture.clone(), font.size);
        }
        for (texture, size) in movie.glyphs.values() {
            files.insert(texture.clone(), *size);
        }
        for (path, [width, height]) in files {
            if self.textures.contains_key(&path) {
                continue;
            }
            let bytes = skate_data::vfs::read(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            if bytes.len() != width as usize * height as usize * 4 {
                return Err(format!("Invalid screen texture size {path}"));
            }
            self.textures.insert(path, texture(images, width, height, bytes));
        }
        Ok(())
    }

    /// Replace what the stage shows. An empty list hides it.
    pub fn render(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ScreenMaterial>,
        draws: &[apt_scene::Draw],
    ) {
        commands
            .entity(self.overlay)
            .insert(if draws.is_empty() { Visibility::Hidden } else { Visibility::Visible });
        let mut index = 0;
        for draw in draws {
            let atlas = match &draw.texture {
                Some(path) => match self.textures.get(path) {
                    Some(handle) => handle.clone(),
                    None => continue,
                },
                None => self.white.clone(),
            };
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                draw.vertices.iter().map(|v| [v.position[0] - 640., 360. - v.position[1], 0.]).collect::<Vec<_>>(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, draw.vertices.iter().map(|v| v.uv).collect::<Vec<_>>());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; draw.vertices.len()]);
            let material =
                ScreenMaterial { color: ColorTransform { multiply: draw.multiply.into(), add: draw.add.into() }, atlas };
            if index == self.slots.len() {
                let mesh = meshes.add(mesh);
                let material = materials.add(material);
                let entity = commands
                    .spawn((
                        Mesh2d(mesh.clone()),
                        MeshMaterial2d(material.clone()),
                        Transform::from_xyz(0., 0., index as f32 * 0.01),
                        RenderLayers::layer(self.layer),
                    ))
                    .id();
                self.slots.push(Slot { entity, mesh, material });
            } else {
                let slot = &self.slots[index];
                if let Some(old) = meshes.get_mut(&slot.mesh) {
                    *old = mesh;
                }
                if let Some(old) = materials.get_mut(&slot.material) {
                    *old = material;
                }
                commands.entity(slot.entity).insert(Visibility::Visible);
            }
            index += 1;
        }
        for slot in &self.slots[index..] {
            commands.entity(slot.entity).insert(Visibility::Hidden);
        }
    }
}

/// Every stage follows the window's physical size.
fn resize(
    stages: Query<&StageTarget>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<ScreenComposite>>,
) {
    let size = Extent3d {
        width: window.physical_width().max(1),
        height: window.physical_height().max(1),
        depth_or_array_layers: 1,
    };
    for StageTarget(target, composite) in &stages {
        if images.get(target).is_some_and(|image| image.texture_descriptor.size != size) {
            images.get_mut(target).unwrap().resize(size);
            if let Some(material) = composites.get_mut(composite) {
                material.image = target.clone();
            }
        }
    }
}
