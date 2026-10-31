/*
 * Layoutlib bridge: renders an Android XML layout with the engine Android
 * Studio's preview pane uses (the real framework View/LayoutInflater shipped
 * in Android Studio's `layoutlib.jar`).
 *
 * STATUS: work in progress; not wired into the host yet.
 *
 * Working (measured on D:/VF/VinfastSystemUI):
 *   - JNI bootstrap (libandroid_runtime.dll + layoutlib_jni.dll), ICU dat,
 *     fonts, keyboard, layoutlib `build.prop`.
 *   - Tables: framework `data/res/values*`, Gradle merged `values/` +
 *     `values-v<N>`, layouts as file paths, framework ids from
 *     `com.android.internal.R`, project ids from the runtime `R.txt`.
 *   - Theme chain (implicit parents), `android:` items in ANDROID, library
 *     items in RES_AUTO.
 *   - Inflation: decor `screen_simple`, then the whole project tree
 *     (ConstraintLayout, AppCompatImageView, RecyclerView, OneUIButton)
 *     through the project classpath.
 *   - The built APK (resources.arsc) is added with `AssetManager.addAssetPath`
 *     so the native table knows app resources; project ids are the real ones
 *     from `R.txt`.
 *
 * Fixed: Material's theme check. `BridgeContext` reads theme attrs through
 * `RenderResources.findItemInTheme`; the resolver's own theme state is empty
 * during inflation, so `StableResolver` flattens the whole theme chain into an
 * `attr name -> value` map (child first, `?attr` indirection resolved) and
 * answers from that. Namespace/type differences can no longer break lookups.
 *
 * Both the app APK (cookie 1) and `framework_res.jar` (cookie 2) are already
 * added with `AssetManager.addAssetPath`.
 *
 * Experimental: framework enums, project IDs and unqualified project bitmap
 * drawables are handled. Missing XML resources are not replaced by invented
 * drawables. Theme and other resource resolution remain incomplete; not integrated.
 *
 * Usage: java -cp <layoutlib + layoutlib-api + sdk-common + kxml2 + guava +
 *        kotlin-stdlib + this class> RenderBridge <config.properties>
 * Config: platform, studio, nativeDir, icuData, layout, namespace, res,
 *         resRoots, mergedDir (or mergedValues), rTxt, appApk, theme, width,
 *         height, out, classpathFile. `-Dlb.debug=true` prints theme/attr probes.
 */
import com.android.ide.common.rendering.api.*;
import com.android.ide.common.resources.ResourceResolver;
import com.android.ide.common.resources.ResourceValueMap;
import com.android.resources.*;
import org.kxml2.io.KXmlParser;
import org.xmlpull.v1.XmlPullParser;

import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;
import java.awt.image.BufferedImage;
import java.io.*;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

/** One-shot layoutlib renderer: `java RenderBridge <config.properties>`. */
public final class RenderBridge {
    static final Map<String, ResourceType> TYPES = new HashMap<>();
    static {
        for (ResourceType t : ResourceType.values()) TYPES.put(t.getName(), t);
    }

