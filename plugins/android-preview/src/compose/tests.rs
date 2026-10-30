//! Unit tests for the Compose scanner.

use super::*;

fn one_frame(src: &str) -> Frame {
    let mut frames = scan(src).expect("scan");
    assert_eq!(frames.len(), 1);
    frames.remove(0)
}

const IMPORTS: &str = "\
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
";

#[test]
fn single_preview_with_column_text_button() {
    let f = one_frame(
        r#"@Preview
@Composable
fun DefaultPreview() {
    Column {
        Text("Hello")
        Button(onClick = {}) {
            Text("Press me")
        }
    }
}"#,
    );
    assert_eq!(f.caption, "DefaultPreview");
    let root = f.root.unwrap();
    assert_eq!(root.children.len(), 1);
    let col = &root.children[0];
    assert_eq!(col.kind, "LinearLayout");
    assert_eq!(col.style.flex_dir, Some(FlexDir::Column));
    assert_eq!(col.children.len(), 2);
    let text = &col.children[0];
    assert_eq!(text.kind, "TextView");
    assert_eq!(text.text.as_deref(), Some("Hello"));
    let btn = &col.children[1];
    assert_eq!(btn.kind, "Button");
    assert_eq!(btn.children.len(), 1);
    assert_eq!(btn.children[0].text.as_deref(), Some("Press me"));
}

#[test]
fn stacked_previews_make_one_frame_each() {
    let frames = scan(
        r#"@Preview(name = "Light")
@Preview(name = "Dark", showBackground = true)
@Composable
fun Both() {
    Text("x")
}"#,
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].caption, "Light");
    assert_eq!(frames[1].caption, "Dark");
    assert!(frames[1].show_background);
    assert!(!frames[0].show_background);
}

#[test]
fn preview_options_parse() {
    let f = one_frame(
        r#"@Preview(name = "Wide", widthDp = 320, heightDp = 240, backgroundColor = 0xFFEEEEEE)
@Composable
fun wide() {
    Text("x")
}"#,
    );
    assert_eq!(f.caption, "Wide");
    assert_eq!(f.width, Some(320.0));
    assert_eq!(f.height, Some(240.0));
    assert_eq!(f.bg_color.as_deref(), Some("#EEEEEE"));
}

#[test]
fn unnamed_multi_previews_get_numbered_captions() {
    let frames = scan(
        r#"@Preview
@Preview(widthDp = 100)
@Composable
fun twice() {
    Text("x")
}"#,
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].caption, "twice · 1");
    assert_eq!(frames[1].caption, "twice · 2");
    assert_eq!(frames[1].width, Some(100.0));
}

#[test]
fn no_previews_is_an_empty_ok() {
    let frames = scan(
        r#"@Composable
fun NoPreviewHere() {
    Text("just a composable")
}"#,
    )
    .unwrap();
    assert!(frames.is_empty());
}

#[test]
fn strings_and_comments_do_not_confuse_the_scanner() {
    let f = one_frame(
        r#"// @Preview in a comment does not count
/* @Preview neither */
@Preview
@Composable
fun tricky() {
    val brace = "}"
    Text("""raw } { string""", maxLines = 2) // trailing
}"#,
    );
    let root = f.root.unwrap();
    assert_eq!(root.children.len(), 1);
    let text = &root.children[0];
    assert_eq!(text.kind, "TextView");
    assert_eq!(text.text.as_deref(), Some("raw } { string"));
    assert_eq!(text.style.max_lines, Some(2));
}

#[test]
fn nested_block_comments_are_nested() {
    let f = one_frame(
        r#"/* /* @Preview */ */
@Preview
@Composable
fun a() {
    Text("x")
}"#,
    );
    assert_eq!(f.caption, "a");
}

