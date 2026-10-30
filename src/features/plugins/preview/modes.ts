/** Plugin preview layout per file: Editor only / Editor and Preview / Preview only (Android Studio's layout editor). */
import { create } from "zustand";

export type PluginPreviewMode = "editor" | "split" | "preview";

const DEFAULT_KEY = "openidea.pluginPreview.defaultMode";
const MODES: readonly PluginPreviewMode[] = ["editor", "split", "preview"];

function storedDefault(): PluginPreviewMode {
  try {
    const v = localStorage.getItem(DEFAULT_KEY);
    return MODES.includes(v as PluginPreviewMode) ? (v as PluginPreviewMode) : "split";
  } catch {
    return "split";
  }
}

interface ModesState {
  byPath: Record<string, PluginPreviewMode>;
  /** Mode of files not switched yet: the last one chosen. */
  fallback: PluginPreviewMode;
}

export const usePluginPreviewModes = create<ModesState>(() => ({ byPath: {}, fallback: storedDefault() }));

export const modeOf = (s: ModesState, path: string): PluginPreviewMode => s.byPath[path] ?? s.fallback;

export function setPluginPreviewMode(path: string, mode: PluginPreviewMode) {
  usePluginPreviewModes.setState((s) => ({ byPath: { ...s.byPath, [path]: mode }, fallback: mode }));
  try {
    localStorage.setItem(DEFAULT_KEY, mode);
  } catch {
    // Per-viewer convenience only.
  }
}
