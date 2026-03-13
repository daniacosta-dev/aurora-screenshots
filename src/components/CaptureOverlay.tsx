import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { pictureDir, downloadDir } from "@tauri-apps/api/path";

// ── Types ──────────────────────────────────────────────────────────────────

type Phase = "idle" | "drawing" | "annotating";
type Tool = "arrow" | "rect" | "circle" | "marker" | "highlight" | "text" | "blur" | "invert" | "bubble" | "ruler";

interface Pt {
  x: number;
  y: number;
}

type Annotation =
  | { kind: "arrow"; from: Pt; to: Pt; color: string; size: number }
  | { kind: "rect"; x: number; y: number; w: number; h: number; color: string; size: number }
  | { kind: "circle"; x: number; y: number; w: number; h: number; color: string; size: number }
  | { kind: "marker"; points: Pt[]; color: string; size: number }
  | { kind: "text"; x: number; y: number; text: string; color: string; size: number }
  | { kind: "blur"; x: number; y: number; w: number; h: number }
  | { kind: "bubble"; x: number; y: number; n: number; color: string; size: number }
  | { kind: "highlight"; points: Pt[]; color: string; size: number }
  | { kind: "invert"; x: number; y: number; w: number; h: number }
  | { kind: "ruler"; from: Pt; to: Pt; color: string };

// ── Resize handles ────────────────────────────────────────────────────────

type HandleId = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

const HANDLE_R = 5;
const HANDLE_HIT = 10;

function getHandles(sx: number, sy: number, sw: number, sh: number): Array<{ id: HandleId; x: number; y: number }> {
  const cx = sx + sw / 2;
  const cy = sy + sh / 2;
  return [
    { id: "nw", x: sx, y: sy },
    { id: "n",  x: cx, y: sy },
    { id: "ne", x: sx + sw, y: sy },
    { id: "e",  x: sx + sw, y: cy },
    { id: "se", x: sx + sw, y: sy + sh },
    { id: "s",  x: cx,      y: sy + sh },
    { id: "sw", x: sx,      y: sy + sh },
    { id: "w",  x: sx,      y: cy },
  ];
}

function getHandleAtPoint(px: number, py: number, sx: number, sy: number, sw: number, sh: number): HandleId | null {
  for (const h of getHandles(sx, sy, sw, sh)) {
    const dx = px - h.x;
    const dy = py - h.y;
    if (dx * dx + dy * dy <= HANDLE_HIT * HANDLE_HIT) return h.id;
  }
  return null;
}

function getHandleCursor(id: HandleId): string {
  switch (id) {
    case "nw": case "se": return "nwse-resize";
    case "ne": case "sw": return "nesw-resize";
    case "n":  case "s":  return "ns-resize";
    case "e":  case "w":  return "ew-resize";
  }
}

function drawHandles(ctx: CanvasRenderingContext2D, sx: number, sy: number, sw: number, sh: number) {
  ctx.save();
  for (const h of getHandles(sx, sy, sw, sh)) {
    ctx.beginPath();
    ctx.arc(h.x, h.y, HANDLE_R + 1.5, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(0,0,0,0.45)";
    ctx.fill();
    ctx.beginPath();
    ctx.arc(h.x, h.y, HANDLE_R, 0, Math.PI * 2);
    ctx.fillStyle = "#ffffff";
    ctx.fill();
  }
  ctx.restore();
}

// ── Pure drawing helpers ───────────────────────────────────────────────────

function getSelRect(start: Pt, end: Pt) {
  return {
    sx: Math.min(start.x, end.x),
    sy: Math.min(start.y, end.y),
    sw: Math.abs(end.x - start.x),
    sh: Math.abs(end.y - start.y),
  };
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  from: Pt,
  to: Pt,
  color: string,
  size: number
) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  if (Math.sqrt(dx * dx + dy * dy) < 2) return;
  const angle = Math.atan2(dy, dx);
  const headLen = Math.max(14, size * 4);

  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = size;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(to.x, to.y);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(to.x, to.y);
  ctx.lineTo(
    to.x - headLen * Math.cos(angle - Math.PI / 6),
    to.y - headLen * Math.sin(angle - Math.PI / 6)
  );
  ctx.lineTo(
    to.x - headLen * Math.cos(angle + Math.PI / 6),
    to.y - headLen * Math.sin(angle + Math.PI / 6)
  );
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function drawRectAnnotation(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
  size: number
) {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size;
  ctx.strokeRect(x, y, w, h);
  ctx.restore();
}

function drawMarker(
  ctx: CanvasRenderingContext2D,
  points: Pt[],
  color: string,
  size: number
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) ctx.lineTo(points[i].x, points[i].y);
  ctx.stroke();
  ctx.restore();
}

function drawTextAnnotation(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  text: string,
  color: string,
  size: number
) {
  ctx.save();
  ctx.fillStyle = color;
  ctx.font = `bold ${12 + size * 3}px sans-serif`;
  ctx.fillText(text, x, y);
  ctx.restore();
}

// Blur via pixelation using getImageData/putImageData.
// Reads pixels already rendered on the canvas — no bgImage dependency.
// ctx.getTransform() maps CSS coords → physical canvas pixels, so this works
// correctly in both the live preview (scale only) and export (scale+translate) contexts.
function drawBlurAnnotation(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number
) {
  if (w < 4 || h < 4) return;
  const m = ctx.getTransform();
  // Map CSS rect → physical pixel rect on the canvas
  const px = Math.round(m.a * x + m.c * y + m.e);
  const py = Math.round(m.b * x + m.d * y + m.f);
  const pw = Math.max(1, Math.round(Math.abs(m.a) * w));
  const ph = Math.max(1, Math.round(Math.abs(m.d) * h));
  const blockPx = Math.max(2, Math.round(Math.abs(m.a) * 12));
  try {
    const imgData = ctx.getImageData(px, py, pw, ph);
    const d = imgData.data;
    for (let by = 0; by < ph; by += blockPx) {
      for (let bx = 0; bx < pw; bx += blockPx) {
        const i0 = (by * pw + bx) * 4;
        const r = d[i0], g = d[i0 + 1], b = d[i0 + 2], a = d[i0 + 3];
        for (let dy = by; dy < Math.min(by + blockPx, ph); dy++) {
          for (let dx = bx; dx < Math.min(bx + blockPx, pw); dx++) {
            const i = (dy * pw + dx) * 4;
            d[i] = r; d[i + 1] = g; d[i + 2] = b; d[i + 3] = a;
          }
        }
      }
    }
    ctx.putImageData(imgData, px, py);
  } catch (e) {
    console.warn("blur: getImageData failed", e);
  }
}

