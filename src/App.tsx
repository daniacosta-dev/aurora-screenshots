import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useHistoryStore } from "./store";
import HistoryList from "./components/HistoryList";
import Settings from "./components/Settings";
import CaptureOverlay from "./components/CaptureOverlay";
import PinView from "./components/PinView";
import { X, Settings as SettingsIcon } from "lucide-react";
import "./App.css";

const windowLabel = getCurrentWindow().label;

function App() {
  if (windowLabel === "capture-overlay" || windowLabel.startsWith("capture-overlay-")) return <CaptureOverlay />;
  if (windowLabel.startsWith("pin-")) return <PinView />;
  return <HistoryApp />;
}

type AppView = "history" | "settings";

function HistoryApp() {
  const { fetchHistory } = useHistoryStore();
  const [view, setView] = useState<AppView>("history");
  const isFullscreen = new URLSearchParams(window.location.search).get("mode") === "fullscreen";
  const lastMouseDown = useRef(0);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    fetchHistory();
    const unlisten = listen("history-updated", () => fetchHistory());
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Registrar el timestamp de cada mousedown en el panel.
  // Clave para distinguir drag-start (mousedown propio) de click-afuera (no hay mousedown).
  useEffect(() => {
    const onDown = () => { lastMouseDown.current = Date.now(); };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, []);

  // Auto-close al perder foco. Lógica: si el foco se perdió dentro de 150ms de un
  // mousedown en el panel, es un drag-start → no cerrar. Si no hubo click reciente
  // en el panel (click afuera, otra ventana tomó foco) → cerrar.
  useEffect(() => {
    let ready = false;
    const readyTimer = setTimeout(() => { ready = true; }, 300);

    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) {
        if (closeTimer.current !== null) {
          clearTimeout(closeTimer.current);
          closeTimer.current = null;
        }
      } else if (ready && Date.now() - lastMouseDown.current > 150) {
        invoke("hide_main_window");
      }
    });
    return () => {
      clearTimeout(readyTimer);
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") invoke("hide_main_window");
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const header = (
    <header
      className={`px-4 py-3 border-b border-gray-800 flex items-center justify-between flex-shrink-0${!isFullscreen ? " cursor-move" : ""}`}
      onMouseDown={(e) => {
        if (isFullscreen) return;
        if ((e.target as HTMLElement).closest("button")) return;
        getCurrentWindow().startDragging();
      }}
    >
      <div className="flex items-center gap-2">
        <img src="/aurora-screenshots-icon.svg" alt="" className="w-4 h-4" />
        <h1 className="text-sm font-semibold text-gray-200 tracking-wide">
          Aurora Screenshots
        </h1>
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={() => setView(view === "settings" ? "history" : "settings")}
          className={`w-6 h-6 flex items-center justify-center rounded transition-colors ${
            view === "settings"
              ? "text-blue-400 bg-blue-400/10"
              : "text-gray-600 hover:text-gray-300 hover:bg-gray-800"
          }`}
          title="Settings"
        >
          <SettingsIcon size={13} />
        </button>
        <button
          onClick={() => invoke("hide_main_window")}
          className="text-gray-600 hover:text-gray-300 transition-colors w-6 h-6 flex items-center justify-center rounded hover:bg-gray-800"
          title="Close"
        >
          <X size={13} />
        </button>
      </div>
    </header>
  );

  const panelContent = (
    <>
      {header}
      {view === "history" ? <HistoryList /> : <Settings />}
    </>
  );

  if (isFullscreen) {
    // Fullscreen transparent window: panel is positioned top-right via CSS.
    // Input region (set in Rust) makes the transparent area click-through.
    return (
      <div
        className="fixed inset-0 flex justify-end items-start p-4"
        onContextMenu={(e) => e.preventDefault()}
      >
        <div className="w-[440px] h-[680px] flex flex-col bg-gray-950 text-gray-100 select-none rounded-xl shadow-2xl border border-gray-800/60 overflow-hidden">
          {panelContent}
        </div>
      </div>
    );
  }

  // Fixed-size window: fills the whole window which is already the panel size.
  return (
    <div
      className="w-full h-full flex flex-col bg-gray-950 text-gray-100 select-none rounded-xl shadow-2xl border border-gray-800/60 overflow-hidden"
      onContextMenu={(e) => e.preventDefault()}
    >
      {panelContent}
    </div>
  );
}

export default App;
