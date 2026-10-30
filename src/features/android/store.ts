import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import { executeAction } from "@/platform/actions";
import type { AndroidSdk, Device, DeviceEntry, LogEntry, LogFilter, RequestId } from "@/platform/ipc";
import { call, commands, isCancelled, nextRequestId } from "@/platform/ipc";
import {
  appendEntry,
  appendLog,
  applicationIdOf,
  bootedEmulator,
  emulatorSerials,
  isEmulator,
  type Level,
  variantsOf,
} from "./model";
import { devicePath, parentOf, sortEntries } from "./explorer";

interface AndroidState {
  /** The project's SDK; `null` when the project is not an Android one or no SDK is installed. */
  sdk: AndroidSdk | null;
  detecting: boolean;
  devices: Device[];
  loading: boolean;
  serial: string | null;
  running: boolean;
  request: RequestId | null;
  entries: LogEntry[];
  level: Level;
  text: string;
  /** Only the app's own process in logcat (`--pid` of the module's application id). */
  appOnly: boolean;
  error: string | null;
  /** Application module being built (`app`, or the first one). */
  module: string;
  variant: string;
  deploying: boolean;
  deployRequest: RequestId | null;
  deployLog: string[];
  /** Virtual devices of the SDK's emulator installation. */
  avds: string[];
  avd: string;
  coldBoot: boolean;
  loadingAvds: boolean;
  /** An emulator is starting; the window waits for it to appear as a device. */
  booting: boolean;
  /** Device File Explorer: the directory of the selected device being shown. */
  explorerPath: string;
  explorerEntries: DeviceEntry[];
  explorerLoading: boolean;
  /** A pull or push is running. */
  transferring: boolean;
}

const initial = {
  sdk: null,
  detecting: false,
  devices: [],
  loading: false,
  serial: null,
  running: false,
  request: null,
  entries: [],
  level: "V" as Level,
  text: "",
  appOnly: false,
  error: null,
  module: "",
  variant: "debug",
  deploying: false,
  deployRequest: null,
  deployLog: [],
  avds: [] as string[],
  avd: "",
  coldBoot: false,
  loadingAvds: false,
  booting: false,
  explorerPath: "/sdcard",
  explorerEntries: [] as DeviceEntry[],
  explorerLoading: false,
  transferring: false,
};

export const useAndroid = create<AndroidState>(() => ({ ...initial }));

/** Bumped when the project changes, so late answers of the old project are dropped. */
let generation = 0;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Finds the Android SDK of the open project; devices are refreshed when one is found. */
export async function detectSdk(): Promise<AndroidSdk | null> {
  const gen = generation;
  useAndroid.setState({ detecting: true, error: null });
  try {
    const sdk = await call(commands.androidSdk());
    if (gen !== generation) return null;
    const found = sdk?.androidProject ? sdk : null;
    const module = found?.modules[0];
    useAndroid.setState({
      sdk: found,
      detecting: false,
      module: module?.dir ?? "",
      variant: module?.variants[0] ?? "debug",
    });
    if (found) {
      void refreshDevices();
      void refreshAvds();
    }
    return found;
  } catch (e) {
    if (gen !== generation) return null;
    useAndroid.setState({ detecting: false, error: message(e) });
    return null;
  }
}

export async function refreshDevices(): Promise<void> {
  const gen = generation;
  const adb = useAndroid.getState().sdk?.adb;
  if (!adb) return;
  useAndroid.setState({ loading: true });
  try {
    const devices = await call(commands.androidDevices(adb, nextRequestId()));
    if (gen !== generation) return;
    const current = useAndroid.getState().serial;
    const serial = devices.some((d) => d.serial === current)
      ? current
      : (devices.find((d) => d.state === "device")?.serial ?? null);
    useAndroid.setState({ devices, loading: false, serial });
  } catch (e) {
    if (gen !== generation) return;
    useAndroid.setState({ loading: false, error: message(e) });
  }
}

