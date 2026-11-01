//! OpenIDEA plugin: live previews for Android XML layouts and Jetpack
//! Compose `@Preview` functions. Build with `../build.sh android-preview`.

mod compose;
mod hosts;
mod layout;
mod render;
mod resources;
mod xml;

use compose::Frame;
use layout::Node;
use render::Screen;
use resources::Resources;

wit_bindgen::generate!({
    world: "plugin-preview",
    path: "../../wit",
});

struct AndroidPreview;

impl Guest for AndroidPreview {
    fn activate(_ctx: Context) -> Result<(), String> {
        Ok(())
    }

    fn deactivate() {}

    fn run_command(_id: String, _arg: Option<String>) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn render_preview(request: PreviewRequest) -> Result<PreviewOutput, String> {
        let title = file_name(&request.path);
        let screens = match request.decl.as_str() {
            "layout-xml" => layout_screens(&request)?,
            "compose" => compose_screens(&request)?,
            other => return Err(format!("unknown preview kind `{other}`")),
        };
        let html = render::page(&title, &screens);
        Ok(PreviewOutput { html, background: None })
    }
}

export!(AndroidPreview);

/// One XML layout file → one screen per toolbar device, each with the node
/// positions re-solved for that viewport: Relative/Constraint rules (and
/// `match_parent` chains under them) depend on the available size, so simply
/// resizing the frame would leave children misplaced.
fn layout_screens(request: &PreviewRequest) -> Result<Vec<Screen>, String> {
    let source = hosts::res_source(&request.project_path, &request.path);
    layout_screens_with(request, &*source)
}

fn layout_screens_with(
    request: &PreviewRequest,
    source: &dyn resources::ResourceSource,
) -> Result<Vec<Screen>, String> {
    let res = Resources::new(source);
    let roots: Vec<Node> = layout::build(&request.content, &res)?;
    Ok(layout::DEVICES
        .iter()
        .enumerate()
        .map(|(device, &(_, w, h))| {
            let mut roots = roots.clone();
            for root in &mut roots {
                layout::resolve_positions(root, w, h);
            }
            Screen {
                caption: None,
                roots,
                width: Some(w),
                height: Some(h),
                bg: None,
                stretch_roots: true,
                device: Some(device),
            }
        })
        .collect())
}

/// Every `@Preview` function in one Kotlin file → one screen per frame.
fn compose_screens(request: &PreviewRequest) -> Result<Vec<Screen>, String> {
    let source = hosts::res_source(&request.project_path, &request.path);
    compose_screens_with(request, &*source)
}

