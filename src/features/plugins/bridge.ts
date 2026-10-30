/**
 * Glue between backend plugins and the workbench: plugin commands become
 * actions, editor requests are applied to the active editor, the selected
 * theme becomes a stylesheet of CSS variables, notifications become balloons.
 */
import { EditorSelection } from "@codemirror/state";
import type { ActiveTheme, PluginEvent, PluginInfo } from "@/bindings";
import { activeEditor, useEditorRegistry } from "@/editor/api";
import { registerAction, type Action } from "@/platform/actions";
import { notifications, registerToolWindow } from "@/workbench/contributions";
import { applyThemeAttributes, currentThemeId, syncWindowBackground, windowBackgroundHex } from "@/workbench/theme";
import { pluginsBackend } from "./backend";
import {
  PluginWindowView,
  forgetPluginWindow,
  pluginWindowId,
  rememberPluginWindow,
  type WindowSpec,
} from "./PluginWindowView";
import { refreshPlugins, runPluginCommand, usePlugins } from "./store";
import { tx } from "@/platform/i18n";

export const pluginActionId = (pluginId: string, commandId: string) => `plugin.${pluginId}.${commandId}`;

/** Actions for every command of every running plugin. */
export function pluginActions(plugins: readonly PluginInfo[]): Action[] {
  return plugins
    .filter((p) => p.status === "active")
    .flatMap((p) =>
      p.commands.map((c): Action => ({
        id: pluginActionId(p.id, c.id),
        title: c.title,
        category: c.category ?? p.name,
        icon: "package",
        keywords: ["plugin", p.name, p.id],
        description: tx`From plugin ${p.name}`,
        run: async ({ arg }) => {
          await runPluginCommand(p.id, c.id, typeof arg === "string" ? arg : null);
        },
      })),
    );
}

/** Keeps the action registry in sync with the plugin list; re-registers only changed actions. */
export function syncPluginActions(): () => void {
  const registered = new Map<string, { key: string; dispose: () => void }>();
  const apply = (plugins: readonly PluginInfo[]) => {
    const next = new Map(pluginActions(plugins).map((a) => [a.id, a]));
    for (const [id, entry] of registered) {
      if (!next.has(id)) {
        entry.dispose();
        registered.delete(id);
      }
    }
    for (const [id, action] of next) {
      const key = `${action.title}\u0000${action.category}`;
      const prev = registered.get(id);
      if (prev?.key === key) continue;
      prev?.dispose();
      registered.set(id, { key, dispose: registerAction(action) });
    }
  };
  apply(usePlugins.getState().plugins);
  const unsubscribe = usePlugins.subscribe((s, prev) => {
    if (s.plugins !== prev.plugins) apply(s.plugins);
  });
  return () => {
    unsubscribe();
    registered.forEach((e) => e.dispose());
    registered.clear();
  };
}

// --- tool windows -----------------------------------------------------------

/**
 * Docks a tool window per window contributed by an enabled plugin. Windows of
 * disabled or failed plugins disappear with them.
 */
export function syncPluginToolWindows(): () => void {
  const registered = new Map<string, { title: string; spec: WindowSpec; dispose: () => void }>();
  const wanted = (plugins: readonly PluginInfo[]) => {
    const out = new Map<string, { title: string; spec: WindowSpec }>();
    for (const p of plugins) {
      if (p.status !== "active" && p.status !== "static") continue;
      for (const w of p.toolWindows) {
        out.set(pluginWindowId(p.id, w.id), {
          title: w.title,
          spec: { pluginId: p.id, pluginName: p.name, window: w },
        });
      }
    }
    return out;
  };
  const apply = (plugins: readonly PluginInfo[]) => {
    const next = wanted(plugins);
    for (const [id, entry] of registered) {
      if (!next.has(id)) {
        entry.dispose();
        forgetPluginWindow(entry.spec);
        registered.delete(id);
      }
    }
    for (const [id, { title, spec }] of next) {
      // Re-register only when the title or anchor changed: the workbench picks
      // the window up from the contribution map.
      const anchor = spec.window.anchor;
      if (registered.get(id)?.title === `${title}/${anchor}`) continue;
      registered.get(id)?.dispose();
      rememberPluginWindow(spec.pluginId, spec.pluginName, spec.window);
      const dispose = registerToolWindow({
        id,
        title,
        icon: spec.window.icon ?? "plugin",
        anchor,
        component: PluginWindowView,
      });
      registered.set(id, { title: `${title}/${anchor}`, spec, dispose });
    }
  };
  apply(usePlugins.getState().plugins);
  const unsubscribe = usePlugins.subscribe((s, prev) => {
    if (s.plugins !== prev.plugins) apply(s.plugins);
  });
  return () => {
    unsubscribe();
    registered.forEach((e) => {
      e.dispose();
      forgetPluginWindow(e.spec);
    });
    registered.clear();
  };
}

