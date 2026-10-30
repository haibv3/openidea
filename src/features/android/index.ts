/**
 * Android support: the Logcat and Device Explorer tool windows, run / restart /
 * debug on device, and the emulator controls for the open project's Android
 * SDK. Registered only while an Android project with an installed SDK is open.
 */
import { registerActions } from "@/platform/actions";
import { setContext } from "@/platform/context";
import { useProject } from "@/platform/project";
import { registerToolWindow, toolWindowService } from "@/workbench/contributions";
import { LogcatView } from "./LogcatView";
import { DeviceExplorer } from "./DeviceExplorer";
import {
  clearLog,
  detectSdk,
  debugOnDevice,
  refreshDevices,
  resetAndroid,
  runOnDevice,
  startLogcat,
  stopLogcat,
  useAndroid,
} from "./store";

export function activate(): () => void {
  let toolWindow: (() => void) | null = null;
  let explorer: (() => void) | null = null;

  const sync = () => {
    const sdk = useAndroid.getState().sdk;
    setContext("android.project", !!sdk);
    if (sdk && !toolWindow) {
      toolWindow = registerToolWindow({
        id: "logcat",
        title: "Logcat",
        icon: "toolwindow-services",
        anchor: "bottom",
        stripe: "onUse",
        order: 27,
        component: LogcatView,
      });
    } else if (!sdk && toolWindow) {
      toolWindow();
      toolWindow = null;
    }
    if (sdk && !explorer) {
      explorer = registerToolWindow({
        id: "device-explorer",
        title: "Device Explorer",
        icon: "folder-open",
        anchor: "right",
        order: 28,
        component: DeviceExplorer,
      });
    } else if (!sdk && explorer) {
      explorer();
      explorer = null;
    }
  };

  const detect = () => {
    void detectSdk().then(sync);
  };

  const disposers: Array<() => void> = [
    registerActions([
      {
        id: "android.runOnDevice",
        title: "Run on Device",
        description: "Build the selected variant, install it and start the app on the selected device",
        category: "Android",
        icon: "run",
        when: "projectOpen && android.project",
        keywords: ["android", "run", "install", "device", "apk", "gradle"],
        run: () => runOnDevice(),
      },
      {
        id: "android.debugOnDevice",
        title: "Debug on Device",
        description: "Forward the app's JDWP socket and attach the Java debugger to the running app",
        category: "Android",
        icon: "debug",
        when: "projectOpen && android.project",
        keywords: ["android", "debug", "jdwp", "adb", "device"],
        run: () => debugOnDevice(),
      },
      {
        id: "android.deviceExplorer",
        title: "Device File Explorer",
        description: "Browse the selected device's file system, download and upload files",
        category: "Android",
        icon: "folder-open",
        when: "projectOpen && android.project",
        keywords: ["adb", "device", "files", "sdcard", "android"],
        run: () => toolWindowService().show("device-explorer", true),
      },
      {
        id: "android.refreshDevices",
        title: "Refresh Android Devices",
        description: "Re-read the devices and emulators attached to the project's Android SDK",
        category: "Android",
        icon: "refresh",
        when: "projectOpen && android.project",
        keywords: ["adb", "device", "emulator", "android"],
        run: () => refreshDevices(),
      },
      {
        id: "android.startLogcat",
        title: "Start Logcat",
        description: "Stream the selected device's log into the Logcat tool window",
        category: "Android",
        icon: "run",
        when: "projectOpen && android.project",
        keywords: ["logcat", "adb", "log", "android"],
        run: () => startLogcat(),
      },
      {
        id: "android.stopLogcat",
        title: "Stop Logcat",
        description: "Stop streaming the device's log",
        category: "Android",
        icon: "stop",
        when: "projectOpen && android.project",
        keywords: ["logcat", "adb", "log", "android"],
        run: () => stopLogcat(),
      },
      {
        id: "android.clearLogcat",
        title: "Clear Logcat",
        description: "Remove the entries shown in the Logcat tool window",
        category: "Android",
        icon: "trash",
        when: "projectOpen && android.project",
        keywords: ["logcat", "clear", "android"],
        run: () => clearLog(),
      },
    ]),
    useProject.subscribe((state, previous) => {
      if (state.project?.root === previous.project?.root) return;
      resetAndroid();
      sync();
      if (state.project) detect();
    }),
  ];

  if (useProject.getState().project) detect();

  return () => {
    disposers.reverse().forEach((dispose) => dispose());
    if (toolWindow) toolWindow();
    toolWindow = null;
    if (explorer) explorer();
    explorer = null;
    resetAndroid();
    setContext("android.project", false);
  };
}
