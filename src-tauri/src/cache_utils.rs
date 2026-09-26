use crate::AppState;
use crate::app_settings::AppSettings;
use image::DynamicImage;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

pub const GEOMETRY_KEYS: &[&str] = &[
    "transformDistortion",
    "transformVertical",
    "transformHorizontal",
    "transformRotate",
    "transformAspect",
    "transformScale",
    "transformXOffset",
    "transformYOffset",
    "lensDistortionAmount",
    "lensVignetteAmount",
    "lensTcaAmount",
    "lensDistortionParams",
    "lensMaker",
    "lensModel",
    "lensDistortionEnabled",
    "lensTcaEnabled",
    "lensVignetteEnabled",
    "guidedPerspective",
];

pub fn calculate_geometry_hash(adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();

    if let Some(patches) = adjustments.get("aiPatches") {
        patches.to_string().hash(&mut hasher);
    }

    for key in GEOMETRY_KEYS {
        if let Some(val) = adjustments.get(key) {
            key.hash(&mut hasher);
            val.to_string().hash(&mut hasher);
        }
    }

    hasher.finish()
}

pub fn calculate_patched_warped_hash(adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();

    calculate_geometry_hash(adjustments).hash(&mut hasher);

    let effects_visible = adjustments
        .get("sectionVisibility")
        .and_then(|v| v.get("effects"))
        .and_then(|s| s.as_bool())
        .unwrap_or(true);

    let blur_enabled = effects_visible && adjustments["lensBlurEnabled"].as_bool().unwrap_or(false);
    blur_enabled.hash(&mut hasher);

    if blur_enabled {
        let blur_keys = [
            "lensBlurAmount",
            "lensBlurDiffusion",
            "lensBlurShape",
            "lensBlurMinDepth",
            "lensBlurMaxDepth",
            "lensBlurMinFade",
            "lensBlurMaxFade",
            "lensBlurDepthMap",
        ];

        for key in blur_keys {
            if let Some(val) = adjustments.get(key) {
                key.hash(&mut hasher);
                val.to_string().hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

pub fn calculate_thumbnail_base_hash(adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();

    calculate_patched_warped_hash(adjustments).hash(&mut hasher);

    adjustments["orientationSteps"]
        .as_u64()
        .unwrap_or(0)
        .hash(&mut hasher);

    hasher.finish()
}

pub fn calculate_visual_hash(path: &str, adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);

    if let Some(obj) = adjustments.as_object() {
        for (key, value) in obj {
            if GEOMETRY_KEYS.contains(&key.as_str()) {
                continue;
            }

            match key.as_str() {
                "crop" | "rotation" | "orientationSteps" | "flipHorizontal" | "flipVertical" => (),
                _ => {
                    key.hash(&mut hasher);
                    value.to_string().hash(&mut hasher);
                }
            }
        }
    }

    hasher.finish()
}

pub fn calculate_transform_hash(adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();

    let orientation_steps = adjustments["orientationSteps"].as_u64().unwrap_or(0);
    orientation_steps.hash(&mut hasher);

    let rotation = adjustments["rotation"].as_f64().unwrap_or(0.0);
    (rotation.to_bits()).hash(&mut hasher);

    let flip_h = adjustments["flipHorizontal"].as_bool().unwrap_or(false);
    flip_h.hash(&mut hasher);

    let flip_v = adjustments["flipVertical"].as_bool().unwrap_or(false);
    flip_v.hash(&mut hasher);

    let effects_visible = adjustments
        .get("sectionVisibility")
        .and_then(|v| v.get("effects"))
        .and_then(|s| s.as_bool())
        .unwrap_or(true);

    let blur_enabled = effects_visible && adjustments["lensBlurEnabled"].as_bool().unwrap_or(false);
    blur_enabled.hash(&mut hasher);
    if blur_enabled {
        if let Some(val) = adjustments.get("lensBlurAmount") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurDiffusion") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurShape") {
            val.as_str().unwrap_or("").hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurMinDepth") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurMaxDepth") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurMinFade") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurMaxFade") {
            val.to_string().hash(&mut hasher);
        }
        if let Some(val) = adjustments.get("lensBlurDepthMap") {
            val.as_str().unwrap_or("").len().hash(&mut hasher);
        }
    }

    if let Some(crop_val) = adjustments.get("crop")
        && !crop_val.is_null()
    {
        crop_val.to_string().hash(&mut hasher);
    }

    for key in GEOMETRY_KEYS {
        if let Some(val) = adjustments.get(key) {
            key.hash(&mut hasher);
            val.to_string().hash(&mut hasher);
        }
    }

    if let Some(patches_val) = adjustments.get("aiPatches")
        && let Some(patches_arr) = patches_val.as_array()
    {
        patches_arr.len().hash(&mut hasher);

        for patch in patches_arr {
            if let Some(id) = patch.get("id").and_then(|v| v.as_str()) {
                id.hash(&mut hasher);
            }

            let is_visible = patch
                .get("visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            is_visible.hash(&mut hasher);

            if let Some(patch_data) = patch.get("patchData") {
                let color_len = patch_data
                    .get("color")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .len();
                color_len.hash(&mut hasher);

                let mask_len = patch_data
                    .get("mask")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .len();
                mask_len.hash(&mut hasher);
            } else {
                let data_len = patch
                    .get("patchDataBase64")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .len();
                data_len.hash(&mut hasher);
            }

            if let Some(sub_masks_val) = patch.get("subMasks") {
                sub_masks_val.to_string().hash(&mut hasher);
            }

            let invert = patch
                .get("invert")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            invert.hash(&mut hasher);
        }
    }

    hasher.finish()
}

pub fn calculate_full_job_hash(path: &str, adjustments: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    adjustments.to_string().hash(&mut hasher);
    hasher.finish()
}

/// A path alone goes stale: tethering reuses file names and raw settings change the pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    settings_hash: u64,
}

impl DecodeFingerprint {
    pub fn for_file(path: &Path, settings: &AppSettings) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let mut hasher = DefaultHasher::new();
        settings
            .raw_highlight_compression
            .map(f32::to_bits)
            .hash(&mut hasher);
        settings.linear_raw_mode.hash(&mut hasher);
        settings
            .raw_preprocessing_color_nr
            .map(f32::to_bits)
            .hash(&mut hasher);
        settings
            .raw_preprocessing_sharpening
            .map(f32::to_bits)
            .hash(&mut hasher);
        settings.apply_preprocessing_to_non_raws.hash(&mut hasher);
        settings.use_apple_raw9.hash(&mut hasher);
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            settings_hash: hasher.finish(),
        })
    }
}

