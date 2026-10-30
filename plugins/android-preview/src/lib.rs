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
            "layout-xml" => vec![layout_screen(&request)?],
            "compose" => compose_screens(&request)?,
            other => return Err(format!("unknown preview kind `{other}`")),
        };
        let html = render::page(&title, &screens);
        Ok(PreviewOutput { html, background: None })
    }
}

export!(AndroidPreview);

/// One XML layout file → one screen with the resolved node roots.
fn layout_screen(request: &PreviewRequest) -> Result<Screen, String> {
    let source = hosts::res_source(&request.path);
    let res = Resources::new(&*source);
    let mut roots: Vec<Node> = layout::build(&request.content, &res)?;
    // Resolve Relative/ConstraintLayout rules into absolute child positions
    // for the default device viewport (the toolbar resize is approximate).
    for root in &mut roots {
        layout::resolve_positions(root, layout::DEVICE_W, layout::DEVICE_H);
    }
    Ok(Screen { caption: None, roots, width: None, height: None, bg: None })
}

/// Every `@Preview` function in one Kotlin file → one screen per frame.
fn compose_screens(request: &PreviewRequest) -> Result<Vec<Screen>, String> {
    let frames: Vec<Frame> = compose::scan(&request.content)?;
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
        })
        .collect())
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
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
}