    public static void main(String[] args) throws Exception {
        Properties cfg = new Properties();
        try (Reader r = Files.newBufferedReader(Paths.get(args[0]), StandardCharsets.UTF_8)) {
            cfg.load(r);
        }
        String platform = cfg.getProperty("platform");
        String layout = cfg.getProperty("layout");
        String resDir = cfg.getProperty("res");
        List<File> resRoots = new ArrayList<>();
        resRoots.add(new File(resDir));
        for (String extra : cfg.getProperty("resRoots", "").split(java.util.regex.Pattern.quote(File.pathSeparator))) {
            if (!extra.isBlank() && new File(extra).isDirectory()) resRoots.add(new File(extra));
        }
        String mergedValues = cfg.getProperty("mergedValues", "");
        String theme = cfg.getProperty("theme", "");
        int width = Integer.parseInt(cfg.getProperty("width", "411"));
        int height = Integer.parseInt(cfg.getProperty("height", "891"));
        String out = cfg.getProperty("out");
        String asHome = cfg.getProperty("studio");

        // --- layoutlib class loader (child of this class' loader so the API
        // classes stay shared) ---
        // layoutlib.jar is on the system classpath: the native library binds
        // JNI lookups to the classloader that loaded the Bridge class, so the
        // API classes must live in that same loader.
        Class<?> impl = Class.forName("com.android.layoutlib.bridge.Bridge");
        Bridge bridge = (Bridge) impl.getDeclaredConstructor().newInstance();
        // The constructor attempts the JNI load with no path (and latches the
        // "attempted" flag); init is the call that supplies the real path.
        for (String f : new String[] { "sJniLibLoadAttempted", "sJniLibLoaded" }) {
            java.lang.reflect.Field fld = impl.getDeclaredField(f);
            fld.setAccessible(true);
            fld.setBoolean(null, false);
        }

        File data = new File(asHome, "plugins/design-tools/resources/layoutlib/data");
        File fonts = new File(data, "fonts");
        Map<String, String> platformProperties = new HashMap<>();
        Properties buildProp = new Properties();
        try (InputStream in = Files.newInputStream(new File(asHome, "plugins/design-tools/resources/layoutlib/build.prop").toPath())) {
            buildProp.load(in);
        }
        for (String k : buildProp.stringPropertyNames()) {
            platformProperties.put(k, buildProp.getProperty(k));
        }
        Map<String, Map<String, Integer>> enums = frameworkEnums(Paths.get(platform, "data/res/values/attrs.xml"));
        ILayoutLog log = new ILayoutLog() {
            public void error(String m, Object... p) { System.err.println("ERR " + fmt(m, p)); }
            public void error(String m, Throwable t) { System.err.println("ERR " + m + " :: " + t); }
            public void error(Throwable t) { t.printStackTrace(); }
            public void warning(String m, Object... p) { System.err.println("WARN " + fmt(m, p)); }
            public void warning(Throwable t) { System.err.println("WARN " + t); }
            public void log(String m, Object... p) { System.err.println("LOG " + fmt(m, p)); }
            public void error(String m, String msg, Throwable t, Object a, Object b) {
                System.err.println("ERR " + m + " :: " + msg);
                if (t != null) t.printStackTrace();
            }
        };
        // init(platformProperties, fontDir, nativeLibPath, icuPath,
        //      hyphenDataDir, keyboardPaths, enumValueMap, log)
        // layoutlib reads framework resources from an apk; AS ships the same
        // set as `framework_res.jar` next to the fonts.
        try {
            Class<?> am = Class.forName("android.content.res.AssetManager");
            java.lang.reflect.Field fw = am.getDeclaredField("FRAMEWORK_APK_PATH");
            fw.setAccessible(true);
            fw.set(null, new File(data, "framework_res.jar").getAbsolutePath());
            System.err.println("framework apk=" + fw.get(null));
        } catch (Throwable t) {
            System.err.println("framework apk path not settable: " + t);
        }

        boolean ok = bridge.init(platformProperties, fonts,
                Objects.requireNonNull(cfg.getProperty("nativeDir"), "nativeDir is required"),
                Objects.requireNonNull(cfg.getProperty("icuData"), "icuData is required"),
                new File(data, "hyphen-data").getAbsolutePath(),
                new String[] { new File(data, "keyboards/Generic.kcm").getAbsolutePath() },
                enums, log);
        System.err.println("init=" + ok);
        if (!ok) { System.exit(2); }

        // The app's compiled resource table (resources.arsc inside the built
        // APK) is what makes `Context.obtainStyledAttributes` see app attrs:
        // that path reads the native AssetManager theme, not RenderResources.
        String appApk = cfg.getProperty("appApk", "");
        if (!appApk.isEmpty() && new File(appApk).isFile()) {
            try {
                Class<?> amCls = Class.forName("android.content.res.AssetManager");
                java.lang.reflect.Field sys = amCls.getDeclaredField("sSystem");
                sys.setAccessible(true);
                Object sysAm = sys.get(null);
                if (sysAm != null) {
                    java.lang.reflect.Method add = amCls.getMethod("addAssetPath", String.class);
                    int cookie = (Integer) add.invoke(sysAm, new File(appApk).getAbsolutePath());
                    System.err.println("app asset cookie=" + cookie);
                }
            } catch (Throwable t) {
                System.err.println("app asset wiring failed: " + t);
            }
        }

        // Framework resources: layoutlib ships them as `framework_res.jar`
        // (an apk); the system AssetManager must know that path.
        try {
            Class<?> amCls = Class.forName("android.content.res.AssetManager");
            java.lang.reflect.Field sys = amCls.getDeclaredField("sSystem");
            sys.setAccessible(true);
            Object sysAm = sys.get(null);
            System.err.println("sSystem=" + sysAm);
            if (sysAm != null) {
                java.lang.reflect.Method add = amCls.getMethod("addAssetPath", String.class);
                int cookie = (Integer) add.invoke(sysAm, new File(data, "framework_res.jar").getAbsolutePath());
                System.err.println("framework asset cookie=" + cookie);
            }
        } catch (Throwable t) {
            System.err.println("framework asset wiring failed: " + t);
        }

        // Diagnostic: can layoutlib's Bitmap/Canvas path produce pixels at all?
        if (Boolean.getBoolean("lb.rectTest")) {
            android.graphics.Bitmap bmp =
                    android.graphics.Bitmap.createBitmap(64, 64, android.graphics.Bitmap.Config.ARGB_8888);
            android.graphics.Canvas c = new android.graphics.Canvas(bmp);
            android.graphics.Paint paint = new android.graphics.Paint();
            paint.setColor(android.graphics.Color.RED);
            c.drawRect(0, 0, 64, 64, paint);
            System.err.println("rect pixel=" + Integer.toHexString(bmp.getPixel(32, 32)));
            try (java.io.OutputStream os = new java.io.FileOutputStream(out)) {
                System.err.println("rect compress=" + bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, os));
            }
            System.exit(0);
        }

        // --- resources ---
        Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps = new HashMap<>();
        parseValues(new File(platform, "data/res"), ResourceNamespace.ANDROID, maps);
        String pkg = cfg.getProperty("namespace", "").trim();
        if (pkg.isEmpty()) throw new IllegalArgumentException("Missing project resource namespace");
        String mergedDir = cfg.getProperty("mergedDir", "");
        if (!mergedDir.isEmpty()) {
            // Gradle's merged values: `values/` then `values-v<N>` ascending.
            // Locale/density variants describe other configurations.
            File dir = new File(mergedDir);
            List<File> ordered = new ArrayList<>();
            File[] subs = dir.listFiles(File::isDirectory);
            if (subs != null) {
                for (File d : subs) {
                    if (d.getName().equals("values") || d.getName().matches("values-v\\d+")) ordered.add(d);
                }
            }
            ordered.sort(Comparator.comparingInt(d -> {
                String n = d.getName();
                return n.equals("values") ? 0 : Integer.parseInt(n.substring("values-v".length()));
            }));
            for (File d : ordered) {
                File[] files = d.listFiles(f -> f.getName().endsWith(".xml"));
                if (files == null) continue;
                for (File f : files) parseValuesFile(f.toPath(), ResourceNamespace.fromPackageName(pkg), maps);
            }
            System.err.println("merged values dirs=" + ordered.size());
        } else if (!mergedValues.isEmpty()) {
            parseValuesFile(Paths.get(mergedValues), ResourceNamespace.fromPackageName(pkg), maps);
        } else {
            parseValues(new File(resDir), ResourceNamespace.fromPackageName(pkg), maps);
        }
        // Layouts are resources too: the decor and <include> go through
        // getResolvedResource(LAYOUT, name) -> callback.getParser().
        registerLayouts(new File(platform, "data/res"), ResourceNamespace.ANDROID, maps);
        for (ResourceType type : new ResourceType[] { ResourceType.ANIMATOR, ResourceType.COLOR, ResourceType.DRAWABLE }) {
            registerXmlFiles(new File(platform, "data/res"), ResourceNamespace.ANDROID, type, maps);
        }
        registerLayouts(new File(resDir), ResourceNamespace.fromPackageName(pkg), maps);
        for (File root : resRoots) {
            registerBitmapDrawables(root, ResourceNamespace.fromPackageName(pkg), maps);
            for (ResourceType type : new ResourceType[] { ResourceType.ANIMATOR, ResourceType.COLOR, ResourceType.DRAWABLE }) {
                registerXmlFiles(root, ResourceNamespace.fromPackageName(pkg), type, maps);
            }
        }
        if (!mergedValues.isEmpty()) {
            registerLayouts(Paths.get(mergedValues).getParent().getParent().getParent().toFile(),
                    ResourceNamespace.fromPackageName(pkg), maps);
        }
        String symbolsFile = cfg.getProperty("rTxt", "");
        if (!symbolsFile.isBlank() && Files.isRegularFile(Paths.get(symbolsFile))) {
            ResourceNamespace appNamespace = ResourceNamespace.fromPackageName(pkg);
            ResourceValueMap projectIds = maps.computeIfAbsent(appNamespace, k -> new HashMap<>())
                    .computeIfAbsent(ResourceType.ID, k -> ResourceValueMap.create());
            for (String line : Files.readAllLines(Paths.get(symbolsFile), StandardCharsets.UTF_8)) {
                String[] parts = line.trim().split("\\s+");
                if (parts.length >= 4 && parts[0].equals("int") && parts[1].equals("id")
                        && projectIds.get(parts[2]) == null) {
                    projectIds.put(new ResourceValueImpl(appNamespace, ResourceType.ID, parts[2], null));
                }
            }
        }

