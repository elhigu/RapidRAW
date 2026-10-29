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
