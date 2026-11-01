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
fun exprs(name: String) {
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

#[test]
fn preview_renders_a_composable_called_from_the_file() {
    // Android Studio renders the preview by invoking the function: a preview
    // that delegates to another composable must show that composable's body,
    // not a placeholder named after the call.
    let f = one_frame(
        r#"@Composable
fun NoteEditor(title: String) {
    Column {
        Text(text = title)
        Button(onClick = {}) { Text("Save") }
    }
}

@Preview
@Composable
private fun NoteEditorPreview() {
    NoteEditor(title = "New note")
}"#,
    );
    let nodes = f.root.unwrap().children;
    assert_eq!(nodes.len(), 1, "one Column, inlined from NoteEditor");
    let column = &nodes[0];
    assert_eq!(column.kind, "LinearLayout");
    // `title` is bound to the caller's string argument.
    assert_eq!(column.children[0].text.as_deref(), Some("New note"));
    assert_eq!(column.children[1].kind, "Button");
}

#[test]
fn recursive_composables_stop_at_the_depth_limit() {
    let frames = scan(
        r#"@Composable
fun Loop() {
    Loop()
}

@Preview
@Composable
fun P() {
    Loop()
}"#,
    )
    .expect("scan");
    assert_eq!(frames.len(), 1);
}

#[test]
fn material_typography_sets_the_text_scale() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Text("Title", style = MaterialTheme.typography.titleLarge)
        Text("Body", style = MaterialTheme.typography.bodyMedium)
        Text("Color", color = MaterialTheme.colorScheme.primary)
    }
}"#,
    );
    let column = &f.root.unwrap().children[0];
    assert_eq!(column.children[0].style.font_size, Some(22.0));
    assert_eq!(column.children[1].style.font_size, Some(14.0));
    // `colorScheme.primary` is still a colour, not a typography token.
    assert_eq!(column.children[2].style.fg.as_deref(), Some("#6750A4"));
}

#[test]
fn dividers_span_their_container() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Divider(color = Color(0xFFE8E0EB), thickness = 2.dp)
    }
}"#,
    );
    let divider = &f.root.unwrap().children[0].children[0];
    assert_eq!(divider.kind, "Divider");
    assert_eq!(divider.style.width, Some(Size::Match));
    assert_eq!(divider.style.height, Some(Size::Px(2.0)));
    assert_eq!(divider.style.bg_color.as_deref(), Some("#E8E0EB"));
}

#[test]
fn arrangements_distribute_instead_of_centring() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column(verticalArrangement = Arrangement.SpaceBetween) { Text("a") }
    Row(horizontalArrangement = Arrangement.SpaceEvenly) { Text("b") }
}"#,
    );
    let nodes = f.root.unwrap().children;
    assert_eq!(nodes[0].style.gravity.unwrap().v, VAlign::Fill);
    assert_eq!(nodes[1].style.gravity.unwrap().h, HAlign::Fill);
}

// ---------------------------------------------------------------------------
// Resource-backed arguments (scan_with)
// ---------------------------------------------------------------------------

use crate::resources::{MemorySource, Resources};

fn test_res() -> Resources<'static> {
    // Leaked for convenience in tests; fine for the test binary.
    let src: &'static MemorySource = Box::leak(Box::new(
        MemorySource::new()
            .file(
                "values/strings.xml",
                r#"<resources>
                    <string name="app_name">My App</string>
                    <string name="hello">Hello</string>
                </resources>"#,
            )
            .file("values/colors.xml", r##"<resources><color name="brand">#00FF00</color></resources>"##)
            .file("values/dimens.xml", r#"<resources><dimen name="gap">8dp</dimen></resources>"#)
            .file(
                "drawable/bg_pill.xml",
                r##"<shape xmlns:android="http://schemas.android.com/apk/res/android">
                    <solid android:color="#FFEEDD"/>
                    <corners android:radius="10dp"/>
                </shape>"##,
            )
            .file(
                "drawable/ic_star.xml",
                r##"<vector xmlns:android="http://schemas.android.com/apk/res/android"
                    android:width="24dp" android:height="24dp"
                    android:viewportWidth="24" android:viewportHeight="24">
                    <path android:pathData="M12,2L15,9L22,9L16,14L18,21L12,17L6,21L8,14L2,9L9,9Z" android:fillColor="#FFD700"/>
                </vector>"##,
            ),
    ));
    Resources::new(src)
}

