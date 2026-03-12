import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useHistoryStore } from "./store";
import HistoryList from "./components/HistoryList";
import CaptureOverlay from "./components/CaptureOverlay";
import PinView from "./components/PinView";
import "./App.css";

const windowLabel = getCurrentWindow().label;

function App() {
  if (windowLabel === "capture-overlay") return <CaptureOverlay />;
  if (windowLabel.startsWith("pin-")) return <PinView />;
  return <HistoryApp />;
}

function HistoryApp() {
  const { fetchHistory } = useHistoryStore();

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
        <div className="flex items-center gap-3">
          <span className="text-xs text-gray-600">history</span>
          <button
            onClick={() => invoke("hide_main_window")}
            className="text-gray-600 hover:text-gray-300 transition-colors text-xs leading-none w-5 h-5 flex items-center justify-center rounded hover:bg-gray-800"
            title="Close"
          >
            ×
          </button>
        </div>
      </header>
      <HistoryList />
    </div>
  );
}

export default App;
