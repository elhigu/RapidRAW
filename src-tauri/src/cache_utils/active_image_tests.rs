use super::*;

fn fingerprint() -> DecodeFingerprint {
    DecodeFingerprint {
        len: 1,
        modified: None,
        settings_hash: 0,
    }
}

fn insert(cache: &mut DecodedImageCache, path: &str) {
    let image = Arc::new(DynamicImage::ImageRgb32F(Rgb32FImage::new(4, 4)));
    cache.insert(path.to_string(), fingerprint(), image, HashMap::new());
}

#[test]
fn the_active_image_is_never_evicted() {
    let mut cache = DecodedImageCache::new(2);
    insert(&mut cache, "current");
    cache.compact_except("current");
    insert(&mut cache, "next");
    insert(&mut cache, "previous");
    assert!(cache.contains("current", &fingerprint()));
    assert!(cache.contains("previous", &fingerprint()));
    assert!(!cache.contains("next", &fingerprint()));
}

#[test]
fn without_an_active_image_the_oldest_is_evicted() {
    let mut cache = DecodedImageCache::new(2);
    for path in ["a", "b", "c"] {
        insert(&mut cache, path);
    }
    assert!(!cache.contains("a", &fingerprint()));
    assert!(cache.contains("b", &fingerprint()));
    assert!(cache.contains("c", &fingerprint()));
}
