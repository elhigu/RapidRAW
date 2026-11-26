use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

fn verify_sha256(path: &Path, expected_hash: &str) -> Result<bool, io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let hash_bytes = hasher.finalize();
    let calculated_hash = hex::encode(hash_bytes);
    Ok(calculated_hash == expected_hash)
}

fn download_and_verify(
    url: &str,
    dest_path: &Path,
    expected_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let temp_filename = dest_path.file_name().unwrap();
    let temp_path = out_dir.join(temp_filename);

    println!(
        "cargo:warning=Downloading to temporary path: {:?}",
        temp_path
    );
    let mut response = reqwest::blocking::get(url)?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .unwrap_or_else(|_| "Could not read error body".to_string());
        return Err(format!("Download failed with status {}: {}", status, error_body).into());
    }

    let mut temp_file = fs::File::create(&temp_path)?;
    response.copy_to(&mut temp_file)?;
    println!("cargo:warning=Download complete. Verifying file integrity...");

    match verify_sha256(&temp_path, expected_hash) {
        Ok(true) => {
            fs::copy(&temp_path, dest_path)?;
            fs::remove_file(&temp_path)?;
            println!(
                "cargo:warning=Successfully downloaded and verified {:?}.",
                dest_path
            );
            Ok(())
        }
        Ok(false) => {
            fs::remove_file(&temp_path)?;
            Err("Verification failed! The downloaded file is corrupt.".into())
        }
        Err(e) => {
            fs::remove_file(&temp_path).ok();
            Err(format!("Could not verify file after download: {}", e).into())
        }
    }
}

fn runtime_libraries(
    target_os: &str,
    target_arch: &str,
) -> Vec<(&'static str, &'static str, &'static str)> {
    match (target_os, target_arch) {
        ("windows", "x86_64") => vec![
            (
                "onnxruntime-directml-windows-x86_64.dll",
                "onnxruntime.dll",
                "95366724919f4e95ecc60010912ed538ad9804b6683fbd0aad389749102834b9",
            ),
            (
                "directml-windows-x86_64.dll",
                "DirectML.dll",
                "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1",
            ),
        ],
        ("windows", "aarch64") => vec![
            (
                "onnxruntime-directml-windows-aarch64.dll",
                "onnxruntime.dll",
                "c544001fbb76c7217fce76e9c24dada4a35d263b7ae3c0024474d9d7323a888f",
            ),
            (
                "directml-windows-aarch64.dll",
                "DirectML.dll",
                "77b0db83ff903f2323f5caf538499d75af6038bbea23b7959f7d232d9a4ab9d4",
            ),
        ],
        ("linux", "x86_64") => vec![],
        ("linux", "aarch64") => vec![(
            "libonnxruntime-linux-aarch64.so",
            "libonnxruntime.so",
            "0afd69a0ae38c5099fd0e8604dda398ac43dee67cd9c6394b5142b19e82528de",
        )],
        ("macos", "x86_64") => vec![(
            "libonnxruntime-macos-x86_64.dylib",
            "libonnxruntime.dylib",
            "283e595e61cf65df7a6b1d59a1616cbd35c8b6399dd90d799d99b71a3ff83160",
        )],
        ("macos", "aarch64") => vec![(
            "libonnxruntime-macos-aarch64.dylib",
            "libonnxruntime.dylib",
            "2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a",
        )],
        ("android", "aarch64") => vec![(
            "libonnxruntime-android-arm64-v8a.so",
            "libonnxruntime.so",
            "999ecfdb5b5a13e4097487773b6d71ce8a075408a237daab072e8f5e817bd78e",
        )],
        _ => panic!("Unsupported target: {}-{}", target_os, target_arch),
    }
}

struct WheelLibrary {
    url: &'static str,
    wheel_sha256: &'static str,
    member: &'static str,
    lib_name: &'static str,
    sha256: &'static str,
}

fn wheel_libraries(target_os: &str, target_arch: &str) -> Vec<WheelLibrary> {
    match (target_os, target_arch) {
        ("linux", "x86_64") => vec![
            WheelLibrary {
                url: "https://files.pythonhosted.org/packages/92/74/1f7c51522822e93aa91f633ebcd75688e2deebeca5901b2e6da8a8c5ad7c/onnxruntime-1.32.0-cp313-cp313-manylinux_2_28_x86_64.whl",
                wheel_sha256: "02ea12a28c069ea8510093d95a7f6ff8126bfd9fc5c0d3381c03b80fb2b424ba",
                member: "onnxruntime/capi/libonnxruntime.so.1.32.0",
                lib_name: "libonnxruntime.so",
                sha256: "6c1585d1bbf45b0712ea4a21b705c5fcb4ba04ff2e30f1fbf81a49faf658638f",
            },
            WheelLibrary {
                url: "https://files.pythonhosted.org/packages/e2/1d/ac0ccbe8bd6435b6e1946fea964790ffa3d73ee2513775e90958fd6b860e/onnxruntime_ep_webgpu-0.5.0-py3-none-manylinux_2_28_x86_64.whl",
                wheel_sha256: "b6baa3e223d9a679023388b1179e5597b6bc0d282ba72f2afe38d46ef37deeb4",
                member: "onnxruntime_ep_webgpu/libonnxruntime_providers_webgpu.so",
                lib_name: "libonnxruntime_providers_webgpu.so",
                sha256: "fc98b10c7d03537694ff4ea6ebca79ed3b41ca71e316dac3292257ee5f85c95d",
            },
        ],
        _ => vec![],
    }
}

