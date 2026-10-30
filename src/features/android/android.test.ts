import { describe, expect, it } from "vitest";
import type { AndroidModule, AndroidSdk, Device, DeviceEntry, LogEntry } from "@/platform/ipc";
import { devicePath, formatBytes, parentOf, pathParts, sortEntries } from "./explorer";
import {
  appendEntry,
  appendLog,
  applicationIdOf,
  bootedEmulator,
  emulatorSerials,
  isEmulator,
  MAX_DEPLOY_LINES,
  MAX_ENTRIES,
  variantsOf,
  visibleEntries,
} from "./model";

function entry(level: string | null, tag: string, message: string): LogEntry {
  return { raw: `${tag}: ${message}`, timestamp: null, pid: null, tid: null, level, tag, message };
}

describe("visibleEntries", () => {
  const entries = [
    entry("V", "Chatty", "beginning of main"),
    entry("D", "ActivityManager", "Start proc com.example"),
    entry("I", "MyApp", "Hello from Compose"),
    entry("W", "MyApp", "Slow frame"),
    entry("E", "MyApp", "Crash in MainActivity"),
  ];

  it("keeps entries at or above the minimum level", () => {
    expect(visibleEntries(entries, "W", "").map((e) => e.level)).toEqual(["W", "E"]);
    expect(visibleEntries(entries, "V", "")).toHaveLength(5);
  });

  it("matches tag and message case-insensitively", () => {
    expect(visibleEntries(entries, "V", "compose").map((e) => e.tag)).toEqual(["MyApp"]);
    expect(visibleEntries(entries, "V", "activitymanager")).toHaveLength(1);
  });

  it("combines the level and text filters", () => {
    expect(visibleEntries(entries, "E", "myapp").map((e) => e.message)).toEqual(["Crash in MainActivity"]);
    expect(visibleEntries(entries, "E", "compose")).toEqual([]);
  });
});

describe("appendEntry", () => {
  it("drops the oldest entries past the limit", () => {
    const entries = Array.from({ length: MAX_ENTRIES }, (_, i) => entry("I", "Tag", String(i)));
    const next = appendEntry(entries, entry("I", "Tag", "new"));
    expect(next).toHaveLength(MAX_ENTRIES);
    expect(next[0]!.message).toBe("1");
    expect(next.at(-1)!.message).toBe("new");
  });
});

describe("appendLog", () => {
  it("keeps the newest build lines", () => {
    const lines = Array.from({ length: MAX_DEPLOY_LINES }, (_, i) => `line ${i}`);
    const next = appendLog(lines, "installing");
    expect(next).toHaveLength(MAX_DEPLOY_LINES);
    expect(next[0]).toBe("line 1");
    expect(next.at(-1)).toBe("installing");
  });
});

describe("emulators", () => {
  const device = (serial: string, state: string): Device => ({
    serial,
    state,
    model: null,
    product: null,
    device: null,
    transportId: null,
    usb: null,
  });
  const devices = [
    device("R58M12345", "device"),
    device("emulator-5554", "offline"),
    device("emulator-5556", "device"),
  ];

  it("tells emulators and physical devices apart by serial", () => {
    expect(isEmulator("emulator-5554")).toBe(true);
    expect(isEmulator("R58M12345")).toBe(false);
    expect(emulatorSerials(devices)).toEqual(["emulator-5554", "emulator-5556"]);
  });

  it("reports only an emulator that booted after the start", () => {
    expect(bootedEmulator(devices, [])).toBe("emulator-5556");
    expect(bootedEmulator(devices, ["emulator-5556"])).toBeNull();
    expect(bootedEmulator([device("emulator-5554", "offline")], [])).toBeNull();
  });

  it("takes the application id of the running module", () => {
    const modules: AndroidModule[] = [
      { dir: "app", applicationId: "com.example.app", variants: ["debug"], path: "app" },
      { dir: "wear", applicationId: "com.example.wear", variants: ["debug"], path: "wear" },
      { dir: "lib", applicationId: null, variants: ["debug"], path: "lib" },
    ];
    expect(applicationIdOf(modules, "wear")).toBe("com.example.wear");
    expect(applicationIdOf(modules, "lib")).toBe("com.example.app");
    expect(applicationIdOf(modules, "missing")).toBe("com.example.app");
    expect(applicationIdOf([], "app")).toBeNull();
  });

  it("lists the build types of the selected module", () => {
    const modules: AndroidModule[] = [
      { dir: "app", applicationId: "com.example.app", variants: ["debug", "release"], path: "app" },
      { dir: "wear", applicationId: "com.example.wear", variants: ["debug"], path: "wear" },
    ];
    expect(variantsOf(modules, "app")).toEqual(["debug", "release"]);
    expect(variantsOf(modules, "wear")).toEqual(["debug"]);
    expect(variantsOf(modules, "gone")).toEqual(["debug", "release"]);
    expect(variantsOf([], "app")).toEqual(["debug"]);
  });
});