#[test]
fn resource_calls_resolve_through_the_tables() {
    let res = test_res();
    let mut frames = scan_with(
        r#"@Preview
@Composable
fun P() {
    Column {
        Text(stringResource(R.string.app_name))
        Text(text = androidx.compose.ui.res.stringResource(id = R.string.hello))
        Text("tinted", color = colorResource(R.color.brand))
        Spacer(modifier = Modifier.size(dimensionResource(R.dimen.gap)))
        Image(painter = painterResource(R.drawable.ic_star))
        Image(painter = painterResource(R.drawable.bg_pill))
        Text(R.string.app_name)
        Text(text = stringResource(R.string.missing))
    }
}"#,
        Some(&res),
    )
    .unwrap();
    assert_eq!(frames.len(), 1);
    let kids = &frames.remove(0).root.unwrap().children[0].children;
    assert_eq!(kids[0].text.as_deref(), Some("My App"));
    assert_eq!(kids[1].text.as_deref(), Some("Hello"));
    assert_eq!(kids[2].style.fg.as_deref(), Some("#00FF00"));
    assert_eq!(kids[3].style.width, Some(Size::Px(8.0)));
    match &kids[4].style.src_image {
        Some(crate::layout::Image::Svg(svg)) => assert!(svg.contains("<svg") && svg.contains("#FFD700"), "{svg}"),
        other => panic!("expected an inline SVG, got {other:?}"),
    }
    // A <shape> drawable becomes the node's shape (render paints its fill).
    let shape = kids[5].style.shape.as_ref().expect("shape drawable");
    assert_eq!(shape.fill.as_deref(), Some("#FFEEDD"));
    assert_eq!(shape.corners[0], 10.0);
    // A bare R reference resolves like its *Resource wrapper.
    assert_eq!(kids[6].text.as_deref(), Some("My App"));
    // Unresolvable references keep the label fallback.
    assert_eq!(kids[7].sub.as_deref(), Some("stringResource(…)"));
    assert_eq!(kids[7].text, None);
}

#[test]
fn scan_without_resources_keeps_the_label_fallbacks() {
    // scan() delegates to scan_with(None): nothing resolves, nothing panics.
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Text(stringResource(R.string.app_name))
}"#,
    );
    assert_eq!(f.root.unwrap().children[0].sub.as_deref(), Some("stringResource(…)"));
}

// ---------------------------------------------------------------------------
// Wrappers, side effects and lazy-list items
// ---------------------------------------------------------------------------

#[test]
fn wrappers_splice_and_effects_vanish() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    MaterialTheme {
        Column {
            LaunchedEffect(Unit) { println("side") }
            val state = remember { mutableStateOf(0) }
            val flow = rememberScrollState()
            AnimatedVisibility(visible = true) {
                Text("anim")
            }
            CompositionLocalProvider(LocalContentColor provides Color.Red) {
                Text("local")
            }
            key("k") { Text("keyed") }
            FlowRow { Text("flow") }
            FlowColumn { Text("flowcol") }
        }
    }
}"#,
    );
    let root = f.root.unwrap();
    assert_eq!(root.children.len(), 1, "MaterialTheme splices its content");
    let col = &root.children[0];
    assert_eq!(col.kind, "LinearLayout");
    assert_eq!(col.children.len(), 5);
    assert_eq!(col.children[0].text.as_deref(), Some("anim"));
    assert_eq!(col.children[1].text.as_deref(), Some("local"));
    assert_eq!(col.children[2].text.as_deref(), Some("keyed"));
    assert_eq!(col.children[3].kind, "LinearLayout");
    assert_eq!(col.children[3].style.flex_dir, Some(FlexDir::Row));
    assert_eq!(col.children[4].style.flex_dir, Some(FlexDir::Column));
    assert!(!col.children.iter().any(|c| c.kind == "Placeholder"), "no effect/wrapper placeholders");
}