#[test]
fn unbalanced_braces_error_with_line() {
    let e = scan(
        r#"@Preview
@Composable
fun broken() {
    Column {
        Text("x")
"#,
    )
    .unwrap_err();
    assert!(e.contains("broken") && e.contains("unbalanced"), "{e}");
    assert!(e.contains("line 4"), "{e}");
}

#[test]
fn fully_qualified_preview_annotation() {
    let f = one_frame(
        r#"@androidx.compose.ui.tooling.preview.Preview
@Composable
fun fq() {
    Text("x")
}"#,
    );
    assert_eq!(f.caption, "fq");
}

#[test]
fn modifier_chain_basics() {
    let f = one_frame(
        r#"@Preview(showBackground = true)
@Composable
fun mods() {
    Text(
        "styled",
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp)
            .background(Color.White, RoundedCornerShape(8.dp))
            .border(1.dp, Color.Red)
            .shadow(4.dp)
            .alpha(0.5f)
            .aspectRatio(1.5f)
            .offset(2.dp, 3.dp)
            .clip(RoundedCornerShape(12.dp)),
        fontSize = 18.sp,
        color = Color(0xFF6750A4),
        fontWeight = FontWeight.Bold,
        fontStyle = FontStyle.Italic,
        textAlign = TextAlign.Center,
    )
}"#,
    );
    let text = &f.root.unwrap().children[0];
    let s = &text.style;
    assert_eq!(s.width, Some(Size::Match));
    assert_eq!(s.padding, crate::layout::Edges { start: 16.0, end: 16.0, top: 16.0, bottom: 16.0 });
    assert_eq!(s.bg_color.as_deref(), Some("#FFFFFF"));
    assert_eq!(s.radius, Some(12.0)); // clip wins (last radius op)
    assert_eq!(s.border, Some((1.0, "#FF0000".to_owned())));
    assert_eq!(s.elevation, Some(4.0));
    assert_eq!(s.alpha, Some(0.5));
    assert_eq!(s.aspect, Some(1.5));
    assert_eq!(s.offset, Some((2.0, 3.0)));
    assert_eq!(s.font_size, Some(18.0));
    assert_eq!(s.fg.as_deref(), Some("#6750A4"));
    assert_eq!(s.bold, Some(true));
    assert_eq!(s.italic, Some(true));
    assert_eq!(s.gravity.map(|g| g.h), Some(HAlign::Center));
}

#[test]
fn padding_named_variants() {
    let f = one_frame(
        r#"@Preview
@Composable
fun pads() {
    Column {
        Text("a", modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp))
        Text("b", modifier = Modifier.padding(start = 2.dp, top = 1.dp, end = 3.dp, bottom = 6.dp))
    }
}"#,
    );
    let children = &f.root.unwrap().children[0].children;
    let a = &children[0].style.padding;
    assert_eq!((a.start, a.end, a.top, a.bottom), (8.0, 8.0, 4.0, 4.0));
    let b = &children[1].style.padding;
    assert_eq!((b.start, b.end, b.top, b.bottom), (2.0, 3.0, 1.0, 6.0));
}

#[test]
fn row_alignment_and_spacing() {
    let f = one_frame(
        r#"@Preview
@Composable
fun row() {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("a")
        Text("b", modifier = Modifier.weight(1f))
        Box(modifier = Modifier.size(24.dp))
    }
}"#,
    );
    let row = &f.root.unwrap().children[0];
    assert_eq!(row.style.flex_dir, Some(FlexDir::Row));
    assert_eq!(row.style.gap, Some(8.0));
    assert_eq!(row.style.gravity.map(|g| g.v), Some(VAlign::Center));
    assert_eq!(row.children[1].style.weight, Some(1.0));
    let box_node = &row.children[2];
    assert_eq!(box_node.kind, "FrameLayout");
    assert_eq!(box_node.style.width, Some(Size::Px(24.0)));
    assert_eq!(box_node.style.height, Some(Size::Px(24.0)));
}

#[test]
fn material_theme_colors_and_arrangement_modes() {
    let f = one_frame(
        r#"@Preview
@Composable
fun m() {
    Column(verticalArrangement = Arrangement.SpaceBetween) {
        Text("x", color = MaterialTheme.colorScheme.primary)
        Text("y", color = MaterialTheme.colorScheme.error)
    }
}"#,
    );
    let col = &f.root.unwrap().children[0];
    assert!(col.style.gravity.is_some());
    assert_eq!(col.children[0].style.fg.as_deref(), Some(crate::resources::M3_PRIMARY));
    assert_eq!(col.children[1].style.fg.as_deref(), Some(crate::resources::M3_ERROR));
}

