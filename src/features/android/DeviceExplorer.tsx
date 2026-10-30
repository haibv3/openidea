/**
 * Device File Explorer: the file system of the selected device or emulator,
 * one `adb shell ls -l` at a time. Files can be downloaded to the computer and
 * local files uploaded into the directory being shown.
 */
import { useEffect, useRef, useState } from "react";
import { Button, EmptyState, IconButton, Input, Select, Spinner } from "@/ui";
import { tx } from "@/platform/i18n";
import { COMMON_ROOTS, formatBytes, pathParts } from "./explorer";
import { browseDevice, deviceUp, downloadDeviceFile, openDeviceEntry, uploadToDevice, useAndroid } from "./store";
import styles from "./Android.module.css";

/** The editable path field; `DeviceExplorer` remounts it per directory, so it starts from the path shown. */
function PathField({ disabled }: { disabled: boolean }) {
  const path = useAndroid((s) => s.explorerPath);
  const [draft, setDraft] = useState(path);
  const go = () => {
    const next = draft.trim();
    if (next.startsWith("/") && next !== path) void browseDevice(next);
  };
  return (
    <Input
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && go()}
      onBlur={go}
      placeholder="/sdcard/Download"
      aria-label={tx("Device path")}
      disabled={disabled}
      className={styles.path}
    />
  );
}

export function DeviceExplorer() {
  const sdk = useAndroid((s) => s.sdk);
  const serial = useAndroid((s) => s.serial);
  const path = useAndroid((s) => s.explorerPath);
  const entries = useAndroid((s) => s.explorerEntries);
  const loading = useAndroid((s) => s.explorerLoading);
  const transferring = useAndroid((s) => s.transferring);
  const error = useAndroid((s) => s.error);
  const loadedFor = useRef<string | null>(null);

  // The window lists the current directory when it opens and when the device changes.
  useEffect(() => {
    if (!sdk?.adb || !serial || loadedFor.current === serial) return;
    loadedFor.current = serial;
    void browseDevice(useAndroid.getState().explorerPath);
  }, [sdk, serial]);

  if (!sdk?.adb) {
    return (
      <EmptyState title={tx("No Android SDK")}>
        Set <code>ANDROID_HOME</code> or install the SDK with Android Studio, then reopen the project.
      </EmptyState>
    );
  }
  if (!serial) {
    return <EmptyState title={tx("No device")}>{tx("Connect a device or start an emulator first.")}</EmptyState>;
  }

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <IconButton
          icon="chevron-up"
          size="small"
          title={tx("Up One Directory")}
          disabled={loading}
          onClick={deviceUp}
        />
        <PathField key={path} disabled={loading} />
        <Select
          value={COMMON_ROOTS.find((root) => path === root || path.startsWith(`${root}/`)) ?? ""}
          options={COMMON_ROOTS.map((root) => ({ value: root, label: root }))}
          onChange={(root) => root && void browseDevice(root)}
          placeholder={tx("Common folders")}
          aria-label={tx("Common folders")}
          className={styles.roots}
        />
        <IconButton
          icon="refresh"
          size="small"
          title={tx("Refresh")}
          disabled={loading}
          onClick={() => void browseDevice(path)}
        />
        <Button size="small" disabled={transferring || loading} onClick={() => void uploadToDevice()}>
          {tx("Upload…")}
        </Button>
        {(loading || transferring) && <Spinner size={14} label={tx("Working")} />}
      </div>
      <nav className={styles.crumbs} aria-label={tx("Path")}>
        {pathParts(path).map((part) => (
          <button key={part.path} type="button" className={styles.crumb} onClick={() => void browseDevice(part.path)}>
            {part.name}
            <span className={styles.crumbSep}>/</span>
          </button>
        ))}
      </nav>
      {error && <div className={styles.error}>{error}</div>}
      {entries.length === 0 && !loading && !error ? (
        <EmptyState title={tx("Empty folder")}>
          {tx("Nothing to show, or the folder cannot be read as root.")}
        </EmptyState>
      ) : (
        <div className={styles.fileList} role="list">
          {entries.map((entry) => (
            <div
              key={entry.name}
              role="listitem"
              className={styles.fileRow}
              onDoubleClick={() => openDeviceEntry(entry)}
            >
              <IconButton
                icon={entry.kind === "dir" ? "folder-open" : "file-unknown"}
                size="small"
                title={entry.name}
                onClick={() => (entry.kind === "dir" ? openDeviceEntry(entry) : void downloadDeviceFile(entry))}
              />
              <button
                type="button"
                className={styles.fileName}
                onClick={() => openDeviceEntry(entry)}
                disabled={entry.kind !== "dir"}
              >
                {entry.name}
              </button>
              <span className={styles.fileSize}>{entry.kind === "dir" ? "" : formatBytes(entry.size)}</span>
              <span className={styles.fileWhen}>{entry.modified ?? ""}</span>
              <span className={styles.fileOwner}>{entry.owner ?? ""}</span>
              {entry.kind !== "dir" && (
                <IconButton
                  icon="arrow-down"
                  size="small"
                  title={tx("Download")}
                  disabled={transferring}
                  onClick={() => void downloadDeviceFile(entry)}
                />
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