fn compose_screens_with(
    request: &PreviewRequest,
    source: &dyn resources::ResourceSource,
) -> Result<Vec<Screen>, String> {
    let res = Resources::new(source);
    let frames: Vec<Frame> = compose::scan_with(&request.content, Some(&res))?;
    if frames.is_empty() {
        return Err("no @Preview composable found in this file".to_owned());
    }
    Ok(frames
        .into_iter()
        .map(|f| Screen {
            caption: Some(f.caption),
            roots: f.root.map(|r| r.children).unwrap_or_default(),
            width: f.width,
            height: f.height,
            bg: if f.show_background { f.bg_color } else { Some("transparent".to_owned()) },
            stretch_roots: false,
            device: None,
        })
        .collect())
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

#[cfg(test)]
mod fixture_check {
    use super::*;
    use resources::MemorySource;
    use std::path::{Path, PathBuf};

    fn walk(dir: &Path, base: &Path, out: &mut MemorySource) -> MemorySource {
        let mut s = std::mem::replace(out, MemorySource::new());
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                s = walk(&path, base, &mut s);
            } else {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                s = s.file(&rel, &std::fs::read_to_string(&path).unwrap());
            }
        }
        s
    }

    #[test]
    fn fixtures_parse() {
        let main = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/android-app/app/src/main"));
        let mut src = MemorySource::new().module("app");
        src = walk(&main.join("res"), &main.join("res"), &mut src);
        src = src.project_file(
            "app/src/main/AndroidManifest.xml",
            &std::fs::read_to_string(main.join("AndroidManifest.xml")).unwrap(),
        );
        src = src.project_file(
            "app/src/main/java/com/example/openidea/ui/BrandButton.kt",
            &std::fs::read_to_string(main.join("java/com/example/openidea/ui/BrandButton.kt")).unwrap(),
        );
        let res = Resources::new(&src);
        for f in std::fs::read_dir(main.join("res/layout")).unwrap() {
            let p = f.unwrap().path();
            let content = std::fs::read_to_string(&p).unwrap();
            layout::build(&content, &res).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        }
        let kt = std::fs::read_to_string(main.join("java/com/example/openidea/MainActivity.kt")).unwrap();
        let frames = compose::scan_with(&kt, Some(&res)).unwrap();
        assert_eq!(frames.len(), 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_layout_xml_page() {
        let request = PreviewRequest {
            path: "app/src/main/res/layout/activity_main.xml".to_owned(),
            project_path: "/p".to_owned(),
            decl: "layout-xml".to_owned(),
            content: r#"
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent" android:layout_height="match_parent"
    android:orientation="vertical" android:padding="16dp">
    <TextView android:layout_width="wrap_content" android:layout_height="wrap_content"
        android:text="Hello <world>" android:textSize="20sp" android:textStyle="bold"/>
    <Button android:layout_width="wrap_content" android:layout_height="wrap_content"
        android:text="OK &amp; save"/>
</LinearLayout>
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        assert!(out.html.contains("Hello &lt;world&gt;"));
        assert!(out.html.contains("OK &amp; save"));
        assert!(out.html.contains("flex-direction:column"));
        assert!(out.html.contains("16px 16px 16px 16px")); // padding
        assert!(out.html.contains("ap-device"));
    }

    #[test]
    fn layouts_are_resolved_per_device() {
        // A centred child in a RelativeLayout gets a different absolute
        // offset per toolbar device: one screen per DEVICES entry.
        let request = PreviewRequest {
            path: "app/src/main/res/layout/f.xml".to_owned(),
            project_path: "/p".to_owned(),
            decl: "layout-xml".to_owned(),
            content: r#"
<RelativeLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent" android:layout_height="match_parent">
    <TextView android:layout_width="100dp" android:layout_height="24dp"
        android:layout_centerInParent="true" android:text="mid"/>
</RelativeLayout>
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        assert_eq!(out.html.matches("class=\"ap-frame\" data-device").count(), layout::DEVICES.len(), "{}", out.html);
        // (411−100)/2, (360−100)/2, (800−100)/2: each variant solved its own centre.
        assert!(out.html.contains("left:155.5px"), "{}", out.html);
        assert!(out.html.contains("left:130px"), "{}", out.html);
        assert!(out.html.contains("left:350px"), "{}", out.html);
    }

    #[test]
    fn renders_compose_preview_frames() {
        let request = PreviewRequest {
            path: "app/src/main/java/MainActivity.kt".to_owned(),
            project_path: "/p".to_owned(),
            decl: "compose".to_owned(),
            content: r#"
@Preview(showBackground = true, backgroundColor = 0xFFEEEEEE)
@Composable
fun One() {
    Column {
        Text("Hello")
    }
}

@Preview
@Composable
fun Two() {
    Text("Second")
}
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        assert!(out.html.contains("Hello"));
        assert!(out.html.contains("Second"));
        assert!(out.html.contains("One"));
        assert!(out.html.contains("Two"));
        assert_eq!(out.html.matches("class=\"ap-screen\"").count(), 2);
    }

    #[test]
    fn preview_frame_sizes_survive_the_device_toolbar() {
        let request = PreviewRequest {
            path: "app/src/main/java/MainActivity.kt".to_owned(),
            project_path: "/p".to_owned(),
            decl: "compose".to_owned(),
            content: r#"
@Preview(widthDp = 320, heightDp = 200)
@Composable
fun Sized() {
    Text("Sized")
}

@Preview
@Composable
fun Free() {
    Text("Free")
}
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        // The pinned frame keeps both axes; the free one follows the toolbar.
        assert!(out.html.contains("data-fixed-w data-fixed-h"));
        assert!(out.html.contains("width:320px;height:200px;"));
        assert_eq!(out.html.matches("class=\"ap-screen\"").count(), 2);
        assert!(out.html.contains("320×200 dp"));
        // Compose roots are content-sized, not stretched to the device.
        assert!(!out.html.contains("width:100%;height:100%;overflow:hidden;"));
    }

    #[test]
    fn widgets_get_material_defaults() {
        let request = PreviewRequest {
            path: "app/src/main/res/layout/x.xml".to_owned(),
            project_path: "/p".to_owned(),
            decl: "layout-xml".to_owned(),
            content: r#"
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent" android:layout_height="match_parent">
    <Button android:layout_width="wrap_content" android:layout_height="wrap_content" android:text="Go"/>
    <CheckBox android:layout_width="wrap_content" android:layout_height="wrap_content" android:text="Done"/>
</LinearLayout>
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        // Filled M3 button: primary background, pill radius, 40 dp minimum.
        assert!(out.html.contains("background:#6750A4;border-radius:20px"));
        assert!(out.html.contains("min-height:40px"));
        assert!(out.html.contains("color:#FFFFFF;font-size:14px;font-weight:500"));
        // A checkbox draws a box, not a disabled native input.
        assert!(out.html.contains("class=\"ap-box\""));
        assert!(!out.html.contains("type=\"checkbox\""));
    }

    #[test]
    fn button_labels_inherit_the_content_colour() {
        // Compose `Button { Text(…) }`: the label takes the button's content
        // colour (`onPrimary`, then `primary` for outlined/text buttons), not
        // the page's body colour.
        let request = PreviewRequest {
            path: "app/src/main/java/MainActivity.kt".to_owned(),
            project_path: "/p".to_owned(),
            decl: "compose".to_owned(),
            content: r#"
@Preview
@Composable
fun P() {
    Column {
        Button(onClick = {}) { Text("Save") }
        OutlinedButton(onClick = {}) { Text("Cancel") }
        Text("Body")
    }
}
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        // The label span inherits the button's content colour…
        assert!(out.html.contains(r#"color:#FFFFFF;white-space:pre-wrap;">Save"#), "{}", out.html);
        assert!(out.html.contains(r#"color:#6750A4;white-space:pre-wrap;">Cancel"#), "{}", out.html);
        // …and the box carries `labelLarge`, so unstyled content inherits it.
        assert!(out.html.contains("font-size:14px;font-weight:500;"), "{}", out.html);
        // Plain text keeps the theme foreground and the `bodyLarge` 16 sp size.
        assert!(out.html.contains(r#"color:var(--ap-fg);font-size:16px;white-space:pre-wrap;">Body"#), "{}", out.html);
    }

    #[test]
    fn rejects_unknown_decl_and_empty_files() {
        let mk = |decl: &str, content: &str| PreviewRequest {
            path: "a.b".to_owned(),
            project_path: "/p".to_owned(),
            decl: decl.to_owned(),
            content: content.to_owned(),
        };
        assert!(AndroidPreview::render_preview(mk("other", "")).is_err());
        assert!(AndroidPreview::render_preview(mk("layout-xml", "not xml at all")).is_err());
        assert!(AndroidPreview::render_preview(mk("compose", "fun a() {}")).is_err());
    }

    #[test]
    fn html_is_escaped_everywhere() {
        let request = PreviewRequest {
            path: "res/layout/x.xml".to_owned(),
            project_path: "/p".to_owned(),
            decl: "layout-xml".to_owned(),
            content: r#"
<FrameLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent" android:layout_height="match_parent">
    <TextView android:layout_width="wrap_content" android:layout_height="wrap_content"
        android:text="a &lt;script&gt;bad()&lt;/script&gt; &quot;q&quot; &amp; more"/>
</FrameLayout>
"#
            .to_owned(),
        };
        let out = AndroidPreview::render_preview(request).unwrap();
        assert!(!out.html.contains("<script>bad"));
        assert!(out.html.contains("&lt;script&gt;bad()&lt;/script&gt;"));
        assert!(out.html.contains("&quot;q&quot; &amp; more"));
    }

    fn render_with(
        content: &str,
        decl: &str,
        source: &dyn resources::ResourceSource,
    ) -> String {
        let request = PreviewRequest {
            path: "app/src/main/res/layout/screen.xml".to_owned(),
            project_path: "/p".to_owned(),
            decl: decl.to_owned(),
            content: content.to_owned(),
        };
        let screens = if decl == "compose" {
            compose_screens_with(&request, source)
        } else {
            layout_screens_with(&request, source)
        };
        render::page("screen", &screens.unwrap())
    }

    #[test]
    fn manifest_theme_drawables_and_library_widgets_reach_the_page() {
        let source = resources::MemorySource::new()
            .module("app")
            .project_file(
                "app/src/main/AndroidManifest.xml",
                r#"<manifest package="com.example.app">
                    <application android:theme="@style/Theme.App"/>
                </manifest>"#,
            )
            .file(
                "values/themes.xml",
                r#"<resources>
                    <style name="Theme.App" parent="Theme.Material3.DayNight.NoActionBar">
                        <item name="colorPrimary">#FF123456</item>
                    </style>
                </resources>"#,
            )
            .file("values/strings.xml", r#"<resources><string name="app_name">My App</string></resources>"#)
            .file(
                "menu/main.xml",
                r#"<menu xmlns:android="http://schemas.android.com/apk/res/android">
                    <item android:id="@+id/search" android:title="Search"/>
                </menu>"#,
            )
            .file(
                "drawable/bg_grad.xml",
                r##"<shape><gradient android:startColor="#FFFF0000" android:endColor="#FF0000FF"
                    android:angle="90"/></shape>"##,
            )
            .project_file(
                "app/src/main/java/com/example/ui/BrandButton.kt",
                "package com.example.ui\nimport androidx.appcompat.widget.AppCompatButton\nclass BrandButton(context: Context) : AppCompatButton(context)\n",
            );
        let html = render_with(
            r##"<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
                xmlns:app="http://schemas.android.com/apk/res-auto"
                android:orientation="vertical">
                <androidx.appcompat.widget.Toolbar android:layout_width="match_parent"
                    android:layout_height="wrap_content" app:title="@string/app_name" app:menu="@menu/main"/>
                <com.google.android.material.textfield.TextInputLayout android:hint="Email">
                    <EditText/>
                </com.google.android.material.textfield.TextInputLayout>
                <com.google.android.material.chip.ChipGroup>
                    <com.google.android.material.chip.Chip android:text="Tag"/>
                </com.google.android.material.chip.ChipGroup>
                <com.example.ui.BrandButton android:text="Go"/>
                <View android:layout_width="40dp" android:layout_height="20dp"
                    android:background="@drawable/bg_grad"/>
                <TextView android:text="Tinted" android:textColor="?attr/colorPrimary"/>
            </LinearLayout>"##,
            "layout-xml",
            &source,
        );
        // Toolbar title + overflow menu from resources.
        assert!(html.contains("ap-toolbar-title"), "{html}");
        assert!(html.contains("My App"), "{html}");
        assert!(html.contains("Search"), "{html}");
        // Material text field chrome and chip labels.
        assert!(html.contains("ap-til-label"), "{html}");
        assert!(html.contains("Email"), "{html}");
        assert!(html.contains("Tag"), "{html}");
        // A custom view renders as the widget it extends.
        assert!(html.contains("background:#6750A4;border-radius:20px"), "{html}");
        // Gradient drawable becomes a CSS gradient.
        assert!(html.contains("linear-gradient(0deg,#FF0000,#0000FF)"), "{html}");
        // `?attr/colorPrimary` resolves against the manifest theme.
        assert!(html.contains("color:#123456"), "{html}");
    }

    #[test]
    fn compose_pages_resolve_project_resources() {
        let source = resources::MemorySource::new()
            .file("values/strings.xml", r#"<resources><string name="app_name">My App</string></resources>"#)
            .file("values/colors.xml", r#"<resources><color name="brand">#00FF00</color></resources>"#)
            .file(
                "drawable/ic_star.xml",
                r##"<vector xmlns:android="http://schemas.android.com/apk/res/android"
                    android:width="24dp" android:height="24dp"
                    android:viewportWidth="24" android:viewportHeight="24">
                    <path android:pathData="M12,2L15,9L22,9L16,14L18,21L12,17L6,21L8,14L2,9L9,9Z"
                        android:fillColor="#FFFFD700"/>
                </vector>"##,
            );
        let html = render_with(
            r#"@Preview
@Composable
fun P() {
    Column {
        Text(stringResource(R.string.app_name))
        Text("Tinted", color = colorResource(R.color.brand))
        Image(painter = painterResource(R.drawable.ic_star))
    }
}"#,
            "compose",
            &source,
        );
        assert!(html.contains("My App"), "{html}");
        assert!(html.contains("color:#00FF00"), "{html}");
        // A vector drawable is inlined so the preview needs no asset server.
        assert!(html.contains("<svg") && html.contains("#FFD700"), "{html}");
    }
}