#[test]
fn lazy_list_items_expand() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    LazyColumn {
        item { Text("head") }
        items(5) { Text("row") }
        items(unknownList) { Text("sample") }
        items(40) { Text("capped") }
    }
}"#,
    );
    let list = &f.root.unwrap().children[0];
    assert_eq!(list.kind, "LazyColumn");
    // 1 head + 5 rows + 3 samples for the unresolvable list + 12 (cap of 40).
    assert_eq!(list.children.len(), 21);
    assert_eq!(list.children[0].text.as_deref(), Some("head"));
    assert_eq!(list.children[5].text.as_deref(), Some("row"));
    assert_eq!(list.children[6].text.as_deref(), Some("sample"));
    assert_eq!(list.children[9].text.as_deref(), Some("capped"));
}

#[test]
fn lazy_list_without_items_keeps_the_placeholders() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    LazyColumn { }
}"#,
    );
    let list = &f.root.unwrap().children[0];
    assert_eq!(list.children.len(), 3);
    assert_eq!(list.children[0].kind, "Placeholder");
    assert_eq!(list.children[0].sub.as_deref(), Some("item 1"));
}

// ---------------------------------------------------------------------------
// Material 3 components
// ---------------------------------------------------------------------------

#[test]
fn scaffold_arranges_bars_content_and_fab() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Scaffold(
        containerColor = Color(0xFF101010),
        topBar = {
            TopAppBar(
                title = { Text("Title") },
                navigationIcon = { Icon(Icons.Default.Menu, "menu") },
                actions = { IconButton(onClick = {}) { Icon(Icons.Default.Search, "search") } },
            )
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = true, onClick = {}, icon = { Icon(Icons.Default.Home, "h") }, label = { Text("Home") })
            }
        },
        floatingActionButton = { FloatingActionButton(onClick = {}) { Icon(Icons.Default.Add, "add") } },
    ) { padding ->
        Text("content")
    }
}"#,
    );
    let scaffold = &f.root.unwrap().children[0];
    assert_eq!(scaffold.kind, "FrameLayout");
    assert_eq!(scaffold.style.bg_color.as_deref(), Some("#101010"));
    // topBar, content, bottomBar, FAB.
    assert_eq!(scaffold.children.len(), 4);
    let top = &scaffold.children[0];
    assert_eq!(top.kind, "Toolbar");
    assert_eq!(top.text.as_deref(), Some("Title"));
    assert_eq!(top.style.height, Some(Size::Px(64.0)));
    assert_eq!(top.style.bg_color.as_deref(), Some(crate::resources::M3_SURFACE));
    assert_eq!(top.children.len(), 2, "navigationIcon + actions");
    let content = &scaffold.children[1];
    assert_eq!(content.text.as_deref(), Some("content"));
    assert_eq!(content.style.width, Some(Size::Match));
    assert_eq!(content.style.margin.top, 64.0);
    assert_eq!(content.style.margin.bottom, 80.0);
    let bottom = &scaffold.children[2];
    assert_eq!(bottom.kind, "LinearLayout");
    assert_eq!(bottom.style.height, Some(Size::Px(80.0)));
    assert_eq!(bottom.style.layout_gravity.map(|g| (g.h, g.v)), Some((HAlign::Start, VAlign::Bottom)));
    let fab = &scaffold.children[3];
    assert_eq!(fab.kind, "FloatingActionButton");
    assert_eq!(fab.style.layout_gravity.map(|g| (g.h, g.v)), Some((HAlign::End, VAlign::Bottom)));
    assert_eq!(fab.style.margin.end, 16.0);
    assert_eq!(fab.style.margin.bottom, 16.0);
}

#[test]
fn scaffold_defaults_to_surface_without_bars() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Scaffold { Text("only content") }
}"#,
    );
    let scaffold = &f.root.unwrap().children[0];
    assert_eq!(scaffold.style.bg_color.as_deref(), Some(crate::resources::M3_SURFACE));
    assert_eq!(scaffold.children.len(), 1);
    assert_eq!(scaffold.children[0].style.margin.top, 0.0, "no topBar, no inset");
}

