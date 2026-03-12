import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useHistoryStore } from "../store";
import type { HistoryItem } from "../types";

interface Props {
  item: HistoryItem;
}

function HistoryItemCard({ item }: Props) {
  const { deleteItem } = useHistoryStore();
  const [copied, setCopied] = useState(false);

  const formattedDate = new Date(item.created_at).toLocaleString("es-AR", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });

  const handleCopy = async () => {
    try {
      await invoke("copy_history_item", { id: item.id });
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Error copiando:", err);
    }
  };

  return (
    <div className="flex items-start gap-3 px-4 py-3 border-b border-gray-800/60 hover:bg-gray-900/40 transition-colors group">
      {/* Thumbnail o placeholder */}
      {item.type === "image" && item.thumbnail ? (
        <img
          src={`data:image/png;base64,${item.thumbnail}`}
          alt="captura"
          className="w-16 h-12 object-cover rounded flex-shrink-0 bg-gray-800"
        />
      ) : (
        <div className="w-16 h-12 flex-shrink-0 rounded bg-gray-800 flex items-center justify-center">
          <span className="text-xs text-gray-500 font-mono">txt</span>
        </div>
      )}

      {/* Contenido */}
      <div className="flex-1 min-w-0">
        <p className="text-xs text-gray-300 truncate leading-relaxed">
          {item.type === "text" ? item.content : "Captura de pantalla"}
        </p>
        <p className="text-xs text-gray-600 mt-1">{formattedDate}</p>
      </div>

      {/* Acciones */}
      <div className="flex items-center gap-1 flex-shrink-0">
        <button
          onClick={handleCopy}
          className={`text-xs px-2 py-1 rounded transition-all ${
            copied
              ? "text-green-400 bg-green-400/10"
              : "text-gray-400 hover:text-blue-400 hover:bg-blue-400/10"
          }`}
          title={item.type === "image" ? "Copiar imagen" : "Copiar texto"}
        >
          {copied ? "✓ Copiado" : "Copiar"}
        </button>

        <button
          onClick={() => deleteItem(item.id)}
          className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-red-400 transition-all text-base leading-none px-1 py-0.5 rounded"
          aria-label="Eliminar"
        >
          ×
        </button>
      </div>
    </div>
  );
}

export default HistoryItemCard;
