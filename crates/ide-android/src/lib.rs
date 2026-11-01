//! Android toolchain support: SDK discovery ([`AndroidSdk`]), adb ([`Adb`]):
//! devices, installs, launches, logcat streams and the device file system, and
//! the emulator ([`start_emulator`]).
//!
//! Every function blocks; callers run them off the UI / command threads.

mod adb;
mod compose_preview;
mod deploy;
mod device_fs;
mod emulator;
mod layoutlib;
mod preview_model;
mod process;
mod project;
mod sdk;

pub use adb::{Adb, Device, LogEntry, LogFilter, parse_devices, parse_threadtime};
pub use compose_preview::render_compose_preview;
pub use deploy::{DeployResult, deploy};
pub use device_fs::{DeviceEntry, DeviceKind, parse_ls};
pub use emulator::{list_avds, parse_avds, start_emulator};
pub use layoutlib::{Layoutlib, RenderRequest, compile_bridge, render, studio_home};
pub use preview_model::{PreviewModel, resolve_preview_model};
pub use project::{AndroidModule, assemble_task, find_apk, modules};
pub use sdk::{AndroidPlatform, AndroidSdk, discover, is_android_project, local_properties_sdk};

/// Bridge bundled into the application rather than loaded from a project.
pub const RENDER_BRIDGE_SOURCE: &str = include_str!("../bridge/RenderBridge.java");