#[test]
fn navigation_bar_items_stack_icon_and_label() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    NavigationBar {
        NavigationBarItem(selected = true, onClick = {}, icon = { Icon(Icons.Default.Home, "h") }, label = { Text("Home") })
        NavigationBarItem(selected = false, onClick = {}, icon = { Icon(Icons.Default.Star, "s") }, label = { Text("Star") })
    }
}"#,
    );
    let nav = &f.root.unwrap().children[0];
    assert_eq!(nav.kind, "LinearLayout");
    assert_eq!(nav.style.flex_dir, Some(FlexDir::Row));
    assert_eq!(nav.style.height, Some(Size::Px(80.0)));
    assert_eq!(nav.style.bg_color.as_deref(), Some(crate::resources::M3_SURFACE));
    let item = &nav.children[0];
    assert_eq!(item.kind, "LinearLayout");
    assert_eq!(item.style.flex_dir, Some(FlexDir::Column));
    assert_eq!(item.children.len(), 2);
    assert_eq!(item.children[0].kind, "Icon");
    let label = &item.children[1];
    assert_eq!(label.text.as_deref(), Some("Home"));
    assert_eq!(label.style.font_size, Some(12.0));
    assert_eq!(label.style.fg.as_deref(), Some(crate::resources::M3_PRIMARY), "selected label is primary");
    let unselected = &nav.children[1].children[1];
    assert_eq!(unselected.style.fg, None);
}

#[test]
fn tab_rows_and_tabs() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        TabRow(selectedTabIndex = 0) {
            Tab(selected = true, onClick = {}, text = { Text("One") })
            Tab(selected = false, onClick = {}, text = { Text("Two") })
        }
        ScrollableTabRow(selectedTabIndex = 1) {
            Tab(selected = false, onClick = {}, text = { Text("Wide") })
        }
    }
}"#,
    );
    let col = &f.root.unwrap().children[0];
    let tab_row = &col.children[0];
    assert_eq!(tab_row.kind, "LinearLayout");
    assert_eq!(tab_row.style.flex_dir, Some(FlexDir::Row));
    let one = &tab_row.children[0];
    assert_eq!(one.kind, "TextView");
    assert_eq!(one.text.as_deref(), Some("One"));
    assert_eq!(one.style.font_size, Some(14.0));
    assert_eq!(one.style.bold, Some(true));
    assert_eq!(one.style.fg.as_deref(), Some(crate::resources::M3_PRIMARY));
    assert_eq!(one.style.weight, Some(1.0), "fixed tabs divide the row");
    let two = &tab_row.children[1];
    assert_eq!(two.style.bold, Some(false));
    assert_eq!(two.style.fg.as_deref(), Some(crate::resources::M3_ON_SURFACE_VARIANT));
    let scrollable = &col.children[1];
    assert_eq!(scrollable.children[0].style.weight, None, "scrollable tabs wrap");
}

#[test]
fn fabs_and_chips() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        ExtendedFloatingActionButton(onClick = {}, icon = { Icon(Icons.Default.Add, "a") }, text = { Text("Create") })
        SmallFloatingActionButton(onClick = {}) { Icon(Icons.Default.Edit, "e") }
        LargeFloatingActionButton(onClick = {}) { Icon(Icons.Default.Edit, "e") }
        FilterChip(selected = true, onClick = {}, label = { Text("Tag") })
        AssistChip(onClick = {}, label = { Text("Assist") })
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].kind, "FloatingActionButton");
    assert_eq!(kids[0].children.len(), 2, "icon + text lambdas become the content");
    assert_eq!(kids[1].kind, "FloatingActionButton");
    assert_eq!(kids[1].style.width, Some(Size::Px(40.0)));
    assert_eq!(kids[2].style.width, Some(Size::Px(96.0)));
    assert_eq!(kids[3].kind, "Chip");
    assert_eq!(kids[3].text.as_deref(), Some("Tag"));
    assert_eq!(kids[3].style.checked, Some(true));
    assert_eq!(kids[4].kind, "Chip");
    assert_eq!(kids[4].text.as_deref(), Some("Assist"));
    assert_eq!(kids[4].style.checked, None);
}