#[test]
fn composable_widget_set() {
    let f = one_frame(
        r#"@Preview
@Composable
fun widgets() {
    Column {
        OutlinedButton(onClick = { }) { Text("o") }
        Icon(imageVector = Icons.Default.Home)
        Image(painter = painterResource(R.drawable.dog))
        Spacer(modifier = Modifier.height(8.dp))
        Divider()
        VerticalDivider()
        Checkbox(checked = true)
        Switch(checked = false)
        RadioButton(selected = true)
        Slider(value = 0.3f)
        CircularProgressIndicator()
        LinearProgressIndicator(progress = { 0.7f })
        LazyColumn { }
        MyCustomThing(1, two = "x")
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids.len(), 14);
    assert_eq!(kids[0].kind, "OutlinedButton");
    assert_eq!(kids[0].children[0].text.as_deref(), Some("o"));
    assert_eq!(kids[1].kind, "Icon");
    assert_eq!(kids[1].sub.as_deref(), Some("Home"));
    assert_eq!(kids[2].kind, "ImageView");
    assert_eq!(kids[3].kind, "Space");
    assert_eq!(kids[3].style.height, Some(Size::Px(8.0)));
    assert_eq!(kids[4].kind, "Divider");
    assert!(!kids[4].style.vertical);
    assert_eq!(kids[5].kind, "Divider");
    assert!(kids[5].style.vertical);
    assert_eq!(kids[6].kind, "CheckBox");
    assert_eq!(kids[6].style.checked, Some(true));
    assert_eq!(kids[7].kind, "Switch");
    assert_eq!(kids[8].kind, "RadioButton");
    assert_eq!(kids[9].kind, "SeekBar");
    assert_eq!(kids[9].style.progress, Some((30.0, 100.0)));
    assert_eq!(kids[10].kind, "ProgressBar");
    assert!(kids[10].style.spinner);
    assert_eq!(kids[11].kind, "ProgressBar");
    assert_eq!(kids[11].style.progress, Some((70.0, 100.0)));
    assert_eq!(kids[12].kind, "LazyColumn");
    assert_eq!(kids[12].children.len(), 3);
    assert_eq!(kids[13].kind, "Placeholder");
    assert_eq!(kids[13].sub.as_deref(), Some("MyCustomThing"));
}

#[test]
fn unresolved_expressions_degrade_to_labels() {
    let f = one_frame(
        r#"@Preview
@Composable
fun exprs(name: String = "x") {
    Text(text = stringResource(R.string.welcome))
    Text(name)
    Text("Hi $name")
}"#,
    );
    let kids = &f.root.unwrap().children;
    assert_eq!(kids[0].sub.as_deref(), Some("stringResource(…)"));
    assert_eq!(kids[0].text, None);
    assert_eq!(kids[1].sub.as_deref(), Some("name"));
    assert_eq!(kids[2].text.as_deref(), Some("Hi $name"));
}

#[test]
fn box_surface_and_alignment() {
    let f = one_frame(
        r#"@Preview
@Composable
fun boxed() {
    Box(contentAlignment = Alignment.Center) {
        Surface(color = Color.Black, shape = RoundedCornerShape(16.dp), shadowElevation = 8.dp) {
            Text("inside")
        }
    }
}"#,
    );
    let bx = &f.root.unwrap().children[0];
    assert_eq!(bx.kind, "FrameLayout");
    assert_eq!(bx.style.gravity.map(|g| (g.h, g.v)), Some((HAlign::Center, VAlign::Center)));
    let surface = &bx.children[0];
    assert_eq!(surface.kind, "Surface");
    assert_eq!(surface.style.bg_color.as_deref(), Some("#000000"));
    assert_eq!(surface.style.radius, Some(16.0));
    assert_eq!(surface.style.elevation, Some(8.0));
}

#[test]
fn private_preview_with_modifiers_between_annotations() {
    let f = one_frame(
        r#"@Preview
@Composable
private fun priv() {
    Column(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("centered")
    }
}"#,
    );
    assert_eq!(f.caption, "priv");
    let col = &f.root.unwrap().children[0];
    assert_eq!(col.style.width, Some(Size::Match));
    assert_eq!(col.style.height, Some(Size::Match));
    assert_eq!(col.style.gravity, Some(Gravity { h: HAlign::Center, v: VAlign::Center }));
}

#[test]
fn preview_annotation_on_class_is_ignored() {
    let frames = scan(
        r#"@Preview
data class Thing(val a: Int)

@Preview
@Composable
fun real() {
    Text("x")
}"#,
    )
    .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].caption, "real");
}

#[test]
fn content_named_lambda_children() {
    let f = one_frame(
        r#"@Preview
@Composable
fun c() {
    Row(content = {
        Text("in content lambda")
    })
}"#,
    );
    let row = &f.root.unwrap().children[0];
    assert_eq!(row.kind, "LinearLayout");
    assert_eq!(row.children.len(), 1);
    assert_eq!(row.children[0].text.as_deref(), Some("in content lambda"));
}

#[test]
fn hex_numbers_and_suffixes() {
    let f = one_frame(
        r#"@Preview
@Composable
fun colors() {
    Column {
        Text("a", color = Color(0x80FF0000))
        Text("b", color = Color(0xFF00FF00))
        Text("c", modifier = Modifier.width(100.5f.dp))
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].style.fg.as_deref(), Some("rgba(255,0,0,0.502)"));
    assert_eq!(kids[1].style.fg.as_deref(), Some("#00FF00"));
    assert_eq!(kids[2].style.width, Some(Size::Px(100.5)));
}

#[test]
fn imports_constant_is_valid_kotlin() {
    // Sanity: the IMPORTS snippet used by other tests lexes cleanly.
    assert!(scan(IMPORTS).unwrap().is_empty());
}
