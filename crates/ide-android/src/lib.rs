//! Android toolchain support: SDK discovery ([`AndroidSdk`]), adb ([`Adb`]):
//! devices, installs, launches, logcat streams and the device file system, and
//! the emulator ([`start_emulator`]).
//!
//! Every function blocks; callers run them off the UI / command threads.

mod adb;
mod deploy;
mod device_fs;
mod emulator;
mod process;
mod project;
mod sdk;

pub use adb::{Adb, Device, LogEntry, LogFilter, parse_devices, parse_threadtime};
pub use deploy::{DeployResult, deploy};
pub use device_fs::{DeviceEntry, DeviceKind, parse_ls};
pub use emulator::{list_avds, parse_avds, start_emulator};
pub use project::{AndroidModule, assemble_task, find_apk, modules};
pub use sdk::{AndroidPlatform, AndroidSdk, discover, is_android_project, local_properties_sdk};
