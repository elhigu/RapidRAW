//! Headless benchmark of the image pipeline (`rapidraw bench`). See bench/README.md.
//!
//! Runs the internal functions behind editor actions on real images and times them, so two
//! builds can be compared. Each phase also hashes its output, which shows whether a change
//! kept the output identical.

use std::time::Instant;

use image::{DynamicImage, GenericImageView};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::export_processing::{ExportSettings, TiffBitDepth};
use crate::gpu_processing::{RenderRequest, get_or_init_gpu_context};
use crate::image_processing::GpuContext;

const USAGE: &str = "Usage: rapidraw bench <image>... [--iters n] [--phases list] [--json file]";

/// Phases in the order they run, with a description of what each one times.
#[rustfmt::skip]
const PHASES: &[(&str, &str)] = &[
    ("decode", "decode and develop the full image (load_base_image_from_bytes)"),
    ("rotate", "rotate the full image by 1.5 degrees (apply_rotation)"),
    ("crop", "crop the full image to its center 80 % (apply_crop)"),
    ("warp", "perspective and distortion correction (warp_image_geometry)"),
    ("downscale", "downscale to a 1920 px preview (downscale_f32_image)"),
    ("gpu-upload", "full-size GPU render including the texture upload"),
    ("gpu", "full-size GPU render of an already uploaded image"),
    ("encode-jpeg", "encode the rendered image as JPEG, quality 90"),
    ("encode-tiff", "encode the rendered image as 16-bit TIFF"),
    ("thumbnail", "generate a library thumbnail (generate_thumbnail_data)"),
    ("open", "open the image in the editor with empty caches (load_image)"),
    ("first-frame", "first editor preview after opening (process_preview_job)"),
    ("style", "editor preview after a style change"),
    ("drag", "live editor preview while dragging the exposure slider"),
    ("geometry", "editor preview after a rotation change"),
    ("full-preview", "full-resolution render with edits (generate_preview_for_path)"),
    ("export", "export as JPEG without writing the file: decode, edit, render, encode"),
];

#[derive(Clone, Debug)]
pub struct BenchSession {
    pub images: Vec<String>,
    pub iters: usize,
    pub phases: Vec<String>,
    pub json_out: Option<String>,
}

pub fn parse_bench_args(args: &[String]) -> Result<BenchSession, String> {
    let mut session = BenchSession {
        images: Vec::new(),
        iters: 5,
        phases: PHASES.iter().map(|(name, _)| name.to_string()).collect(),
        json_out: None,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("Missing value for {}", arg))
        };
        match arg.as_str() {
            "--iters" => {
                session.iters = value()?
                    .parse()
                    .map_err(|_| "Invalid --iters".to_string())?
            }
            "--phases" => {
                session.phases = value()?.split(',').map(|s| s.trim().to_string()).collect();
                if let Some(unknown) = session
                    .phases
                    .iter()
                    .find(|p| !PHASES.iter().any(|(name, _)| name == p))
                {
                    let known: Vec<&str> = PHASES.iter().map(|(name, _)| *name).collect();
                    return Err(format!(
                        "Unknown phase '{}'; expected a comma-separated subset of {}",
                        unknown,
                        known.join(",")
                    ));
                }
            }
            "--json" => session.json_out = Some(value()?),
            s if !s.starts_with('-') => session.images.push(s.to_string()),
            other => return Err(format!("Unknown bench argument '{}'. {}", other, USAGE)),
        }
    }
    if session.images.is_empty() {
        return Err(USAGE.to_string());
    }
    session.iters = session.iters.max(1);
    Ok(session)
}

/// Durations of one phase's iterations and a hash over all of their outputs.
struct Phase {
    name: &'static str,
    millis: Vec<f64>,
    hasher: blake3::Hasher,
}

