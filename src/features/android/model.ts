import type { AndroidModule, Device, LogEntry } from "@/platform/ipc";

export const LEVELS = ["V", "D", "I", "W", "E"] as const;

export type Level = (typeof LEVELS)[number];

/** `logcat` severity order; `S` (silent) ranks lowest. */
const RANK: Record<string, number> = { S: 0, V: 0, D: 1, I: 2, W: 3, E: 4, F: 5, A: 5 };

/** Entries kept in the window; older ones fall off the top. */
export const MAX_ENTRIES = 2000;

/** Build output lines kept in the deploy panel. */
export const MAX_DEPLOY_LINES = 200;

/** Entries at or above `minLevel` whose tag or message contains `text`. */
export function visibleEntries(entries: readonly LogEntry[], minLevel: Level, text: string): LogEntry[] {
  const min = RANK[minLevel] ?? 0;
  const needle = text.trim().toLowerCase();
  return entries.filter((entry) => {
    const rank = entry.level ? RANK[entry.level] : undefined;
    if (rank !== undefined && rank < min) return false;
    if (!needle) return true;
    return (entry.tag ?? "").toLowerCase().includes(needle) || entry.message.toLowerCase().includes(needle);
  });
}

/** `entries` plus `entry`, dropping the oldest past `limit`. */
export function appendEntry(entries: readonly LogEntry[], entry: LogEntry, limit = MAX_ENTRIES): LogEntry[] {
  const keep = entries.length >= limit ? entries.slice(entries.length - limit + 1) : entries.slice();
  keep.push(entry);
  return keep;
}

/** `lines` plus `line`, dropping the oldest past `limit`. */
export function appendLog(lines: readonly string[], line: string, limit = MAX_DEPLOY_LINES): string[] {
  const keep = lines.length >= limit ? lines.slice(lines.length - limit + 1) : lines.slice();
  keep.push(line);
  return keep;
}

/** Emulators are `emulator-<console port>`; a physical device has another serial. */
export function isEmulator(serial: string): boolean {
  return serial.startsWith("emulator-");
}

/** Serials of the emulators `devices` already has, booted or still booting. */
export function emulatorSerials(devices: readonly Device[]): string[] {
  return devices.filter((d) => isEmulator(d.serial)).map((d) => d.serial);
}

/**
 * The application id to filter logcat by: the one of the module being run, or
 * the first module that has one. Null when no module declares an id.
 */
export function applicationIdOf(modules: readonly AndroidModule[], dir: string): string | null {
  const wanted = modules.find((m) => m.dir === dir);
  return wanted?.applicationId ?? modules.find((m) => m.applicationId)?.applicationId ?? null;
}

/** Build types of the selected module; `debug` when it has none or is gone. */
export function variantsOf(modules: readonly AndroidModule[], dir: string): string[] {
  return modules.find((m) => m.dir === dir)?.variants ?? modules[0]?.variants ?? ["debug"];
}

/**
 * The serial of an emulator that finished booting and was not already running —
 * what a start was waiting for.
 */
export function bootedEmulator(devices: readonly Device[], known: Iterable<string>): string | null {
  const before = new Set(known);
  return devices.find((d) => isEmulator(d.serial) && d.state === "device" && !before.has(d.serial))?.serial ?? null;
}
