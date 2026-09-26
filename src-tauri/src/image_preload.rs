use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use tauri::Manager;

use crate::AppState;
use crate::app_settings::{AppSettings, load_settings};
use crate::file_management::parse_virtual_path;
use crate::image_loader::get_or_decode_pristine;

pub struct PreloadJob {
    paths: Vec<String>,
    generation: usize,
}

/// Each request replaces the previous one, so only the latest neighbours are decoded.
pub fn start_preload_worker(app_handle: tauri::AppHandle) {
    let (tx, rx): (Sender<PreloadJob>, Receiver<PreloadJob>) = mpsc::channel();
    *app_handle
        .state::<AppState>()
        .preload_worker_tx
        .lock()
        .unwrap() = Some(tx);

    std::thread::spawn(move || {
        let pool = match preload_pool() {
            Ok(pool) => pool,
            Err(e) => {
                log::error!("Failed to start the image preload pool: {}", e);
                return;
            }
        };

        while let Ok(mut job) = rx.recv() {
            while let Ok(latest) = rx.try_recv() {
                job = latest;
            }
            let settings = load_settings(app_handle.clone()).unwrap_or_default();
            if preload_enabled(&settings) {
                let state = app_handle.state::<AppState>();
                run_preload(&state, &pool, &settings, &job.paths, job.generation);
            }
        }
    });
}

/// The current image and both neighbours must fit in the decoded-image cache with room to spare.
pub const PRELOAD_MIN_CACHE_SIZE: u32 = 4;

pub fn preload_enabled(settings: &AppSettings) -> bool {
    settings.image_cache_size.unwrap_or(5) >= PRELOAD_MIN_CACHE_SIZE
}

/// Half the cores, so preloading leaves room for the image being viewed.
pub fn preload_pool() -> Result<rayon::ThreadPool, rayon::ThreadPoolBuildError> {
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1))
        .unwrap_or(2);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("image-preload-{}", i))
        .build()
}

pub fn run_preload(
    state: &AppState,
    pool: &rayon::ThreadPool,
    settings: &AppSettings,
    paths: &[String],
    generation: usize,
) {
    for path in paths {
        if state.preload_generation.load(Ordering::SeqCst) != generation {
            break;
        }
        let (source_path, _) = parse_virtual_path(path);
        let start = Instant::now();
        let cancel_token = Some((state.preload_generation.clone(), generation));
        let result =
            pool.install(|| get_or_decode_pristine(state, &source_path, settings, cancel_token));
        match result {
            Ok(_) => log::info!(
                "Preloaded '{}' in {:?}",
                source_path.display(),
                start.elapsed()
            ),
            Err(e) if e.contains("cancelled") => break,
            Err(e) => log::warn!("Preloading '{}' failed: {}", source_path.display(), e),
        }
    }
}

#[tauri::command]
pub fn preload_images(paths: Vec<String>, state: tauri::State<'_, AppState>) {
    let generation = state.preload_generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(tx) = state.preload_worker_tx.lock().unwrap().as_ref() {
        let _ = tx.send(PreloadJob { paths, generation });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preloading_needs_a_cache_of_at_least_four_images() {
        let with_cache = |size| AppSettings {
            image_cache_size: size,
            ..Default::default()
        };
        assert!(!preload_enabled(&with_cache(Some(2))));
        assert!(!preload_enabled(&with_cache(Some(3))));
        assert!(preload_enabled(&with_cache(Some(4))));
        assert!(preload_enabled(&with_cache(Some(10))));
        assert!(
            preload_enabled(&with_cache(None)),
            "unset means the default of 5"
        );
    }
}
