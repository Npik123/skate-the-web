use crate::{read_bounded, Manifest};
use serde::Deserialize;
use serde_json::Value;
use skate_dynamics::{BodyDesc, JointMotorDesc, JointSpringDesc, Shape};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeleportOptions {
    pub position: [f32; 3],
    #[serde(default)]
    pub heading: Option<f32>,
    #[serde(default)]
    pub velocity: Option<[f32; 3]>,
}

impl TeleportOptions {
    pub fn validate(&self) -> bool {
        let point = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.);
        let vel = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 200.);
        point(&self.position)
            && self.heading.is_none_or(|h| h.is_finite() && h.abs() <= 1000.)
            && self.velocity.as_ref().is_none_or(vel)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeOptions {
    pub position: [f32; 3],
    pub size: [f32; 3],
    #[serde(default)]
    pub rotation: Option<[f32; 4]>,
    #[serde(default)]
    pub visible: bool,
    #[serde(default = "volume_color")]
    pub color: [f32; 3],
    #[serde(default = "volume_opacity")]
    pub opacity: f32,
}

fn volume_color() -> [f32; 3] {
    [0.2, 0.85, 1.0]
}
fn volume_opacity() -> f32 {
    0.35
}

impl VolumeOptions {
    pub fn validate(&self) -> bool {
        let point = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.);
        point(&self.position)
            && self.size.iter().all(|v| v.is_finite() && *v > 0.01 && *v <= 500.)
            && self
                .rotation
                .as_ref()
                .is_none_or(|q| crate::scene::valid_quaternion(q))
            && self.color.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.opacity.is_finite()
            && (0. ..=1.).contains(&self.opacity)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureOptions {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    #[serde(default = "capture_fov")]
    pub fov: f32,
    #[serde(default = "capture_size")]
    pub width: u32,
    #[serde(default = "capture_size")]
    pub height: u32,
}

fn capture_fov() -> f32 {
    70.0_f32.to_radians()
}
fn capture_size() -> u32 {
    256
}

impl CaptureOptions {
    pub fn validate(&self) -> bool {
        let point = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.);
        point(&self.position)
            && point(&self.look_at)
            && self.fov.is_finite()
            && (0.2..=2.5).contains(&self.fov)
            && (64..=512).contains(&self.width)
            && (64..=512).contains(&self.height)
            && self.width % 16 == 0
            && self.height % 16 == 0
    }
}

