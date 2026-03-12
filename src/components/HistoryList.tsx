import { useHistoryStore } from "../store";
import HistoryItemCard from "./HistoryItem";

function HistoryList() {
  const { items, isLoading, error, captureScreenshot, clearHistory } =
    useHistoryStore();

  if (isLoading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <p className="text-sm text-gray-500">Cargando...</p>
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
        <div className="w-12 h-12 rounded-xl bg-gray-800 flex items-center justify-center">
          <span className="text-2xl">📋</span>
        </div>
        <p className="text-sm text-gray-400 text-center">
          El historial está vacío.
        </p>
        <p className="text-xs text-gray-600 text-center">
          Usá{" "}
          <kbd className="bg-gray-800 text-gray-400 px-1.5 py-0.5 rounded text-xs font-mono">
            Ctrl+Shift+S
          </kbd>{" "}
          para capturar.
        </p>
        <button
          onClick={captureScreenshot}
          className="mt-2 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-sm rounded-lg transition-colors"
        >
          Capturar ahora
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
            className="text-xs text-gray-400 hover:text-blue-400 transition-colors"
          >
            Capturar
          </button>
          <button
            onClick={clearHistory}
            className="text-xs text-gray-500 hover:text-red-400 transition-colors"
          >
            Limpiar todo
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