/** How often the window looks for a starting emulator, and how long it waits. */
const BOOT_POLL_MS = 2000;
const BOOT_TIMEOUT_MS = 180_000;

/** The virtual devices of the SDK's emulator installation. */
export async function refreshAvds(): Promise<void> {
  const gen = generation;
  const emulator = useAndroid.getState().sdk?.emulator;
  if (!emulator) return;
  useAndroid.setState({ loadingAvds: true });
  try {
    const avds = await call(commands.androidAvds(emulator, nextRequestId()));
    if (gen !== generation) return;
    const current = useAndroid.getState().avd;
    useAndroid.setState({ avds, loadingAvds: false, avd: avds.includes(current) ? current : (avds[0] ?? "") });
  } catch (e) {
    if (gen !== generation) return;
    useAndroid.setState({ loadingAvds: false, error: message(e) });
  }
}

export function selectAvd(avd: string) {
  useAndroid.setState({ avd });
}

export function setColdBoot(coldBoot: boolean) {
  useAndroid.setState({ coldBoot });
}

/** Starts the selected AVD and selects it once adb reports it as a device. */
export async function startEmulator(): Promise<void> {
  const { sdk, avd, booting } = useAndroid.getState();
  if (!sdk?.emulator || !avd || booting) return;
  const gen = generation;
  const known = new Set(emulatorSerials(useAndroid.getState().devices));
  useAndroid.setState({ booting: true, error: null });
  try {
    await call(commands.androidEmulatorStart(sdk.emulator, avd, useAndroid.getState().coldBoot));
  } catch (e) {
    if (gen === generation) useAndroid.setState({ booting: false, error: message(e) });
    return;
  }
  await waitForEmulator(gen, known);
}

/** Polls the device list until the new emulator has booted (or time runs out). */
async function waitForEmulator(gen: number, known: ReadonlySet<string>): Promise<void> {
  const deadline = Date.now() + BOOT_TIMEOUT_MS;
  while (gen === generation && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, BOOT_POLL_MS));
    await refreshDevices();
    if (gen !== generation) return;
    const serial = bootedEmulator(useAndroid.getState().devices, known);
    if (serial) {
      useAndroid.setState({ booting: false, serial });
      return;
    }
  }
  if (gen === generation) {
    useAndroid.setState({ booting: false, error: "The emulator did not show up in time." });
  }
}

/** `adb emu kill`: asks the selected emulator to shut down. */
export async function stopEmulator(): Promise<void> {
  const { sdk, serial } = useAndroid.getState();
  if (!sdk?.adb || !serial || !isEmulator(serial)) return;
  const gen = generation;
  try {
    await call(commands.androidEmulatorStop(sdk.adb, serial, nextRequestId()));
  } catch (e) {
    if (gen === generation) useAndroid.setState({ error: message(e) });
  }
  await refreshDevices();
}

export function selectSerial(serial: string) {
  if (useAndroid.getState().running) stopLogcat();
  if (useAndroid.getState().deploying) cancelDeploy();
  useAndroid.setState({ serial, explorerEntries: [] });
}

/**
 * Picks the application module to build and filter by. Its first build type
 * becomes the active variant, and a running logcat restarts so the app-only
 * filter follows the new module.
 */
export function selectModule(dir: string) {
  const { sdk, module, running, variant } = useAndroid.getState();
  if (dir === module) return;
  const variants = variantsOf(sdk?.modules ?? [], dir);
  useAndroid.setState({ module: dir, variant: variants.includes(variant) ? variant : (variants[0] ?? "debug") });
  if (running) {
    stopLogcat();
    void startLogcat();
  }
}

export function setLevel(level: Level) {
  const restart = useAndroid.getState().running;
  useAndroid.setState({ level });
  if (restart) {
    stopLogcat();
    void startLogcat();
  }
}

export function setText(text: string) {
  useAndroid.setState({ text });
}

/** Restricts (or opens) the running stream to the app's own process. */
export function setAppOnly(appOnly: boolean) {
  const restart = useAndroid.getState().running;
  useAndroid.setState({ appOnly });
  if (restart) {
    stopLogcat();
    void startLogcat();
  }
}