        // Framework ids (`@android:id/content`) must resolve to their real
        // values; the platform's `data/res` has no id table.
        {
            ResourceValueMap ids = maps.computeIfAbsent(ResourceNamespace.ANDROID, k -> new HashMap<>())
                    .computeIfAbsent(ResourceType.ID, k -> ResourceValueMap.create());
            for (Map.Entry<Integer, ResourceReference> e : frameworkIds().entrySet()) {
                ResourceReference r = e.getValue();
                if (r.getResourceType() != ResourceType.ID) continue;
                if (ids.get(r.getName()) != null) continue;
                ids.put(new ResourceValueImpl(ResourceNamespace.ANDROID, ResourceType.ID, r.getName(), null));
            }
            System.err.println("framework ids=" + ids.size());
        }

        ResourceReference themeRef = theme.isEmpty() ? null
                : ResourceReference.style(ResourceNamespace.fromPackageName(pkg), theme);
        ResourceResolver resolver = ResourceResolver.create(maps, themeRef);
        resolver.setLogger(log);
        // The preview pane has no system bars: skip the decor's status-bar
        // background view (its drawable inflates to null without a full
        // AssetManager) by overriding the theme items that create it.
        StyleResourceValueImpl t = (StyleResourceValueImpl) resolver.getDefaultTheme();
        if (t != null) {
            // Framework attrs live in the android namespace, unprefixed name.
            t.addItem(new StyleItemResourceValueImpl(ResourceNamespace.ANDROID,
                    "windowDrawsSystemBarBackgrounds", "false", null));
            t.addItem(new StyleItemResourceValueImpl(ResourceNamespace.ANDROID,
                    "windowTranslucentStatus", "false", null));
        }
        // `getTheme()` is the *current* theme (null until one is applied);
        // the app theme is the default one `create` resolved.
        StyleResourceValue appTheme = resolver.getDefaultTheme();
        resolver.applyStyle(appTheme, true);
        boolean debug = Boolean.getBoolean("lb.debug");
        StyleResourceValue th = resolver.getTheme();
        if (debug) System.err.println("THEME name=" + (th == null ? null : th.getName())
                + " items=" + (th instanceof StyleResourceValueImpl ? ((StyleResourceValueImpl) th).getDefinedItems().size() : -1)
                + " parent=" + (th == null ? null : resolver.getParent(th)));
        StyleResourceValue walk = debug ? th : null;
        for (int i = 0; i < 12 && walk != null; i++) {
            System.err.println("CHAIN " + i + " " + walk.getName());
            walk = resolver.getParent(walk);
        }
        for (String a : debug ? new String[] { "colorPrimary", "colorAccent", "windowActionModeOverlay" } : new String[0]) {
            System.err.println("ATTR " + a + " = " + resolver.findItemInTheme(ResourceReference.attr(ResourceNamespace.ANDROID, a))
                + " | RES_AUTO = " + resolver.findItemInTheme(ResourceReference.attr(ResourceNamespace.fromPackageName(pkg), a)));
        }
        System.err.println("theme=" + themeRef + " resolved=" + resolver.getTheme()
                + " default=" + resolver.getDefaultTheme());

        // --- parser + callback ---
        String layoutName = new File(layout).getName().replace(".xml", "");
        ILayoutPullParser parser = parserFor(layout, ResourceNamespace.fromPackageName(pkg));
        System.err.println("parser root=" + parser.getName() + " event=" + parser.getEventType());
        ClassLoader projectLoader = new com.android.layoutlib.LayoutlibClassLoader(RenderBridge.class.getClassLoader());
        Path cpFile = Paths.get(cfg.getProperty("classpathFile", ""));
        String cp = Files.exists(cpFile) ? Files.readString(cpFile, StandardCharsets.UTF_8).trim() : "";
        if (!cp.isEmpty()) {
            List<URL> urls = new ArrayList<>();
            for (String part : cp.split(java.io.File.pathSeparator)) {
                if (!part.isBlank()) urls.add(new File(part).toURI().toURL());
            }
            projectLoader = new URLClassLoader(urls.toArray(new URL[0]), projectLoader);
        }
        LayoutlibCallback cb = new Cb(resDir, resRoots, platform, log, projectLoader, cfg);
        ((Cb) cb).resolver = resolver;