function drawCircleAnnotation(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  color: string, size: number
) {
  if (Math.abs(w) < 2 || Math.abs(h) < 2) return;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = size;
  ctx.beginPath();
  ctx.ellipse(x + w / 2, y + h / 2, Math.abs(w / 2), Math.abs(h / 2), 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

function drawBubble(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, n: number,
  color: string, size: number
) {
  const r = 12 + size * 2;
  ctx.save();
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
  ctx.fillStyle = "#fff";
  ctx.font = `bold ${10 + size * 2}px sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(String(n), x, y);
  ctx.restore();
}

function drawRulerAnnotation(
  ctx: CanvasRenderingContext2D,
  from: Pt, to: Pt, color: string, dpr: number
) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.sqrt(dx * dx + dy * dy);
  if (dist < 2) return;
  const angle = Math.atan2(dy, dx);
  const perp = angle + Math.PI / 2;
  const tickLen = 6;

  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.setLineDash([5, 4]);
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(to.x, to.y);
  ctx.stroke();
  ctx.setLineDash([]);

  // End ticks
  for (const pt of [from, to]) {
    ctx.beginPath();
    ctx.moveTo(pt.x + Math.cos(perp) * tickLen, pt.y + Math.sin(perp) * tickLen);
    ctx.lineTo(pt.x - Math.cos(perp) * tickLen, pt.y - Math.sin(perp) * tickLen);
    ctx.stroke();
  }

  // Distance label
  const physDist = Math.round(dist * dpr);
  const label = `${physDist} px`;
  const mx = (from.x + to.x) / 2;
  const my = (from.y + to.y) / 2;
  ctx.font = "bold 11px monospace";
  const tw = ctx.measureText(label).width + 12;
  const th = 18;
  ctx.fillStyle = "rgba(0,0,0,0.78)";
  ctx.beginPath();
  ctx.roundRect(mx - tw / 2, my - th / 2, tw, th, 4);
  ctx.fill();
  ctx.fillStyle = color;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(label, mx, my);
  ctx.restore();
}

function drawHighlight(
  ctx: CanvasRenderingContext2D,
  points: Pt[],
  color: string,
  size: number
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.globalAlpha = 0.38;
  ctx.strokeStyle = color;
  ctx.lineWidth = size * 7;
  ctx.lineCap = "square";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) ctx.lineTo(points[i].x, points[i].y);
  ctx.stroke();
  ctx.restore();
}

function drawInvertAnnotation(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number
) {
  if (w < 2 || h < 2) return;
  const m = ctx.getTransform();
  const px = Math.round(m.a * x + m.c * y + m.e);
  const py = Math.round(m.b * x + m.d * y + m.f);
  const pw = Math.max(1, Math.round(Math.abs(m.a) * w));
  const ph = Math.max(1, Math.round(Math.abs(m.d) * h));
  try {
    const imgData = ctx.getImageData(px, py, pw, ph);
    const d = imgData.data;
    for (let i = 0; i < d.length; i += 4) {
      d[i]     = 255 - d[i];
      d[i + 1] = 255 - d[i + 1];
      d[i + 2] = 255 - d[i + 2];
    }
    ctx.putImageData(imgData, px, py);
  } catch (e) {
    console.warn("invert: getImageData failed", e);
  }
}

function drawAnnotation(ctx: CanvasRenderingContext2D, ann: Annotation, dpr = 1) {
  switch (ann.kind) {
    case "arrow":
      drawArrow(ctx, ann.from, ann.to, ann.color, ann.size);
      break;
    case "rect":
      drawRectAnnotation(ctx, ann.x, ann.y, ann.w, ann.h, ann.color, ann.size);
      break;
    case "circle":
      drawCircleAnnotation(ctx, ann.x, ann.y, ann.w, ann.h, ann.color, ann.size);
      break;
    case "marker":
      drawMarker(ctx, ann.points, ann.color, ann.size);
      break;
    case "text":
      drawTextAnnotation(ctx, ann.x, ann.y, ann.text, ann.color, ann.size);
      break;
    case "blur":
      drawBlurAnnotation(ctx, ann.x, ann.y, ann.w, ann.h);
      break;
    case "highlight":
      drawHighlight(ctx, ann.points, ann.color, ann.size);
      break;
    case "invert":
      drawInvertAnnotation(ctx, ann.x, ann.y, ann.w, ann.h);
      break;
    case "bubble":
      drawBubble(ctx, ann.x, ann.y, ann.n, ann.color, ann.size);
      break;
    case "ruler":
      drawRulerAnnotation(ctx, ann.from, ann.to, ann.color, dpr);
      break;
  }
}

function drawDimLabel(
  ctx: CanvasRenderingContext2D,
  label: string,
  cx: number,
  sy: number,
  sh: number
) {
  ctx.save();
  ctx.font = "bold 11px monospace";
  const tw = ctx.measureText(label).width + 14;
  const th = 20;
  const tx = cx - tw / 2;
  const ty = sy > th + 8 ? sy - th - 4 : sy + sh + 4;
  ctx.fillStyle = "rgba(0,0,0,0.78)";
  ctx.beginPath();
  ctx.roundRect(tx, ty, tw, th, 4);
  ctx.fill();
  ctx.fillStyle = "#fff";
  ctx.fillText(label, tx + 7, ty + 14);
  ctx.restore();
}

// ── Component ─────────────────────────────────────────────────────────────

const PRESET_COLORS = ["#ff3b3b", "#ff9f0a", "#30d158", "#0a84ff", "#ffffff", "#000000"];

const PALETTE: string[] = [
  "#ff3b3b", "#ff6b6b", "#ff9f0a", "#ffd60a", "#30d158", "#34c759",
  "#0a84ff", "#5ac8fa", "#bf5af2", "#ff375f", "#ff2d55", "#ff6900",
  "#00c7be", "#32ade6", "#6e6e73", "#aeaeb2", "#ffffff", "#000000",
];

// Selector de color personalizado — sin input[type=color] nativo (rompe XGrabKeyboard).
function ColorPicker({
  value,
  onChange,
}: {
  value: string;
  onChange: (c: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [hex, setHex] = useState(value);
  const ref = useRef<HTMLDivElement>(null);

  // Sincronizar hex input cuando cambia el color externamente
  useEffect(() => { setHex(value); }, [value]);

  // Cerrar al hacer click fuera
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const commit = (c: string) => {
    onChange(c);
    setHex(c);
    setOpen(false);
  };

  return (
    <div ref={ref} style={{ position: "relative" }} onMouseDown={(e) => e.stopPropagation()}>
      {/* Swatch botón */}
      <button
        onClick={() => setOpen((o) => !o)}
        title="Custom color"
        style={{
          width: 30,
          height: 30,
          borderRadius: 6,
          background: open ? "rgba(255,255,255,0.12)" : "transparent",
          border: open ? "1px solid rgba(255,255,255,0.5)" : "1px solid transparent",
          cursor: "pointer",
          padding: 0,
          flexShrink: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 3,
        }}
      >
        {/* Color swatch + pencil icon */}
        <span style={{
          width: 12,
          height: 12,
          borderRadius: 3,
          background: value,
          border: "1.5px solid rgba(255,255,255,0.4)",
          display: "inline-block",
          flexShrink: 0,
        }} />
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="#e5e7eb" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M8.5 1.5l2 2-6 6H2.5v-2l6-6z"/>
        </svg>
      </button>
      {/* Popover */}
      {open && (
        <div
          style={{
            position: "absolute",
            bottom: "calc(100% + 8px)",
            left: "50%",
            transform: "translateX(-50%)",
            background: "rgba(18,18,20,0.97)",
            border: "1px solid rgba(255,255,255,0.12)",
            borderRadius: 10,
            padding: "10px",
            boxShadow: "0 8px 32px rgba(0,0,0,0.6)",
            zIndex: 300,
            width: 156,
          }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {/* Paleta */}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(6, 1fr)", gap: 5, marginBottom: 8 }}>
            {PALETTE.map((c) => (
              <button
                key={c}
                onClick={() => commit(c)}
                style={{
                  width: 18,
                  height: 18,
                  borderRadius: 3,
                  background: c,
                  border: value === c ? "2px solid #fff" : "1px solid rgba(255,255,255,0.2)",
                  cursor: "pointer",
                  padding: 0,
                }}
              />
            ))}
          </div>
          {/* Hex input */}
          <div style={{ display: "flex", gap: 5, alignItems: "center" }}>
            <div style={{ width: 18, height: 18, borderRadius: 3, background: hex, border: "1px solid rgba(255,255,255,0.2)", flexShrink: 0 }} />
            <input
              type="text"
              value={hex}
              spellCheck={false}
              maxLength={7}
              onChange={(e) => {
                const v = e.target.value;
                setHex(v);
                if (/^#[0-9a-fA-F]{6}$/.test(v)) onChange(v);
              }}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") commit(hex);
                if (e.key === "Escape") setOpen(false);
              }}
              style={{
                flex: 1,
                background: "rgba(255,255,255,0.07)",
                border: "1px solid rgba(255,255,255,0.18)",
                borderRadius: 4,
                color: "#e5e7eb",
                fontSize: 11,
                padding: "2px 5px",
                fontFamily: "monospace",
                outline: "none",
                width: 0,
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

function CaptureOverlay() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const phase = useRef<Phase>("idle");
  const startPos = useRef<Pt>({ x: 0, y: 0 });
  const endPos = useRef<Pt>({ x: 0, y: 0 });
  const bgImageRef = useRef<HTMLImageElement | null>(null);
  const annotationsRef = useRef<Annotation[]>([]);
  const currentAnnRef = useRef<Annotation | null>(null);
  const isDraggingAnn = useRef(false);
  const resizingHandleRef = useRef<HandleId | null>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const textInputRef = useRef<HTMLInputElement>(null);

  // Trigger re-render without state change (for toolbar repositioning during resize)
  const [, forceUpdate] = useReducer((x: number) => x + 1, 0);

  const [displayPhase, setDisplayPhase] = useState<Phase>("idle");
  const [activeTool, setActiveTool] = useState<Tool | null>(null);
  const [strokeColor, setStrokeColor] = useState("#ff3b3b");
  const [strokeSize, setStrokeSize] = useState(3);
  const [textInput, setTextInput] = useState<Pt | null>(null);
  const [textValue, setTextValue] = useState("");
  const [annCount, setAnnCount] = useState(0);

  // Forzar foco en el input de texto cuando aparece (autoFocus no es confiable en Tauri)
  useEffect(() => {
    if (textInput) {
      setTimeout(() => textInputRef.current?.focus(), 20);
    }
  }, [textInput]);

  // ── Canvas init ────────────────────────────────────────────────────────
  const initCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const W = window.innerWidth;
    const H = window.innerHeight;
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    canvas.style.width = `${W}px`;
    canvas.style.height = `${H}px`;
  }, []);

  // ── Draw ──────────────────────────────────────────────────────────────
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const W = window.innerWidth;
    const H = window.innerHeight;
    const bg = bgImageRef.current;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.scale(dpr, dpr);

    // Background screenshot (frozen desktop)
    if (bg) ctx.drawImage(bg, 0, 0, W, H);

    const p = phase.current;

    if (p === "idle") {
      ctx.fillStyle = "rgba(0,0,0,0.25)";
      ctx.fillRect(0, 0, W, H);
      return;
    }

    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);

    if (p === "drawing") {
      // Dark vignette over full canvas
      ctx.fillStyle = "rgba(0,0,0,0.5)";
      ctx.fillRect(0, 0, W, H);
      // Reveal selection area
      if (sw > 0 && sh > 0) {
        ctx.clearRect(sx, sy, sw, sh);
        if (bg) {
          const scaleX = bg.naturalWidth / W;
          const scaleY = bg.naturalHeight / H;
          ctx.drawImage(bg, sx * scaleX, sy * scaleY, sw * scaleX, sh * scaleY, sx, sy, sw, sh);
        }
      }
      // Selection border
      ctx.save();
      ctx.strokeStyle = "rgba(255,255,255,0.9)";
      ctx.lineWidth = 1.5;
      ctx.strokeRect(sx + 0.5, sy + 0.5, sw - 1, sh - 1);
      ctx.restore();
      // Dimension label
      if (sw > 20 && sh > 20) {
        drawDimLabel(ctx, `${Math.round(sw * dpr)} × ${Math.round(sh * dpr)} px`, sx + sw / 2, sy, sh);
      }
      return;
    }

    if (p === "annotating") {
      // Subtle vignette outside selection
      ctx.fillStyle = "rgba(0,0,0,0.38)";
      ctx.fillRect(0, 0, W, H);
      // Reveal and redraw bg in selection
      ctx.clearRect(sx, sy, sw, sh);
      if (bg) {
        const scaleX = bg.naturalWidth / W;
        const scaleY = bg.naturalHeight / H;
        ctx.drawImage(bg, sx * scaleX, sy * scaleY, sw * scaleX, sh * scaleY, sx, sy, sw, sh);
      }
      // Draw annotations clipped to selection area
      ctx.save();
      ctx.beginPath();
      ctx.rect(sx, sy, sw, sh);
      ctx.clip();
      for (const ann of annotationsRef.current) {
        drawAnnotation(ctx, ann, dpr);
      }
      if (currentAnnRef.current) {
        drawAnnotation(ctx, currentAnnRef.current, dpr);
      }
      ctx.restore();
      // Selection border (dashed green)
      ctx.save();
      ctx.setLineDash([6, 4]);
      ctx.strokeStyle = "rgba(74,222,128,0.85)";
      ctx.lineWidth = 1.5;
      ctx.strokeRect(sx + 0.5, sy + 0.5, sw - 1, sh - 1);
      ctx.restore();

      // Resize handles
      drawHandles(ctx, sx, sy, sw, sh);
    }
  }, []);

  // ── Load background screenshot ─────────────────────────────────────────
  const loadBackground = useCallback(async () => {
    try {
      const bg = await invoke<string | null>("get_desktop_background");
      if (bg) {
        const img = new Image();
        img.onload = () => {
          bgImageRef.current = img;
          draw();
        };
        img.src = `data:image/png;base64,${bg}`;
      }
    } catch (e) {
      console.warn("Background not available:", e);
    }
  }, [draw]);

  // ── Reset ─────────────────────────────────────────────────────────────
  const reset = useCallback(() => {
    phase.current = "idle";
    startPos.current = { x: 0, y: 0 };
    endPos.current = { x: 0, y: 0 };
    annotationsRef.current = [];
    currentAnnRef.current = null;
    isDraggingAnn.current = false;
    resizingHandleRef.current = null;
    if (resizeCleanupRef.current) { resizeCleanupRef.current(); }
    bgImageRef.current = null;
    if (canvasRef.current) canvasRef.current.style.cursor = "crosshair";
    setDisplayPhase("idle");
    setActiveTool(null);
    setTextInput(null);
    setTextValue("");
    setAnnCount(0);
    initCanvas();
    draw();
  }, [initCanvas, draw]);

  // Ocultar overlay via Rust (más confiable que getCurrentWindow().hide() en Tauri)
  const hideOverlay = useCallback(async () => {
    try {
      await invoke("hide_capture_overlay");
    } catch {
      await getCurrentWindow().hide();
    }
  }, []);

  // ── Export annotated capture ───────────────────────────────────────────
  const exportCapture = useCallback(async () => {
    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
    if (sw < 5 || sh < 5) return;

    const dpr = window.devicePixelRatio || 1;
    const W = window.innerWidth;
    const H = window.innerHeight;
    const bg = bgImageRef.current;

    const offscreen = document.createElement("canvas");
    offscreen.width = Math.round(sw * dpr);
    offscreen.height = Math.round(sh * dpr);
    const ctx = offscreen.getContext("2d");
    if (!ctx) return;

    // Draw background region at physical resolution
    if (bg) {
      const scaleX = bg.naturalWidth / W;
      const scaleY = bg.naturalHeight / H;
      ctx.drawImage(
        bg,
        sx * scaleX,
        sy * scaleY,
        sw * scaleX,
        sh * scaleY,
        0,
        0,
        sw * dpr,
        sh * dpr
      );
    }

    // Draw annotations offset to selection origin
    ctx.save();
    ctx.scale(dpr, dpr);
    ctx.translate(-sx, -sy);
    for (const ann of annotationsRef.current) {
      drawAnnotation(ctx, ann, dpr);
    }
    ctx.restore();

    // Export to base64
    const blob = await new Promise<Blob | null>((resolve) =>
      offscreen.toBlob(resolve, "image/png")
    );
    if (!blob) return;

    const ab = await blob.arrayBuffer();
    const bytes = new Uint8Array(ab);
    let binary = "";
    for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
    const base64 = btoa(binary);

    reset();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));

    try {
      await invoke("finalize_annotated_capture", { imageData: base64 });
    } catch (err) {
      console.error("Error al guardar captura:", err);
      await hideOverlay();
    }
  }, [reset, hideOverlay]);

  // Ctrl+C sin selección: copiar el escritorio completo
  const exportFullDesktop = useCallback(async () => {
    try {
      const bg = await invoke<string | null>("get_desktop_background");
      if (!bg) return;
      await invoke("finalize_annotated_capture", { imageData: bg });
      reset();
    } catch (err) {
      console.error("Error capturando pantalla completa:", err);
      reset();
      await hideOverlay();
    }
  }, [reset, hideOverlay]);

  // ── Pin screenshot ─────────────────────────────────────────────────────
  const pinCapture = useCallback(async () => {
    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
    if (sw < 5 || sh < 5) return;

    const dpr = window.devicePixelRatio || 1;
    const W = window.innerWidth;
    const H = window.innerHeight;
    const bg = bgImageRef.current;

    const offscreen = document.createElement("canvas");
    offscreen.width = Math.round(sw * dpr);
    offscreen.height = Math.round(sh * dpr);
    const ctx = offscreen.getContext("2d");
    if (!ctx) return;

    if (bg) {
      const scaleX = bg.naturalWidth / W;
      const scaleY = bg.naturalHeight / H;
      ctx.drawImage(bg, sx * scaleX, sy * scaleY, sw * scaleX, sh * scaleY, 0, 0, sw * dpr, sh * dpr);
    }
    ctx.save();
    ctx.scale(dpr, dpr);
    ctx.translate(-sx, -sy);
    for (const ann of annotationsRef.current) drawAnnotation(ctx, ann, dpr);
    ctx.restore();

    const blob = await new Promise<Blob | null>((resolve) => offscreen.toBlob(resolve, "image/png"));
    if (!blob) return;
    const ab = await blob.arrayBuffer();
    const bytes = new Uint8Array(ab);
    let binary = "";
    for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
    const base64 = btoa(binary);

    reset();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));

    try {
      await invoke("pin_screenshot", { imageData: base64, width: Math.round(sw), height: Math.round(sh) });
      await hideOverlay();
    } catch (err) {
      console.error("Error al pinear captura:", err);
      await hideOverlay();
    }
  }, [reset, hideOverlay]);

  // ── Save to file ──────────────────────────────────────────────────────
  const saveCapture = useCallback(async () => {
    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
    if (sw < 5 || sh < 5) return;

    const dpr = window.devicePixelRatio || 1;
    const W = window.innerWidth;
    const H = window.innerHeight;
    const bg = bgImageRef.current;

    const offscreen = document.createElement("canvas");
    offscreen.width = Math.round(sw * dpr);
    offscreen.height = Math.round(sh * dpr);
    const ctx = offscreen.getContext("2d");
    if (!ctx) return;

    if (bg) {
      const scaleX = bg.naturalWidth / W;
      const scaleY = bg.naturalHeight / H;
      ctx.drawImage(bg, sx * scaleX, sy * scaleY, sw * scaleX, sh * scaleY, 0, 0, sw * dpr, sh * dpr);
    }
    ctx.save();
    ctx.scale(dpr, dpr);
    ctx.translate(-sx, -sy);
    for (const ann of annotationsRef.current) drawAnnotation(ctx, ann, dpr);
    ctx.restore();

    const blob = await new Promise<Blob | null>((resolve) => offscreen.toBlob(resolve, "image/png"));
    if (!blob) return;
    const ab = await blob.arrayBuffer();
    const bytes = new Uint8Array(ab);
    let binary = "";
    for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
    const base64 = btoa(binary);

    // Ocultar overlay ANTES del diálogo para que aparezca encima (override_redirect cubre todo)
    reset();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    await invoke("hide_capture_overlay");

    // Carpeta por defecto: Imágenes → Descargas → home
    const defaultDir = await pictureDir().catch(() => downloadDir().catch(() => ""));
    const filename = `screenshot-${new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19)}.png`;
    const defaultPath = defaultDir ? `${defaultDir}/${filename}` : filename;

    const path = await saveDialog({
      title: "Save screenshot",
      defaultPath,
      filters: [{ name: "PNG Image", extensions: ["png"] }],
    });
    if (!path) return; // usuario canceló

    try {
      await invoke("write_screenshot_file", { path, imageData: base64 });
    } catch (err) {
      console.error("Error al guardar archivo:", err);
    }
  }, [reset]);

  // ── Effects ───────────────────────────────────────────────────────────
  useEffect(() => {
    initCanvas();
    draw();

    const onFocus = () => {
      console.log("[overlay] window focus — phase:", phase.current, "hasFocus:", document.hasFocus());
      if (phase.current !== "idle") return;
      reset();
      loadBackground();
    };
    const onBlur = () => {
      console.warn("[overlay] window BLUR — phase:", phase.current, "activeElement:", document.activeElement?.tagName);
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);

    // Background may arrive after overlay is already shown (async capture).
    // reset() primero para limpiar anotaciones/estado de la captura anterior.
    let unlistenBg: (() => void) | null = null;
    listen("background-ready", () => {
      console.log("[overlay] background-ready received");
      reset();
      loadBackground();
    }).then((fn) => { unlistenBg = fn; });

    const onKeydown = async (e: KeyboardEvent) => {
      console.log("[overlay] keydown:", e.key, "ctrl:", e.ctrlKey, "phase:", phase.current, "hasFocus:", document.hasFocus());
      // Dejar que el input de texto maneje sus propias teclas
      if (textInputRef.current && document.activeElement === textInputRef.current) return;

      if (e.key === "Escape") {
        if (phase.current === "idle") {
          await hideOverlay();
        } else {
          reset();
        }
        return;
      }

      if (e.ctrlKey && e.key === "z" && phase.current === "annotating") {
        e.preventDefault();
        annotationsRef.current = annotationsRef.current.slice(0, -1);
        setAnnCount(annotationsRef.current.length);
        draw();
        return;
      }

      if (e.ctrlKey && e.key === "c") {
        e.preventDefault();
        e.stopPropagation();
        if (phase.current === "annotating") {
          await exportCapture();
        } else if (phase.current === "idle") {
          await exportFullDesktop();
        }
      }
    };

    window.addEventListener("keydown", onKeydown);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("keydown", onKeydown);
      if (unlistenBg) unlistenBg();
    };
  }, [reset, draw, initCanvas, loadBackground, exportCapture, exportFullDesktop, pinCapture, hideOverlay]);

  // ── Mouse handlers ────────────────────────────────────────────────────
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const pos = { x: e.clientX, y: e.clientY };

    if (phase.current === "annotating") {
      // Resize handles take priority over everything else
      {
        const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
        const handle = getHandleAtPoint(pos.x, pos.y, sx, sy, sw, sh);
        if (handle) {
          e.preventDefault();
          resizingHandleRef.current = handle;
          if (canvasRef.current) canvasRef.current.style.cursor = getHandleCursor(handle);

          // Use window-level listeners so events are captured even when cursor moves
          // over the toolbar or other overlay elements (avoids falling through to WM)
          const onWinMove = (ev: MouseEvent) => {
            ev.preventDefault();
            const h = resizingHandleRef.current!;
            const { sx: rx, sy: ry, sw: rw, sh: rh } = getSelRect(startPos.current, endPos.current);
            let x1 = rx, y1 = ry, x2 = rx + rw, y2 = ry + rh;
            const movingE = h === "ne" || h === "e"  || h === "se";
            const movingW = h === "nw" || h === "w"  || h === "sw";
            const movingN = h === "nw" || h === "n"  || h === "ne";
            const movingS = h === "sw" || h === "s"  || h === "se";
            if (movingE) x2 = ev.clientX;
            if (movingW) x1 = ev.clientX;
            if (movingN) y1 = ev.clientY;
            if (movingS) y2 = ev.clientY;
            const min = 20;
            if (movingE && x2 - x1 < min) x2 = x1 + min;
            if (movingW && x2 - x1 < min) x1 = x2 - min;
            if (movingS && y2 - y1 < min) y2 = y1 + min;
            if (movingN && y2 - y1 < min) y1 = y2 - min;
            startPos.current = { x: x1, y: y1 };
            endPos.current   = { x: x2, y: y2 };
            draw();
            forceUpdate(); // re-render toolbar to new position
          };
          const onWinUp = () => {
            resizingHandleRef.current = null;
            cleanup();
            draw();
            forceUpdate();
          };
          const cleanup = () => {
            window.removeEventListener("mousemove", onWinMove);
            window.removeEventListener("mouseup", onWinUp);
            resizeCleanupRef.current = null;
          };
          resizeCleanupRef.current = cleanup;
          window.addEventListener("mousemove", onWinMove);
          window.addEventListener("mouseup", onWinUp);
          return;
        }
      }

      if (activeTool === "text") {
        // Clamp to selection bounds so text stays inside the capture area
        const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
        const clampedX = Math.max(sx + 4, Math.min(pos.x, sx + sw - 4));
        const clampedY = Math.max(sy + 16, Math.min(pos.y, sy + sh - 4));
        setTextInput({ x: clampedX, y: clampedY });
        setTextValue("");
        return;
      }
      if (activeTool === null) {
        const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
        const inside = pos.x >= sx && pos.x <= sx + sw && pos.y >= sy && pos.y <= sy + sh;
        if (inside) {
          // Mover la selección completa con drag via window-level listeners
          if (canvasRef.current) canvasRef.current.style.cursor = "grabbing";
          let lastPos = pos;
          const onWinMove = (ev: MouseEvent) => {
            const dx = ev.clientX - lastPos.x;
            const dy = ev.clientY - lastPos.y;
            lastPos = { x: ev.clientX, y: ev.clientY };
            startPos.current = { x: startPos.current.x + dx, y: startPos.current.y + dy };
            endPos.current = { x: endPos.current.x + dx, y: endPos.current.y + dy };
            draw();
            forceUpdate();
          };
          const onWinUp = () => {
            if (canvasRef.current) canvasRef.current.style.cursor = "crosshair";
            window.removeEventListener("mousemove", onWinMove);
            window.removeEventListener("mouseup", onWinUp);
          };
          window.addEventListener("mousemove", onWinMove);
          window.addEventListener("mouseup", onWinUp);
          return;
        }
        // Click fuera de la selección: re-dibujar
        phase.current = "drawing";
        annotationsRef.current = [];
        currentAnnRef.current = null;
        startPos.current = pos;
        endPos.current = pos;
        setDisplayPhase("drawing");
        setAnnCount(0);
        draw();
        return;
      }
      // Bubble: place immediately on click, no drag
      if (activeTool === "bubble") {
        const n = annotationsRef.current.filter((a) => a.kind === "bubble").length + 1;
        annotationsRef.current = [...annotationsRef.current, { kind: "bubble", x: pos.x, y: pos.y, n, color: strokeColor, size: strokeSize }];
        setAnnCount(annotationsRef.current.length);
        draw();
        return;
      }

      // Start annotation drag
      isDraggingAnn.current = true;
      switch (activeTool) {
        case "arrow":
          currentAnnRef.current = { kind: "arrow", from: pos, to: pos, color: strokeColor, size: strokeSize };
          break;
        case "rect":
          currentAnnRef.current = { kind: "rect", x: pos.x, y: pos.y, w: 0, h: 0, color: strokeColor, size: strokeSize };
          break;
        case "circle":
          currentAnnRef.current = { kind: "circle", x: pos.x, y: pos.y, w: 0, h: 0, color: strokeColor, size: strokeSize };
          break;
        case "marker":
          currentAnnRef.current = { kind: "marker", points: [pos], color: strokeColor, size: strokeSize };
          break;
        case "highlight":
          currentAnnRef.current = { kind: "highlight", points: [pos], color: strokeColor, size: strokeSize };
          break;
        case "blur":
          currentAnnRef.current = { kind: "blur", x: pos.x, y: pos.y, w: 0, h: 0 };
          break;
        case "invert":
          currentAnnRef.current = { kind: "invert", x: pos.x, y: pos.y, w: 0, h: 0 };
          break;
        case "ruler":
          currentAnnRef.current = { kind: "ruler", from: pos, to: pos, color: strokeColor };
          break;
      }
      draw();
      return;
    }

    // Selection drawing
    phase.current = "drawing";
    startPos.current = pos;
    endPos.current = pos;
    setDisplayPhase("drawing");
    draw();
  };

  const onMouseMove = (e: React.MouseEvent) => {
    const pos = { x: e.clientX, y: e.clientY };

    // Cursor update in annotating phase (skip while actively resizing — window listener owns that)
    if (phase.current === "annotating" && !resizingHandleRef.current) {
      const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
      const handle = getHandleAtPoint(pos.x, pos.y, sx, sy, sw, sh);
      const newCursor = handle
        ? getHandleCursor(handle)
        : activeTool === "text" ? "text"
        : activeTool === null && pos.x >= sx && pos.x <= sx + sw && pos.y >= sy && pos.y <= sy + sh ? "grab"
        : "crosshair";
      if (canvasRef.current) canvasRef.current.style.cursor = newCursor;
    }

    if (phase.current === "annotating" && isDraggingAnn.current && currentAnnRef.current) {
      const ann = currentAnnRef.current;
      switch (ann.kind) {
        case "arrow":
          currentAnnRef.current = { ...ann, to: pos };
          break;
        case "rect": {
          let w = pos.x - ann.x;
          let h = pos.y - ann.y;
          if (e.ctrlKey) {
            const side = Math.min(Math.abs(w), Math.abs(h));
            w = Math.sign(w) * side;
            h = Math.sign(h) * side;
          }
          currentAnnRef.current = { ...ann, w, h };
          break;
        }
        case "circle": {
          let w = pos.x - ann.x;
          let h = pos.y - ann.y;
          if (e.ctrlKey) {
            const side = Math.min(Math.abs(w), Math.abs(h));
            w = Math.sign(w) * side;
            h = Math.sign(h) * side;
          }
          currentAnnRef.current = { ...ann, w, h };
          break;
        }
        case "marker":
          currentAnnRef.current = { ...ann, points: [...ann.points, pos] };
          break;
        case "highlight":
          currentAnnRef.current = { ...ann, points: [...ann.points, pos] };
          break;
        case "blur":
          currentAnnRef.current = { ...ann, w: pos.x - ann.x, h: pos.y - ann.y };
          break;
        case "invert":
          currentAnnRef.current = { ...ann, w: pos.x - ann.x, h: pos.y - ann.y };
          break;
        case "ruler":
          currentAnnRef.current = { ...ann, to: pos };
          break;
      }
      draw();
      return;
    }

    if (phase.current === "drawing") {
      endPos.current = pos;
      draw();
    }
  };

  const onMouseUp = (e: React.MouseEvent) => {
    const pos = { x: e.clientX, y: e.clientY };

    if (phase.current === "annotating" && isDraggingAnn.current && currentAnnRef.current) {
      isDraggingAnn.current = false;
      const ann = currentAnnRef.current;

      // Normalize and validate before committing
      let finalAnn: Annotation | null = ann;
      if (ann.kind === "rect" || ann.kind === "blur" || ann.kind === "circle" || ann.kind === "invert") {
        const nx = Math.min(ann.x, ann.x + ann.w);
        const ny = Math.min(ann.y, ann.y + ann.h);
        const nw = Math.abs(ann.w);
        const nh = Math.abs(ann.h);
        if (nw < 3 || nh < 3) {
          finalAnn = null;
        } else if (ann.kind === "rect") {
          finalAnn = { ...ann, x: nx, y: ny, w: nw, h: nh };
        } else if (ann.kind === "circle") {
          finalAnn = { ...ann, x: nx, y: ny, w: nw, h: nh };
        } else if (ann.kind === "invert") {
          finalAnn = { kind: "invert", x: nx, y: ny, w: nw, h: nh };
        } else {
          finalAnn = { kind: "blur", x: nx, y: ny, w: nw, h: nh };
        }
      } else if (ann.kind === "arrow") {
        finalAnn = { ...ann, to: pos };
        const dx = pos.x - ann.from.x;
        const dy = pos.y - ann.from.y;
        if (Math.sqrt(dx * dx + dy * dy) < 5) finalAnn = null;
      } else if (ann.kind === "ruler") {
        finalAnn = { ...ann, to: pos };
        const dx = pos.x - ann.from.x;
        const dy = pos.y - ann.from.y;
        if (Math.sqrt(dx * dx + dy * dy) < 5) finalAnn = null;
      }

      if (finalAnn) {
        annotationsRef.current = [...annotationsRef.current, finalAnn];
        setAnnCount(annotationsRef.current.length);
      }
      currentAnnRef.current = null;
      draw();
      return;
    }

    if (phase.current === "drawing") {
      endPos.current = pos;
      const sw = Math.abs(endPos.current.x - startPos.current.x);
      const sh = Math.abs(endPos.current.y - startPos.current.y);
      if (sw < 5 || sh < 5) {
        phase.current = "idle";
        setDisplayPhase("idle");
      } else {
        phase.current = "annotating";
        setDisplayPhase("annotating");
      }
      draw();
    }
  };

  // ── Text commit ────────────────────────────────────────────────────────
  const commitText = useCallback(() => {
    if (textValue.trim() && textInput) {
      const newAnn: Annotation = {
        kind: "text",
        x: textInput.x,
        y: textInput.y + 20,
        text: textValue.trim(),
        color: strokeColor,
        size: strokeSize,
      };
      annotationsRef.current = [...annotationsRef.current, newAnn];
      setAnnCount(annotationsRef.current.length);
      draw();
    }
    setTextInput(null);
    setTextValue("");
  }, [textValue, textInput, strokeColor, strokeSize, draw]);

  // ── Toolbar position ───────────────────────────────────────────────────
  const getToolbarStyle = (): React.CSSProperties => {
    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);
    const W = window.innerWidth;
    const TOOLBAR_W = 750;
    const TOOLBAR_H = 50;
    const spaceBelow = window.innerHeight - (sy + sh);
    const top = spaceBelow > TOOLBAR_H + 12 ? sy + sh + 8 : sy - TOOLBAR_H - 8;
    const left = Math.max(4, Math.min(sx + sw / 2 - TOOLBAR_W / 2, W - TOOLBAR_W - 4));
    return { top, left, width: TOOLBAR_W };
  };

  const getSelectionSize = () => {
    const { sw, sh } = getSelRect(startPos.current, endPos.current);
    const dpr = window.devicePixelRatio || 1;
    return `${Math.round(sw * dpr)} × ${Math.round(sh * dpr)} px`;
  };

  // ── Render ────────────────────────────────────────────────────────────
  const toolDefs: { tool: Tool; icon: React.ReactNode; title: string }[] = [
    { tool: "arrow",     icon: "→",  title: "Arrow" },
    { tool: "rect",      icon: "□",  title: "Rectangle (Ctrl=square)" },
    { tool: "circle",    icon: "○",  title: "Circle (Ctrl=perfect)" },
    { tool: "marker",    icon: "~",  title: "Marker" },
    {
      tool: "highlight",
      title: "Highlighter",
      icon: (
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" strokeLinecap="round" strokeLinejoin="round">
          <rect x="1.5" y="9" width="13" height="4.5" rx="2" fill="currentColor" fillOpacity="0.35" stroke="none"/>
          <line x1="4" y1="8.5" x2="12" y2="3.5" stroke="currentColor" strokeWidth="1.4"/>
          <line x1="12" y1="3.5" x2="14" y2="5.5" stroke="currentColor" strokeWidth="1.4"/>
          <line x1="4" y1="8.5" x2="6" y2="10.5" stroke="currentColor" strokeWidth="1.4"/>
        </svg>
      ),
    },
    { tool: "text",  icon: "T",  title: "Text" },
    { tool: "blur",  icon: "◌",  title: "Blur" },
    {
      tool: "invert",
      title: "Invert colors",
      icon: (
        <svg width="16" height="16" viewBox="0 0 16 16">
          <circle cx="8" cy="8" r="5.5" fill="currentColor" stroke="currentColor" strokeWidth="1.2"/>
          <path d="M8 2.5a5.5 5.5 0 0 0 0 11Z" fill="white" stroke="none"/>
        </svg>
      ),
    },
    { tool: "bubble", icon: "①", title: "Numbered bubble" },
    {
      tool: "ruler",
      title: "Ruler (measure px)",
      icon: (
        <svg width="17" height="17" viewBox="0 0 17 17" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <rect x="1" y="5.5" width="15" height="6" rx="1.2" strokeWidth="1.3" fill="none"/>
          <line x1="4"  y1="5.5" x2="4"  y2="8"/>
          <line x1="7"  y1="5.5" x2="7"  y2="9.5"/>
          <line x1="10" y1="5.5" x2="10" y2="8"/>
          <line x1="13" y1="5.5" x2="13" y2="9.5"/>
        </svg>
      ),
    },
  ];

  return (
    <div
      style={{ position: "fixed", inset: 0, overflow: "hidden", userSelect: "none" }}
      onMouseDown={(e) => {
        // Commitear texto al hacer clic fuera del input
        if (textInput && textInputRef.current && !textInputRef.current.contains(e.target as Node)) {
          commitText();
        }
      }}
    >
      <canvas
        ref={canvasRef}
        style={{
          position: "absolute",
          inset: 0,
          cursor: "crosshair",
          display: "block",
        }}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
      />

      {/* Annotation toolbar */}
      {displayPhase === "annotating" && (
        <div
          style={{
            position: "absolute",
            ...getToolbarStyle(),
            background: "rgba(18, 18, 20, 0.92)",
            border: "1px solid rgba(255,255,255,0.1)",
            borderRadius: 10,
            padding: "6px 10px",
            display: "flex",
            alignItems: "center",
            gap: 4,
            backdropFilter: "blur(16px)",
            zIndex: 100,
            boxShadow: "0 4px 24px rgba(0,0,0,0.5)",
          }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {/* Tool buttons */}
          {toolDefs.map(({ tool, icon, title }) => (
            <button
              key={tool}
              onClick={() => setActiveTool(activeTool === tool ? null : tool)}
              title={title}
              style={{
                background: activeTool === tool ? "rgba(74,222,128,0.18)" : "transparent",
                border: activeTool === tool ? "1px solid #4ade80" : "1px solid transparent",
                borderRadius: 6,
                color: activeTool === tool ? "#4ade80" : "#e5e7eb",
                cursor: "pointer",
                width: 30,
                height: 30,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                fontSize: 15,
                fontWeight: "bold",
              }}
            >
              {icon}
            </button>
          ))}

          <div style={{ width: 1, height: 22, background: "rgba(255,255,255,0.12)", margin: "0 3px" }} />

          {/* Color presets */}
          {PRESET_COLORS.map((c) => (
            <button
              key={c}
              onClick={() => setStrokeColor(c)}
              style={{
                width: 16,
                height: 16,
                borderRadius: "50%",
                background: c,
                border: strokeColor === c ? "2px solid #fff" : "1px solid rgba(255,255,255,0.25)",
                cursor: "pointer",
                padding: 0,
                flexShrink: 0,
              }}
            />
          ))}

          <ColorPicker value={strokeColor} onChange={setStrokeColor} />

          <div style={{ width: 1, height: 22, background: "rgba(255,255,255,0.12)", margin: "0 3px" }} />

          {/* Stroke size */}
          {([{ v: 2, label: "S" }, { v: 4, label: "M" }, { v: 6, label: "L" }] as const).map(({ v, label }) => (
            <button
              key={v}
              onClick={() => setStrokeSize(v)}
              style={{
                background: strokeSize === v ? "rgba(255,255,255,0.18)" : "transparent",
                border: strokeSize === v ? "1px solid rgba(255,255,255,0.5)" : "1px solid transparent",
                borderRadius: 5,
                color: strokeSize === v ? "#fff" : "#9ca3af",
                cursor: "pointer",
                width: 24,
                height: 24,
                fontSize: 11,
                fontWeight: "bold",
              }}
            >
              {label}
            </button>
          ))}

          <div style={{ width: 1, height: 22, background: "rgba(255,255,255,0.12)", margin: "0 3px" }} />

          {/* Undo */}
          <button
            onClick={() => {
              annotationsRef.current = annotationsRef.current.slice(0, -1);
              setAnnCount(annotationsRef.current.length);
              draw();
            }}
            disabled={annCount === 0}
            title="Undo (Ctrl+Z)"
            style={{
              background: "transparent",
              border: "1px solid transparent",
              borderRadius: 6,
              color: annCount === 0 ? "#4b5563" : "#e5e7eb",
              cursor: annCount === 0 ? "default" : "pointer",
              width: 30,
              height: 30,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 15,
            }}
          >
            <svg width="15" height="15" viewBox="0 0 15 15" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
              <path d="M4.5 8.5a4 4 0 1 0 .8-2.8"/>
              <polyline points="2,5 4.5,5 4.5,7.5"/>
            </svg>
          </button>

          {/* Pin screenshot */}
          <button
            onClick={pinCapture}
            title="Pin on screen"
            style={{
              background: "rgba(0,229,255,0.12)",
              border: "1px solid rgba(0,229,255,0.45)",
              borderRadius: 6,
              color: "rgba(0,229,255,0.9)",
              cursor: "pointer",
              padding: "4px 8px",
              fontSize: 11,
              fontWeight: "bold",
              whiteSpace: "nowrap",
            }}
          >
            📌
          </button>

          {/* Save to file */}
          <button
            onClick={saveCapture}
            title="Save as PNG"
            style={{
              background: "transparent",
              border: "1px solid transparent",
              borderRadius: 6,
              color: "#e5e7eb",
              cursor: "pointer",
              width: 30,
              height: 30,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
              {/* Cuerpo del disquete */}
              <rect x="2" y="2" width="12" height="12" rx="1.5"/>
              {/* Etiqueta inferior */}
              <rect x="4.5" y="8.5" width="7" height="4" rx="0.8"/>
              {/* Ranura superior (área de escritura) */}
              <rect x="5" y="2" width="5" height="3.5" rx="0.5"/>
              {/* Tapa de la ranura */}
              <line x1="8.5" y1="2.5" x2="8.5" y2="5"/>
            </svg>
          </button>

          {/* Confirm capture */}
          <button
            onClick={exportCapture}
            title="Capture (Ctrl+C)"
            style={{
              background: "rgba(74,222,128,0.18)",
              border: "1px solid #4ade80",
              borderRadius: 6,
              color: "#4ade80",
              cursor: "pointer",
              padding: "4px 10px",
              fontSize: 11,
              fontWeight: "bold",
              whiteSpace: "nowrap",
            }}
          >
            Capture
          </button>

          {/* Cancel */}
          <button
            onClick={async () => {
              reset();
              await hideOverlay();
            }}
            title="Cancel (Esc)"
            style={{
              background: "transparent",
              border: "1px solid rgba(255,255,255,0.15)",
              borderRadius: 6,
              color: "#6b7280",
              cursor: "pointer",
              padding: "4px 8px",
              fontSize: 11,
              whiteSpace: "nowrap",
            }}
          >
            Esc
          </button>

          <div style={{ width: 1, height: 22, background: "rgba(255,255,255,0.12)", margin: "0 3px" }} />

          {/* Selection size */}
          <span
            style={{
              color: "#9ca3af",
              fontSize: 10,
              fontFamily: "monospace",
              whiteSpace: "nowrap",
              userSelect: "none",
            }}
          >
            {getSelectionSize()}
          </span>
        </div>
      )}

      {/* Floating text input */}
      {textInput && (
        <input
          ref={textInputRef}
          value={textValue}
          onChange={(e) => setTextValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitText();
            }
            if (e.key === "Escape") {
              setTextInput(null);
              setTextValue("");
            }
          }}
          onBlur={commitText}
          placeholder=""
          style={{
            position: "absolute",
            left: textInput.x,
            top: textInput.y,
            background: "transparent",
            border: "none",
            outline: "none",
            color: strokeColor,
            padding: 0,
            margin: 0,
            fontSize: `${12 + strokeSize * 3}px`,
            fontWeight: "bold",
            fontFamily: "sans-serif",
            minWidth: 80,
            zIndex: 200,
            caretColor: strokeColor,
            textShadow: "0 1px 3px rgba(0,0,0,0.55)",
          }}
        />
      )}

      {/* Bottom hint (only during selection) */}
      {displayPhase !== "annotating" && (
        <div
          style={{
            position: "absolute",
            bottom: 20,
            left: "50%",
            transform: "translateX(-50%)",
            background: "rgba(0,0,0,0.72)",
            color: "#fff",
            fontSize: 12,
            padding: "6px 16px",
            borderRadius: 6,
            pointerEvents: "none",
            whiteSpace: "nowrap",
            display: "flex",
            gap: 16,
          }}
        >
          <span>Drag to select an area</span>
          <span style={{ color: "#6b7280" }}>ESC cancel</span>
        </div>
      )}

    </div>
  );
}

export default CaptureOverlay;
