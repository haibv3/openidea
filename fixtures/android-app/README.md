# Android app fixture

A tiny Android project shape for manual QA of the plugin **preview** feature:

- `app/src/main/res/layout/*.xml` — layout files exercising the supported
  preview subset: `LinearLayout` weights and gravity, `RelativeLayout` rules,
  a `ConstraintLayout` subset, `include` + resource references
  (`@string/…`, `@color/…`, `@dimen/…`), and `tools:` overrides.
- `app/src/main/java/com/example/openidea/MainActivity.kt` — `@Preview`
  composable functions exercising the Compose approximation (Column/Row/Box,
  Text/Button, modifiers, colors, `widthDp`/`heightDp`/`showBackground`).
- `preview-demo.demo` + `.openidea/plugins/preview-fixture/` — a bundled dev
  plugin that previews `*.demo` files, so the preview pipeline can be tried
  without installing anything (Settings › Plugins shows it with a _dev_ tag).
  The Android layouts above need the `android-preview` plugin
  (`plugins/android-preview`) installed first.

There is no buildable Gradle project here on purpose: this fixture needs no
Android SDK — it only exists so the previews have realistic files to render.
Open the project root in the IDE and open any of the files above.