// --- theme ------------------------------------------------------------------

export const PLUGIN_THEME_STYLE_ID = "openidea-plugin-theme";

/** Stylesheet for a plugin theme. Its selector outranks every built-in theme block. */
export function pluginThemeCss(theme: ActiveTheme["theme"]): string {
  const kind = theme.kind === "light" ? "light" : "dark";
  const decls = [`  color-scheme: ${kind};`];
  for (const [name, value] of Object.entries(theme.tokens)) {
    // Backend validates; keep the guard so a malformed payload can't inject declarations.
    if (!/^--[a-z0-9-]+$/.test(name) || value == null || /[;{}<>\\]/.test(value)) continue;
    decls.push(`  ${name}: ${value};`);
  }
  return `:root[data-plugin-theme]:not([data-plugin-theme=""]) {\n${decls.join("\n")}\n}\n`;
}

/**
 * Applies a plugin theme (null restores the built-in one): its tokens as a
 * stylesheet over the built-in light or dark base named by its `kind`. A
 * stylesheet rather than inline styles, so the accent picked in Settings ›
 * Appearance and the editor colour scheme still layer on top.
 */
export function applyPluginTheme(theme: ActiveTheme | null, root: HTMLElement = document.documentElement) {
  const doc = root.ownerDocument;
  doc.getElementById(PLUGIN_THEME_STYLE_ID)?.remove();
  const had = root.dataset.pluginTheme !== undefined;
  if (!theme) {
    delete root.dataset.pluginTheme;
    delete root.dataset.pluginThemeKind;
    if (had) applyThemeAttributes(currentThemeId(), root);
  } else {
    const style = doc.createElement("style");
    style.id = PLUGIN_THEME_STYLE_ID;
    style.textContent = pluginThemeCss(theme.theme);
    doc.head.appendChild(style);
    root.dataset.pluginTheme = `${theme.pluginId}/${theme.theme.id}`;
    root.dataset.pluginThemeKind = theme.theme.kind === "light" ? "light" : "dark";
    applyThemeAttributes(currentThemeId(), root);
  }
  if (had || theme) syncWindowBackground();
}

/** localStorage key read by public/boot.js: the selected theme's stylesheet, applied before the app loads. */
export const PLUGIN_THEME_CACHE_KEY = "openidea.pluginTheme";

export interface CachedPluginTheme {
  /** `data-plugin-theme`: `<pluginId>/<themeId>`. */
  id: string;
  kind: "light" | "dark";
  css: string;
}

function cachePluginTheme(theme: ActiveTheme | null) {
  try {
    if (!theme) {
      localStorage.removeItem(PLUGIN_THEME_CACHE_KEY);
      return;
    }
    const cached: CachedPluginTheme = {
      id: `${theme.pluginId}/${theme.theme.id}`,
      kind: theme.theme.kind === "light" ? "light" : "dark",
      css: pluginThemeCss(theme.theme),
    };
    localStorage.setItem(PLUGIN_THEME_CACHE_KEY, JSON.stringify(cached));
  } catch {
    /* storage unavailable */
  }
}

let reportedFrame: string | undefined;

