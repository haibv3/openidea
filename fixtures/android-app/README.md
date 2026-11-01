# Android app fixture

A tiny Android project shape for manual QA of the plugin **preview** feature:

- `app/src/main/res/layout/*.xml` — layout files exercising the supported
  preview subset: `LinearLayout` weights and gravity, `RelativeLayout` rules,
  a `ConstraintLayout` subset, `include` + resource references
  (`@string/…`, `@color/…`, `@dimen/…`), and `tools:` overrides.
  `material_form.xml` adds the library-backed surface: an AppCompat `Toolbar`
  with an `app:menu`, `TextInputLayout`/`TextInputEditText`, chips, a
  `CardView`, a gradient/vector drawable, a color state list, a `RecyclerView`
  with `tools:listitem`, and `com.example.openidea.ui.BrandButton` — a custom
  view whose superclass (`MaterialButton`) is read from its Kotlin source.
- `app/src/main/AndroidManifest.xml` + `values/themes.xml` — the app/activity
  theme, so `?attr/colorPrimary`-style references resolve like in the IDE.
- `app/src/main/java/com/example/openidea/MainActivity.kt` — `@Preview`
  composable functions exercising the Compose approximation (Column/Row/Box,
  Text/Button, modifiers, colors, `widthDp`/`heightDp`/`showBackground`) and
  project resources (`stringResource`, `painterResource`, `colorResource`,
  `dimensionResource`).
- `preview-demo.demo` + `.openidea/plugins/preview-fixture/` — a bundled dev
  plugin that previews `*.demo` files, so the preview pipeline can be tried
  without installing anything (Settings › Plugins shows it with a _dev_ tag).
  The Android layouts above need the `android-preview` plugin
  (`plugins/android-preview`) installed first.

There is no buildable Gradle project here on purpose: this fixture needs no
Android SDK — it only exists so the previews have realistic files to render.
Open the project root in the IDE and open any of the files above.
