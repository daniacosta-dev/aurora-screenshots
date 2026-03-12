import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const win = getCurrentWindow();
const pinId = win.label.slice("pin-".length);

export default function PinView() {
  const [src, setSrc] = useState<string | null>(null);
  const [imageData, setImageData] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [hover, setHover] = useState(false);

  useEffect(() => {
    invoke<string | null>("get_pin_image", { id: pinId }).then((data) => {
      if (data) {
        setImageData(data);
        setSrc(`data:image/png;base64,${data}`);
      }
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") win.close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const handleCopy = async () => {
    if (!imageData) return;
    try {
      await invoke("copy_png_to_clipboard", { imageData });
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error("copy error:", err);
    }
  };

  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onMouseDown={(e) => {
        if (e.button === 0) win.startDragging();
      }}
      style={{
        width: "100vw",
        height: "100vh",
        background: "#0d1117",
        position: "relative",
        overflow: "hidden",
        cursor: "grab",
        userSelect: "none",
        outline: "1px solid rgba(255,255,255,0.12)",
      }}
    >
      {/* Imagen */}
      {src ? (
        <img
          src={src}
          draggable={false}
          style={{
            width: "100%",
            height: "100%",
            objectFit: "contain",
            display: "block",
            pointerEvents: "none",
          }}
        />
      ) : (
        <div style={{ width: "100%", height: "100%", background: "#161b22" }} />
      )}

      {/* Controles flotantes — aparecen al hacer hover */}
      <div
        onMouseDown={(e) => { e.stopPropagation(); e.preventDefault(); }}
        style={{
          position: "absolute",
          top: 6,
          right: 6,
          display: "flex",
          gap: 4,
          opacity: hover ? 1 : 0,
          transition: "opacity 0.15s",
          pointerEvents: hover ? "auto" : "none",
        }}
      >
        <Btn onClick={handleCopy} title="Copy to clipboard">
          {copied ? "✓" : (
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
            </svg>
          )}
        </Btn>
        <Btn onClick={() => win.close()} title="Close (Esc)">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </Btn>
      </div>
    </div>
  );
}

function Btn({ onClick, title, children }: { onClick: () => void; title: string; children: React.ReactNode }) {
  const [hov, setHov] = useState(false);
  return (
    <button
      onClick={onClick}
      title={title}
      onMouseEnter={() => setHov(true)}
      onMouseLeave={() => setHov(false)}
      style={{
        background: hov ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.6)",
        border: "1px solid rgba(255,255,255,0.15)",
        borderRadius: 5,
        color: "#fff",
        cursor: "pointer",
        width: 26,
        height: 26,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        transition: "background 0.1s",
      }}
    >
      {children}
    </button>
  );
}
