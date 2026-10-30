/**
 * Text editor wrapper of files a plugin preview covers: the editor, the
 * plugin's sandboxed HTML preview, or both side by side, switched from the
 * floating toolbar at the top right (Android Studio's layout preview). The
 * plugin renders the document text as HTML; the result is untrusted and only
 * ever shown inside a sandboxed iframe.
 */
import { useEffect, useMemo, useState, type ReactNode } from "react";
import clsx from "clsx";
import { useProject } from "@/platform/project";
import { tx } from "@/platform/i18n";
import { IconButton, selectableText } from "@/ui";
import { pluginsBackend } from "../backend";
import { errorText, usePlugins } from "../store";
import { usePluginPreviewSource } from "./live";
import { findPreviewProvider } from "./match";
import { modeOf, usePluginPreviewModes } from "./modes";
import styles from "./PluginPreview.module.css";

/** Debounce before re-rendering: the render call runs a WASM plugin. */
const RENDER_DELAY = 400;
/** Last successful render per path, so a remounted pane doesn't flash empty. */
const renderCache = new Map<string, { html: string; background: string | null }>();
const CACHE_LIMIT = 16;
/** Monotonic request counter per path; stale responses are dropped. */
const requestSeq = new Map<string, number>();

export function PluginPreviewEditor({ path, children }: { path: string; tabId: string; children: ReactNode }) {
  const mode = usePluginPreviewModes((s) => modeOf(s, path));
  const [refreshSeq, setRefreshSeq] = useState(0);
  return (
    <div className={clsx(styles.split, styles[`mode-${mode}`])}>
      <div className={styles.editor}>{children}</div>
      {mode !== "editor" && <PluginPreviewPane path={path} refreshSeq={refreshSeq} />}
      <div className={styles.modeBar} role="toolbar" aria-label={tx("Plugin preview layout")}>
        <IconButton
          icon="edit"
          size="small"
          action="pluginPreview.editorOnly"
          actionArg={path}
          pressed={mode === "editor"}
        />
        <IconButton
          icon="split"
          size="small"
          action="pluginPreview.editorAndPreview"
          actionArg={path}
          pressed={mode === "split"}
        />
        <IconButton
          icon="eye"
          size="small"
          action="pluginPreview.previewOnly"
          actionArg={path}
          pressed={mode === "preview"}
        />
        <IconButton icon="refresh" size="small" title={tx("Refresh Preview")} onClick={() => setRefreshSeq((n) => n + 1)} />
      </div>
    </div>
  );
}

function PluginPreviewPane({ path, refreshSeq }: { path: string; refreshSeq: number }) {
  const text = usePluginPreviewSource((s) => s[path]?.text ?? "");
  const plugins = usePlugins((s) => s.plugins);
  const root = useProject((s) => s.project?.root ?? null);
  const provider = useMemo(() => findPreviewProvider(path, plugins, root), [path, plugins, root]);

  // The text to render: debounced, because rendering executes the plugin.
  const [rendered, setRendered] = useState(text);
  useEffect(() => {
    const timer = setTimeout(() => setRendered(text), RENDER_DELAY);
    return () => clearTimeout(timer);
  }, [text]);

  const [result, setResult] = useState(() => renderCache.get(path) ?? null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!provider) return;
    const seq = (requestSeq.get(path) ?? 0) + 1;
    requestSeq.set(path, seq);
    let live = true;
    pluginsBackend()
      .renderPreview(provider.pluginId, provider.declId, path, rendered)
      .then(
        (r) => {
          if (!live || requestSeq.get(path) !== seq) return;
          if (renderCache.size >= CACHE_LIMIT) renderCache.delete(renderCache.keys().next().value!);
          renderCache.set(path, { html: r.html, background: r.background });
          setResult(r);
          setError(null);
        },
        (e: unknown) => {
          if (!live || requestSeq.get(path) !== seq) return;
          setError(errorText(e));
        },
      );
    return () => {
      live = false;
    };
  }, [path, provider, rendered, refreshSeq]);

  if (!provider) return null;
  const label = `${provider.title} — ${provider.pluginName}`;
  return (
    <div className={styles.preview}>
      {error ? (
        <div className={clsx(styles.message, selectableText)} role="alert">
          <p className={styles.messageTitle}>{label}</p>
          <p>{tx("The preview could not be rendered.")}</p>
          <p>{error}</p>
        </div>
      ) : result ? (
        <iframe
          className={styles.frame}
          srcDoc={result.html}
          sandbox="allow-scripts"
          title={label}
          aria-label={label}
          style={{ background: result.background ?? undefined }}
        />
      ) : (
        <div className={styles.frame} aria-hidden="true" />
      )}
    </div>
  );
}
