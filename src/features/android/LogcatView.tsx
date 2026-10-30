/**
 * Logcat tool window: the devices and emulators of the open project's Android
 * SDK, and their logcat stream (level column, tag and message, filtered as it
 * arrives). "Run on Device" builds a variant of the chosen module, installs it
 * and starts the app; "Restart App" re-starts it without building.
 */
import { useEffect, useMemo, useRef } from "react";
import clsx from "clsx";
import { Checkbox, EmptyState, IconButton, Input, Select } from "@/ui";
import { tx } from "@/platform/i18n";
import { LEVELS, applicationIdOf, isEmulator, variantsOf, visibleEntries } from "./model";
import {
  cancelDeploy,
  clearDeploy,
  clearLog,
  debugOnDevice,
  refreshAvds,
  refreshDevices,
  restartApp,
  runOnDevice,
  selectAvd,
  selectSerial,
  selectModule,
  setAppOnly,
  setColdBoot,
  setLevel,
  setText,
  setVariant,
  startEmulator,
  startLogcat,
  stopEmulator,
  stopLogcat,
  useAndroid,
} from "./store";
import styles from "./Android.module.css";

export function LogcatView() {
  const sdk = useAndroid((s) => s.sdk);
  const devices = useAndroid((s) => s.devices);
  const serial = useAndroid((s) => s.serial);
  const entries = useAndroid((s) => s.entries);
  const level = useAndroid((s) => s.level);
  const text = useAndroid((s) => s.text);
  const running = useAndroid((s) => s.running);
  const loading = useAndroid((s) => s.loading);
  const error = useAndroid((s) => s.error);
  const variant = useAndroid((s) => s.variant);
  const deploying = useAndroid((s) => s.deploying);
  const deployLog = useAndroid((s) => s.deployLog);
  const avds = useAndroid((s) => s.avds);
  const avd = useAndroid((s) => s.avd);
  const coldBoot = useAndroid((s) => s.coldBoot);
  const loadingAvds = useAndroid((s) => s.loadingAvds);
  const booting = useAndroid((s) => s.booting);
  const module = useAndroid((s) => s.module);
  const appOnly = useAndroid((s) => s.appOnly);

  const scroller = useRef<HTMLDivElement>(null);
  const deployScroller = useRef<HTMLPreElement>(null);
  const shown = useMemo(() => visibleEntries(entries, level, text), [entries, level, text]);
  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [shown.length]);
  useEffect(() => {
    const el = deployScroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [deployLog.length]);

  if (!sdk) {
    return (
      <EmptyState title="No Android SDK">
        Set <code>ANDROID_HOME</code> or install the SDK with Android Studio, then reopen the project.
      </EmptyState>
    );
  }

  const deviceOptions = devices.map((device) => ({
    value: device.serial,
    label: device.model ? `${device.serial} · ${device.model}` : device.serial,
    detail: device.state,
  }));
  const variants = variantsOf(sdk.modules, module);
  const appId = applicationIdOf(sdk.modules, module);

  return (
    <div className={styles.view}>
      <div className={styles.toolbar}>
        <Select
          value={serial}
          options={deviceOptions}
          onChange={selectSerial}
          placeholder={loading ? "Loading devices…" : "No device"}
          aria-label="Device"
          className={styles.device}
        />
        {sdk.modules.length > 1 && (
          <Select
            value={module}
            options={sdk.modules.map((m) => ({
              value: m.dir,
              label: m.dir || ".",
              detail: m.applicationId ?? undefined,
            }))}
            onChange={selectModule}
            aria-label="Module"
            className={styles.device}
          />
        )}
        <Select
          value={variant}
          options={variants.map((value) => ({ value, label: value }))}
          onChange={setVariant}
          aria-label="Build variant"
          className={styles.variant}
        />
        <Select
          value={level}
          options={LEVELS.map((value) => ({ value, label: value }))}
          onChange={setLevel}
          aria-label="Minimum level"
          className={styles.level}
        />
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Filter log"
          aria-label="Filter log"
          className={styles.filter}
        />
        <Checkbox
          checked={appOnly}
          onChange={setAppOnly}
          label={appId ? tx`App only (${appId})` : tx("App only")}
          disabled={!appId}
        />
        <IconButton
          icon={deploying ? "stop" : "run"}
          size="small"
          title={deploying ? "Stop Build & Install" : "Run on Device"}
          disabled={!serial || loading}
          onClick={() => (deploying ? cancelDeploy() : void runOnDevice())}
        />
        <IconButton
          icon="debug"
          size="small"
          title="Debug on Device"
          disabled={!serial || !appId || deploying}
          onClick={() => void debugOnDevice()}
        />
        <IconButton
          icon="restart-debug"
          size="small"
          title="Restart App"
          disabled={!serial || !appId || deploying}
          onClick={() => void restartApp()}
        />
        <IconButton
          icon="refresh"
          size="small"
          title="Refresh Devices and Emulators"
          onClick={() => {
            void refreshDevices();
            void refreshAvds();
          }}
        />
        {sdk.emulator && (
          <>
            <Select
              value={avd}
              options={avds.map((value) => ({ value, label: value }))}
              onChange={selectAvd}
              placeholder={loadingAvds ? "Loading emulators…" : "No emulator (AVD)"}
              aria-label="Emulator"
              className={styles.device}
            />
            <Checkbox checked={coldBoot} onChange={setColdBoot} label="Cold boot" />
            <IconButton
              icon="run"
              size="small"
              title={booting ? tx`Booting ${avd}…` : "Start Emulator"}
              disabled={!avd || booting}
              onClick={() => void startEmulator()}
            />
            <IconButton
              icon="stop"
              size="small"
              title="Stop Emulator"
              disabled={!serial || !isEmulator(serial)}
              onClick={() => void stopEmulator()}
            />
          </>
        )}
        <IconButton
          icon={running ? "stop" : "rerun"}
          size="small"
          title={running ? "Stop Logcat" : "Start Logcat"}
          disabled={!serial}
          onClick={() => (running ? stopLogcat() : void startLogcat())}
        />
        <IconButton icon="trash" size="small" title="Clear Logcat" onClick={clearLog} />
      </div>
      {error && <div className={styles.error}>{error}</div>}
      {booting && (
        <div className={styles.status}>
          {tx`Booting ${avd}…`} {tx("The device appears in the list once it is ready.")}
        </div>
      )}
      {(deploying || deployLog.length > 0) && (
        <div className={styles.deploy}>
          <div className={styles.deployHeader}>
            <span>{deploying ? "Building & installing…" : "Build & Install"}</span>
            {!deploying && <IconButton icon="close" size="small" title="Hide Build Output" onClick={clearDeploy} />}
          </div>
          <pre className={styles.deployLog} ref={deployScroller}>
            {deployLog.join("\n")}
          </pre>
        </div>
      )}
      {devices.length === 0 ? (
        <EmptyState
          title="No devices"
          actions={
            <>
              <IconButton icon="refresh" size="small" title="Refresh Devices" onClick={() => void refreshDevices()} />
              {avd && (
                <IconButton
                  icon="run"
                  size="small"
                  title={tx`Start Emulator (${avd})`}
                  disabled={booting}
                  onClick={() => void startEmulator()}
                />
              )}
            </>
          }
        >
          Connect a device over USB or start an emulator, then refresh.
        </EmptyState>
      ) : (
        <div className={styles.log} ref={scroller} role="log" aria-label="Logcat">
          {shown.map((entry, index) => (
            <div key={`${index}-${entry.timestamp ?? ""}`} className={styles.row}>
              <span className={styles.time}>{entry.timestamp ?? ""}</span>
              <span className={clsx(styles.levelBadge, styles[`l${entry.level ?? "V"}`])}>{entry.level ?? "·"}</span>
              <span className={styles.tag}>{entry.tag ?? ""}</span>
              <span className={styles.message}>{entry.message}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