describe("selectModule", () => {
  it("follows the build types of the module it switches to", async () => {
    const { selectModule, useAndroid } = await import("./store");
    const modules: AndroidModule[] = [
      { dir: "app", applicationId: "com.example.app", variants: ["debug", "release"], path: "app" },
      { dir: "wear", applicationId: "com.example.wear", variants: ["wearDebug"], path: "wear" },
    ];
    useAndroid.setState({
      sdk: { modules } as unknown as AndroidSdk,
      module: "app",
      variant: "release",
      running: false,
    });
    selectModule("wear");
    expect(useAndroid.getState()).toMatchObject({ module: "wear", variant: "wearDebug" });
    selectModule("app");
    expect(useAndroid.getState()).toMatchObject({ module: "app", variant: "debug" });
    useAndroid.setState({ sdk: null, module: "", variant: "", running: false });
  });

  it("keeps a variant the new module also builds", async () => {
    const { selectModule, useAndroid } = await import("./store");
    const modules: AndroidModule[] = [
      { dir: "app", applicationId: null, variants: ["debug", "release"], path: "app" },
      { dir: "tv", applicationId: null, variants: ["debug", "release"], path: "tv" },
    ];
    useAndroid.setState({
      sdk: { modules } as unknown as AndroidSdk,
      module: "app",
      variant: "release",
      running: false,
    });
    selectModule("tv");
    expect(useAndroid.getState().variant).toBe("release");
    useAndroid.setState({ sdk: null, module: "", variant: "", running: false });
  });
});

describe("device file explorer", () => {
  const at = (name: string, kind: DeviceEntry["kind"], size = 0): DeviceEntry => ({
    name,
    kind,
    size,
    modified: null,
    owner: null,
    permissions: kind === "dir" ? "drwxr-xr-x" : "-rw-r--r--",
  });

  it("joins and walks device paths", () => {
    expect(devicePath("/sdcard", "Download")).toBe("/sdcard/Download");
    expect(devicePath("/", "sdcard")).toBe("/sdcard");
    expect(parentOf("/sdcard/Download")).toBe("/sdcard");
    expect(parentOf("/sdcard")).toBe("/");
    expect(parentOf("/")).toBeNull();
    expect(pathParts("/sdcard/Download").map((p) => p.path)).toEqual(["/", "/sdcard", "/sdcard/Download"]);
  });

  it("sizes what the list shows", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1024)).toBe("1 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(10_485_760)).toBe("10 MB");
    expect(formatBytes(1_500_000_000)).toBe("1.4 GB");
  });

  it("lists directories before files", () => {
    const sorted = sortEntries([at("zeta", "file"), at("Beta", "dir"), at("link", "link"), at("alpha", "file")]);
    expect(sorted.map((e) => e.name)).toEqual(["Beta", "link", "alpha", "zeta"]);
  });
});
