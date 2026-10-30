/**
 * A tool window contributed by a plugin (`[[contributes.tool-windows]]` in
 * `plugin.toml`): the text of a file shipped in its package, or the text one of
 * its commands returns. The workbench mounts it through `PluginWindowView`,
 * which looks the window up by its contribution id.
 */
import { useCallback, useEffect, useState } from "react";
import type { PluginToolWindow } from "@/bindings";
import { Button, EmptyState, IconButton, SourcePreview } from "@/ui";
import { tx } from "@/platform/i18n";
import { pluginsBackend } from "./backend";
import { runPluginCommand } from "./store";
import styles from "./PluginsPage.module.css";

export interface WindowSpec {
  pluginId: string;
  pluginName: string;
  window: PluginToolWindow;
}

const specs = new Map<string, WindowSpec>();

/** Contribution id of a plugin's window — the same prefix as its actions. */
export const pluginWindowId = (pluginId: string, windowId: string) => `plugin.${pluginId}.${windowId}`;

/** Remembers what a window shows; the returned token releases it again. */
export function rememberPluginWindow(pluginId: string, pluginName: string, window: PluginToolWindow): WindowSpec {
  const spec: WindowSpec = { pluginId, pluginName, window };
  specs.set(pluginWindowId(pluginId, window.id), spec);
  return spec;
}

export function forgetPluginWindow(spec: WindowSpec) {
  const id = pluginWindowId(spec.pluginId, spec.window.id);
  if (specs.get(id) === spec) specs.delete(id);
}

/** Stable loader: `SourcePreview` reloads whenever the function identity changes. */
const loadPackageText = (path: string) => {
  const at = path.indexOf("/");
  return pluginsBackend().toolWindowContent(path.slice(0, at), path.slice(at + 1));
};

export function PluginWindowView({ id }: { id: string }) {
  const spec = specs.get(id);
  const [version, setVersion] = useState(0);
  const [result, setResult] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const command = spec?.window.command ?? null;
  const pluginId = spec?.pluginId ?? "";

  const run = useCallback(async () => {
    if (!command) return;
    setRunning(true);
    // `runPluginCommand` reports failures itself and answers null.
    setResult(await runPluginCommand(pluginId, command));
    setRunning(false);
  }, [command, pluginId]);

  useEffect(() => {
    if (command) void run();
  }, [command, run]);

  if (!spec) {
    return (
      <EmptyState title={tx("No plugin window")}>{tx("The plugin that contributed this window is gone.")}</EmptyState>
    );
  }
  if (spec.window.content) {
    return (
      <div className={styles.window}>
        <div className={styles.windowBar}>
          <span className={styles.windowHint}>{tx`From plugin ${spec.pluginName}`}</span>
          <IconButton icon="refresh" size="small" title={tx("Reload File")} onClick={() => setVersion((v) => v + 1)} />
        </div>
        <SourcePreview
          path={`${spec.pluginId}/${spec.window.id}`}
          line={0}
          load={loadPackageText}
          version={version}
          className={styles.windowText}
        />
      </div>
    );
  }
  return (
    <div className={styles.window}>
      <div className={styles.windowBar}>
        <span className={styles.windowHint}>{tx`From plugin ${spec.pluginName}`}</span>
        <Button size="small" disabled={running} onClick={() => void run()}>
          {running ? tx("Running…") : tx("Run Again")}
        </Button>
      </div>
      <pre className={styles.windowText}>{result || tx("The command returned no text.")}</pre>
    </div>
  );
}
