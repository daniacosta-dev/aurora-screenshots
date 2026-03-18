import { useEffect } from "react";
import { useSettingsStore } from "../store";
import ShortcutRecorder from "./ShortcutRecorder";

export default function Settings() {
  const {
    captureShortcut, isSaving, error, fetchShortcut, updateShortcut,
    autostart, fetchAutostart, updateAutostart,
  } = useSettingsStore();

  useEffect(() => {
    fetchShortcut();
    fetchAutostart();
  }, [fetchShortcut, fetchAutostart]);

  return (
    <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-6">
      <section className="flex flex-col gap-3">
        <h2 className="text-xs font-semibold text-gray-500 uppercase tracking-wider">
          General
        </h2>
        <div className="flex flex-col gap-4 rounded-lg bg-gray-900/60 border border-gray-800 p-4">
          <div className="flex items-center justify-between">
            <div className="flex flex-col gap-0.5">
              <span className="text-sm text-gray-300">Launch at startup</span>
              <span className="text-xs text-gray-600">App will open in the tray when the system starts</span>
            </div>
            <button
              onClick={() => updateAutostart(!autostart)}
              className={`relative w-10 h-5 rounded-full transition-colors flex-shrink-0 ${
                autostart ? "bg-blue-500" : "bg-gray-700"
              }`}
              role="switch"
              aria-checked={autostart}
            >
              <span
                className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                  autostart ? "translate-x-5" : "translate-x-0"
                }`}
              />
            </button>
          </div>
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="text-xs font-semibold text-gray-500 uppercase tracking-wider">
          Keyboard shortcuts
        </h2>
        <div className="flex flex-col gap-4 rounded-lg bg-gray-900/60 border border-gray-800 p-4">
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-sm text-gray-300">New capture</span>
            </div>
            <ShortcutRecorder
              value={captureShortcut}
              onSave={updateShortcut}
              isSaving={isSaving}
              error={error}
            />
          </div>
        </div>
      </section>
    </div>
  );
}
