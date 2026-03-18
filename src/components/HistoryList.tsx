import { invoke } from "@tauri-apps/api/core";
import { useHistoryStore, useSettingsStore } from "../store";
import HistoryItemCard from "./HistoryItem";
import { Clipboard, Camera, Trash2, FolderOpen } from "lucide-react";

function HistoryList() {
  const { items, isLoading, error, captureScreenshot, clearHistory } =
    useHistoryStore();
  const { captureShortcut } = useSettingsStore();

  if (isLoading && items.length === 0) {
    return (
      <div className="flex-1 flex flex-col overflow-hidden">
        {[...Array(5)].map((_, i) => (
          <div key={i} className="flex items-start gap-3 px-4 py-3 border-b border-gray-800/60">
            <div className="w-16 h-12 rounded bg-gray-800/70 flex-shrink-0 animate-pulse" />
            <div className="flex-1 flex flex-col gap-2 py-1">
              <div className="h-2.5 rounded bg-gray-800/70 animate-pulse" style={{ width: `${60 + (i % 3) * 15}%` }} />
              <div className="h-2 rounded bg-gray-800/50 animate-pulse w-16" />
            </div>
          </div>
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex-1 flex items-center justify-center px-6">
        <p className="text-sm text-red-400 text-center">{error}</p>
      </div>
    );
  }

  if (items.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-3 px-6">
        <div className="w-12 h-12 rounded-xl bg-gray-800 flex items-center justify-center text-gray-500">
          <Clipboard size={22} />
        </div>
        <p className="text-sm text-gray-400 text-center">
          History is empty.
        </p>
        <p className="text-xs text-gray-600 text-center flex items-center justify-center gap-1 flex-wrap">
          Press{" "}
          {captureShortcut.split("+").map((k) => (
            <kbd key={k} className="bg-gray-800 text-gray-400 px-1.5 py-0.5 rounded text-xs font-mono">
              {k}
            </kbd>
          ))}
          {" "}to capture.
        </p>
        <button
          onClick={captureScreenshot}
          className="mt-2 flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-sm rounded-lg transition-colors"
        >
          <Camera size={14} />
          Capture now
        </button>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Barra con contador y acciones */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <span className="text-xs text-gray-500">
          {items.length} {items.length === 1 ? "item" : "items"}
        </span>
        <div className="flex items-center gap-3">
          <button
            onClick={captureScreenshot}
            className="flex items-center gap-1 text-xs text-gray-400 hover:text-blue-400 transition-colors"
          >
            <Camera size={12} />
            Capture
          </button>
          <button
            onClick={() => invoke("open_screenshots_folder")}
            className="flex items-center gap-1 text-xs text-gray-400 hover:text-blue-400 transition-colors"
            title="Open screenshots folder"
          >
            <FolderOpen size={12} />
            Open folder
          </button>
          <button
            onClick={clearHistory}
            className="flex items-center gap-1 text-xs text-gray-500 hover:text-red-400 transition-colors"
          >
            <Trash2 size={12} />
            Clear all
          </button>
        </div>
      </div>

      {/* Lista scrolleable */}
      <div className="flex-1 overflow-y-auto">
        {items.map((item) => (
          <HistoryItemCard key={item.id} item={item} />
        ))}
      </div>
    </div>
  );
}

export default HistoryList;