fn ensure_wheel_library(dest_dir: &Path, library: &WheelLibrary) -> PathBuf {
    let dest_path = dest_dir.join(library.lib_name);
    if verify_sha256(&dest_path, library.sha256).unwrap_or(false) {
        return dest_path;
    }
    let wheel_name = library.url.rsplit('/').next().unwrap();
    let wheel_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set")).join("wheels");
    let wheel_path = wheel_dir.join(wheel_name);
    let local_path = env::var_os("RAPIDRAW_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join(wheel_name))
        .filter(|path| path.exists());
    let result = fs::create_dir_all(&wheel_dir)
        .map_err(Into::into)
        .and_then(|()| match local_path {
            Some(local_path) => copy_and_verify(&local_path, &wheel_path, library.wheel_sha256),
            None => download_and_verify(library.url, &wheel_path, library.wheel_sha256),
        })
        .and_then(|()| {
            let mut wheel = zip::ZipArchive::new(fs::File::open(&wheel_path)?)?;
            io::copy(
                &mut wheel.by_name(library.member)?,
                &mut fs::File::create(&dest_path)?,
            )?;
            if !verify_sha256(&dest_path, library.sha256)? {
                return Err(format!("{} has an incorrect hash", library.member).into());
            }
            Ok(())
        });
    if let Err(e) = result {
        panic!("Failed to download and verify {}: {}", library.lib_name, e);
    }
    println!("cargo:warning=Extracted and verified {:?}.", dest_path);
    dest_path
}

fn copy_and_verify(
    source_path: &Path,
    dest_path: &Path,
    expected_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !verify_sha256(source_path, expected_hash)? {
        return Err(format!("Local file {:?} has an incorrect hash.", source_path).into());
    }
    fs::copy(source_path, dest_path)?;
    println!(
        "cargo:warning=Copied and verified {:?} from {:?}.",
        dest_path, source_path
    );
    Ok(())
}

fn ensure_runtime_library(
    dest_dir: &Path,
    download_filename: &str,
    lib_name: &str,
    expected_hash: &str,
) -> PathBuf {
    let dest_path = dest_dir.join(lib_name);

    if dest_path.exists() {
        match verify_sha256(&dest_path, expected_hash) {
            Ok(true) => {
                println!(
                    "cargo:warning={} already exists and is valid. Skipping download.",
                    lib_name
                );
                return dest_path;
            }
            Ok(false) => {
                println!(
                    "cargo:warning=File {:?} exists but has incorrect hash. Deleting and re-downloading.",
                    dest_path
                );
                fs::remove_file(&dest_path).unwrap();
            }
            Err(e) => {
                println!(
                    "cargo:warning=Could not verify file {:?}: {}. Re-downloading.",
                    dest_path, e
                );
            }
        }
    }

    let local_path = env::var_os("RAPIDRAW_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join(download_filename))
        .filter(|path| path.exists());

    let result = if let Some(local_path) = local_path {
        copy_and_verify(&local_path, &dest_path, expected_hash)
    } else {
        println!("cargo:warning=Downloading {}...", download_filename);
        let base_url =
            "https://huggingface.co/CyberTimon/RapidRAW-Models/resolve/main/onnxruntimes-v1.22.0/";
        let download_url = format!("{}{}?download=true", base_url, download_filename);
        println!("cargo:warning=URL: {}", download_url);
        download_and_verify(&download_url, &dest_path, expected_hash)
    };

    if let Err(e) = result {
        panic!("Failed to download and verify {}: {}", lib_name, e);
    }

    dest_path
}

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let dest_dir = if target_os == "android" {
        manifest_dir.join("libs").join("arm64-v8a")
    } else {
        manifest_dir.join("resources")
    };

    fs::create_dir_all(&dest_dir).unwrap();

    let lib_paths: Vec<PathBuf> = runtime_libraries(&target_os, &target_arch)
        .into_iter()
        .map(|(download_filename, lib_name, expected_hash)| {
            ensure_runtime_library(&dest_dir, download_filename, lib_name, expected_hash)
        })
        .chain(
            wheel_libraries(&target_os, &target_arch)
                .iter()
                .map(|library| ensure_wheel_library(&dest_dir, library)),
        )
        .collect();

    if target_os == "android" {
        let jni_libs_dir = manifest_dir.join("gen/android/app/src/main/jniLibs/arm64-v8a");
        fs::create_dir_all(&jni_libs_dir).unwrap();
        for lib_path in &lib_paths {
            fs::copy(lib_path, jni_libs_dir.join(lib_path.file_name().unwrap())).unwrap();
        }

        println!("cargo:rustc-env=ORT_LIB_LOCATION={}", dest_dir.display());
        println!("cargo:rustc-env=ORT_STRATEGY=manual");
        println!("cargo:rustc-link-search=native={}", dest_dir.display());
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RAPIDRAW_RUNTIME_DIR");

    tauri_build::build()
}