fn valid_peer(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 20
        && s.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Log {
        text: String,
    },
    Overlay {
        key: String,
        text: String,
    },
    MultiplayerDebug { key: String, text: String },
    UiCanvas { key: String, options: crate::presentation::CanvasOptions },
    UiRemove { key: String },
    CameraRig { body: String, options: crate::presentation::CameraRigOptions },
    AudioPreload { path: String },
    AudioPlay { key: String, options: crate::audio::AudioPlayOptions },
    AudioUpdate { key: String, options: crate::audio::AudioUpdateOptions },
    AudioStop {
        key: String,
        #[serde(default = "crate::audio::default_fade")]
        fade_out: f32,
    },
    AudioStopAll {},
    GraphicsMeshBuffer {
        key: String,
        options: crate::graphics_dynamic::MeshBufferOptions,
    },
    GraphicsMeshBufferWrite {
        key: String,
        data: crate::graphics_dynamic::MeshBufferWrite,
    },
    GraphicsMeshBufferAppend {
        key: String,
        data: crate::graphics_dynamic::MeshBufferWrite,
    },
    GraphicsLight {
        key: String,
        options: crate::graphics_dynamic::LightOptions,
    },
    PhysicsSpawn {
        key: String,
        body: BodyDesc,
    },
    PhysicsRemove {
        key: String,
    },
    PhysicsForce {
        key: String,
        force: [f32; 3],
        #[serde(default)]
        point: Option<[f32; 3]>,
    },
    PhysicsImpulse {
        key: String,
        impulse: [f32; 3],
        #[serde(default)]
        point: Option<[f32; 3]>,
    },
    PhysicsTorque {
        key: String,
        torque: [f32; 3],
    },
    PhysicsTorqueImpulse {
        key: String,
        torque: [f32; 3],
    },
    PhysicsSetLinvel {
        key: String,
        linvel: [f32; 3],
    },
    PhysicsSetAngvel {
        key: String,
        angvel: [f32; 3],
    },
    PhysicsSetPose {
        key: String,
        position: [f32; 3],
        rotation: [f32; 4],
    },
    PhysicsRevolute {
        key: String,
        body_a: String,
        body_b: String,
        anchor_a: [f32; 3],
        anchor_b: [f32; 3],
        #[serde(default = "axis_y")]
        axis: [f32; 3],
        #[serde(default)]
        limits: Option<[f32; 2]>,
        #[serde(default = "true_fn")]
        contacts_enabled: bool,
    },
    PhysicsJointMotor {
        key: String,
        motor: JointMotorDesc,
    },
    PhysicsPrismatic {
        key: String,
        body_a: String,
        body_b: String,
        anchor_a: [f32; 3],
        anchor_b: [f32; 3],
        #[serde(default = "neg_y")]
        axis: [f32; 3],
        #[serde(default)]
        limits: Option<[f32; 2]>,
        #[serde(default = "true_fn")]
        contacts_enabled: bool,
    },
    PhysicsJointSpring {
        key: String,
        spring: JointSpringDesc,
    },
    PhysicsRemoveJoint {
        key: String,
    },
    /// Extra convex/mesh collider on an existing body (e.g. high-friction tires).
    PhysicsAddCollider {
        key: String,
        shape: Shape,
        #[serde(default)]
        position: [f32; 3],
        #[serde(default = "one_friction")]
        friction: f32,
    },
    GraphicsMesh {
        key: String,
        #[serde(default)]
        deform_nodes: Vec<String>,
        /// Package-relative GLB path, or empty for a debug box.
        #[serde(default)]
        path: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        position: Option<[f32; 3]>,
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        #[serde(default = "one_scale")]
        scale: [f32; 3],
        #[serde(default = "white")]
        color: [f32; 3],
        #[serde(default = "true_fn")]
        visible: bool,
        #[serde(default = "opaque")]
        opacity: f32,
    },
    GraphicsTransform { key:String, options:crate::scene::TransformOptions },
    GraphicsNode { key:String, node:String, options:crate::scene::TransformOptions },
    GraphicsResetNode { key:String, node:String },
    PhysicsDebug { enabled:bool },
    GraphicsRemove {
        key: String,
    },
    GraphicsVisibility {
        key: String,
        visible: bool,
    },
    PlayerAttach {
        body: String,
        #[serde(default)]
        offset: [f32; 3],
    },
    PlayerDetach { #[serde(default)] options:crate::scene::DetachOptions },
    CameraFollow {
        body: Option<String>,
        #[serde(default = "cam_offset")]
        offset: [f32; 3],
    },
    CameraSet {
        position: [f32; 3],
        #[serde(default)]
        look_at: Option<[f32; 3]>,
    },
    CameraWatch {
        #[serde(default)]
        peer: Option<String>,
    },
    NetworkState {
        key: String,
        #[serde(default)]
        value: Value,
    },
    UiMenu { key:String, options:crate::extensions::MenuOptions },
    UiRemoveMenu { key:String },
    RigPart {index:usize, options:Option<crate::extensions::PartOverride>},
    GraphGate { graph:String, target:String, index:usize, enabled:Option<bool> },
    EngineInspect { system:String },
    Request { key:String, #[serde(default)] token:u64, command:Box<Command> },
    InputOverride { action:usize, value:Option<f32> },
    NativeImpulse { body:crate::extensions::NativeBodyRef, impulse:[f32;3], point:Option<[f32;3]>, angular:bool },
    PlayerJoint { joint:usize, options:crate::extensions::JointOverride },
    PlayerResetJoint { joint:usize },
    PlayerResetJoints {},
    PlayerSuspend { suspended: bool },
    PlayerTeleport {
        options: TeleportOptions,
    },
    SessionClaim {},
    SessionTransfer {
        peer: String,
    },
    SessionTeleport {
        peer: String,
        options: TeleportOptions,
    },
    VolumeBox {
        key: String,
        options: VolumeOptions,
    },
    VolumeRemove {
        key: String,
    },
    CameraCapture {
        key: String,
        options: CaptureOptions,
    },
    CameraClearCapture {
        key: String,
    },
}

fn opaque() -> f32 {
    1.0
}
fn one_scale() -> [f32; 3] {
    [1., 1., 1.]
}
fn one_friction() -> f32 {
    0.7
}
fn white() -> [f32; 3] {
    [0.85, 0.85, 0.9]
}
fn true_fn() -> bool {
    true
}
fn cam_offset() -> [f32; 3] {
    [0., 2.5, -6.]
}
fn axis_y() -> [f32; 3] {
    [0., 1., 0.]
}
fn neg_y() -> [f32; 3] {
    [0., -1., 0.]
}

impl Command {
    pub fn validate(&self) -> bool {
        let point = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.);
        let vec3 = |p: &[f32; 3]| p.iter().all(|v| v.is_finite());
        let quat = crate::scene::valid_quaternion;
        match self {
            Self::RigPart {index,options} => *index<26 && options.as_ref().is_none_or(|o|o.validate()),
            Self::GraphGate {graph,target,index,..} => matches!(graph.as_str(),"action"|"motion") && matches!(target.as_str(),"state"|"transition"|"behavior") && *index<65536,
            Self::EngineInspect {system} => matches!(system.as_str(),"graphs"|"scoring"),
            Self::Request {key,command,token} => *token<=9_007_199_254_740_991 && crate::schema::valid_id(key) && !matches!(**command,Self::Request{..}) && command.validate(),
            Self::InputOverride {action,value} => (64..=81).contains(action) && value.is_none_or(|v|v.is_finite() && (-1.0..=1.0).contains(&v)),
            Self::NativeImpulse {body,impulse,point:p,..} => body.validate() && impulse.iter().all(|v|v.is_finite() && v.abs()<=100_000.) && p.as_ref().is_none_or(point),
            Self::Log { text } => text.len() <= 2048,
            Self::Overlay { key, text } | Self::MultiplayerDebug { key, text } => crate::schema::valid_id(key) && text.len() <= 1024,
            Self::UiCanvas { key, options } => crate::schema::valid_id(key) && options.validate(),
            Self::UiRemove { key } => crate::schema::valid_id(key),
            Self::CameraRig { body, options } => crate::schema::valid_id(body) && options.validate(),
            Self::AudioPreload { path } => crate::audio::valid_audio_path(path),
            Self::AudioPlay { key, options } => {
                crate::schema::valid_id(key) && options.validate()
            }
            Self::AudioUpdate { key, options } => {
                crate::schema::valid_id(key) && options.validate()
            }
            Self::AudioStop { key, fade_out } => {
                crate::schema::valid_id(key) && fade_out.is_finite()
                    && (0.0..=2.0).contains(fade_out)
            }
            Self::AudioStopAll {} => true,
            Self::GraphicsMeshBuffer { key, options } => {
                crate::schema::valid_id(key) && options.validate()
            }
            Self::GraphicsMeshBufferWrite { key, data } => {
                crate::schema::valid_id(key) && data.validate()
            }
            Self::GraphicsMeshBufferAppend { key, data } => {
                crate::schema::valid_id(key)
                    && !data.positions.is_empty()
                    && data.positions.len() <= crate::graphics_dynamic::MAX_MESH_BUFFER_APPEND_VERTICES
            }
            Self::GraphicsLight { key, options } => {
                crate::schema::valid_id(key) && options.validate()
            }
            Self::PhysicsSpawn { key, body } => {
                crate::schema::valid_id(key) && body.deformation.as_ref().is_none_or(|o|o.validate().is_ok()) && serde_json::to_value(body).is_ok()
            }
            Self::PhysicsRemove { key }
            | Self::PhysicsRemoveJoint { key }
            | Self::GraphicsRemove { key }
            | Self::GraphicsVisibility { key, .. } => crate::schema::valid_id(key),
            Self::PhysicsAddCollider {
                key,
                shape,
                position,
                friction,
            } => {
                crate::schema::valid_id(key)
                    && point(position)
                    && friction.is_finite()
                    && (0. ..=2.).contains(friction)
                    && match shape {
                        Shape::Convex { points } => {
                            (4..=512).contains(&points.len())
                                && points.iter().flatten().all(|v| v.is_finite() && v.abs() <= 1000.)
                        }
                        Shape::Mesh { path, object } => {
                            !path.is_empty()
                                && path.len() <= 256
                                && !path.contains("..")
                                && !path.contains('\\')
                                && !object.is_empty()
                                && object.len() <= 120
                        }
                        _ => false,
                    }
            }
            Self::PhysicsForce { key, force, point: p }
            | Self::PhysicsImpulse {
                key,
                impulse: force,
                point: p,
            } => {
                crate::schema::valid_id(key)
                    && vec3(force)
                    && p.as_ref().is_none_or(|p| point(p))
            }
            Self::PhysicsTorque { key, torque }
            | Self::PhysicsTorqueImpulse { key, torque }
            | Self::PhysicsSetLinvel { key, linvel: torque }
            | Self::PhysicsSetAngvel { key, angvel: torque } => {
                crate::schema::valid_id(key) && vec3(torque)
            }
            Self::PhysicsSetPose {
                key,
                position,
                rotation,
            } => crate::schema::valid_id(key) && point(position) && quat(rotation),
            Self::PhysicsRevolute {
                key,
                body_a,
                body_b,
                anchor_a,
                anchor_b,
                axis,
                limits,
                ..
            } => {
                crate::schema::valid_id(key)
                    && crate::schema::valid_id(body_a)
                    && crate::schema::valid_id(body_b)
                    && vec3(anchor_a)
                    && vec3(anchor_b)
                    && vec3(axis)
                    && limits.is_none_or(|[min, max]| {
                        min.is_finite()
                            && max.is_finite()
                            && min <= max
                            && min.abs() <= std::f32::consts::TAU
                            && max.abs() <= std::f32::consts::TAU
                    })
            }
            Self::PhysicsJointMotor { key, motor } => {
                crate::schema::valid_id(key)
                    && match motor {
                        JointMotorDesc::Velocity {
                            target_velocity,
                            factor,
                            max_force,
                        } => {
                            target_velocity.is_finite()
                                && factor.is_finite()
                                && *factor >= 0.
                                && max_force.is_none_or(|force| {
                                    force.is_finite() && (0. ..=1_000_000.).contains(&force)
                                })
                        }
                        JointMotorDesc::Position {
                            target_position,
                            stiffness,
                            damping,
                            max_force,
                        } => {
                            target_position.is_finite()
                                && stiffness.is_finite()
                                && (0. ..=100_000.).contains(stiffness)
                                && damping.is_finite()
                                && (0. ..=10_000.).contains(damping)
                                && max_force.is_none_or(|force| {
                                    force.is_finite() && (0. ..=1_000_000.).contains(&force)
                                })
                        }
                    }
            }
            Self::PhysicsPrismatic {
                key,
                body_a,
                body_b,
                anchor_a,
                anchor_b,
                axis,
                limits,
                contacts_enabled: _,
            } => {
                crate::schema::valid_id(key)
                    && crate::schema::valid_id(body_a)
                    && crate::schema::valid_id(body_b)
                    && point(anchor_a)
                    && point(anchor_b)
                    && vec3(axis)
                    && limits.is_none_or(|[min, max]| {
                        min.is_finite() && max.is_finite() && min <= max && min.abs() <= 10. && max.abs() <= 10.
                    })
            }
            Self::PhysicsJointSpring { key, spring } => {
                crate::schema::valid_id(key)
                    && spring.target_position.is_finite()
                    && spring.stiffness.is_finite()
                    && (0. ..=100_000.).contains(&spring.stiffness)
                    && spring.damping.is_finite()
                    && (0. ..=10_000.).contains(&spring.damping)
                    && spring
                        .max_force
                        .is_none_or(|f| f.is_finite() && (0. ..=1_000_000.).contains(&f))
            }
            Self::GraphicsMesh {
                key,
                path,
                body,
                position,
                rotation,
                scale,
                color,
                opacity,
                deform_nodes,
                ..
            } => {
                crate::schema::valid_id(key)
                    && crate::scene::valid_asset(path)
                    && deform_nodes.len()<=64 && deform_nodes.iter().all(|s|crate::scene::valid_node(s))
                    && body.as_ref().is_none_or(|b| crate::schema::valid_id(b))
                    && position.as_ref().is_none_or(|p| point(p))
                    && rotation.as_ref().is_none_or(|q| quat(q))
                    && scale.iter().all(|v| v.is_finite() && *v > 0. && *v <= 100.)
                    && color
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v))
                    && opacity.is_finite()
                    && (0. ..=1.).contains(opacity)
            }
            Self::PlayerAttach { body, offset } => {
                crate::schema::valid_id(body) && point(offset)
            }
            Self::GraphicsTransform { key, options } => crate::schema::valid_id(key) && options.validate()
                && options.relative.is_none() && options.linear_velocity.is_none() && options.angular_velocity.is_none(),
            Self::GraphicsNode { key, node, options } => crate::schema::valid_id(key) && crate::scene::valid_node(node) && options.validate(),
            Self::GraphicsResetNode { key, node } => crate::schema::valid_id(key) && crate::scene::valid_node(node),
            Self::PhysicsDebug { .. } => true,
            Self::PlayerDetach { options } => options.validate(),
            Self::CameraFollow { body, offset } => {
                body.as_ref().is_none_or(|b| crate::schema::valid_id(b)) && vec3(offset)
            }
            Self::CameraSet { position, look_at } => {
                point(position) && look_at.as_ref().is_none_or(|p| point(p))
            }
            Self::CameraWatch { peer } => peer
                .as_ref()
                .is_none_or(|p| p.is_empty() || valid_peer(p)),
            Self::NetworkState { key, value } => {
                crate::schema::valid_id(key)
                    && serde_json::to_vec(value).is_ok_and(|v| v.len() <= 512)
            }
            Self::UiMenu {key,options} => crate::schema::valid_id(key) && options.validate(),
            Self::UiRemoveMenu {key} => crate::schema::valid_id(key),
            Self::PlayerJoint {joint,options} => *joint < 22 && options.validate(),
            Self::PlayerResetJoint {joint} => *joint < 22,
            Self::PlayerResetJoints {} => true,
            Self::PlayerSuspend { .. } => true,
            Self::PlayerTeleport { options } => options.validate(),
            Self::SessionClaim {} => true,
            Self::SessionTransfer { peer } => valid_peer(peer),
            Self::SessionTeleport { peer, options } => valid_peer(peer) && options.validate(),
            Self::VolumeBox { key, options } => crate::schema::valid_id(key) && options.validate(),
            Self::VolumeRemove { key } => crate::schema::valid_id(key),
            Self::CameraCapture { key, options } => {
                crate::schema::valid_id(key) && options.validate()
            }
            Self::CameraClearCapture { key } => crate::schema::valid_id(key),
        }
    }
}