export function setVariant(variant: string) {
  useAndroid.setState({ variant });
}

/** Streams the selected device's logcat into the window until stopped. */
export function startLogcat(): Promise<void> {
  const { sdk, serial, level, running, appOnly, module } = useAndroid.getState();
  if (!sdk?.adb || !serial || running) return Promise.resolve();
  const request = nextRequestId();
  const channel = new Channel<LogEntry>();
  channel.onmessage = (entry) => {
    if (useAndroid.getState().request !== request) return;
    useAndroid.setState((s) => ({ entries: appendEntry(s.entries, entry) }));
  };
  useAndroid.setState({ running: true, request, entries: [], error: null });
  const filter: LogFilter = { package: appOnly ? applicationIdOf(sdk.modules, module) : null, level };
  return call(commands.androidLogcat(sdk.adb, serial, filter, request, channel))
    .then(
      () => undefined,
      (e) => {
        if (!isCancelled(e)) useAndroid.setState({ error: message(e) });
      },
    )
    .finally(() => {
      useAndroid.setState((s) => (s.request === request ? { running: false, request: null } : {}));
    });
}

export function stopLogcat() {
  const { request } = useAndroid.getState();
  useAndroid.setState({ running: false, request: null });
  if (request !== null) void commands.cancelRequest(request).catch(() => {});
}

export function clearLog() {
  useAndroid.setState({ entries: [] });
}

/** Builds the selected variant, installs it and starts the app on the selected device. */
export function runOnDevice(): Promise<void> {
  const { sdk, serial, module, variant, deploying } = useAndroid.getState();
  if (!sdk?.adb || !serial || deploying) return Promise.resolve();
  const gen = generation;
  const request = nextRequestId();
  const channel = new Channel<string>();
  channel.onmessage = (line) => {
    if (gen !== generation || useAndroid.getState().deployRequest !== request) return;
    useAndroid.setState((s) => ({ deployLog: appendLog(s.deployLog, line) }));
  };
  useAndroid.setState({ deploying: true, deployRequest: request, deployLog: [] });
  return call(commands.androidDeploy(sdk.adb, serial, module, variant, request, channel))
    .then(
      (result) => {
        if (gen !== generation) return;
        useAndroid.setState((s) =>
          s.deployRequest === request
            ? {
                deploying: false,
                deployRequest: null,
                deployLog: appendLog(
                  s.deployLog,
                  result.applicationId ? `Launched ${result.applicationId}` : "Installed",
                ),
              }
            : {},
        );
      },
      (e) => {
        if (gen === generation && !isCancelled(e)) useAndroid.setState({ error: message(e) });
      },
    )
    .finally(() => {
      useAndroid.setState((s) => (s.deployRequest === request ? { deploying: false, deployRequest: null } : {}));
    });
}

/**
 * Restarts the app of the selected module on the device (`am force-stop` then
 * launch). Without an application id — a project whose modules declare none —
 * there is nothing to restart.
 */
export async function restartApp(): Promise<void> {
  const { sdk, serial, module } = useAndroid.getState();
  const pkg = sdk ? applicationIdOf(sdk.modules, module) : null;
  if (!sdk?.adb || !serial || !pkg) return;
  try {
    await call(commands.androidRestart(sdk.adb, serial, pkg, null, nextRequestId()));
  } catch (e) {
    if (!isCancelled(e)) useAndroid.setState({ error: message(e) });
  }
}

export function cancelDeploy() {
  const { deployRequest } = useAndroid.getState();
  useAndroid.setState((s) => ({
    deploying: false,
    deployRequest: null,
    deployLog: deployRequest !== null ? appendLog(s.deployLog, "Cancelled.") : s.deployLog,
  }));
  if (deployRequest !== null) void commands.cancelRequest(deployRequest).catch(() => {});
}

export function clearDeploy() {
  useAndroid.setState({ deployLog: [] });
}

