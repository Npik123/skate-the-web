//! Player and camera preferences (settings/player.json). The values feed the
//! original systems that already read them: the camera graph's Low/High
//! selection, look inversion and shake samples, and the riding intents'
//! automatic-push profile fields, which no host setting reached before.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Field-of-view choices, as a percentage of the authored camera lens.
pub(crate) const FOV_STEPS: &[u32] = &[80, 85, 90, 95, 100, 105, 110, 115, 120, 130];

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PlayerSettings {
    /// Stock camera graph selection: false = High (the default), true = Low.
    pub low_camera: bool,
    pub invert_x: bool,
    pub invert_y: bool,
    pub camera_shake: bool,
    pub fov_percent: u32,
    pub auto_push: bool,
    /// Automatic pushes with the right foot instead of the left.
    pub push_right: bool,
    /// Trackpad Mode (browser): mouse or trackpad movement drives the right
    /// stick. 0 = off, 1-3 = low, medium and high sensitivity.
    pub trackpad: u8,
    /// Sound Volume, percent (game audio from the converted disc).
    pub sound_volume: u32,
}
impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            low_camera: false,
            invert_x: false,
            invert_y: false,
            camera_shake: false,
            fov_percent: 100,
            auto_push: false,
            push_right: false,
            trackpad: 0,
            sound_volume: 80,
        }
    }
}
impl PlayerSettings {
    fn validated(mut self) -> Self {
        if !FOV_STEPS.contains(&self.fov_percent) {
            self.fov_percent = 100;
        }
        self.trackpad = self.trackpad.min(3);
        self.sound_volume = ((self.sound_volume.min(100) + 10) / 20) * 20;
        self
    }
    pub(crate) fn path(asset_root: &std::path::Path) -> PathBuf {
        asset_root.parent().unwrap_or(asset_root).join("settings/player.json")
    }
    pub(crate) fn load(asset_root: &std::path::Path) -> Self {
        skate_data::vfs::read(Self::path(asset_root))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
            .validated()
    }
    pub(crate) fn save(&self, asset_root: &std::path::Path) -> Result<(), String> {
        let path = Self::path(asset_root);
        skate_data::vfs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        skate_data::vfs::write(&path, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
}

pub(crate) struct PlayerSettingsPlugin;
impl Plugin for PlayerSettingsPlugin {
    fn build(&self, app: &mut App) {
        let root = app.world().resource::<crate::config::Config>().asset_root.clone();
        app.insert_resource(PlayerSettings::load(&root))
            .add_systems(PreUpdate, apply.after(crate::graphics_menu::MenuInput));
    }
}

/// Copy the preferences into the systems that consume them. Cheap, and
/// re-applied every frame so a map change (which rebuilds both) keeps them.
fn apply(
    settings: Res<PlayerSettings>,
    camera: Option<ResMut<crate::camera::CameraRuntime>>,
    controls: Option<ResMut<crate::physics::PlayerControls>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    mut trackpad: Local<Option<(u8, bool)>>,
) {
    // The page turns pointer movement into the right stick; it needs the level,
    // and whether play has the input (it frees the pointer for menus). The
    // Object Dropper is in the world, not a pause: the page keeps its paused
    // buttons off its corners, and the pointer turns the prop.
    let wanted = (settings.trackpad, crate::graphics_menu::gameplay_active(menu) || crate::object_dropper::is_open());
    if *trackpad != Some(wanted) {
        *trackpad = Some(wanted);
        #[cfg(target_arch = "wasm32")]
        crate::web::set_trackpad(wanted.0, wanted.1);
    }
    if let Some(mut camera) = camera {
        let user = crate::camera::UserCamera {
            camera_type: if settings.low_camera { 0 } else { 1 },
            invert_look: [settings.invert_x, settings.invert_y],
            shake_variant: u8::from(settings.camera_shake),
            fov_scale: settings.fov_percent as f32 / 100.0,
        };
        if camera.user != user {
            camera.user = user;
        }
    }
    if let Some(mut controls) = controls {
        let wanted = (settings.auto_push, settings.push_right);
        let current = (controls.preferences.automatic_push_enabled, controls.preferences.automatic_push_right);
        if wanted != current {
            controls.preferences.automatic_push_enabled = settings.auto_push;
            controls.preferences.automatic_push_right = settings.push_right;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_settings_round_trip_and_reject_bad_fov() {
        let s = PlayerSettings { low_camera: true, invert_y: true, fov_percent: 115, auto_push: true, ..Default::default() };
        let back: PlayerSettings = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let odd: PlayerSettings = serde_json::from_str(r#"{"fov_percent": 7}"#).unwrap();
        assert_eq!(odd.validated().fov_percent, 100);
        let old: PlayerSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old, PlayerSettings::default());
    }
}