/// Query results must reach Lua as `nil` when absent. `lua.to_value` maps JSON
/// null to a null *userdata*, which is truthy and blows up on indexing.
fn command_kind(command: &Command) -> &'static str {
    match command {
        Command::RigPart {..} => "rig_part",
        Command::GraphGate {..} => "graph_gate",
        Command::EngineInspect {..} => "engine_inspect",
        Command::Request {..} => "request",
        Command::InputOverride {..} => "input_override",
        Command::NativeImpulse {..} => "native_impulse",
        Command::Log { .. } => "log",
        Command::Overlay { .. } => "overlay",
        Command::MultiplayerDebug { .. } => "multiplayer_debug",
        Command::UiCanvas { .. } => "ui_canvas",
        Command::UiRemove { .. } => "ui_remove",
        Command::CameraRig { .. } => "camera_rig",
        Command::AudioPreload { .. } => "audio_preload",
        Command::AudioPlay { .. } => "audio_play",
        Command::AudioUpdate { .. } => "audio_update",
        Command::AudioStop { .. } => "audio_stop",
        Command::AudioStopAll {} => "audio_stop_all",
        Command::GraphicsMeshBuffer { .. } => "graphics_mesh_buffer",
        Command::GraphicsMeshBufferWrite { .. } => "graphics_mesh_buffer_write",
        Command::GraphicsMeshBufferAppend { .. } => "graphics_mesh_buffer_append",
        Command::GraphicsLight { .. } => "graphics_light",
        Command::PhysicsSpawn { .. } => "physics_spawn",
        Command::PhysicsRemove { .. } => "physics_remove",
        Command::PhysicsForce { .. } => "physics_force",
        Command::PhysicsImpulse { .. } => "physics_impulse",
        Command::PhysicsTorque { .. } => "physics_torque",
        Command::PhysicsTorqueImpulse { .. } => "physics_torque_impulse",
        Command::PhysicsSetLinvel { .. } => "physics_set_linvel",
        Command::PhysicsSetAngvel { .. } => "physics_set_angvel",
        Command::PhysicsSetPose { .. } => "physics_set_pose",
        Command::PhysicsRevolute { .. } => "physics_revolute",
        Command::PhysicsJointMotor { .. } => "physics_joint_motor",
        Command::PhysicsPrismatic { .. } => "physics_prismatic",
        Command::PhysicsJointSpring { .. } => "physics_joint_spring",
        Command::PhysicsRemoveJoint { .. } => "physics_remove_joint",
        Command::PhysicsAddCollider { .. } => "physics_add_collider",
        Command::GraphicsMesh { .. } => "graphics_mesh",
        Command::GraphicsTransform { .. } => "graphics_transform",
        Command::GraphicsNode { .. } => "graphics_node",
        Command::GraphicsResetNode { .. } => "graphics_reset_node",
        Command::PhysicsDebug { .. } => "physics_debug",
        Command::GraphicsRemove { .. } => "graphics_remove",
        Command::GraphicsVisibility { .. } => "graphics_visibility",
        Command::PlayerAttach { .. } => "player_attach",
        Command::PlayerDetach { .. } => "player_detach",
        Command::CameraFollow { .. } => "camera_follow",
        Command::CameraSet { .. } => "camera_set",
        Command::CameraWatch { .. } => "camera_watch",
        Command::NetworkState { .. } => "network_state",
        Command::UiMenu { .. } => "ui_menu",
        Command::UiRemoveMenu { .. } => "ui_remove_menu",
        Command::PlayerJoint { .. } => "player_joint",
        Command::PlayerResetJoint { .. } => "player_reset_joint",
        Command::PlayerResetJoints {} => "player_reset_joints",
        Command::PlayerSuspend { .. } => "player_suspend",
        Command::PlayerTeleport { .. } => "player_teleport",
        Command::SessionClaim { .. } => "session_claim",
        Command::SessionTransfer { .. } => "session_transfer",
        Command::SessionTeleport { .. } => "session_teleport",
        Command::VolumeBox { .. } => "volume_box",
        Command::VolumeRemove { .. } => "volume_remove",
        Command::CameraCapture { .. } => "camera_capture",
        Command::CameraClearCapture { .. } => "camera_clear_capture",
    }
}

