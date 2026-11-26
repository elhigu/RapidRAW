use ort::AsPointer;
use ort::session::builder::SessionBuilder;
use ort::sys::{OrtEpDevice, OrtHardwareDeviceType, OrtStatusPtr};
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::OnceLock;

const REGISTRATION_NAME: &CStr = c"WebGPU";
const EP_NAME: &CStr = c"WebGpuExecutionProvider";
const LIBRARY: &str = "libonnxruntime_providers_webgpu.so";

fn check(status: OrtStatusPtr) -> Result<(), String> {
    if status.0.is_null() {
        return Ok(());
    }
    let api = ort::api();
    let message = unsafe {
        let message = CStr::from_ptr((api.GetErrorMessage)(status.0))
            .to_string_lossy()
            .into_owned();
        (api.ReleaseStatus)(status.0);
        message
    };
    Err(message)
}

fn library_path() -> Option<PathBuf> {
    let ort_library = PathBuf::from(std::env::var_os("ORT_DYLIB_PATH")?);
    Some(ort_library.with_file_name(LIBRARY)).filter(|path| path.exists())
}

fn register() -> &'static Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let path = library_path().ok_or_else(|| format!("{} not found", LIBRARY))?;
        let path = CString::new(path.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
        let environment = ort::environment::get_environment().map_err(|e| e.to_string())?;
        check(unsafe {
            (ort::api().RegisterExecutionProviderLibrary)(
                environment.ptr().cast_mut(),
                REGISTRATION_NAME.as_ptr(),
                path.as_ptr(),
            )
        })
        .inspect(|_| log::info!("Registered the WebGPU execution provider"))
        .inspect_err(|e| log::warn!("WebGPU execution provider unavailable: {}", e))
    })
}

fn gpu_devices() -> Result<Vec<*const OrtEpDevice>, String> {
    register().clone()?;
    let api = ort::api();
    let environment = ort::environment::get_environment().map_err(|e| e.to_string())?;
    let mut devices: *const *const OrtEpDevice = std::ptr::null();
    let mut count = 0;
    check(unsafe { (api.GetEpDevices)(environment.ptr(), &mut devices, &mut count) })?;
    if devices.is_null() {
        return Ok(Vec::new());
    }
    let devices = unsafe { std::slice::from_raw_parts(devices, count) };
    Ok(devices
        .iter()
        .copied()
        .filter(|&device| unsafe {
            CStr::from_ptr((api.EpDevice_EpName)(device)) == EP_NAME
                && (api.HardwareDevice_Type)((api.EpDevice_Device)(device))
                    == OrtHardwareDeviceType::OrtHardwareDeviceType_GPU
        })
        .collect())
}

pub fn available() -> bool {
    gpu_devices().is_ok_and(|devices| !devices.is_empty())
}

pub fn append_to(builder: &mut SessionBuilder) -> Result<(), String> {
    let devices = gpu_devices()?;
    let device = *devices.first().ok_or("no WebGPU device")?;
    let environment = ort::environment::get_environment().map_err(|e| e.to_string())?;
    let keys: [*const c_char; 1] = [c"powerPreference".as_ptr()];
    let values: [*const c_char; 1] = [c"high-performance".as_ptr()];
    check(unsafe {
        (ort::api().SessionOptionsAppendExecutionProvider_V2)(
            builder.ptr_mut(),
            environment.ptr().cast_mut(),
            &device,
            1,
            keys.as_ptr(),
            values.as_ptr(),
            keys.len(),
        )
    })
}
