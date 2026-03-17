import { useEffect } from "react";
import { useSettingsStore } from "../store";
import ShortcutRecorder from "./ShortcutRecorder";

export default function Settings() {
  const { captureShortcut, isSaving, error, fetchShortcut, updateShortcut } =
    useSettingsStore();

  useEffect(() => {
    fetchShortcut();
  }, [fetchShortcut]);

  return (
    <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-6">
      <section className="flex flex-col gap-3">
        <h2 className="text-xs font-semibold text-gray-500 uppercase tracking-wider">
          Atajos de teclado
        </h2>
        <div className="flex flex-col gap-4 rounded-lg bg-gray-900/60 border border-gray-800 p-4">
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-sm text-gray-300">Nueva captura</span>
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
