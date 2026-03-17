import { useEffect, useRef, useState } from "react";
import { Keyboard } from "lucide-react";

const MODIFIERS = new Set(["Control", "Shift", "Alt", "Meta"]);

// Normaliza e.key al nombre que usa el parser de Rust
const NORMALIZE_KEY: Record<string, string> = {
  " ": "Space",
  "ArrowUp": "Up", "ArrowDown": "Down",
  "ArrowLeft": "Left", "ArrowRight": "Right",
};

// Teclas soportadas por el parser de Rust — misma lista que en parse_shortcut()
const VALID_KEYS = new Set([
  "A","B","C","D","E","F","G","H","I","J","K","L","M",
  "N","O","P","Q","R","S","T","U","V","W","X","Y","Z",
  "0","1","2","3","4","5","6","7","8","9",
  "F1","F2","F3","F4","F5","F6","F7","F8","F9","F10","F11","F12",
  "Home","End","PageUp","PageDown","Insert","Delete",
  "Up","Down","Left","Right",
  "Space","Tab","Enter","Backspace",
  "-","=","[","]","\\",";","'","`",",",".","/",
]);

interface Props {
  value: string;
  onSave: (shortcut: string) => Promise<void>;
  isSaving: boolean;
  error: string | null;
}

export default function ShortcutRecorder({ value, onSave, isSaving, error }: Props) {
  const [recording, setRecording] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const [keyError, setKeyError] = useState<string | null>(null);
  const divRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (recording) divRef.current?.focus();
  }, [recording]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      setRecording(false);
      setKeyError(null);
      return;
    }

    if (MODIFIERS.has(e.key)) return;

    const parts: string[] = [];
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    if (e.metaKey) parts.push("Super");

    if (parts.length === 0) return;

    const raw = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    const key = NORMALIZE_KEY[e.key] ?? raw;

    if (!VALID_KEYS.has(key)) {
      setKeyError(`Tecla "${key}" no soportada. Usá letras, números o F1-F12.`);
      return;
    }

    setKeyError(null);
    parts.push(key);
    setPending(parts.join("+"));
    setRecording(false);
  };

  const handleSave = async () => {
    if (!pending) return;
    await onSave(pending);
    setPending(null);
  };

  const handleCancel = () => {
    setPending(null);
  };

  const displayShortcut = pending ?? value;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        {/* Teclas actuales */}
        <div className="flex items-center gap-1 flex-1">
          {displayShortcut.split("+").map((k) => (
            <kbd
              key={k}
              className="px-2 py-0.5 rounded text-xs font-mono bg-gray-800 text-gray-200 border border-gray-700"
            >
              {k}
            </kbd>
          ))}
          {pending && <span className="text-xs text-amber-400 ml-1">sin guardar</span>}
        </div>

        {/* Botones */}
        {!recording && !pending && (
          <button
            onClick={() => { setRecording(true); setKeyError(null); }}
            disabled={isSaving}
            className="flex items-center gap-1.5 text-xs px-2.5 py-1.5 rounded bg-gray-800 hover:bg-gray-700 text-gray-300 hover:text-white border border-gray-700 transition-colors disabled:opacity-40"
          >
            <Keyboard size={11} />
            Cambiar
          </button>
        )}
        {pending && (
          <>
            <button
              onClick={handleSave}
              disabled={isSaving}
              className="text-xs px-2.5 py-1.5 rounded bg-blue-600 hover:bg-blue-500 text-white border border-blue-500 transition-colors disabled:opacity-40"
            >
              {isSaving ? "Guardando…" : "Guardar"}
            </button>
            <button
              onClick={handleCancel}
              className="text-xs px-2.5 py-1.5 rounded bg-gray-800 hover:bg-gray-700 text-gray-400 border border-gray-700 transition-colors"
            >
              Cancelar
            </button>
          </>
        )}
      </div>

      {/* Área de grabación */}
      {recording && (
        <div
          ref={divRef}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          onBlur={() => { setRecording(false); setKeyError(null); }}
          className="flex flex-col gap-1 px-3 py-2 rounded border border-blue-500/60 bg-blue-500/8 outline-none cursor-default"
        >
          <div className="flex items-center gap-2 text-xs text-blue-300">
            <span className="animate-pulse text-blue-400">●</span>
            Presioná la combinación de teclas… (Esc para cancelar)
          </div>
          {keyError && <p className="text-xs text-amber-400">{keyError}</p>}
        </div>
      )}

      {/* Error del backend */}
      {error && !recording && !pending && (
        <p className="text-xs text-red-400">{error}</p>
      )}
    </div>
  );
}