/**
 * Forwards the running app's JDWP socket to a local port (`adb forward tcp:0`)
 * and attaches the Java debugger to it — Android Studio's "Debug" on a device.
 */
export async function debugOnDevice(): Promise<void> {
  const { sdk, serial, module } = useAndroid.getState();
  const pkg = sdk ? applicationIdOf(sdk.modules, module) : null;
  if (!sdk?.adb || !serial || !pkg) return;
  const gen = generation;
  useAndroid.setState({ error: null });
  try {
    const port = await call(commands.androidJdwpForward(sdk.adb, serial, pkg, nextRequestId()));
    if (gen !== generation) return;
    await executeAction("debug.attach", { name: `${pkg} (${serial})`, module, hostName: "127.0.0.1", port });
  } catch (e) {
    if (gen === generation && !isCancelled(e)) useAndroid.setState({ error: message(e) });
  }
}

// --- device file explorer ---------------------------------------------------

/** `adb shell ls -l <path>` of the selected device. */
export async function browseDevice(path: string): Promise<void> {
  const { sdk, serial } = useAndroid.getState();
  if (!sdk?.adb || !serial) return;
  const gen = generation;
  useAndroid.setState({ explorerPath: path, explorerEntries: [], explorerLoading: true, error: null });
  try {
    const entries = await call(commands.androidLs(sdk.adb, serial, path, nextRequestId()));
    if (gen !== generation) return;
    useAndroid.setState({ explorerEntries: sortEntries(entries), explorerLoading: false });
  } catch (e) {
    if (gen !== generation) return;
    useAndroid.setState({ explorerLoading: false, error: message(e) });
  }
}

/** Enters a directory; a file or symlink is opened by the download action instead. */
export function openDeviceEntry(entry: DeviceEntry) {
  if (entry.kind !== "dir") return;
  const { explorerPath } = useAndroid.getState();
  void browseDevice(devicePath(explorerPath, entry.name));
}

/** One directory up; does nothing at the root. */
export function deviceUp(): void {
  const { explorerPath } = useAndroid.getState();
  const parent = parentOf(explorerPath);
  if (parent) void browseDevice(parent);
}

/** `adb pull` of one file, after asking where to save it. */
export async function downloadDeviceFile(entry: DeviceEntry): Promise<void> {
  const { sdk, serial, explorerPath } = useAndroid.getState();
  if (!sdk?.adb || !serial || entry.kind === "dir") return;
  const remote = devicePath(explorerPath, entry.name);
  const { save } = await import("@tauri-apps/plugin-dialog");
  const local = await save({ title: "Save From Device", defaultPath: entry.name });
  if (!local) return;
  const gen = generation;
  useAndroid.setState({ transferring: true, error: null });
  try {
    await call(commands.androidPull(sdk.adb, serial, remote, local, nextRequestId()));
  } catch (e) {
    if (gen === generation && !isCancelled(e)) useAndroid.setState({ error: message(e) });
  } finally {
    if (gen === generation) useAndroid.setState({ transferring: false });
  }
}

/** `adb push` of a local file into the directory being shown. */
export async function uploadToDevice(): Promise<void> {
  const { sdk, serial, explorerPath } = useAndroid.getState();
  if (!sdk?.adb || !serial) return;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const local = await open({ title: "Upload to Device", multiple: false, directory: false });
  if (!local) return;
  const name = local.split(/[\\/]/).pop() ?? local;
  const gen = generation;
  useAndroid.setState({ transferring: true, error: null });
  try {
    await call(commands.androidPush(sdk.adb, serial, local, devicePath(explorerPath, name), nextRequestId()));
    if (gen === generation) await browseDevice(explorerPath);
  } catch (e) {
    if (gen === generation && !isCancelled(e)) useAndroid.setState({ error: message(e) });
  } finally {
    if (gen === generation) useAndroid.setState({ transferring: false });
  }
}

export function resetAndroid() {
  generation++;
  if (useAndroid.getState().deploying) cancelDeploy();
  stopLogcat();
  useAndroid.setState({ ...initial });
}
