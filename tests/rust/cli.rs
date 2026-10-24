// Command-line export runs the app binary. Invalid arguments must fail before the app opens a
// display, so these tests also run on machines without one.

use std::process::Command;

fn export_with_tiff_bit_depth(value: Option<&str>) -> (std::process::Output, tempfile::TempDir) {
    let output_dir = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_RapidRAW"));
    command
        .args(["export", "photo.png", "--output"])
        .arg(output_dir.path())
        .arg("--tiff-bit-depth")
        .args(value)
        // GTK finds a display through any of these.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("XDG_RUNTIME_DIR", runtime_dir.path());
    (command.output().unwrap(), output_dir)
}

#[test]
fn invalid_tiff_bit_depth_fails_without_exporting() {
    for value in [Some("12"), Some("abc"), None] {
        let (output, output_dir) = export_with_tiff_bit_depth(value);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{value:?}: {stderr}");
        assert!(
            stderr.contains("Headless export failed:"),
            "{value:?}: {stderr}"
        );
        assert!(stderr.contains("expected 8 or 16"), "{value:?}: {stderr}");
        assert_eq!(std::fs::read_dir(output_dir.path()).unwrap().count(), 0);
    }
}
