/**
 * Plugins: Settings › Plugins (installed list, enable/disable, install from
 * disk, uninstall, theme selection) and the runtime bridge to the WASM plugin
 * host (`ide-plugin`).
 *
 * Actions: `plugins.settings`, `plugins.installFromDisk`, `plugins.reloadAll`,
 * plus one action per plugin command: `plugin.<pluginId>.<commandId>`
 * (searchable in Search Everywhere › Actions and bindable in the keymap editor;
 * a string arg is forwarded to the plugin).
 */
import { lazyComponent } from "@/ui";
import { executeAction, registerActions, type Action } from "@/platform/actions";
import { getContext } from "@/platform/context";
import { useProject } from "@/platform/project";
import { registerSettingsPage, registerTextEditorWrapper } from "@/workbench/contributions";
import { startPluginBridge, trackPluginTheme } from "./bridge";
import { registerPluginPreviewLiveSource } from "./preview/live";
import { findPreviewProvider } from "./preview/match";
import { setPluginPreviewMode, type PluginPreviewMode } from "./preview/modes";
import { installFromDisk, refreshPlugins, reloadPlugin, SETTINGS_PAGE_ID, usePlugins } from "./store";

function registerPluginsPage(): () => void {
  return registerSettingsPage({
    id: SETTINGS_PAGE_ID,
    title: "Plugins",
    // After Editor, as in IntelliJ IDEA.
    order: 250,
    keywords: ["plugin", "extension", "wasm", "install", "theme", "marketplace"],
    component: lazyComponent(() => import("./PluginsPage").then((m) => m.PluginsPage)),
  });
}

/** Settings › Plugins for the Settings window: the page plus the plugin theme. */
export function activateSettingsPages(): () => void {
  const disposers = [registerPluginsPage(), trackPluginTheme()];
  void refreshPlugins();
  return () => disposers.reverse().forEach((d) => d());
}

/** Layout actions for files a plugin preview covers (mirrors the Markdown modes). */
function previewModeActions(): Action[] {
  const mode = (id: string, title: string, m: PluginPreviewMode, icon: string): Action => ({
    id,
    title,
    category: "Preview",
    icon,
    keywords: ["plugin", "preview", "layout"],
    when: "textEditorActive",
    run: ({ arg }) => {
      const path = typeof arg === "string" ? arg : getContext().editorPath;
      if (typeof path === "string") setPluginPreviewMode(path, m);
    },
  });
  return [
    mode("pluginPreview.editorOnly", "Editor Only", "editor", "edit"),
    mode("pluginPreview.editorAndPreview", "Editor and Preview", "split", "split"),
    mode("pluginPreview.previewOnly", "Preview Only", "preview", "eye"),
  ];
}

const previewProviderCovers = (path: string) =>
  findPreviewProvider(path, usePlugins.getState().plugins, useProject.getState().project?.root ?? null) !== null;

export function activate(): () => void {
  const disposers = [
    registerPluginsPage(),
    registerActions([
      {
        id: "plugins.settings",
        title: "Plugins…",
        category: "Plugins",
        icon: "plugin",
        keywords: ["extensions", "manage plugins"],
        run: () => void executeAction("file.settings", SETTINGS_PAGE_ID),
      },
      {
        id: "plugins.installFromDisk",
        title: "Install Plugin from Disk…",
        category: "Plugins",
        icon: "plugin",
        keywords: ["extension", "wasm", "plugin.toml"],
        run: async ({ arg }) => {
          await installFromDisk(typeof arg === "string" ? arg : undefined);
        },
      },
      {
        id: "plugins.reloadAll",
        title: "Reload Plugins",
        category: "Plugins",
        icon: "refresh",
        keywords: ["dev plugin", "rebuild"],
        run: async () => {
          const ids = usePlugins.getState().plugins.map((p) => p.id);
          for (const id of ids) await reloadPlugin(id);
        },
      },
    ]),
    startPluginBridge(),
    registerTextEditorWrapper({
      id: "plugins.preview",
      matches: previewProviderCovers,
      component: lazyComponent(() => import("./preview/PluginPreviewEditor").then((m) => m.PluginPreviewEditor)),
    }),
    registerPluginPreviewLiveSource(),
    registerActions([...previewModeActions()]),
    // Dev plugins (`<project>/.openidea/plugins`) depend on the open project.
    useProject.subscribe((s, prev) => {
      if (s.project?.root !== prev.project?.root) void refreshPlugins();
    }),
  ];
  void refreshPlugins();
  return () => disposers.reverse().forEach((d) => d());
}