#[test]
fn text_fields_map_to_edit_text() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        TextField(value = "typed", onValueChange = {}, label = { Text("Name") })
        OutlinedTextField(value = "", onValueChange = {}, placeholder = { Text("Email") }, modifier = Modifier.fillMaxWidth())
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].kind, "EditText");
    assert_eq!(kids[0].text.as_deref(), Some("typed"));
    assert_eq!(kids[0].hint.as_deref(), Some("Name"));
    assert_eq!(kids[0].style.bg_color.as_deref(), Some(crate::resources::M3_SURFACE_VARIANT));
    assert_eq!(kids[0].style.radius, Some(4.0));
    assert_eq!(kids[0].style.height, Some(Size::Px(56.0)));
    assert_eq!(kids[1].kind, "EditText");
    assert_eq!(kids[1].hint.as_deref(), Some("Email"));
    assert_eq!(kids[1].style.border, Some((1.0, crate::resources::M3_OUTLINE.to_owned())));
    assert_eq!(kids[1].style.radius, Some(4.0));
    assert_eq!(kids[1].style.width, Some(Size::Match), "the modifier still applies");
}

#[test]
fn badge_list_item_menu_item_and_dialog() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Badge { Text("8") }
        ListItem(
            headlineContent = { Text("Head") },
            supportingContent = { Text("Sub") },
            leadingContent = { Icon(Icons.Default.Star, "s") },
        )
        DropdownMenuItem(text = { Text("Open") }, onClick = {}, leadingIcon = { Icon(Icons.Default.Home, "o") })
        AlertDialog(
            onDismissRequest = {},
            title = { Text("Delete?") },
            text = { Text("This cannot be undone.") },
            confirmButton = { TextButton(onClick = {}) { Text("Delete") } },
        )
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].kind, "TextView");
    assert_eq!(kids[0].text.as_deref(), Some("8"));
    assert_eq!(kids[0].style.bg_color.as_deref(), Some(crate::resources::M3_ERROR));
    assert_eq!(kids[0].style.radius, Some(999.0));
    assert_eq!(kids[1].kind, "LinearLayout");
    assert_eq!(kids[1].children.len(), 2, "leading + centre column");
    let center = &kids[1].children[1];
    assert_eq!(center.children[0].text.as_deref(), Some("Head"));
    assert_eq!(center.children[1].text.as_deref(), Some("Sub"));
    assert_eq!(kids[2].kind, "LinearLayout");
    assert_eq!(kids[2].style.height, Some(Size::Px(48.0)));
    assert_eq!(kids[2].children[1].text.as_deref(), Some("Open"));
    let dialog = &kids[3];
    assert_eq!(dialog.kind, "CardView");
    assert_eq!(dialog.style.radius, Some(28.0));
    assert_eq!(dialog.style.bg_color.as_deref(), Some(crate::resources::M3_SURFACE));
    let column = &dialog.children[0];
    assert_eq!(column.kind, "LinearLayout");
    assert_eq!(column.style.flex_dir, Some(FlexDir::Column));
    assert_eq!(column.children.len(), 3, "title, text, button row");
    assert_eq!(column.children[0].text.as_deref(), Some("Delete?"));
    assert_eq!(column.children[1].text.as_deref(), Some("This cannot be undone."));
    let buttons = &column.children[2];
    assert_eq!(buttons.style.flex_dir, Some(FlexDir::Row));
    assert_eq!(buttons.style.gravity.map(|g| g.h), Some(HAlign::End));
    assert_eq!(buttons.children[0].kind, "TextButton");
}

// ---------------------------------------------------------------------------
// Buttons and containers
// ---------------------------------------------------------------------------

#[test]
fn button_colors_padding_shape_state_and_border() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Button(
            onClick = {},
            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF0000FF), contentColor = Color.Yellow),
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            shape = RoundedCornerShape(4.dp),
            elevation = ButtonDefaults.elevation(defaultElevation = 6.dp),
        ) { Text("Go") }
        OutlinedButton(onClick = {}, enabled = false, border = BorderStroke(2.dp, Color.Red)) { Text("Off") }
        TextButton(onClick = {}, shape = CircleShape) { Text("Round") }
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    let b = &kids[0];
    assert_eq!(b.style.bg_color.as_deref(), Some("#0000FF"));
    assert_eq!(b.style.fg.as_deref(), Some("#FFFF00"));
    assert_eq!(b.style.padding, crate::layout::Edges { start: 12.0, end: 12.0, top: 8.0, bottom: 8.0 });
    assert_eq!(b.style.radius, Some(4.0));
    assert_eq!(b.style.elevation, Some(6.0));
    // The label inherits the button's labelLarge instead of the Text default.
    assert_eq!(b.children[0].style.font_size, None);
    let o = &kids[1];
    assert_eq!(o.style.alpha, Some(0.38));
    assert_eq!(o.style.border, Some((2.0, "#FF0000".to_owned())));
    assert_eq!(kids[2].style.radius, Some(999.0));
}