/**
 * Tells the backend the theme's resolved frame colour, which new windows open
 * in. Selecting a theme clears it there, so it is sent once per selection.
 */
function reportThemeFrame(theme: ActiveTheme | null) {
  const frame = theme ? windowBackgroundHex() : null;
  if (!theme || !frame) {
    reportedFrame = undefined;
    return;
  }
  const key = `${theme.pluginId}/${theme.theme.id}|${frame}`;
  if (key === reportedFrame) return;
  reportedFrame = key;
  void pluginsBackend()
    .setThemeFrame(frame)
    .catch(() => (reportedFrame = undefined));
}

/**
 * Applies the selected plugin theme and keeps it for the next start-up. Until
 * the first plugin list arrives, the theme public/boot.js restored stays.
 */
export function trackPluginTheme(): () => void {
  const sync = (s: ReturnType<typeof usePlugins.getState>) => {
    if (!s.activeTheme && (!s.loaded || s.error)) return;
    applyPluginTheme(s.activeTheme);
    cachePluginTheme(s.activeTheme);
    reportThemeFrame(s.activeTheme);
  };
  sync(usePlugins.getState());
  const unsubscribe = usePlugins.subscribe((s, prev) => {
    if (s.activeTheme !== prev.activeTheme || s.loaded !== prev.loaded) sync(s);
  });
  return () => {
    unsubscribe();
    applyPluginTheme(null);
  };
}

// --- editor -----------------------------------------------------------------

/** Inserts (`select: false`) or replaces-and-selects (`select: true`) at every selection range. */
export function applyTextToActiveEditor(text: string, select: boolean, pluginId: string): boolean {
  const editor = activeEditor();
  const name = usePlugins.getState().plugins.find((p) => p.id === pluginId)?.name ?? pluginId;
  if (!editor || editor.ctx.readOnly) {
    notifications().notify({
      group: "plugins",
      level: "warning",
      title: `${name}: ${editor ? "the editor is read-only" : "no editor to insert into"}`,
    });
    return false;
  }
  const { view } = editor;
  view.dispatch(
    view.state.changeByRange((range) => ({
      changes: { from: range.from, to: range.to, insert: text },
      range: select
        ? EditorSelection.range(range.from, range.from + text.length)
        : EditorSelection.cursor(range.from + text.length),
    })),
    { scrollIntoView: true, userEvent: "input.plugin" },
  );
  view.focus();
  return true;
}

/** Reports the focused editor's file to the backend (`active-file` for plugins). */
function trackActiveFile(): () => void {
  let last: string | null | undefined;
  const report = (path: string | null) => {
    if (path === last) return;
    last = path;
    void pluginsBackend()
      .setActiveFile(path)
      .catch(() => {});
  };
  report(useEditorRegistry.getState().active?.ctx.path ?? null);
  return useEditorRegistry.subscribe((s) => report(s.active?.ctx.path ?? null));
}

// --- events -----------------------------------------------------------------

export function handlePluginEvent(e: PluginEvent) {
  switch (e.type) {
    case "commandsChanged":
      scheduleRefresh();
      break;
    case "notification": {
      const name = usePlugins.getState().plugins.find((p) => p.id === e.pluginId)?.name ?? e.pluginId;
      notifications().notify({ group: "plugins", level: e.level, title: e.title || name, body: e.body || undefined });
      break;
    }
    case "insertText":
      applyTextToActiveEditor(e.text, false, e.pluginId);
      break;
    case "replaceSelection":
      applyTextToActiveEditor(e.text, true, e.pluginId);
      break;
    case "themeChanged":
      usePlugins.setState({ activeTheme: e.theme });
      break;
  }
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void refreshPlugins();
  }, 30);
}

export function startPluginBridge(): () => void {
  const disposers = [
    pluginsBackend().onEvent(handlePluginEvent),
    syncPluginActions(),
    syncPluginToolWindows(),
    trackPluginTheme(),
    trackActiveFile(),
  ];
  return () => {
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = null;
    disposers.reverse().forEach((d) => d());
  };
}