struct DecodedEntry {
    path: String,
    fingerprint: DecodeFingerprint,
    image: Arc<DynamicImage>,
    exif: HashMap<String, String>,
}

pub struct DecodedImageCache {
    capacity: usize,
    items: Vec<DecodedEntry>,
}

impl DecodedImageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: Vec::with_capacity(capacity),
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.items.len() > self.capacity {
            self.items.remove(0);
        }
    }

    pub fn get(
        &mut self,
        path: &str,
        fingerprint: &DecodeFingerprint,
    ) -> Option<(Arc<DynamicImage>, HashMap<String, String>)> {
        let pos = self.items.iter().position(|e| e.path == path)?;
        let entry = self.items.remove(pos);
        if entry.fingerprint != *fingerprint {
            return None;
        }
        let result = (entry.image.clone(), entry.exif.clone());
        self.items.push(entry);
        Some(result)
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn insert(
        &mut self,
        path: String,
        fingerprint: DecodeFingerprint,
        image: Arc<DynamicImage>,
        exif: HashMap<String, String>,
    ) {
        if let Some(pos) = self.items.iter().position(|e| e.path == path) {
            self.items.remove(pos);
        } else if self.items.len() >= self.capacity {
            self.items.remove(0);
        }
        self.items.push(DecodedEntry {
            path,
            fingerprint,
            image,
            exif,
        });
    }
}

/// Lets concurrent callers share one decode of a file instead of each decoding it.
#[derive(Default)]
pub struct DecodeFlights {
    inflight: Mutex<HashMap<String, Arc<Flight>>>,
}

#[derive(Default)]
pub struct Flight {
    done: Mutex<bool>,
    finished: Condvar,
}

impl Flight {
    pub fn wait(&self) {
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        while !*done {
            done = self.finished.wait(done).unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub enum FlightRole<'a> {
    Leader(FlightGuard<'a>),
    Follower(Arc<Flight>),
}

/// Wakes waiters when dropped, including on error or panic.
pub struct FlightGuard<'a> {
    flights: &'a DecodeFlights,
    path: String,
    flight: Arc<Flight>,
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        self.flights
            .inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.path);
        *self.flight.done.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.flight.finished.notify_all();
    }
}

impl DecodeFlights {
    pub fn begin(&self, path: &str) -> FlightRole<'_> {
        let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(flight) = inflight.get(path) {
            return FlightRole::Follower(Arc::clone(flight));
        }
        let flight = Arc::new(Flight::default());
        inflight.insert(path.to_string(), Arc::clone(&flight));
        FlightRole::Leader(FlightGuard {
            flights: self,
            path: path.to_string(),
            flight,
        })
    }
}

