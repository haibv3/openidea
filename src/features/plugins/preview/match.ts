/**
 * Which plugin (and preview declaration) renders a file's preview: extension
 * and path-glob matching with the same rules as the Rust manifest validator
 * (`crates/ide-plugin`) — keep both sides in step. Globs match the path
 * relative to the project root with `/` separators; `**` spans any number of
 * directories (also none), `*`/`?` stay within one segment. Matching is
 * case-sensitive except the extension comparison.
 */
import type { PluginInfo, PluginPreviewDecl } from "@/bindings";

/** The plugin and `contributes.previews` entry that renders a file's preview. */
export interface PreviewProvider {
  pluginId: string;
  pluginName: string;
  /** Shown label; the plugin name. */
  pluginTitle: string;
  /** `contributes.previews[].id` of the matching declaration. */
  declId: string;
  /** `contributes.previews[].title` of the matching declaration. */
  title: string;
}

const globCache = new Map<string, RegExp>();
const GLOB_CACHE_LIMIT = 128;

/**
 * Full-path glob match: `**` spans directories (also none), `*` matches any
 * characters within one segment, `?` one character. Case-sensitive.
 */
export function pathMatchesGlob(path: string, glob: string): boolean {
  const segments = glob.replace(/\\/g, "/").split("/");
  let re = globCache.get(glob);
  if (!re) {
    let pattern = "^";
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i]!;
      if (seg === "**") {
        // Zero or more whole segments; everything left when last.
        pattern += i === segments.length - 1 ? ".*" : "(?:[^/]+/)*";
      } else {
        pattern += seg.replace(/[.*?+^${}()|[\]\\]/g, (c) =>
          c === "*" ? "[^/]*" : c === "?" ? "[^/]" : `\\${c}`,
        );
        if (i < segments.length - 1) pattern += "/";
      }
    }
    re = new RegExp(pattern + "$");
    if (globCache.size >= GLOB_CACHE_LIMIT) globCache.delete(globCache.keys().next().value!);
    globCache.set(glob, re);
  }
  return re.test(path.replace(/\\/g, "/"));
}

/** Lowercased text after the last dot of the file name (`layout.xml` → `xml`). */
function extensionOf(path: string): string {
  const dot = path.lastIndexOf(".");
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return dot > slash ? path.slice(dot + 1).toLowerCase() : "";
}

function matchesDecl(path: string, candidates: readonly string[], decl: PluginPreviewDecl): boolean {
  // Both lists may be empty: an empty one imposes no constraint.
  if (decl.extensions.length > 0) {
    const ext = extensionOf(path);
    if (!decl.extensions.some((e) => e.toLowerCase() === ext)) return false;
  }
  if (
    decl.pathGlobs.length > 0 &&
    !decl.pathGlobs.some((g) => candidates.some((c) => pathMatchesGlob(c, g)))
  ) {
    return false;
  }
  return true;
}

/**
 * The first active plugin whose preview declaration covers `path`, or null.
 * Globs match the project-relative path when `projectRoot` is known, falling
 * back to the full path (so a leading `**` glob segment works with or without
 * a root).
 */
export function findPreviewProvider(
  path: string,
  plugins: readonly PluginInfo[],
  projectRoot?: string | null,
): PreviewProvider | null {
  const full = path.replace(/\\/g, "/");
  const root = projectRoot ? projectRoot.replace(/\\/g, "/").replace(/\/+$/, "") : null;
  const relative = root !== null && full.startsWith(root + "/") ? full.slice(root.length + 1) : null;
  const candidates = relative !== null ? [relative, full] : [full];
  for (const p of plugins) {
    if (p.status !== "active" || p.previews.length === 0) continue;
    for (const decl of p.previews) {
      if (matchesDecl(full, candidates, decl)) {
        return { pluginId: p.id, pluginName: p.name, pluginTitle: p.name, declId: decl.id, title: decl.title };
      }
    }
  }
  return null;
}