impl Phase {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            millis: Vec::new(),
            hasher: blake3::Hasher::new(),
        }
    }

    /// Times one call of `f`. The output is hashed afterwards, outside the measurement.
    fn run<T>(&mut self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let start = Instant::now();
        let out = f();
        self.millis.push(start.elapsed().as_secs_f64() * 1000.0);
        out.map_err(|e| format!("{}: {}", self.name, e))
    }

    fn hash_image(&mut self, image: &DynamicImage) {
        let (width, height) = image.dimensions();
        self.hasher.update(&width.to_le_bytes());
        self.hasher.update(&height.to_le_bytes());
        self.hasher
            .update(format!("{:?}", image.color()).as_bytes());
        self.hasher.update(image.as_bytes());
    }

    fn hash_bytes(&mut self, bytes: &[u8]) {
        self.hasher.update(&(bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
    }

    /// Hashes the pixels of an editor preview, so that two JPEG encodings of the same
    /// pixels count as identical. Anything before the JPEG data, such as the ROI header of
    /// an interactive preview, is hashed as bytes.
    fn hash_preview(&mut self, response: &[u8]) {
        let jpeg_start = response
            .windows(3)
            .position(|w| w == [0xFF, 0xD8, 0xFF])
            .unwrap_or(response.len());
        self.hash_bytes(&response[..jpeg_start]);
        match image::load_from_memory(&response[jpeg_start..]) {
            Ok(decoded) => self.hash_image(&decoded),
            Err(_) => self.hash_bytes(&response[jpeg_start..]),
        }
    }

    fn summary(&self) -> Value {
        let mut sorted = self.millis.clone();
        sorted.sort_by(f64::total_cmp);
        let mid = sorted.len() / 2;
        let median = if sorted.len().is_multiple_of(2) {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        };
        json!({
            "ms": self.millis,
            "median_ms": median,
            "min_ms": sorted[0],
            "output": self.hasher.finalize().to_hex().to_string(),
        })
    }
}

fn load_adjustments(path: &str) -> Value {
    std::fs::read_to_string(format!("{}.rrdata", path))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|sidecar| sidecar.get("adjustments").cloned())
        .filter(|adjustments| adjustments.is_object())
        .unwrap_or_else(|| json!({}))
}

fn with_values(adjustments: &Value, values: &[(&str, f64)]) -> Value {
    let mut adjustments = adjustments.clone();
    if let Some(object) = adjustments.as_object_mut() {
        for (key, value) in values {
            object.insert(key.to_string(), json!(value));
        }
    }
    adjustments
}

/// The edits of a style change, different in each iteration.
fn with_style(adjustments: &Value, i: usize) -> Value {
    let t = (i % 7) as f64;
    with_values(
        adjustments,
        &[
            ("exposure", 0.1 * t - 0.3),
            ("contrast", 5.0 * t),
            ("highlights", -10.0 * t),
            ("shadows", 8.0 * t),
            ("saturation", 3.0 * t - 10.0),
            ("vibrance", 4.0 * t),
            ("clarity", 6.0 * t),
            ("temperature", 2.0 * t - 6.0),
        ],
    )
}

fn export_settings() -> ExportSettings {
    ExportSettings {
        jpeg_quality: 90,
        tiff_bit_depth: TiffBitDepth::Sixteen,
        resize: None,
        border: None,
        pad: None,
        keep_metadata: false,
        preserve_timestamps: false,
        strip_gps: false,
        filename_template: None,
        watermark: None,
        export_masks: false,
        preserve_folders: false,
        destination_type: None,
        subfolder: None,
        dpi: None,
    }
}

fn response_bytes(response: tauri::ipc::Response) -> Vec<u8> {
    match tauri::ipc::IpcResponse::body(response) {
        Ok(tauri::ipc::InvokeResponseBody::Raw(bytes)) => bytes,
        Ok(tauri::ipc::InvokeResponseBody::Json(text)) => text.into_bytes(),
        Err(_) => Vec::new(),
    }
}

pub async fn run_headless_bench(
    session: BenchSession,
    app_handle: AppHandle,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    let context = get_or_init_gpu_context(&state, &app_handle)?;
    let adapter = context.device.adapter_info();
    let adapter = format!(
        "{} ({:?}, {:?}, {} {})",
        adapter.name, adapter.backend, adapter.device_type, adapter.driver, adapter.driver_info
    );
    let settings = crate::app_settings::load_settings(app_handle.clone()).unwrap_or_default();

    cli_println!("RapidRAW pipeline benchmark");
    cli_println!("  GPU         {}", adapter);
    cli_println!("  CPU threads {}", rayon::current_num_threads());
    cli_println!("  iterations  {} per phase", session.iters);

    let mut images = Vec::new();
    for path in &session.images {
        cli_println!("\n{}", path);
        let phases = bench_image(&session, path, &context, &app_handle).await?;
        let mut results = serde_json::Map::new();
        for phase in &phases {
            let summary = phase.summary();
            cli_println!(
                "  {:<13} median {:>9.1} ms   min {:>9.1} ms   output {}",
                phase.name,
                summary["median_ms"].as_f64().unwrap_or(0.0),
                summary["min_ms"].as_f64().unwrap_or(0.0),
                &summary["output"].as_str().unwrap_or("")[..12]
            );
            results.insert(phase.name.to_string(), summary);
        }
        images.push(json!({ "image": path, "phases": results }));
    }

    if let Some(out) = &session.json_out {
        // Settings that change the output or the work done, so that compare-bench.mjs can
        // warn when two runs used different settings.
        let report = json!({
            "meta": {
                "gpu_adapter": adapter,
                "cpu_threads": rayon::current_num_threads(),
                "iterations": session.iters,
                "editor_preview_resolution": settings.editor_preview_resolution,
                "live_preview_quality": settings.live_preview_quality,
                "raw_highlight_compression": settings.raw_highlight_compression,
                "linear_raw_mode": settings.linear_raw_mode,
            },
            "phases": PHASES
                .iter()
                .map(|(name, description)| json!({ "name": name, "description": description }))
                .collect::<Vec<_>>(),
            "images": images,
        });
        std::fs::write(out, serde_json::to_string_pretty(&report).unwrap())
            .map_err(|e| format!("Failed to write {}: {}", out, e))?;
        cli_println!("\nWrote {}", out);
    }
    Ok(())
}