#[test]
fn cards_and_surface_borders_and_elevation() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        OutlinedCard(onClick = {}) { Text("c") }
        Card(onClick = {}, border = BorderStroke(3.dp, Color.Blue)) { Text("b") }
        ElevatedCard(elevation = CardDefaults.elevatedCardElevation(defaultElevation = 3.dp)) { Text("e") }
        Surface(border = BorderStroke(1.dp, Color.Black), tonalElevation = 2.dp) { Text("s") }
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].style.border, Some((1.0, crate::resources::M3_OUTLINE_VARIANT.to_owned())));
    assert_eq!(kids[1].style.border, Some((3.0, "#0000FF".to_owned())));
    assert_eq!(kids[2].style.elevation, Some(3.0));
    assert_eq!(kids[3].style.border, Some((1.0, "#000000".to_owned())));
    assert_eq!(kids[3].style.elevation, Some(2.0));
    assert_eq!(kids[0].style.radius, Some(12.0));
}

// ---------------------------------------------------------------------------
// Modifier fixes
// ---------------------------------------------------------------------------

#[test]
fn padding_positional_overloads() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Text("a", modifier = Modifier.padding(8.dp, 4.dp))
        Text("b", modifier = Modifier.padding(1.dp, 2.dp, 3.dp, 6.dp))
        Text("c", modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp))
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    let a = &kids[0].style.padding;
    assert_eq!((a.start, a.end, a.top, a.bottom), (8.0, 8.0, 4.0, 4.0), "(horizontal, vertical)");
    let b = &kids[1].style.padding;
    assert_eq!((b.start, b.top, b.end, b.bottom), (1.0, 2.0, 3.0, 6.0), "(start, top, end, bottom)");
    let c = &kids[2].style.padding;
    assert_eq!((c.start, c.end, c.top, c.bottom), (8.0, 8.0, 4.0, 4.0), "named variant keeps working");
}

#[test]
fn size_constraints_and_misc_modifiers() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Box(modifier = Modifier.sizeIn(minWidth = 40.dp, minHeight = 24.dp))
        Box(modifier = Modifier.width(10.dp).sizeIn(minWidth = 40.dp))
        Box(modifier = Modifier.defaultMinSize(minWidth = 48.dp))
        Box(modifier = Modifier.widthIn(max = 120.dp))
        Text("half", modifier = Modifier.fillMaxWidth(0.5f).fillMaxHeight(0.25f))
        Box(modifier = Modifier.rotate(45f).graphicsLayer { alpha = 0.5f })
        Text("w", modifier = Modifier.weight(1f, fill = false).testTag("tag").clickable { })
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].style.width, Some(Size::Px(40.0)));
    assert_eq!(kids[0].style.height, Some(Size::Px(24.0)));
    assert_eq!(kids[1].style.width, Some(Size::Px(10.0)), "an explicit size wins over sizeIn");
    assert_eq!(kids[2].style.width, Some(Size::Px(48.0)));
    assert_eq!(kids[3].style.width, Some(Size::Px(120.0)), "a lone max still pins the axis");
    assert_eq!(kids[4].style.width, Some(Size::Percent(50.0)));
    assert_eq!(kids[4].style.height, Some(Size::Percent(25.0)));
    assert_eq!(kids[5].style.rotation, Some(45.0));
    assert_eq!(kids[5].style.alpha, Some(0.5));
    assert_eq!(kids[6].style.weight, Some(1.0));
}

#[test]
fn spaced_by_keeps_its_cross_axis_alignment() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically)) { Text("a") }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterHorizontally)) { Text("b") }
}"#,
    );
    let nodes = f.root.unwrap().children;
    assert_eq!(nodes[0].style.gap, Some(8.0));
    assert_eq!(nodes[0].style.gravity.map(|g| g.v), Some(VAlign::Center));
    assert_eq!(nodes[1].style.gap, Some(4.0));
    assert_eq!(nodes[1].style.gravity.map(|g| g.h), Some(HAlign::Center));
}

// ---------------------------------------------------------------------------
// Text fidelity
// ---------------------------------------------------------------------------