#[tauri::command]
pub fn clear_image_caches(state: tauri::State<AppState>) {
    if let Ok(mut decoded_cache) = state.decoded_image_cache.lock() {
        decoded_cache.clear();
    }
    if let Ok(mut gpu_cache) = state.gpu_image_cache.lock() {
        *gpu_cache = None;
    }
    if let Ok(mut preview_cache) = state.cached_preview.lock() {
        *preview_cache = None;
    }
    if let Ok(mut warped_cache) = state.full_warped_cache.lock() {
        *warped_cache = None;
    }
    if let Ok(mut patched_warped_cache) = state.patched_warped_cache.lock() {
        *patched_warped_cache = None;
    }
    if let Ok(mut transformed_cache) = state.full_transformed_cache.lock() {
        *transformed_cache = None;
    }
}

#[tauri::command]
pub fn clear_session_caches(state: tauri::State<AppState>) {
    if let Ok(mut patch_cache) = state.patch_cache.lock() {
        patch_cache.clear();
    }
    if let Ok(mut mask_cache) = state.mask_cache.lock() {
        mask_cache.clear();
    }
    if let Ok(mut geometry_cache) = state.geometry_cache.lock() {
        geometry_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    fn image(value: u8) -> Arc<DynamicImage> {
        Arc::new(DynamicImage::ImageLuma8(image::ImageBuffer::from_pixel(
            2,
            2,
            image::Luma([value]),
        )))
    }

    fn file_with(contents: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn fingerprint_changes_when_file_is_rewritten() {
        let settings = AppSettings::default();
        let file = file_with(b"first capture");
        let before = DecodeFingerprint::for_file(file.path(), &settings).unwrap();

        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(file.path(), b"second capture, same name").unwrap();
        let after = DecodeFingerprint::for_file(file.path(), &settings).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn fingerprint_changes_with_decode_settings() {
        let file = file_with(b"raw");
        let mut settings = AppSettings::default();
        let base = DecodeFingerprint::for_file(file.path(), &settings).unwrap();

        settings.linear_raw_mode = format!("{}-changed", settings.linear_raw_mode);
        assert_ne!(
            base,
            DecodeFingerprint::for_file(file.path(), &settings).unwrap()
        );

        let settings = AppSettings {
            raw_preprocessing_sharpening: Some(0.9),
            ..Default::default()
        };
        assert_ne!(
            base,
            DecodeFingerprint::for_file(file.path(), &settings).unwrap()
        );

        let unchanged = DecodeFingerprint::for_file(file.path(), &AppSettings::default()).unwrap();
        assert_eq!(base, unchanged);
    }

    #[test]
    fn stale_entry_is_dropped_instead_of_returned() {
        let settings = AppSettings::default();
        let file = file_with(b"old");
        let old = DecodeFingerprint::for_file(file.path(), &settings).unwrap();
        let mut cache = DecodedImageCache::new(3);
        cache.insert("a".into(), old.clone(), image(1), HashMap::new());
        assert!(cache.get("a", &old).is_some());

        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(file.path(), b"new contents").unwrap();
        let new = DecodeFingerprint::for_file(file.path(), &settings).unwrap();

        assert!(cache.get("a", &new).is_none());
        assert!(
            cache.get("a", &old).is_none(),
            "stale entry must be evicted"
        );
    }

    #[test]
    fn least_recently_used_entry_is_evicted() {
        let file = file_with(b"x");
        let fp = DecodeFingerprint::for_file(file.path(), &AppSettings::default()).unwrap();
        let mut cache = DecodedImageCache::new(2);
        cache.insert("a".into(), fp.clone(), image(1), HashMap::new());
        cache.insert("b".into(), fp.clone(), image(2), HashMap::new());
        cache.get("a", &fp);
        cache.insert("c".into(), fp.clone(), image(3), HashMap::new());

        assert!(cache.get("b", &fp).is_none());
        assert!(cache.get("a", &fp).is_some());
        assert!(cache.get("c", &fp).is_some());
    }

    #[test]
    fn concurrent_decodes_of_one_file_run_once() {
        let flights = Arc::new(DecodeFlights::default());
        let extra_leaders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let registered = Arc::new(std::sync::Barrier::new(5));
        let leader = match flights.begin("a") {
            FlightRole::Leader(guard) => guard,
            FlightRole::Follower(_) => panic!("first caller must lead"),
        };

        let waiters: Vec<_> = (0..4)
            .map(|_| {
                let flights = Arc::clone(&flights);
                let extra_leaders = Arc::clone(&extra_leaders);
                let registered = Arc::clone(&registered);
                std::thread::spawn(move || {
                    let role = flights.begin("a");
                    registered.wait();
                    match role {
                        FlightRole::Follower(flight) => flight.wait(),
                        FlightRole::Leader(_) => {
                            extra_leaders.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                })
            })
            .collect();
        registered.wait();
        drop(leader);
        for waiter in waiters {
            waiter.join().unwrap();
        }

        assert_eq!(extra_leaders.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(matches!(flights.begin("a"), FlightRole::Leader(_)));
    }
}
