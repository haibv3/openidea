/**
 * The device side of the Device File Explorer: paths and sizes are what the
 * view needs to show; the listing itself comes from adb (see `store.ts`).
 */
import type { DeviceEntry } from "@/platform/ipc";

/** Directories first, then links and files, each alphabetically. */
export function sortEntries(entries: readonly DeviceEntry[]): DeviceEntry[] {
  const rank = (e: DeviceEntry) => (e.kind === "dir" ? 0 : e.kind === "link" ? 1 : 2);
  return entries
    .slice()
    .sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

/** `/sdcard/Download`, never `/` or `//`. */
export function devicePath(parent: string, name: string): string {
  return `${parent.endsWith("/") ? parent.slice(0, -1) : parent}/${name}`;
}

/** The directory above `path`; `/` has none. */
export function parentOf(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed || trimmed === "/") return null;
  const at = trimmed.lastIndexOf("/");
  return at <= 0 ? "/" : trimmed.slice(0, at);
}

/** `1.4 MB`, `512 KB`, `0 B` — what the size column shows. */
export function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const text = value >= 10 || value === Math.round(value) ? Math.round(value).toString() : value.toFixed(1);
  return `${text} ${units[unit]}`;
}

/** Where an explorer starts: the shared storage first, then the usual places. */
export const COMMON_ROOTS = ["/sdcard", "/data/local/tmp", "/storage", "/data/data", "/"] as const;

/** The parts of `/sdcard/Download` for the breadcrumb. */
export function pathParts(path: string): { name: string; path: string }[] {
  const out = [{ name: "/", path: "/" }];
  let current = "";
  for (const part of path.split("/").filter(Boolean)) {
    current += `/${part}`;
    out.push({ name: part, path: current });
  }
  return out;
}