/// JSON null becomes a truthy null userdata via `lua.to_value`; map it to real nil.
/// Fills in any snapshot field the API reads, so `sdk.snapshot` accessors are
/// total.
///
/// `on_load`, `on_unload`, `on_settings` and `on_event` can all run before the
/// host has ever built a snapshot — `Manager::start` passes whatever it has,
/// which on the first frame is `Value::Null`. A nil `sdk.snapshot` turns every
/// reader (`sdk.player.attached`, `sdk.input.down`, ...) into a hard Lua error
/// during startup, which is a host-lifecycle detail no mod can be expected to
/// defend against. Missing subtables are filled individually so a partial
/// snapshot cannot throw either.
fn complete(snapshot: &Value) -> Value {
    let mut value = snapshot.clone();
    let Some(object) = value.as_object_mut() else {
        return default_snapshot();
    };
    if let Value::Object(defaults) = default_snapshot() {
        for (key, default) in defaults {
            let missing = object.get(&key).is_none_or(Value::is_null);
            // `attach`, `camera` and `detach_error` are meaningfully null, so a
            // null default keeps them absent rather than inventing a value.
            if missing && !default.is_null() {
                object.insert(key, default);
            }
        }
    }
    value
}

fn default_snapshot() -> Value {
    serde_json::json!({
        "player": {
            "position": [0.0, 0.0, 0.0],
            "velocity": [0.0, 0.0, 0.0],
            "angvel": [0.0, 0.0, 0.0],
            "forward": [0.0, 0.0, 1.0],
            "heading": 0.0,
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "speed": 0.0,
            "on_board": false,
            "state": 0,
            "category": 0,
            "filtered": 0,
            "mode": "ground",
            "grind": Value::Null,
            "bailing": false,
            "trick": "",
            "trick_seq": 0,
            "landing_seq": 0, "landed_trick": "", "bail_seq": 0,
            "new_trick": false,
            "modified_trick": false,
            "close_tricks": false,
            "sequence": false,
            "score": 0,
            "line": 0,
            "multiplier": 1.0,
            "line_time": 0,
            "clean": false,
            "sketchy": false,
            "switch": false,
            "fakie": false,
            "nollie": false,
            "intents": {},
        },
        "skaters": {},
        "player_physics": {"contacts":[],"joints":[],"parts":[],"dt":0,"ragdoll":false,"partial_ragdoll":false},
        "session": {
            "active": false,
            "local_id": "0",
            "is_host": true,
            "authority": "0",
            "players": ["0"]
        },
        "volumes": {},
        "attach": Value::Null,
        "detach_error": Value::Null,
        "detach_pending": false,
        "map": {"name": "", "generation": 0},
        "tick": 0,
        "keys": {},
        // 18 gameplay actions, IDs 64..=81.
        "actions": vec![0.0f32; 18],
        "pad": {"buttons": 0, "triggers": [0.0, 0.0], "left": [0.0, 0.0], "right": [0.0, 0.0]},
        "paused": false,
        "replay": false,
        "camera": Value::Null,
        "physics": {"bodies": {}, "contacts": []},
        "network": {
            "active": false,
            "local_id": "0",
            "is_host": true,
            "host_id": "0",
            "players": ["0"],
            "states": {},
            "status": "",
        },
    })
}


#[cfg(not(target_arch = "wasm32"))]
mod lua;
#[cfg(not(target_arch = "wasm32"))]
pub use lua::*;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;
