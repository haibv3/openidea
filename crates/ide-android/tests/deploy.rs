//! End-to-end deploy against fake `gradlew` and `adb` scripts (Unix).
#![cfg(unix)]

use ide_android::{Adb, deploy};
use ide_core::CancellationToken;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn builds_installs_and_launches_the_app() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.gradle"), "include ':app'\n").unwrap();
    std::fs::create_dir_all(dir.path().join("app")).unwrap();
    std::fs::write(
        dir.path().join("app/build.gradle"),
        "plugins { id 'com.android.application' }\nandroid {\n  defaultConfig { applicationId 'com.example.app' }\n}\n",
    )
    .unwrap();
    script(
        &dir.path().join("gradlew"),
        "#!/bin/sh\nmkdir -p app/build/outputs/apk/debug\necho apk > app/build/outputs/apk/debug/app-debug.apk\necho BUILD SUCCESSFUL\n",
    );
    script(
        &dir.path().join("adb"),
        "#!/bin/sh\ncase \"$*\" in\n  *install*) echo Success ;;\n  *monkey*) echo 'Events injected: 1' ;;\nesac\nexit 0\n",
    );

    let mut lines = Vec::new();
    let result = deploy(
        dir.path(),
        &Adb::new(dir.path().join("adb")),
        "emulator-5554",
        "",
        "debug",
        &CancellationToken::new(),
        &mut |line| lines.push(line),
    )
    .unwrap();

    assert_eq!(result.module, "app");
    assert_eq!(result.application_id.as_deref(), Some("com.example.app"));
    assert!(result.apk.0.ends_with("app/build/outputs/apk/debug/app-debug.apk"));
    let output = lines.join("\n");
    assert!(output.contains("BUILD SUCCESSFUL"), "{output}");
    assert!(output.contains("Installing"), "{output}");
    assert!(output.contains("Events injected: 1"), "{output}");
}
