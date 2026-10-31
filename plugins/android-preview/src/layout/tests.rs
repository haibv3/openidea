//! Unit tests for the XML layout pipeline.

use super::*;
use crate::resources::{MemorySource, Resources};

fn res(src: MemorySource) -> Resources<'static> {
    Resources::new(Box::leak(Box::new(src)))
}

fn std_res() -> Resources<'static> {
    res(MemorySource::new()
        .file(
            "values/strings.xml",
            r#"<resources>
                    <string name="title">Hello</string>
                    <string name="missing">@string/nope</string>
                </resources>"#,
        )
        .file("values/colors.xml", r##"<resources><color name="accent">#FF0000</color></resources>"##)
        .file("values/dimens.xml", r#"<resources><dimen name="pad">16dp</dimen></resources>"#)
        .file(
            "layout/simple.xml",
            r#"<LinearLayout android:orientation="vertical"><TextView android:text="included"/></LinearLayout>"#,
        )
        .file("layout/merge_inc.xml", r#"<merge><TextView android:text="m1"/><TextView android:text="m2"/></merge>"#)
        .file("layout/cyclic.xml", r#"<LinearLayout><include layout="@layout/cyclic"/></LinearLayout>"#)
        .file(
            "drawable/btn_bg.xml",
            r##"<shape><solid android:color="#FFCC00"/><corners android:radius="6dp"/></shape>"##,
        ))
}

fn one(content: &str) -> Node {
    let mut nodes = build(content, &std_res()).expect("build");
    assert_eq!(nodes.len(), 1);
    nodes.remove(0)
}

#[test]
fn linear_orientation_weights_and_margins() {
    let root = one(
        r#"<LinearLayout android:orientation="vertical" android:layout_width="match_parent" android:layout_height="wrap_content"
            android:padding="8dp" android:paddingTop="4dp">
            <View android:layout_width="match_parent" android:layout_height="0dp" android:layout_weight="1"
                android:layout_marginStart="6dp" android:layout_margin="2dp"/>
            <View android:layout_width="match_parent" android:layout_height="40dp"/>
        </LinearLayout>"#,
    );
    assert_eq!(root.kind, "LinearLayout");
    assert_eq!(root.style.flex_dir, Some(FlexDir::Column));
    assert_eq!(root.style.width, Some(Size::Match));
    assert_eq!(root.style.height, Some(Size::Wrap));
    assert_eq!(root.style.padding, Edges { start: 8.0, end: 8.0, top: 4.0, bottom: 8.0 });
    let c0 = &root.children[0];
    assert_eq!(c0.style.weight, Some(1.0));
    assert_eq!(c0.style.margin, Edges { start: 6.0, end: 2.0, top: 2.0, bottom: 2.0 });

    // Default orientation is horizontal.
    let root = one(r#"<LinearLayout><View/></LinearLayout>"#);
    assert_eq!(root.style.flex_dir, Some(FlexDir::Row));
}

#[test]
fn gravity_parsing() {
    assert_eq!(parse_gravity("center"), Gravity { h: HAlign::Center, v: VAlign::Center });
    assert_eq!(parse_gravity("start|center_vertical"), Gravity { h: HAlign::Start, v: VAlign::Center });
    assert_eq!(parse_gravity("fill_horizontal|bottom"), Gravity { h: HAlign::Fill, v: VAlign::Bottom });
    assert_eq!(parse_gravity("weird"), Gravity::default());
}

#[test]
fn text_resources_and_typography() {
    let root = one(r#"<TextView android:text="@string/title" android:textSize="18sp" android:textColor="@color/accent"
            android:textStyle="bold|italic" android:typeface="monospace" android:maxLines="2" android:ellipsize="end"
            android:letterSpacing="0.05" android:textAllCaps="true" android:gravity="center"/>"#);
    assert_eq!(root.text.as_deref(), Some("Hello"));
    assert_eq!(root.style.font_size, Some(18.0));
    assert_eq!(root.style.fg.as_deref(), Some("#FF0000"));
    assert_eq!(root.style.bold, Some(true));
    assert_eq!(root.style.italic, Some(true));
    assert_eq!(root.style.font_family, Some("monospace"));
    assert_eq!(root.style.max_lines, Some(2));
    assert!(root.style.ellipsize);
    assert_eq!(root.style.letter_spacing, Some(0.05));
    assert!(root.style.all_caps);
    assert_eq!(root.style.gravity, Some(Gravity { h: HAlign::Center, v: VAlign::Center }));

    // Unresolvable @string falls back to the raw reference.
    let root = one(r#"<TextView android:text="@string/missing"/>"#);
    assert_eq!(root.text.as_deref(), Some("@string/missing"));
}

#[test]
fn tools_override_runtime_attrs() {
    let root = one(r##"<TextView xmlns:tools="http://schemas.android.com/tools"
            android:text="@string/title" android:textColor="#111111" android:visibility="visible"
            tools:text="Design time" tools:textColor="#222222" tools:visibility="gone" tools:ignore="UnusedIds"/>"##);
    assert_eq!(root.text.as_deref(), Some("Design time"));
    assert_eq!(root.style.fg.as_deref(), Some("#222222"));
    assert_eq!(root.style.visibility, Visibility::Gone);
}

#[test]
fn backgrounds_from_color_and_shape() {
    let root = one(r#"<View android:background="@color/accent"/>"#);
    assert_eq!(root.style.bg_color.as_deref(), Some("#FF0000"));

    let root = one(r#"<View android:background="@drawable/btn_bg"/>"#);
    let shape = root.style.shape.as_ref().expect("shape");
    assert_eq!(shape.fill.as_deref(), Some("#FFCC00"));
    assert_eq!(shape.corners, [6.0; 4]);

    // Unknown drawable → labelled placeholder block.
    let root = one(r#"<View android:background="@drawable/nope"/>"#);
    assert_eq!(root.style.drawable.as_deref(), Some("nope"));
}

#[test]
fn relative_layout_rules_resolve() {
    let mut root = one(r#"<RelativeLayout android:layout_width="200dp" android:layout_height="300dp">
            <View android:id="@+id/top" android:layout_width="50dp" android:layout_height="20dp"
                android:layout_alignParentStart="true" android:layout_marginStart="10dp"/>
            <View android:id="@+id/next" android:layout_width="40dp" android:layout_height="30dp"
                android:layout_below="@id/top" android:layout_toEndOf="@id/top" android:layout_marginTop="5dp"/>
            <View android:layout_width="30dp" android:layout_height="30dp" android:layout_centerInParent="true"/>
            <View android:layout_width="20dp" android:layout_height="20dp"
                android:layout_alignParentEnd="true" android:layout_marginEnd="8dp"
                android:layout_alignParentBottom="true" android:layout_marginBottom="4dp"/>
        </RelativeLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    let c = &root.children;
    assert_eq!(c[0].style.pos, Some(Pos { left: Some(10.0), top: Some(0.0), right: None, bottom: None }));
    // below top (20) + margin 5 → y=25; toEndOf top (10+50=60) → x=60
    assert_eq!(c[1].style.pos, Some(Pos { left: Some(60.0), top: Some(25.0), right: None, bottom: None }));
    // centerInParent in a 200×300 parent with a 30×30 child
    assert_eq!(c[2].style.pos, Some(Pos { left: Some(85.0), top: Some(135.0), right: None, bottom: None }));
    // alignParentEnd/Bottom: only right/bottom offsets
    assert_eq!(c[3].style.pos, Some(Pos { left: None, top: None, right: Some(8.0), bottom: Some(4.0) }));
}

#[test]
fn relative_layout_match_parent_stretches() {
    let mut root = one(r#"<RelativeLayout android:layout_width="150dp" android:layout_height="100dp">
            <View android:layout_width="match_parent" android:layout_height="match_parent" android:layout_margin="12dp"/>
        </RelativeLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    assert_eq!(
        root.children[0].style.pos,
        Some(Pos { left: Some(12.0), top: Some(12.0), right: Some(12.0), bottom: Some(12.0) })
    );
}

#[test]
fn constraint_layout_parent_and_percent() {
    let mut root = one(r#"<ConstraintLayout android:layout_width="300dp" android:layout_height="200dp">
            <View android:layout_width="0dp" android:layout_height="40dp"
                android:layout_constraintStart_toStartOf="parent" android:layout_constraintEnd_toEndOf="parent"
                android:layout_constraintTop_toTopOf="parent"
                android:layout_marginStart="20dp" android:layout_marginEnd="10dp"/>
            <View android:layout_width="0dp" android:layout_height="50dp"
                android:layout_constraintWidth_percent="0.5"
                android:layout_constraintStart_toStartOf="parent" android:layout_constraintTop_toTopOf="parent"/>
        </ConstraintLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    // Stretch between margins.
    assert_eq!(
        root.children[0].style.pos,
        Some(Pos { left: Some(20.0), top: Some(0.0), right: Some(10.0), bottom: None })
    );
    // 50 % width, anchored top-start.
    assert_eq!(root.children[1].style.pos, Some(Pos { left: Some(0.0), top: Some(0.0), right: None, bottom: None }));
}

#[test]
fn constraint_layout_guideline_and_bias() {
    let mut root = one(r#"<ConstraintLayout android:layout_width="400dp" android:layout_height="300dp">
            <androidx.constraintlayout.widget.Guideline
                android:id="@+id/mid" android:orientation="vertical" android:layout_constraintGuide_percent="0.5"/>
            <View android:layout_width="40dp" android:layout_height="20dp"
                android:layout_constraintStart_toEndOf="@id/mid" android:layout_constraintTop_toTopOf="parent"/>
            <View android:layout_width="40dp" android:layout_height="20dp"
                android:layout_constraintStart_toStartOf="parent" android:layout_constraintEnd_toEndOf="parent"
                android:layout_constraintTop_toTopOf="parent" android:layout_constraintHorizontal_bias="0.25"/>
        </ConstraintLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    // Guideline at 50 % of 400.
    assert_eq!(
        root.children[0].style.pos,
        Some(Pos { left: Some(200.0), top: Some(0.0), right: None, bottom: Some(0.0) })
    );
    // Child starts at the guideline (x=200).
    assert_eq!(root.children[1].style.pos, Some(Pos { left: Some(200.0), top: Some(0.0), right: None, bottom: None }));
    // Centered between parents with bias 0.25: (400-40)*0.25 = 90.
    assert_eq!(root.children[2].style.pos, Some(Pos { left: Some(90.0), top: Some(0.0), right: None, bottom: None }));
}

#[test]
fn constraint_layout_ratio_sizes() {
    let mut root = one(r#"<ConstraintLayout android:layout_width="300dp" android:layout_height="300dp">
            <View android:layout_width="0dp" android:layout_height="60dp"
                android:layout_dimensionRatio="16:9"
                android:layout_constraintStart_toStartOf="parent" android:layout_constraintTop_toTopOf="parent"/>
        </ConstraintLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    let c = &root.children[0];
    assert_eq!(c.style.pos.unwrap().left, Some(0.0));
    // Ratio-derived width is written back for the renderer.
    let w = c.style.width.unwrap();
    assert!(matches!(w, Size::Px(v) if (v - 60.0 * 16.0 / 9.0).abs() < 0.01), "{w:?}");
}

#[test]
fn include_resolution_and_merge() {
    let nodes = build(
        r#"<LinearLayout><include layout="@layout/simple" android:layout_width="100dp"/></LinearLayout>"#,
        &std_res(),
    )
    .unwrap();
    let inc = &nodes[0].children[0];
    assert_eq!(inc.kind, "LinearLayout");
    assert_eq!(inc.style.width, Some(Size::Px(100.0)));
    assert_eq!(inc.children[0].text.as_deref(), Some("included"));

    // merge children splice directly into the including parent.
    let nodes = build(r#"<FrameLayout><include layout="@layout/merge_inc"/></FrameLayout>"#, &std_res()).unwrap();
    assert_eq!(nodes[0].children.len(), 2);
    assert_eq!(nodes[0].children[1].text.as_deref(), Some("m2"));
}

#[test]
fn include_cycle_becomes_placeholder() {
    // The first expansion happens; the re-entry is caught and degraded.
    let nodes = build(r#"<FrameLayout><include layout="@layout/cyclic"/></FrameLayout>"#, &std_res()).unwrap();
    let expanded = &nodes[0].children[0];
    assert_eq!(expanded.kind, "LinearLayout");
    assert_eq!(expanded.children[0].kind, "Placeholder");
    assert!(expanded.children[0].sub.as_deref().unwrap_or_default().contains("cyclic"));
}

#[test]
fn include_missing_becomes_placeholder() {
    let nodes = build(r#"<FrameLayout><include layout="@layout/absent"/></FrameLayout>"#, &std_res()).unwrap();
    assert_eq!(nodes[0].children[0].kind, "Placeholder");
}

#[test]
fn top_level_merge_unwraps() {
    let nodes = build(
        r#"<merge><TextView android:text="a"/><LinearLayout android:layout_width="match_parent"/></merge>"#,
        &std_res(),
    )
    .unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].kind, "TextView");
    assert_eq!(nodes[1].kind, "LinearLayout");
}

#[test]
fn unknown_views_and_fragments() {
    // A custom class with no built-in equivalent becomes a labelled box.
    let root = one(r#"<com.example.CustomView android:layout_width="40dp"/>"#);
    assert_eq!(root.kind, "View");
    assert_eq!(root.sub.as_deref(), Some("CustomView"));

    // Custom classes that end in a known widget name preview as that widget.
    let root = one(r#"<com.vinfast.ivi.oneui.button.OneUIButton android:text="Go"/>"#);
    assert_eq!(root.kind, "Button");
    assert_eq!(root.text.as_deref(), Some("Go"));

    let root = one(r#"<fragment android:name="com.example.ui.MyFragment"/>"#);
    assert_eq!(root.kind, "Placeholder");
    assert_eq!(root.sub.as_deref(), Some("MyFragment"));
}

#[test]
fn scroll_and_card_defaults() {
    let root = one(
        r#"<androidx.core.widget.NestedScrollView android:layout_width="match_parent" android:layout_height="match_parent">
            <TextView android:text="long"/>
        </androidx.core.widget.NestedScrollView>"#,
    );
    assert_eq!(root.kind, "NestedScrollView");

    let root = one(
        r##"<androidx.cardview.widget.CardView app:cardCornerRadius="12dp" app:cardElevation="4dp" app:cardBackgroundColor="#FFFFFF"
            android:layout_width="match_parent" android:layout_height="wrap_content">
            <TextView android:text="in card"/>
        </androidx.cardview.widget.CardView>"##,
    );
    assert_eq!(root.style.radius, Some(12.0));
    assert_eq!(root.style.elevation, Some(4.0));
    assert_eq!(root.style.bg_color.as_deref(), Some("#FFFFFF"));
}

#[test]
fn progress_and_seek() {
    let root = one(r#"<ProgressBar android:layout_width="wrap_content" android:layout_height="wrap_content"/>"#);
    assert!(root.style.spinner);

    let root = one(r#"<ProgressBar android:indeterminate="false" android:progress="30" android:max="200"/>"#);
    assert!(!root.style.spinner);
    assert_eq!(root.style.progress, Some((30.0, 200.0)));

    let root = one(r#"<SeekBar android:progress="50"/>"#);
    assert_eq!(root.style.progress, Some((50.0, 100.0)));
}

#[test]
fn image_view_src_and_scale() {
    let root = one(r#"<ImageView android:src="@drawable/photo" android:scaleType="centerCrop"/>"#);
    assert_eq!(root.sub.as_deref(), Some("photo"));
    assert_eq!(root.style.scale_type, Some("cover"));

    let root = one(r#"<ImageView android:src="@drawable/btn_bg"/>"#);
    assert!(root.style.shape.is_some());
    assert_eq!(root.sub, None);
}

#[test]
fn errors() {
    assert_eq!(build("", &std_res()).unwrap_err(), "no root element");
    assert_eq!(build("<!-- only a comment -->", &std_res()).unwrap_err(), "no root element");
    let e = build("<LinearLayout><TextView></LinearLayout>", &std_res()).unwrap_err();
    assert!(e.contains("XML error") && e.contains("line"), "{e}");
}

#[test]
fn estimates_are_sane() {
    let node = one(r#"<TextView android:text="Hello World" android:textSize="20sp"/>"#);
    let (w, h) = estimate_size(&node, DEVICE_W, DEVICE_H);
    assert!(w > 100.0 && w < 200.0, "{w}");
    assert!((h - 28.0).abs() < 0.01, "{h}");

    let btn = one(r#"<Button android:text="OK"/>"#);
    let (w, h) = estimate_size(&btn, DEVICE_W, DEVICE_H);
    assert!(h >= 36.0, "{h}");
    assert!(w > 40.0, "{w}");
}

#[test]
fn grid_layout_columns() {
    let root = one(r#"<GridLayout android:columnCount="3">
            <Button/><Button android:layout_column="2"/>
        </GridLayout>"#);
    assert_eq!(root.style.grid_cols, Some(3));
    assert_eq!(root.children[1].style.grid_col, Some(2));
}

#[test]
fn gone_children_are_marked() {
    let root = one(
        r#"<LinearLayout><TextView android:visibility="gone"/><TextView tools:visibility="invisible"/></LinearLayout>"#,
    );
    assert_eq!(root.children[0].style.visibility, Visibility::Gone);
    assert_eq!(root.children[1].style.visibility, Visibility::Invisible);
}

#[test]
fn constraint_rules_come_from_the_app_namespace() {
    // Real layouts put ConstraintLayout rules in `app:`; ignoring them collapses
    // every child onto the parent's origin.
    let mut root = one(r#"<androidx.constraintlayout.widget.ConstraintLayout
                xmlns:app="http://schemas.android.com/apk/res-auto"
                android:layout_width="400dp" android:layout_height="300dp" android:padding="16dp">
            <androidx.constraintlayout.widget.Guideline android:id="@+id/rail"
                android:layout_width="wrap_content" android:layout_height="wrap_content"
                android:orientation="vertical" app:layout_constraintGuide_begin="72dp"/>
            <View android:layout_width="0dp" android:layout_height="40dp"
                app:layout_constraintStart_toEndOf="@id/rail" app:layout_constraintEnd_toEndOf="parent"
                app:layout_constraintTop_toTopOf="parent"/>
            <View android:layout_width="0dp" android:layout_height="20dp"
                app:layout_constraintWidth_percent="0.5"
                app:layout_constraintStart_toStartOf="parent" app:layout_constraintTop_toTopOf="parent"/>
        </androidx.constraintlayout.widget.ConstraintLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    // The guideline is an anchor, not a drawn box.
    assert!(root.children[0].style.con.is_guideline());
    assert_eq!(
        root.children[1].style.pos,
        Some(Pos { left: Some(72.0), top: Some(0.0), right: Some(0.0), bottom: None })
    );
    // 0.5 of the 400 dp parent, not of the device.
    assert_eq!(root.children[2].style.width, Some(Size::Percent(50.0)));
}

#[test]
fn wrap_content_absolute_container_is_measured() {
    // A wrap_content ConstraintLayout/RelativeLayout has to end up with a real
    // size: absolutely positioned children cannot measure it in CSS, and a
    // centre rule must land inside the content, not the device.
    let mut root = one(r#"<RelativeLayout android:layout_width="match_parent" android:layout_height="wrap_content"
                android:padding="12dp">
            <CheckBox android:id="@+id/done" android:layout_width="wrap_content"
                android:layout_height="wrap_content" android:layout_alignParentStart="true"
                android:layout_centerVertical="true"/>
            <TextView android:layout_width="wrap_content" android:layout_height="wrap_content"
                android:layout_toEndOf="@id/done" android:text="One line"/>
        </RelativeLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    let h = match root.style.height {
        Some(Size::Px(v)) => v,
        other => panic!("wrap height not pinned: {other:?}"),
    };
    assert!(h > 40.0 && h < 100.0, "height {h} should wrap the one-line row");
    // Centred vertically inside that 56-ish box, not in an 891 dp device.
    let top = root.children[0].style.pos.unwrap().top.unwrap();
    assert!(top < 20.0, "centred checkbox top {top}");
    // The title starts after the checkbox (wrap estimate 32 wide).
    assert_eq!(root.children[1].style.pos.unwrap().left, Some(32.0));
}

#[test]
fn appcompat_and_material_widgets_map_to_base_kinds() {
    let root = one(r#"<LinearLayout android:orientation="vertical">
            <androidx.appcompat.widget.AppCompatButton android:text="Go"/>
            <com.google.android.material.button.MaterialButton android:text="Go"/>
            <com.google.android.material.card.MaterialCardView/>
            <androidx.appcompat.widget.SwitchCompat/>
            <com.google.android.material.textfield.TextInputEditText android:hint="Name"/>
            <com.google.android.material.floatingactionbutton.FloatingActionButton/>
        </LinearLayout>"#);
    let kinds: Vec<&str> = root.children.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["Button", "Button", "CardView", "Switch", "EditText", "FloatingActionButton"]);
}

#[test]
fn relative_baseline_and_editor_positions() {
    let mut root = one(r#"<RelativeLayout android:layout_width="200dp" android:layout_height="100dp">
            <TextView android:id="@+id/title" android:layout_width="wrap_content"
                android:layout_height="wrap_content" android:textSize="20sp" android:text="Title"/>
            <TextView android:layout_width="wrap_content" android:layout_height="wrap_content"
                android:textSize="12sp" android:text="12:00" android:layout_alignParentEnd="true"
                android:layout_alignBaseline="@id/title"/>
        </RelativeLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    // Baselines coincide: the smaller text sits (20-12)*0.8 = 6.4 px lower.
    let top = root.children[1].style.pos.unwrap().top.unwrap();
    assert!((top - 6.4).abs() < 0.01, "baseline offset {top}");

    let mut root = one(r#"<ConstraintLayout android:layout_width="200dp" android:layout_height="100dp">
            <View android:layout_width="30dp" android:layout_height="30dp"
                android:layout_editor_absoluteX="40dp" android:layout_editor_absoluteY="25dp"/>
        </ConstraintLayout>"#);
    resolve_positions(&mut root, DEVICE_W, DEVICE_H);
    assert_eq!(root.children[0].style.pos, Some(Pos { left: Some(40.0), top: Some(25.0), right: None, bottom: None }));
}
