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

#[test]
fn compacted_entry_is_found_by_fingerprint_and_dropped_when_stale() {
    let settings = AppSettings::default();
    let file = file_with(b"raw");
    let fingerprint = DecodeFingerprint::for_file(file.path(), &settings).unwrap();
    // Values that f16 represents exactly, so the compacted copy expands to the same pixels.
    let pixels = vec![0.25f32, 0.5, 1.0, 2.0, 0.125, 0.0];
    let rgb = Arc::new(DynamicImage::ImageRgb32F(
        Rgb32FImage::from_raw(2, 1, pixels.clone()).unwrap(),
    ));
    let mut cache = DecodedImageCache::new(3);
    cache.insert("a".into(), fingerprint.clone(), rgb, HashMap::new());
    cache.insert("b".into(), fingerprint.clone(), image(1), HashMap::new());

    cache.compact_except("b");
    let (expanded, _) = cache
        .get("a", &fingerprint)
        .expect("compacted entry is still cached");
    assert_eq!(expanded.as_rgb32f().unwrap().as_raw(), &pixels);
    drop(expanded);

    cache.compact_except("b");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(file.path(), b"rewritten").unwrap();
    let stale = DecodeFingerprint::for_file(file.path(), &settings).unwrap();
    assert!(cache.get("a", &stale).is_none());
    assert!(
        !cache.contains("a", &fingerprint),
        "stale entry must be evicted"
    );
}