async fn bench_image(
    session: &BenchSession,
    path: &str,
    context: &GpuContext,
    app_handle: &AppHandle,
) -> Result<Vec<Phase>, String> {
    let state = app_handle.state::<AppState>();
    let settings = crate::app_settings::load_settings(app_handle.clone()).unwrap_or_default();
    let adjustments = load_adjustments(path);
    let is_raw = crate::formats::is_raw_file(path);
    let file_bytes = std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;
    let n = session.iters;
    let wants = |name: &str| session.phases.iter().any(|p| p == name);
    let mut phases = Vec::new();

    // The full image that the function phases work on, decoded as when opening it.
    let mut decode = Phase::new("decode");
    let mut base = None;
    for _ in 0..if wants("decode") { n } else { 1 } {
        let image = decode.run(|| {
            crate::image_loader::load_base_image_from_bytes(
                &file_bytes,
                path,
                false,
                &settings,
                None,
            )
            .map_err(|e| e.to_string())
        })?;
        decode.hash_image(&image);
        base = Some(image);
    }
    let base = base.unwrap();
    let (width, height) = base.dimensions();
    if wants("decode") {
        phases.push(decode);
    }

    if wants("rotate") {
        let mut phase = Phase::new("rotate");
        for _ in 0..n {
            let out = phase.run(|| Ok(crate::image_processing::apply_rotation(&base, 1.5)))?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    if wants("crop") {
        let mut phase = Phase::new("crop");
        let crop = json!({
            "x": width / 10, "y": height / 10, "width": width * 8 / 10, "height": height * 8 / 10
        });
        for _ in 0..n {
            let out = phase.run(|| Ok(crate::image_processing::apply_crop(&base, &crop)))?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    if wants("warp") {
        let mut phase = Phase::new("warp");
        let params = crate::image_processing::get_geometry_params_from_json(&json!({
            "transformVertical": 12.0,
            "transformHorizontal": -6.0,
            "transformDistortion": 8.0,
            "transformRotate": 1.0,
        }));
        for _ in 0..n {
            let out = phase.run(|| {
                Ok(crate::image_processing::warp_image_geometry(
                    &base,
                    params.clone(),
                ))
            })?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    if wants("downscale") {
        let mut phase = Phase::new("downscale");
        let scale = 1920.0 / width.max(height) as f32;
        let (w, h) = (
            (width as f32 * scale).round() as u32,
            (height as f32 * scale).round() as u32,
        );
        for _ in 0..n {
            let out =
                phase.run(|| Ok(crate::image_processing::downscale_f32_image(&base, w, h)))?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    let render = crate::image_processing::resolve_render_options_from_handle(app_handle, is_raw);
    let gpu_render = |adjustments: &Value| {
        crate::gpu_processing::process_and_get_dynamic_image(
            context,
            &state,
            &base,
            crate::cache_utils::calculate_transform_hash(&json!({ "bench": path })),
            RenderRequest {
                adjustments: crate::image_processing::get_all_adjustments_from_json(
                    adjustments,
                    is_raw,
                    crate::as_shot::as_shot(path),
                    render,
                ),
                mask_bitmaps: &[],
                lut: None,
                roi: None,
            },
            "bench",
        )
    };

    if wants("gpu-upload") {
        let mut phase = Phase::new("gpu-upload");
        for _ in 0..n {
            *state.gpu_image_cache.lock().unwrap() = None;
            let out = phase.run(|| gpu_render(&adjustments))?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    if wants("gpu") {
        let mut phase = Phase::new("gpu");
        gpu_render(&adjustments)?;
        for i in 0..n {
            let style = with_style(&adjustments, i);
            let out = phase.run(|| gpu_render(&style))?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }
    *state.gpu_image_cache.lock().unwrap() = None;

    let export = export_settings();
    let export_render = |base: &DynamicImage, format: &str| {
        crate::export_processing::process_image_for_export(
            path,
            base,
            &adjustments,
            &export,
            context,
            &state,
            is_raw,
            app_handle,
            format,
        )
    };
    for (name, format) in [("encode-jpeg", "jpg"), ("encode-tiff", "tiff")] {
        if wants(name) {
            let mut phase = Phase::new(name);
            let rendered = export_render(&base, format)?;
            for _ in 0..n {
                let out = phase.run(|| {
                    crate::export_processing::encode_image_to_bytes(&rendered, format, &export)
                })?;
                phase.hash_bytes(&out);
            }
            phases.push(phase);
        }
    }
    drop(base);

    if wants("thumbnail") {
        let mut phase = Phase::new("thumbnail");
        for _ in 0..n {
            // Otherwise later iterations reuse the first one's decoded image.
            state.thumbnail_geometry_cache.lock().unwrap().clear();
            let out = phase.run(|| {
                crate::file_management::generate_thumbnail_data(
                    path,
                    Some(context),
                    None,
                    app_handle,
                )
                .map_err(|e| e.to_string())
            })?;
            phase.hash_image(&out);
        }
        phases.push(phase);
    }

    // Editor actions. These need the image opened in the editor, so `open` always runs.
    let editor_phases = ["open", "first-frame", "style", "drag", "geometry"];
    if editor_phases.iter().any(|p| wants(p)) {
        let mut phase = Phase::new("open");
        for _ in 0..if wants("open") { n } else { 1 } {
            crate::cache_utils::clear_image_caches(app_handle.state::<AppState>());
            let start = Instant::now();
            let result = crate::image_loader::load_image(
                path.to_string(),
                app_handle.state::<AppState>(),
                app_handle.clone(),
            )
            .await;
            phase.millis.push(start.elapsed().as_secs_f64() * 1000.0);
            result.map_err(|e| format!("open: {}", e))?;
            if let Some(loaded) = state.original_image.lock().unwrap().as_ref() {
                phase.hash_image(&loaded.image);
            }
        }
        if wants("open") {
            phases.push(phase);
        }

        let preview = |adjustments: Value, interactive: bool| {
            crate::process_preview_job(
                app_handle,
                app_handle.state::<AppState>(),
                adjustments,
                interactive,
                None,
                None,
                false,
                false,
                None,
            )
        };

        let mut phase = Phase::new("first-frame");
        for _ in 0..if wants("first-frame") { n } else { 1 } {
            crate::cache_utils::clear_image_caches(app_handle.state::<AppState>());
            let out = phase.run(|| preview(adjustments.clone(), false))?;
            phase.hash_preview(&out);
        }
        if wants("first-frame") {
            phases.push(phase);
        }

        if wants("style") {
            let mut phase = Phase::new("style");
            for i in 0..n {
                let out = phase.run(|| preview(with_style(&adjustments, i), false))?;
                phase.hash_preview(&out);
            }
            phases.push(phase);
        }

        if wants("drag") {
            let mut phase = Phase::new("drag");
            let exposure = adjustments["exposure"].as_f64().unwrap_or(0.0);
            for i in 0..n {
                let adjustments = with_values(
                    &adjustments,
                    &[("exposure", exposure + 0.02 * (i + 1) as f64)],
                );
                let out = phase.run(|| preview(adjustments, true))?;
                phase.hash_preview(&out);
            }
            phases.push(phase);
        }

        if wants("geometry") {
            let mut phase = Phase::new("geometry");
            for i in 0..n {
                let adjustments =
                    with_values(&adjustments, &[("rotation", 0.25 * ((i % 8) + 1) as f64)]);
                let out = phase.run(|| preview(adjustments, false))?;
                phase.hash_preview(&out);
            }
            phases.push(phase);
        }
    }

    if wants("full-preview") {
        let mut phase = Phase::new("full-preview");
        for i in 0..n {
            let start = Instant::now();
            let result = crate::generate_preview_for_path(
                path.to_string(),
                with_style(&adjustments, i),
                app_handle.clone(),
            )
            .await;
            phase.millis.push(start.elapsed().as_secs_f64() * 1000.0);
            let response = result.map_err(|e| format!("full-preview: {}", e))?;
            phase.hash_preview(&response_bytes(response));
        }
        phases.push(phase);
    }

    if wants("export") {
        let mut phase = Phase::new("export");
        for _ in 0..n {
            let out = phase.run(|| {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                let base = crate::image_loader::load_and_composite(
                    &bytes,
                    path,
                    &adjustments,
                    false,
                    &settings,
                    None,
                )
                .map_err(|e| e.to_string())?;
                let rendered = export_render(&base, "jpg")?;
                crate::export_processing::encode_image_to_bytes(&rendered, "jpg", &export)
            })?;
            phase.hash_bytes(&out);
        }
        phases.push(phase);
    }

    // Leave nothing cached for the next image.
    crate::cache_utils::clear_image_caches(app_handle.state::<AppState>());
    *state.original_image.lock().unwrap() = None;
    Ok(phases)
}
