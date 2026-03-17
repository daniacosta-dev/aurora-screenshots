import { useEffect, useState } from "react";
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
  if (windowLabel === "capture-overlay") return <CaptureOverlay />;
  if (windowLabel.startsWith("pin-")) return <PinView />;
  return <HistoryApp />;
}

type AppView = "history" | "settings";

function HistoryApp() {
  const { fetchHistory } = useHistoryStore();
  const [view, setView] = useState<AppView>("history");

  useEffect(() => {
    fetchHistory();

    const unlisten = listen("history-updated", () => {
      fetchHistory();
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <div className="h-screen flex flex-col bg-gray-950 text-gray-100 select-none" onContextMenu={(e) => e.preventDefault()}>
      <header
        className="px-4 py-3 border-b border-gray-800 flex items-center justify-between flex-shrink-0"
        style={{ cursor: "default" }}
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
      {view === "history" ? <HistoryList /> : <Settings />}
    </div>
  );
}

export default App;