        HardwareConfig hw = new HardwareConfig(width, height, Density.MEDIUM, 160, 160,
                ScreenSize.NORMAL, width > height ? ScreenOrientation.LANDSCAPE : ScreenOrientation.PORTRAIT,
                ScreenRound.NOTROUND, false);
        System.err.println("appTheme=" + appTheme + " default=" + resolver.getDefaultTheme());
        RenderResources sessionResources = new StableResolver(resolver, appTheme);
        SessionParams params = new SessionParams(parser, SessionParams.RenderingMode.NORMAL,
                new Object(), hw, sessionResources, cb, 21, 34, log);
        params.setAssetRepository(new AssetRepository() {
            public boolean isSupported() { return true; }
            private Path underRoot(Path root, Path candidate) {
                try {
                    Path realRoot = root.toRealPath();
                    Path realFile = (candidate.isAbsolute() ? candidate : realRoot.resolve(candidate)).toRealPath();
                    return realFile.startsWith(realRoot) && Files.isRegularFile(realFile) ? realFile : null;
                } catch (IOException | java.nio.file.InvalidPathException e) {
                    return null;
                }
            }
            private Path resourcePath(String path) {
                try {
                    Path requested = Paths.get(path);
                    List<File> roots = new ArrayList<>();
                    roots.add(new File(platform, "data/res"));
                    roots.addAll(resRoots);
                    for (File root : roots) {
                        Path found = underRoot(root.toPath(), requested);
                        if (found == null && !requested.isAbsolute() && path.startsWith("res/")) {
                            found = underRoot(root.toPath(), Paths.get(path.substring(4)));
                        }
                        if (found != null) return found;
                    }
                } catch (java.nio.file.InvalidPathException ignored) { }
                return null;
            }
            public java.io.InputStream openAsset(String path, int mode) throws java.io.IOException {
                Path file = underRoot(Paths.get(resDir, "assets"), Paths.get(path));
                if (file == null) throw new java.io.FileNotFoundException(path);
                return Files.newInputStream(file);
            }
            public java.io.InputStream openNonAsset(int cookie, String path, int mode) throws java.io.IOException {
                Path file = resourcePath(path);
                if (file == null) throw new java.io.FileNotFoundException(path);
                return Files.newInputStream(file);
            }
            public boolean isFileResource(String path) { return resourcePath(path) != null; }
        });
        System.err.println("noDecor=" + params.isForceNoDecor());
        // Without an image factory layoutlib takes the hardware path and dies in
        // `ImageReader.acquireNextImage()`; a plain BufferedImage keeps it on
        // the software renderer, which is what a preview needs.
        params.setSizeProvider((w, h) -> new java.awt.Dimension(width, height));
        RenderSession session = bridge.createSession(params);
        Result sr = session.getResult();
        System.err.println("session status=" + sr.getStatus() + " msg=" + sr.getErrorMessage()
                + " roots=" + (session.getRootViews() == null ? -1 : session.getRootViews().size()));
        if (sr.getException() != null) sr.getException().printStackTrace();
        System.err.println("hw=" + hw.getScreenWidth() + "x" + hw.getScreenHeight() + " density=" + hw.getDensity());
        Result res = session.render(5000);
        System.err.println("render=" + res.isSuccess() + " " + res.getStatus() + " " + res.getErrorMessage());
        BufferedImage img = session.getImage();
        if (img == null) {
            RecyclableImage rec = session.getRecyclableImage();
            if (rec != null) {
                System.err.println("recyclable image " + rec.getWidth() + "x" + rec.getHeight());
                img = rec.getCopy();
            }
        }
        if (img == null) { System.err.println("no image"); System.exit(2); }
        javax.imageio.ImageIO.write(img, "png", new File(out));
        System.err.println("wrote " + out + " " + img.getWidth() + "x" + img.getHeight());
        session.dispose();
        System.exit(res.isSuccess() ? 0 : 3);
    }

    /** Enum and flag values are required by layoutlib when parsing framework XML. */
    static Map<String, Map<String, Integer>> frameworkEnums(Path attrs) throws Exception {
        DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
        factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
        org.w3c.dom.NodeList nodes = factory.newDocumentBuilder().parse(attrs.toFile()).getElementsByTagName("attr");
        Map<String, Map<String, Integer>> result = new HashMap<>();
        for (int i = 0; i < nodes.getLength(); i++) {
            org.w3c.dom.Element attr = (org.w3c.dom.Element) nodes.item(i);
            String name = attr.getAttribute("name");
            for (org.w3c.dom.Node child = attr.getFirstChild(); child != null; child = child.getNextSibling()) {
                if (!(child instanceof org.w3c.dom.Element)) continue;
                org.w3c.dom.Element value = (org.w3c.dom.Element) child;
                if (!"enum".equals(value.getTagName()) && !"flag".equals(value.getTagName())) continue;
                String key = value.getAttribute("name");
                try {
                    int number = (int) Long.decode(value.getAttribute("value")).longValue();
                    result.computeIfAbsent(name, ignored -> new HashMap<>()).put(key, number);
                } catch (NumberFormatException ignored) { }
            }
        }
        return result;
    }

    /** `com.android.internal.R` id → framework ResourceReference. */
    static Map<Integer, ResourceReference> sFrameworkIds;

    static Map<String, Integer> sFrameworkIdByName;

    static Integer frameworkIdOf(ResourceReference ref) {
        if (sFrameworkIdByName == null) {
            Map<String, Integer> byName = new HashMap<>();
            for (Map.Entry<Integer, ResourceReference> e : frameworkIds().entrySet()) {
                byName.put(e.getValue().getResourceType().getName() + "/" + e.getValue().getName(), e.getKey());
            }
            sFrameworkIdByName = byName;
        }
        return sFrameworkIdByName.get(ref.getResourceType().getName() + "/" + ref.getName());
    }

    static Map<Integer, ResourceReference> frameworkIds() {
        if (sFrameworkIds != null) return sFrameworkIds;
        Map<Integer, ResourceReference> map = new HashMap<>();
        try {
            Class<?> r = Class.forName("com.android.internal.R");
            for (Class<?> inner : r.getDeclaredClasses()) {
                String simple = inner.getSimpleName();
                if (simple.isEmpty()) continue;
                ResourceType type = ResourceType.fromClassName(simple);
                if (type == null) continue;
                for (java.lang.reflect.Field f : inner.getDeclaredFields()) {
                    if (f.getType() != int.class) continue;
                    try {
                        map.put(f.getInt(null), new ResourceReference(ResourceNamespace.ANDROID, type, f.getName()));
                    } catch (Throwable ignored) { }
                }
            }
        } catch (Throwable t) {
            System.err.println("framework R unavailable: " + t);
        }
        sFrameworkIds = map;
        return map;
    }

    static void walk(android.view.View v, int depth) {
        StringBuilder pad = new StringBuilder();
        for (int i = 0; i < depth; i++) pad.append("  ");
        System.err.println("TREE " + pad + v.getClass().getSimpleName() + " "
                + v.getLeft() + "," + v.getTop() + " " + v.getWidth() + "x" + v.getHeight()
                + (v.getId() == android.R.id.content ? " (content)" : ""));
        if (v instanceof android.view.ViewGroup) {
            android.view.ViewGroup g = (android.view.ViewGroup) v;
            for (int i = 0; i < g.getChildCount(); i++) walk(g.getChildAt(i), depth + 1);
        }
    }

    /** Marks the whole tree for re-measure (`forceLayout` is shallow). */
    static void forceLayoutDeep(android.view.View v) {
        v.forceLayout();
        if (v instanceof android.view.ViewGroup) {
            android.view.ViewGroup g = (android.view.ViewGroup) v;
            for (int i = 0; i < g.getChildCount(); i++) forceLayoutDeep(g.getChildAt(i));
        }
    }

    static String data0(String asHome) {
        return asHome + "/plugins/design-tools/resources/layoutlib/data";
    }


    /** Keeps the defining style's namespace separate from its attribute namespace. */
    static final class OwnedStyleItem extends StyleItemResourceValueImpl {
        final ResourceNamespace owner;
        OwnedStyleItem(ResourceNamespace owner, ResourceNamespace attribute, String name, String value) {
            super(attribute, name, value, null);
            this.owner = owner;
        }
    }

    /**
     * layoutlib clears the resolver's themes when a session starts and then
     * relies on `getDefaultTheme()`; with a resolver built from parsed tables
     * that leaves no theme at all, so `obtainStyledAttributes` sees nothing and
     * Material's `checkAppCompatTheme` fails. This shim keeps the app theme
     * alive and ignores the clears.
     */
    static final class StableResolver extends RenderResources {
        private final ResourceResolver inner;
        private StyleResourceValue theme;

        StableResolver(ResourceResolver inner, StyleResourceValue theme) {
            this.inner = inner;
            this.theme = theme;
            inner.applyStyle(theme, true);
        }

        public void setLogger(ILayoutLog log) { inner.setLogger(log); }
        public StyleResourceValue getDefaultTheme() { return theme; }
        public StyleResourceValue getTheme() { return theme; }
        public void applyStyle(StyleResourceValue style, boolean force) {
            if (style != null) theme = style;
        }

        /** Walks the theme chain — the resolver's own theme state is empty. */
        public ResourceValue findItemInTheme(ResourceReference ref) {
            boolean probe = Boolean.getBoolean("lb.debug") && "colorPrimary".equals(ref.getName());
            if (probe) System.err.println("SHIM-FIND colorPrimary ns=" + ref.getNamespace() + " theme=" + (theme == null ? null : theme.getName()));
            // Library attrs live in RES_AUTO, framework ones in ANDROID; the
            // caller's namespace depends on which R class it used.
            ResourceReference[] refs = {
                ref,
                ResourceReference.attr(ResourceNamespace.RES_AUTO, ref.getName()),
                ResourceReference.attr(ResourceNamespace.ANDROID, ref.getName()),
            };
            StyleResourceValue t = theme;
            for (int i = 0; i < 32 && t != null; i++) {
                ResourceValue v = null;
                for (ResourceReference r : refs) {
                    v = inner.findItemInStyle(t, r);
                    if (v != null) break;
                }
                if (v != null) {
                    // `?attr/x` items resolve through the same chain.
                    for (int d = 0; d < 8 && v.getValue() != null && v.getValue().startsWith("?"); d++) {
                        String name = v.getValue().substring(v.getValue().lastIndexOf('/') + 1);
                        ResourceValue next = null;
                        for (StyleResourceValue c = theme; c != null && next == null; c = inner.getParent(c)) {
                            next = inner.findItemInStyle(c, ResourceReference.attr(ref.getNamespace(), name));
                        }
                        if (next == null || next == v) break;
                        v = next;
                    }
                    if (probe) System.err.println("SHIM-FIND -> " + v);
                    return v;
                }
                t = inner.getParent(t);
            }
            if (probe) System.err.println("SHIM-FIND -> null");
            return null;
        }
        public void clearStyles() { }
        public void clearAllThemes() { }
        public List<StyleResourceValue> getAllThemes() { return inner.getAllThemes(); }
        public ResourceValue findItemInStyle(StyleResourceValue style, ResourceReference ref) {
            return inner.findItemInStyle(style, ref);
        }
        public ResourceValue findResValue(String value, boolean force) { return inner.findResValue(value, force); }
        public ResourceValue dereference(ResourceValue value) {
            ResourceValue v = inner.dereference(value);
            return v != null ? v : value;
        }
        public ResourceValue getUnresolvedResource(ResourceReference ref) {
            ResourceValue v = findItemInTheme(ref);
            return v != null ? v : inner.getUnresolvedResource(ref);
        }

        public ResourceValue getResolvedResource(ResourceReference ref) {
            ResourceValue v = findItemInTheme(ref);
            if (v != null) {
                ResourceValue r = inner.dereference(v);
                return r != null ? r : v;
            }
            return inner.getResolvedResource(ref);
        }
        public ResourceValue resolveResValue(ResourceValue value) {
            return resolveResValue(value, new HashSet<>());
        }
        private ResourceValue resolveResValue(ResourceValue value, Set<ResourceReference> visited) {
            if (value == null || value.getValue() == null) return value;
            ResourceNamespace ns = value instanceof OwnedStyleItem
                    ? ((OwnedStyleItem) value).owner : value.getNamespace();
            ResourceNamespace.Resolver namespaces = value.getNamespaceResolver();
            if (namespaces == null) namespaces = ResourceNamespace.Resolver.EMPTY_RESOLVER;
            ResourceUrl url = ResourceUrl.parse(value.getValue());
            if (url != null) {
                ResourceReference ref = url.resolve(ns, namespaces);
                if (ref != null && visited.add(ref)) {
                    ResourceValue target = url.isTheme() ? findItemInTheme(ref) : inner.getUnresolvedResource(ref);
                    if (target != null && target != value) return resolveResValue(target, visited);
                }
            }
            if (value instanceof OwnedStyleItem) {
                return new com.android.layoutlib.bridge.android.UnresolvedResourceValue(
                        value.getValue(), ns, namespaces);
            }
            return value;
        }
        public StyleResourceValue getParent(StyleResourceValue style) { return inner.getParent(style); }
        public StyleResourceValue getStyle(ResourceReference ref) { return inner.getStyle(ref); }
    }

    static String fmt(String m, Object[] p) {
        try { return String.format(m, p); } catch (Exception e) { return m + Arrays.toString(p); }
    }

    // ---------------- resources ----------------

    static void parseValues(File resDir, ResourceNamespace ns,
                            Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps) throws Exception {
        File[] dirs = resDir.listFiles(f -> f.isDirectory() && f.getName().startsWith("values"));
        if (dirs == null) return;
        for (File d : dirs) {
            File[] files = d.listFiles(f -> f.getName().endsWith(".xml"));
            if (files == null) continue;
            for (File f : files) parseValuesFile(f.toPath(), ns, maps);
        }
    }

    static void parseValuesFile(Path file, ResourceNamespace ns,
                                Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps) {
        try {
            DocumentBuilderFactory dbf = DocumentBuilderFactory.newInstance();
            dbf.setNamespaceAware(false);
            DocumentBuilder db = dbf.newDocumentBuilder();
            org.w3c.dom.Document doc = db.parse(file.toFile());
            org.w3c.dom.Element root = doc.getDocumentElement();
            for (org.w3c.dom.Node n = root.getFirstChild(); n != null; n = n.getNextSibling()) {
                if (n.getNodeType() != org.w3c.dom.Node.ELEMENT_NODE) continue;
                org.w3c.dom.Element el = (org.w3c.dom.Element) n;
                String tag = el.getTagName();
                String name = el.getAttribute("name");
                if (name.isEmpty()) continue;
                ResourceValueImpl v;
                switch (tag) {
                    case "style": {
                        String parent = el.getAttribute("parent");
                        String parentName = parent.isEmpty() ? null : parent.replace("@style/", "").replace("@android:style/", "");
                        if (parentName == null && name.contains(".")) {
                            // Implicit parent: `A.B.C` extends `A.B`.
                            parentName = name.substring(0, name.lastIndexOf('.'));
                        }
                        v = new StyleResourceValueImpl(ns, name, parentName, null);
                        ResourceValueMap items = null;
                        for (org.w3c.dom.Node c = el.getFirstChild(); c != null; c = c.getNextSibling()) {
                            if (c.getNodeType() != org.w3c.dom.Node.ELEMENT_NODE) continue;
                            org.w3c.dom.Element item = (org.w3c.dom.Element) c;
                            if (!"item".equals(item.getTagName())) continue;
                            String in = item.getAttribute("name");
                            String iv = item.getTextContent().trim();
                            // Unprefixed framework style items are framework attrs;
                            // unprefixed project/library items use res-auto.
                            boolean prefixed = in.startsWith("android:");
                            boolean fwAttr = prefixed || ns.equals(ResourceNamespace.ANDROID);
                            String itemName = prefixed ? in.substring("android:".length()) : in;
                            StyleItemResourceValueImpl si = new OwnedStyleItem(ns,
                                    fwAttr ? ResourceNamespace.ANDROID : ResourceNamespace.RES_AUTO, itemName, iv);
                            ((StyleResourceValueImpl) v).addItem(si);
                        }
                        break;
                    }
                    case "attr": {
                        // Formats matter: `obtainStyledAttributes` reports a
                        // value only for attrs it knows how to read.
                        AttrResourceValueImpl attr = new AttrResourceValueImpl(
                                el.getAttribute("format").startsWith("android:") ? ResourceNamespace.ANDROID : ns,
                                name.replace("android:", ""), null);
                        String formats = el.getAttribute("format");
                        if (!formats.isEmpty()) {
                            java.util.Set<AttributeFormat> set = java.util.EnumSet.noneOf(AttributeFormat.class);
                            for (String f : formats.split("\\|")) {
                                try {
                                    set.add(AttributeFormat.valueOf(f.trim().toUpperCase(Locale.ROOT)));
                                } catch (IllegalArgumentException ignored) { }
                            }
                            if (!set.isEmpty()) attr.setFormats(set);
                        }
                        for (org.w3c.dom.Node c = el.getFirstChild(); c != null; c = c.getNextSibling()) {
                            if (c.getNodeType() != org.w3c.dom.Node.ELEMENT_NODE) continue;
                            org.w3c.dom.Element en = (org.w3c.dom.Element) c;
                            if (!"enum".equals(en.getTagName()) && !"flag".equals(en.getTagName())) continue;
                            try {
                                attr.addValue(en.getAttribute("name"),
                                        Integer.decode(en.getAttribute("value")), null);
                            } catch (NumberFormatException ignored) { }
                        }
                        v = attr;
                        break;
                    }
                    case "string":
                        v = new TextResourceValueImpl(ns, name, el.getTextContent(), null, null);
                        break;
                    case "color":
                        v = new ResourceValueImpl(ns, ResourceType.COLOR, name, el.getTextContent().trim(), null);
                        break;
                    case "dimen":
                        v = new ResourceValueImpl(ns, ResourceType.DIMEN, name, el.getTextContent().trim(), null);
                        break;
                    case "bool":
                        v = new ResourceValueImpl(ns, ResourceType.BOOL, name, el.getTextContent().trim(), null);
                        break;
                    case "integer":
                        v = new ResourceValueImpl(ns, ResourceType.INTEGER, name, el.getTextContent().trim(), null);
                        break;
                    case "array":
                    case "string-array":
                    case "integer-array":
                        v = new ArrayResourceValueImpl(ns, name, null);
                        for (org.w3c.dom.Node c = el.getFirstChild(); c != null; c = c.getNextSibling()) {
                            if (c.getNodeType() == org.w3c.dom.Node.ELEMENT_NODE) {
                                ((ArrayResourceValueImpl) v).addElement(c.getTextContent().trim());
                            }
                        }
                        break;
                    case "id":
                        v = new ResourceValueImpl(ns, ResourceType.ID, name, null);
                        break;
                    default:
                        continue;
                }
                ResourceValueMap map = maps.computeIfAbsent(ns, k -> new HashMap<>())
                        .computeIfAbsent(v.getResourceType(), k -> ResourceValueMap.create());
                map.put(v);
            }
        } catch (Exception e) {
            System.err.println("values parse failed " + file + ": " + e);
        }
    }

    /** Adds unqualified bitmap drawables from project resource roots. */
    static void registerBitmapDrawables(File resDir, ResourceNamespace ns,
                                        Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps) {
        File[] files = new File(resDir, "drawable").listFiles(File::isFile);
        if (files == null) return;
        ResourceValueMap drawables = maps.computeIfAbsent(ns, k -> new HashMap<>())
                .computeIfAbsent(ResourceType.DRAWABLE, k -> ResourceValueMap.create());
        for (File file : files) {
            String filename = file.getName();
            int dot = filename.indexOf('.');
            if (dot <= 0) continue;
            String ext = filename.substring(dot).toLowerCase(Locale.ROOT);
            if (!ext.equals(".png") && !ext.equals(".jpg") && !ext.equals(".jpeg")
                    && !ext.equals(".webp") && !ext.equals(".9.png")) continue;
            String name = filename.substring(0, dot);
            if (drawables.get(name) == null) {
                drawables.put(new ResourceValueImpl(ns, ResourceType.DRAWABLE, name, file.getAbsolutePath(), null));
            }
        }
    }

    /** Adds unqualified XML resources of a specific type. */
    static void registerXmlFiles(File resDir, ResourceNamespace ns, ResourceType type,
                                 Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps) {
        File[] files = new File(resDir, type.getName()).listFiles(f -> f.isFile() && f.getName().endsWith(".xml"));
        if (files == null) return;
        ResourceValueMap values = maps.computeIfAbsent(ns, k -> new HashMap<>())
                .computeIfAbsent(type, k -> ResourceValueMap.create());
        for (File file : files) {
            String name = file.getName().substring(0, file.getName().length() - 4);
            if (values.get(name) == null) {
                values.put(new ResourceValueImpl(ns, type, name, file.getAbsolutePath(), null));
            }
        }
    }

    /** Registers every layout dir entry (name.xml) as a LAYOUT resource. */
    static void registerLayouts(File resDir, ResourceNamespace ns,
                                Map<ResourceNamespace, Map<ResourceType, ResourceValueMap>> maps) {
        File[] dirs = resDir.listFiles(f -> f.isDirectory() && f.getName().startsWith("layout"));
        if (dirs == null) return;
        ResourceValueMap map = maps.computeIfAbsent(ns, k -> new HashMap<>())
                .computeIfAbsent(ResourceType.LAYOUT, k -> ResourceValueMap.create());
        for (File d : dirs) {
            File[] files = d.listFiles(f -> f.getName().endsWith(".xml"));
            if (files == null) continue;
            for (File f : files) {
                String name = f.getName().substring(0, f.getName().length() - 4);
                if (map.get(name) != null) continue; // base `layout/` wins
                map.put(new ResourceValueImpl(ns, ResourceType.LAYOUT, name, f.getAbsolutePath(), null));
            }
        }
    }

    // ---------------- parser / callback ----------------

    static ILayoutPullParser parserFor(String path, ResourceNamespace ns) throws Exception {
        LayoutParser p = new LayoutParser(ns);
        p.setFeature(XmlPullParser.FEATURE_PROCESS_NAMESPACES, true);
        // The parser reads lazily: the reader must stay open.
        p.setInput(Files.newBufferedReader(Paths.get(path), StandardCharsets.UTF_8));
        return p;
    }

    static final class LayoutParser extends KXmlParser implements ILayoutPullParser {
        private final ResourceNamespace ns;
        LayoutParser(ResourceNamespace ns) { this.ns = ns; }
        public Object getViewCookie() { return null; }
        public ResourceNamespace getLayoutNamespace() { return ns; }
    }

    static final class Cb extends LayoutlibCallback {
        final String resDir;
        final ILayoutLog log;
        final Map<ResourceReference, Integer> ids = new HashMap<>();
        final Map<Integer, ResourceReference> byId = new HashMap<>();
        Map<Integer, ResourceReference> symbols;
        Map<String, Integer> idByName;
        ResourceResolver resolver;
        final Properties cfg;
        int next = 0x7f100000;

        final ClassLoader projectLoader;

        final String platform;

        final List<File> resRoots;

        Cb(String resDir, List<File> resRoots, String platform, ILayoutLog log, ClassLoader projectLoader,
           Properties cfg) {
            this.cfg = cfg;
            this.resDir = resDir;
            this.resRoots = resRoots;
            this.platform = platform;
            this.log = log;
            this.projectLoader = projectLoader;
        }

        public Object loadView(String name, Class[] sig, Object[] args) throws Exception {
            System.err.println("loadView " + name);
            try {
                Class<?> c = Class.forName(name, true, projectLoader);
                return c.getConstructor(sig).newInstance(args);
            } catch (ClassNotFoundException e) {
                System.err.println("missing view class " + name);
                return null;
            }
        }
        public Class<?> findClass(String name) throws ClassNotFoundException {
            return Class.forName(name, false, projectLoader);
        }
        public Object loadClass(String name, Class[] sig, Object[] args) throws ClassNotFoundException {
            // layoutlib asks for optional helpers (a sample RecyclerView adapter,
            // for instance); null means "none" and it keeps inflating.
            try {
                Class<?> c = Class.forName(name, true, projectLoader);
                if (java.lang.reflect.Modifier.isAbstract(c.getModifiers())) return null;
                return c.getConstructor(sig).newInstance(args);
            } catch (Throwable t) {
                return null;
            }
        }
        public boolean isClassLoaded(String name) {
            try { Class.forName(name, false, projectLoader); return true; } catch (Throwable t) { return false; }
        }
        public boolean shouldUseCustomInflater() { return true; }
        public ResourceReference resolveResourceId(int id) {
            ResourceReference r0 = byId.get(id);
            ResourceReference sym0 = projectSymbols().get(id);
            ResourceReference fw0 = frameworkIds().get(id);
            ResourceReference probe = r0 != null ? r0 : (sym0 != null ? sym0 : fw0);
            if (Boolean.getBoolean("lb.debug") && probe != null && probe.getResourceType() == ResourceType.ATTR
                    && "colorPrimary".equals(probe.getName())) {
                ResourceResolver r = resolver;
                System.err.println("RESOLVE-ID colorPrimary ns=" + probe.getNamespace()
                        + " theme=" + (r == null || r.getTheme() == null ? null : r.getTheme().getName())
                        + " value=" + (r == null ? null : r.findItemInTheme(probe))
                        + " unresolved=" + (r == null ? null : r.getUnresolvedResource(probe)));
            }
            ResourceReference r = byId.get(id);
            if (r != null) return r;
            ResourceReference sym = projectSymbols().get(id);
            if (sym != null) return sym;
            return frameworkIds().get(id);
        }

        /** ref (type/name) -> real id, from the same `R.txt`. */
        Map<String, Integer> projectIdByName() {
            if (idByName == null) {
                Map<String, Integer> map = new HashMap<>();
                for (Map.Entry<Integer, ResourceReference> e : projectSymbols().entrySet()) {
                    map.put(e.getValue().getResourceType().getName() + "/" + e.getValue().getName(), e.getKey());
                }
                idByName = map;
            }
            return idByName;
        }

        Integer projectIdOf(ResourceReference ref) {
            return projectIdByName().get(ref.getResourceType().getName() + "/" + ref.getName());
        }

        /** `R.txt` lines: `int type name 0x7f0e0001`. */
        Map<Integer, ResourceReference> projectSymbols() {
            if (symbols != null) return symbols;
            Map<Integer, ResourceReference> map = new HashMap<>();
            String path = cfg.getProperty("rTxt", "");
            if (!path.isEmpty() && new File(path).isFile()) {
                try {
                    for (String line : Files.readAllLines(Paths.get(path), StandardCharsets.UTF_8)) {
                        String[] parts = line.trim().split("\\s+");
                        if (parts.length < 4 || !parts[0].equals("int")) continue;
                        ResourceType type = ResourceType.fromClassName(parts[1]);
                        if (type == null || type == ResourceType.STYLEABLE) continue;
                        try {
                            int id = (int) Long.parseLong(parts[3].replace("0x", ""), 16);
                            map.put(id, new ResourceReference(
                                    ResourceNamespace.fromPackageName(cfg.getProperty("namespace")), type, parts[2]));
                        } catch (NumberFormatException ignored) { }
                    }
                } catch (Exception e) {
                    System.err.println("R.txt parse failed: " + e);
                }
            }
            System.err.println("project symbols=" + map.size());
            symbols = map;
            return map;
        }
        public int getOrGenerateResourceId(ResourceReference ref) {
            // Framework ids must stay the real `android.R` values: the
            // framework's own `findViewById(R.id.content)` compares against them.
            if (Boolean.getBoolean("lb.debug") && ref.getResourceType() == ResourceType.ATTR
                    && "colorPrimary".equals(ref.getName())) {
                System.err.println("ID-REQ colorPrimary ns=" + ref.getNamespace());
            }
            if (ref.isFramework()) {
                Integer real = frameworkIdOf(ref);
                if (ref.getResourceType() == ResourceType.ID) {
                    System.err.println("idRef " + ref.getName() + " fw=" + real);
                }
                if (real != null) return real;
            }
            if (!ref.isFramework()) {
                Integer real = projectIdOf(ref);
                if (real != null) return real;
            }
            return ids.computeIfAbsent(ref, r -> {
                while (projectSymbols().containsKey(next) || frameworkIds().containsKey(next)
                        || byId.containsKey(next)) next++;
                int id = next++;
                byId.put(id, r);
                return id;
            });
        }
        public ILayoutPullParser getParser(ResourceValue v) {
            try {
                String val = v.getValue();
                // AndroidX can request an XML parser for a literal color when
                // layoutlib reports Resources.getValue() as TYPE_STRING.
                if (v.getResourceType() == ResourceType.COLOR && val != null
                        && val.matches("#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})")) {
                    LayoutParser parser = new LayoutParser(v.getNamespace());
                    parser.setFeature(XmlPullParser.FEATURE_PROCESS_NAMESPACES, true);
                    parser.setInput(new StringReader("<selector xmlns:android=\"http://schemas.android.com/apk/res/android\">"
                            + "<item android:color=\"" + val + "\"/></selector>"));
                    return parser;
                }
                String file = null;
                if (val != null) {
                    File direct = new File(val);
                    file = direct.isFile() ? direct.getAbsolutePath() : resolve(val);
                }
                if (file == null) file = resolve(v.getName());
                if (file == null) {
                    System.err.println("no parser for " + v);
                    return null;
                }
                System.err.println("getParser " + v.getName() + " -> " + file);
                return parserFor(file, v.getNamespace());
            } catch (Exception e) {
                System.err.println("getParser failed " + v + ": " + e);
                return null;
            }
        }
        public AdapterBinding getAdapterBinding(Object o, Map<String, String> m) { return null; }
        public XmlPullParser createXmlParser() { return new KXmlParser(); }
        public XmlPullParser createXmlParserForPsiFile(String path) { System.err.println("xmlForPsi " + path); return createXmlParserForFile(path); }
        public XmlPullParser createXmlParserForFile(String path) {
            System.err.println("xmlForFile " + path + " -> " + resolve(path));
            try {
                String file = resolve(path);
                if (file == null) {
                    ResourceUrl url = ResourceUrl.parse(path);
                    if (url != null && url.type == ResourceType.COLOR && !url.isTheme()) {
                        ResourceReference ref = url.resolve(ResourceNamespace.fromPackageName(cfg.getProperty("namespace")),
                                ResourceNamespace.Resolver.EMPTY_RESOLVER);
                        ResourceValue color = ref == null || resolver == null ? null : resolver.getResolvedResource(ref);
                        ILayoutPullParser parsed = color == null ? null : getParser(color);
                        if (parsed != null) return parsed;
                    }
                }
                if (file == null) {
                    System.err.println("resource file not found: " + path);
                    return null;
                }
                // A file under the platform's `data/res` is a framework
                // resource: its `@+id/...` belong to the android namespace.
                boolean framework = file.replace('\\', '/').contains("/platforms/") || path.startsWith("@android");
                return parserFor(file, framework ? ResourceNamespace.ANDROID
                        : ResourceNamespace.fromPackageName(cfg.getProperty("namespace")));
            } catch (Exception e) {
                System.err.println("parser for " + path + " failed: " + e);
                return null;
            }
        }

        /** `@android:anim/x`, `@anim/x` or a plain path → a file on disk. */
        private String resolve(String path) {
            String p = path.trim();
            boolean fw = p.startsWith("@android:");
            if (fw) p = p.substring("@android:".length());
            else if (p.startsWith("@")) p = p.substring(1);
            if (p.contains("/")) {
                String type = p.substring(0, p.indexOf('/'));
                String name = p.substring(p.indexOf('/') + 1);
                if (name.endsWith(".xml")) name = name.substring(0, name.length() - 4);
                List<File> bases = fw ? List.of(new File(platform, "data/res")) : resRoots;
                for (File base : bases) {
                    File direct = new File(base, type + "/" + name + ".xml");
                    if (direct.isFile()) return direct.getAbsolutePath();
                    File[] dirs = base.listFiles(f -> f.isDirectory() && f.getName().startsWith(type));
                    if (dirs == null) continue;
                    for (File d : dirs) {
                        File f = new File(d, name + ".xml");
                        if (f.isFile()) return f.getAbsolutePath();
                    }
                }
                return null;
            }
            File f = new File(p);
            if (f.isFile()) return f.getAbsolutePath();
            // A bare resource name (framework `@android:anim/linear_out_slow_in`).
            List<String> baseList = new ArrayList<>();
            baseList.add(platform + "/data/res");
            for (File r : resRoots) baseList.add(r.getAbsolutePath());
            for (String base : baseList) {
                for (String type : new String[] { "anim", "interpolator", "drawable", "color", "xml", "layout" }) {
                    File[] dirs = new File(base).listFiles(d -> d.isDirectory() && d.getName().startsWith(type));
                    if (dirs == null) continue;
                    for (File d : dirs) {
                        File cand = new File(d, p + ".xml");
                        if (cand.isFile()) return cand.getAbsolutePath();
                    }
                }
            }
            return null;
        }
        public ActionBarCallback getActionBarCallback() { return null; }
    }
}
