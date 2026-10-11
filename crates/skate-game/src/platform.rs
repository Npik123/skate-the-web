//! Seams between the desktop and browser builds. Desktop values are exactly
//! what the engine used before the web port.
use bevy::{app::PluginGroupBuilder, prelude::*, render::settings::Backends};

#[cfg(target_arch = "wasm32")]
pub(crate) use crate::web::asset_reader::VfsAssetReader as DirReader;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use bevy::asset::io::file::FileAssetReader as DirReader;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const BACKENDS: Backends = Backends::VULKAN;
#[cfg(all(target_arch = "wasm32", not(feature = "web-webgl2")))]
pub(crate) const BACKENDS: Backends = Backends::BROWSER_WEBGPU;
#[cfg(all(target_arch = "wasm32", feature = "web-webgl2"))]
pub(crate) const BACKENDS: Backends = Backends::GL;

/// Report a startup stage to the page's loading screen. Desktop builds have
/// nobody to report to, so the call disappears.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub(crate) fn stage(_label: &str) {}
#[cfg(target_arch = "wasm32")]
#[inline]
pub(crate) fn stage(label: &str) {
    crate::web::stage(label);
}

/// Start downloading a world before it is asked for (the browser only;
/// desktop builds read from disk).
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub(crate) fn prefetch(_path: &std::path::Path, _background: bool) {}
#[cfg(target_arch = "wasm32")]
pub(crate) fn prefetch(path: &std::path::Path, background: bool) {
    let path = path.to_string_lossy().replace('\\', "/");
    crate::web::prefetch(&path, background);
    let probes = std::path::Path::new(&path).with_extension("irradiance");
    crate::web::prefetch(&probes.to_string_lossy(), background);
}

/// Ask the player for a file of the given type. In the browser this opens a
/// file picker; on the desktop the player drops the file on the window.
#[cfg(target_arch = "wasm32")]
pub(crate) fn pick_file(accept: &str) {
    crate::web::pick_file(accept);
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_file(_accept: &str) {}

/// The bytes of a file the player chose or dropped, once. The file is read
/// where the game runs and never sent anywhere.
#[cfg(target_arch = "wasm32")]
pub(crate) fn take_picked() -> Option<Vec<u8>> {
    crate::web::take_picked()
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn take_picked() -> Option<Vec<u8>> {
    DROPPED.lock().ok()?.take()
}
#[cfg(not(target_arch = "wasm32"))]
static DROPPED: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// Desktop: hand the game a file as if it had been dropped on the window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn offer_file(bytes: Vec<u8>) {
    if let Ok(mut slot) = DROPPED.lock() {
        *slot = Some(bytes);
    }
}

/// Desktop: a file dropped on the window waits for whoever asked for one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dropped_files(mut drops: MessageReader<bevy::window::FileDragAndDrop>) {
    for drop in drops.read() {
        if let bevy::window::FileDragAndDrop::DroppedFile { path_buf, .. } = drop {
            match std::fs::read(path_buf) {
                Ok(bytes) => {
                    if let Ok(mut slot) = DROPPED.lock() {
                        *slot = Some(bytes);
                    }
                }
                Err(e) => warn!("{}: {e}", path_buf.display()),
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn dropped_files() {}

/// There are no process ids in the browser; identity mixes time instead.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn process_id() -> u32 {
    std::process::id()
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn process_id() -> u32 {
    0
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn window_defaults() -> Window {
    Window::default()
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn window_defaults() -> Window {
    Window {
        canvas: Some("#skate-canvas".into()),
        fit_canvas_to_parent: true,
        prevent_default_event_handling: true,
        ..default()
    }
}

/// A phone or tablet browser, which has far less memory for a tab than a
/// desktop. iPadOS reports itself as a Mac, so touch support decides there.
#[cfg(target_arch = "wasm32")]
pub(crate) fn small_device() -> bool {
    let Some(navigator) = web_sys::window().map(|w| w.navigator()) else { return false };
    let agent = navigator.user_agent().unwrap_or_default();
    ["iPhone", "iPad", "iPod", "Android", "Mobile"].iter().any(|k| agent.contains(k))
        || (agent.contains("Macintosh") && navigator.max_touch_points() > 1)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn small_device() -> bool {
    false
}

pub(crate) trait PlatformPlugins {
    fn platform(self) -> Self;
}
impl PlatformPlugins for PluginGroupBuilder {
    #[cfg(not(target_arch = "wasm32"))]
    fn platform(self) -> Self {
        self.disable::<bevy::log::LogPlugin>()
    }
    /// The browser build installs its own console logger (`web::log`):
    /// Bevy's wasm layer turns every span into a performance mark and measure.
    #[cfg(target_arch = "wasm32")]
    fn platform(self) -> Self {
        self.disable::<bevy::log::LogPlugin>()
    }
}