#[test]
fn text_default_size_and_fidelity_options() {
    let f = one_frame(
        r#"@Preview
@Composable
fun P() {
    Column {
        Text("plain")
        Text("lh", lineHeight = 20.sp)
        Text("ell", overflow = TextOverflow.Ellipsis)
        Text("nowrap", softWrap = false)
        Text("one", maxLines = 1)
        Text("mono", fontFamily = FontFamily.Monospace)
        Text("serif", fontFamily = FontFamily.Serif)
        Text("w500", fontWeight = FontWeight.W500)
        Text("styled", style = MaterialTheme.typography.titleLarge)
        Text("sized", fontSize = 20.sp)
        Text("uncolored", color = Color.Unspecified)
    }
}"#,
    );
    let kids = &f.root.unwrap().children[0].children;
    assert_eq!(kids[0].style.font_size, Some(16.0), "Compose defaults to bodyLarge");
    assert_eq!(kids[1].style.line_height, Some(20.0));
    assert_eq!(kids[1].style.font_size, Some(16.0));
    assert!(kids[2].style.ellipsize);
    assert!(kids[3].style.ellipsize, "softWrap = false clips with an ellipsis");
    assert!(kids[4].style.ellipsize);
    assert_eq!(kids[4].style.max_lines, Some(1));
    assert_eq!(kids[5].style.font_family, Some("monospace"));
    assert_eq!(kids[6].style.font_family, Some("serif"));
    assert_eq!(kids[7].style.bold, Some(false), "W500 is medium, not bold");
    assert_eq!(kids[8].style.font_size, Some(22.0), "the typography style still wins");
    assert_eq!(kids[9].style.font_size, Some(20.0), "an explicit fontSize wins");
    assert_eq!(kids[10].style.fg, None, "Color.Unspecified must not paint");
}

// ---------------------------------------------------------------------------
// Function parameter defaults
// ---------------------------------------------------------------------------

#[test]
fn function_parameter_defaults_render() {
    let frames = scan(
        r#"@Composable
fun Greeting(title: String = "Hi", count: Int = 2) {
    Column {
        Text(title)
    }
}

@Preview
@Composable
fun Defaults() {
    Greeting()
}

@Preview
@Composable
fun Overrides() {
    Greeting(title = "Hello")
}

@Preview
@Composable
fun SelfDefault(label: String = "Self") {
    Text(label)
}"#,
    )
    .unwrap();
    assert_eq!(frames.len(), 3);
    let defaults = frames[0].root.as_ref().unwrap();
    assert_eq!(defaults.children[0].children[0].text.as_deref(), Some("Hi"));
    let overrides = frames[1].root.as_ref().unwrap();
    assert_eq!(overrides.children[0].children[0].text.as_deref(), Some("Hello"), "explicit arguments still win");
    let self_default = frames[2].root.as_ref().unwrap();
    assert_eq!(self_default.children[0].text.as_deref(), Some("Self"), "a preview uses its own defaults");
}

// ---------------------------------------------------------------------------
// @Preview(device = "spec:…")
// ---------------------------------------------------------------------------

#[test]
fn device_spec_sets_the_frame_size() {
    let frames = scan(
        r#"@Preview(device = "spec:width=411dp,height=891dp,dpi=420,orientation=portrait")
@Composable
fun Portrait() { Text("p") }

@Preview(device = "spec:width=411dp,height=891dp,dpi=420,orientation=landscape")
@Composable
fun Landscape() { Text("l") }

@Preview(device = "spec:width=411dp,height=891dp", widthDp = 100)
@Composable
fun Explicit() { Text("e") }

@Preview(device = "id/pixel_5")
@Composable
fun Named() { Text("n") }"#,
    )
    .unwrap();
    assert_eq!(frames.len(), 4);
    assert_eq!((frames[0].width, frames[0].height), (Some(411.0), Some(891.0)));
    assert_eq!((frames[1].width, frames[1].height), (Some(891.0), Some(411.0)), "landscape swaps the axes");
    assert_eq!((frames[2].width, frames[2].height), (Some(100.0), Some(891.0)), "explicit widthDp wins");
    assert_eq!((frames[3].width, frames[3].height), (None, None), "a named device carries no size here");
}
