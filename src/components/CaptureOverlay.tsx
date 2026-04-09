import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { pictureDir, downloadDir } from "@tauri-apps/api/path";
import {
  ArrowUpRight, Square, Circle as CircleIcon, Pencil, Highlighter,
  Type, Blend, Contrast, MessageCircle, Ruler, Undo2, Save, Pin,
} from "lucide-react";

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
  | { kind: "bubble"; x: number; y: number; n: number; color: string; size: number; tail?: Pt }
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
  color: string, size: number, tail?: Pt
) {
  const r = 12 + size * 2;
  ctx.save();
  // Cono desde el borde de la burbuja hacia el punto señalado
  if (tail) {
    const dx = tail.x - x;
    const dy = tail.y - y;
    const dist = Math.sqrt(dx * dx + dy * dy);
    if (dist > r) {
      const angle = Math.atan2(dy, dx);
      const wing = Math.PI / 7; // ~26°
      ctx.beginPath();
      ctx.moveTo(x + r * Math.cos(angle + wing), y + r * Math.sin(angle + wing));
      ctx.lineTo(tail.x, tail.y);
      ctx.lineTo(x + r * Math.cos(angle - wing), y + r * Math.sin(angle - wing));
      ctx.closePath();
      ctx.fillStyle = color;
      ctx.fill();
    }
  }
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
      drawBubble(ctx, ann.x, ann.y, ann.n, ann.color, ann.size, ann.tail);
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


function CaptureOverlay() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const phase = useRef<Phase>("idle");
  const startPos = useRef<Pt>({ x: 0, y: 0 });
  const endPos = useRef<Pt>({ x: 0, y: 0 });
  const bgImageRef = useRef<HTMLImageElement | null>(null);
  // Copia del base64 original del escritorio; permite exportFullDesktop sin volver a pedir a Rust.
  const bgDataRef = useRef<string | null>(null);
  // Evita llamar overlay_ready más de una vez por ciclo de captura.
  const overlayReadySent = useRef(false);
  const annotationsRef = useRef<Annotation[]>([]);
  const currentAnnRef = useRef<Annotation | null>(null);
  const isDraggingAnn = useRef(false);
  const resizingHandleRef = useRef<HandleId | null>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const textInputRef = useRef<HTMLInputElement>(null);
  const colorPickerPosRef = useRef<{ x: number; y: number } | null>(null);

  // Trigger re-render without state change (for toolbar repositioning during resize)
  const [, forceUpdate] = useReducer((x: number) => x + 1, 0);

  const [displayPhase, setDisplayPhase] = useState<Phase>("idle");
  const [activeTool, setActiveTool] = useState<Tool | null>(null);
  const [strokeColor, setStrokeColor] = useState("#ff3b3b");
  const [strokeSize, setStrokeSize] = useState(3);
  const [textInput, setTextInput] = useState<Pt | null>(null);
  const [textValue, setTextValue] = useState("");
  const [annCount, setAnnCount] = useState(0);
  const [colorPickerPos, setColorPickerPos] = useState<{ x: number; y: number; above?: boolean } | null>(null);
  const [colorPickerHex, setColorPickerHex] = useState(strokeColor);
  const [hoveredTool, setHoveredTool] = useState<Tool | null>(null);
  const [hoveredSize, setHoveredSize] = useState<number | null>(null);

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
      ctx.fillStyle = "rgba(0,0,0,0.5)";
      ctx.fillRect(0, 0, W, H);
      return;
    }

    const { sx, sy, sw, sh } = getSelRect(startPos.current, endPos.current);

    if (p === "drawing") {
      // Overlay oscuro solo FUERA de la selección usando clip evenodd.
      // El fondo (ya dibujado arriba) queda visible en la zona seleccionada
      // sin necesidad de clearRect — evita el negro cuando bg carga lento.
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, 0, W, H);
      // Solo abrir el "agujero" si el fondo ya cargó; si no, la selección quedaría negra.
      if (bg && sw > 0 && sh > 0) ctx.rect(sx, sy, sw, sh);
      ctx.clip("evenodd");
      ctx.fillStyle = "rgba(0,0,0,0.5)";
      ctx.fillRect(0, 0, W, H);
      ctx.restore();
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
      // Same opacity as idle/drawing — sin parpadeo al transicionar entre fases
      ctx.fillStyle = "rgba(0,0,0,0.5)";
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

  // ── Signal Rust that the canvas is ready → hace show() del overlay ────
  const signalReady = useCallback(async () => {
    if (overlayReadySent.current) return;
    overlayReadySent.current = true;
    try {
      await invoke("overlay_ready");
    } catch (e) {
      console.warn("[overlay] overlay_ready invoke failed:", e);
    }
  }, []);

  // ── Load Wayland pending capture (Wayland annotation mode) ───────────────

  // Espera hasta que la ventana tenga dimensiones de pantalla completa o timeout.
  const waitForFullscreen = useCallback((): Promise<void> => {
    return new Promise((resolve) => {
      const startW = window.innerWidth;
      const startH = window.innerHeight;
      // Si ya parece fullscreen (> 1000px), resolver inmediatamente.
      if (startW > 1000 && startH > 600) { resolve(); return; }
      // Si no, esperar el evento resize o un timeout de 400ms.
      const timeout = setTimeout(resolve, 400);
      const onResize = () => {
        clearTimeout(timeout);
        window.removeEventListener("resize", onResize);
        resolve();
      };
      window.addEventListener("resize", onResize);
    });
  }, []);

  const loadWaylandCapture = useCallback(async (): Promise<boolean> => {
    type WaylandCapture = { content: string; thumbnail: string; width: number; height: number };
    let result: WaylandCapture | null = null;
    try {
      result = await invoke<WaylandCapture | null>("get_wayland_pending_capture");
    } catch {
      return false;
    }
    if (!result) return false;

    return new Promise<boolean>((resolve) => {
      const img = new Image();
      img.onload = async () => {
        bgDataRef.current = result!.content;

        // Esperar a que el compositor termine de mapear la ventana fullscreen.
        await waitForFullscreen();

        initCanvas();
        const W = window.innerWidth;
        const H = window.innerHeight;
        const dpr = window.devicePixelRatio || 1;
        const naturalW = img.naturalWidth;
        const naturalH = img.naturalHeight;

        // Tamaño lógico de la imagen (el portal ya devuelve píxeles físicos)
        const logW = naturalW / dpr;
        const logH = naturalH / dpr;

        // Centrar la imagen en la ventana fullscreen
        const imgX = Math.round((W - logW) / 2);
        const imgY = Math.round((H - logH) / 2);

        // Composite: canvas del tamaño de la ventana con la imagen centrada.
        // El resto queda transparente → draw() muestra el overlay oscuro de la app.
        // Permite que draw()/exportCapture() funcionen sin cambios usando bg.naturalWidth/W.
        const composite = document.createElement("canvas");
        composite.width = W * dpr;
        composite.height = H * dpr;
        composite.getContext("2d")!.drawImage(img, imgX * dpr, imgY * dpr, naturalW, naturalH);

        const compositeImg = new Image();
        compositeImg.onload = () => {
          bgImageRef.current = compositeImg;
          startPos.current = { x: imgX, y: imgY };
          endPos.current = { x: imgX + logW, y: imgY + logH };
          phase.current = "annotating";
          setDisplayPhase("annotating");
          draw();
          resolve(true);
        };
        compositeImg.onerror = () => resolve(false);
        compositeImg.src = composite.toDataURL("image/jpeg", 0.97);
      };
      img.onerror = () => resolve(false);
      img.src = `data:image/png;base64,${result!.content}`;
    });
  }, [draw, initCanvas, waitForFullscreen]);

  // ── Load background screenshot ─────────────────────────────────────────
  const loadBackground = useCallback(async () => {
    try {
      const t0 = performance.now();
      type MonitorCapture = { x: number; y: number; width: number; height: number; data: string };
      const monitors = await invoke<MonitorCapture[] | null>("get_desktop_background");
      console.log(`[timing] IPC: ${(performance.now() - t0).toFixed(1)}ms  monitors=${monitors?.length}`);

      if (!monitors || monitors.length === 0) return;

      const totalW = Math.max(...monitors.map(m => m.x + m.width));
      const totalH = Math.max(...monitors.map(m => m.y + m.height));

      // HTMLCanvasElement — compatible con WebKitGTK (OffscreenCanvas.convertToBlob no está disponible)
      const canvas = document.createElement("canvas");
      canvas.width = totalW;
      canvas.height = totalH;
      const ctx = canvas.getContext("2d")!;

      const t1 = performance.now();
      await Promise.all(monitors.map(m => new Promise<void>((resolve, reject) => {
        const img = new Image();
        img.onload = () => { ctx.drawImage(img, m.x, m.y); resolve(); };
        img.onerror = reject;
        img.src = `data:image/jpeg;base64,${m.data}`;
      })));
      console.log(`[timing] decode + composite: ${(performance.now() - t1).toFixed(1)}ms`);

      const dataUrl = canvas.toDataURL("image/jpeg", 0.97);

      // Awaitar la carga de la imagen final para poder llamar signalReady en el mismo tick.
      await new Promise<void>((resolve) => {
        const finalImg = new Image();
        finalImg.onload = () => {
          console.log(`[timing] background total: ${(performance.now() - t0).toFixed(1)}ms`);
          bgImageRef.current = finalImg;
          bgDataRef.current = dataUrl.slice("data:image/jpeg;base64,".length);
          draw();
          resolve();
        };
        finalImg.onerror = () => resolve();
        finalImg.src = dataUrl;
      });

      // Canvas listo → decirle a Rust que haga show() del overlay.
      await signalReady();

      // show() en GTK es asíncrono: la ventana se mapea unos ms después.
      // Dar tiempo para que GTK la mapee y window.innerWidth/innerHeight sean correctos,
      // luego reinicializar el canvas con las dimensiones reales y redibujar.
      await new Promise<void>((r) => setTimeout(r, 30));
      initCanvas();
      if (bgImageRef.current) draw();
    } catch (e) {
      console.warn("Background not available:", e);
    }
  }, [draw, initCanvas, signalReady]);

  // ── Reset ─────────────────────────────────────────────────────────────
  const reset = useCallback(() => {
    phase.current = "idle";
    overlayReadySent.current = false;
    startPos.current = { x: 0, y: 0 };
    endPos.current = { x: 0, y: 0 };
    annotationsRef.current = [];
    currentAnnRef.current = null;
    isDraggingAnn.current = false;
    resizingHandleRef.current = null;
    if (resizeCleanupRef.current) { resizeCleanupRef.current(); }
    bgImageRef.current = null;
    bgDataRef.current = null;
    if (canvasRef.current) canvasRef.current.style.cursor = "crosshair";
    setDisplayPhase("idle");
    setActiveTool(null);
    setTextInput(null);
    setTextValue("");
    setAnnCount(0);
    colorPickerPosRef.current = null;
    setColorPickerPos(null);
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

  // Cerrar (destruir) el overlay — usado en ESC para que la próxima captura
  // arranque con una ventana WebKit limpia, sin bugs de foco.
  const closeOverlay = useCallback(async () => {
    try {
      await invoke("close_capture_overlay");
    } catch {
      await getCurrentWindow().close();
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

    // Export to base64 — toDataURL evita la cadena Blob→ArrayBuffer→Uint8Array→btoa
    const dataUrl = offscreen.toDataURL("image/png");
    offscreen.width = 0; // libera el buffer del canvas inmediatamente
    const base64 = dataUrl.slice("data:image/png;base64,".length);

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
      // Usar el base64 ya cargado en memoria; no volver a pedirlo a Rust.
      const bg = bgDataRef.current;
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

    const dataUrl = offscreen.toDataURL("image/png");
    offscreen.width = 0; // libera el buffer del canvas inmediatamente
    const base64 = dataUrl.slice("data:image/png;base64,".length);

    reset();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));

    try {
      await invoke("pin_screenshot", { imageData: base64, width: Math.round(sw), height: Math.round(sh) });
      await closeOverlay();
    } catch (err) {
      console.error("Error al pinear captura:", err);
      await closeOverlay();
    }
  }, [reset, closeOverlay]);

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

    const dataUrl = offscreen.toDataURL("image/png");
    offscreen.width = 0; // libera el buffer del canvas inmediatamente
    const base64 = dataUrl.slice("data:image/png;base64,".length);

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
    // Chequear primero si hay una captura Wayland pendiente (race con wayland-capture-ready).
    // Si la hay, ir directo a annotating; si no, cargar el background X11.
    loadWaylandCapture().then((loaded) => {
      if (!loaded) loadBackground();
    });

    // Fallback: si la carga del fondo falla o demora demasiado, mostrar el overlay
    // de todas formas después de 800ms para no dejar la ventana oculta para siempre.
    const safetyTimer = setTimeout(() => signalReady(), 800);

    const onFocus = () => {
      console.log("[overlay] window focus — phase:", phase.current, "hasFocus:", document.hasFocus());
      if (phase.current !== "idle") return;
      // No llamar reset() aquí: en el nuevo flujo, background-ready ya llamó reset()
      // antes de loadBackground() y signalReady(). Si llamamos reset() aquí (después de
      // que show() disparó el focus), borraríamos el fondo recién pintado.
      // Solo intentamos cargar si hay datos nuevos en AppState (take() devuelve null si no).
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
      console.log("[overlay] background-ready received, phase:", phase.current);
      if (phase.current === "idle") reset();
      loadBackground();
    }).then((fn) => { unlistenBg = fn; });

    // El grab X11 se establece ~80ms después del show(). Cuando termina, emite
    // "grab-ready" y re-solicitamos foco a WebKit para evitar el bug donde el
    // primer teclazo solo activa el foco y el segundo ejecuta la acción.
    let unlistenGrab: (() => void) | null = null;
    listen("grab-ready", async () => {
      await getCurrentWindow().setFocus();
    }).then((fn) => { unlistenGrab = fn; });

    // En Wayland, el portal XDG ya hizo la selección de región.
    // Rust emite este evento con la imagen lista; vamos directo a la fase de anotación.
    let unlistenWayland: (() => void) | null = null;
    listen("wayland-capture-ready", async () => {
      console.log("[overlay] wayland-capture-ready received, phase:", phase.current);
      // Si el mount effect ya cargó la imagen (ventana nueva), no hacer nada.
      // Si la ventana se reusa (phase sigue en idle), cargar ahora.
      if (phase.current !== "idle") return;
      await loadWaylandCapture();
    }).then((fn) => { unlistenWayland = fn; });

    const onKeydown = async (e: KeyboardEvent) => {
      console.log("[overlay] keydown:", e.key, "ctrl:", e.ctrlKey, "phase:", phase.current, "hasFocus:", document.hasFocus());
      // Dejar que el input de texto maneje sus propias teclas
      if (textInputRef.current && document.activeElement === textInputRef.current) return;

      if (e.key === "Escape") {
        if (colorPickerPosRef.current !== null) {
          colorPickerPosRef.current = null;
          setColorPickerPos(null);
          return;
        }
        reset();
        await closeOverlay();
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
        return;
      }

      // Atajos de herramientas (solo en fase de anotación, sin modificadores)
      if (phase.current === "annotating" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        const toolMap: Record<string, Tool> = {
          a: "arrow", s: "rect", c: "circle", p: "marker",
          h: "highlight", t: "text", b: "blur", i: "invert",
          n: "bubble", l: "ruler",
        };
        const tool = toolMap[e.key.toLowerCase()];
        if (tool) {
          e.preventDefault();
          setActiveTool((prev) => (prev === tool ? null : tool));
          return;
        }
      }
    };

    window.addEventListener("keydown", onKeydown);
    return () => {
      clearTimeout(safetyTimer);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("keydown", onKeydown);
      if (unlistenBg) unlistenBg();
      if (unlistenGrab) unlistenGrab();
      if (unlistenWayland) unlistenWayland();
    };
  }, [reset, draw, initCanvas, loadBackground, loadWaylandCapture, waitForFullscreen, signalReady, exportCapture, exportFullDesktop, pinCapture, hideOverlay, closeOverlay]);

  // ── Mouse handlers ────────────────────────────────────────────────────
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button === 2) {
      setColorPickerHex(strokeColor);
      const pos = { x: e.clientX, y: e.clientY };
      colorPickerPosRef.current = pos;
      setColorPickerPos(pos);
      return;
    }
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
        // Detectar click sobre una burbuja existente para moverla
        let bubbleIdx = -1;
        for (let i = annotationsRef.current.length - 1; i >= 0; i--) {
          const a = annotationsRef.current[i];
          if (a.kind === "bubble") {
            const r = 12 + a.size * 2;
            if (Math.sqrt((pos.x - a.x) ** 2 + (pos.y - a.y) ** 2) <= r) {
              bubbleIdx = i;
              break;
            }
          }
        }
        if (bubbleIdx >= 0) {
          if (canvasRef.current) canvasRef.current.style.cursor = "grabbing";
          let lastDragPos = pos;
          const onWinMove = (ev: MouseEvent) => {
            const dx = ev.clientX - lastDragPos.x;
            const dy = ev.clientY - lastDragPos.y;
            lastDragPos = { x: ev.clientX, y: ev.clientY };
            const b = annotationsRef.current[bubbleIdx] as Extract<Annotation, { kind: "bubble" }>;
            annotationsRef.current = [
              ...annotationsRef.current.slice(0, bubbleIdx),
              { ...b, x: b.x + dx, y: b.y + dy, tail: b.tail ? { x: b.tail.x + dx, y: b.tail.y + dy } : undefined },
              ...annotationsRef.current.slice(bubbleIdx + 1),
            ];
            draw();
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
      // Bubble: click → coloca sin cola; drag → crea burbuja con cono apuntando al destino
      if (activeTool === "bubble") {
        isDraggingAnn.current = true;
        const n = annotationsRef.current.filter((a) => a.kind === "bubble").length + 1;
        currentAnnRef.current = { kind: "bubble", x: pos.x, y: pos.y, n, color: strokeColor, size: strokeSize };
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
      const overBubble = activeTool === null && annotationsRef.current.some(
        (a) => a.kind === "bubble" && Math.sqrt((pos.x - a.x) ** 2 + (pos.y - a.y) ** 2) <= 12 + a.size * 2
      );
      const newCursor = handle
        ? getHandleCursor(handle)
        : activeTool === "text" ? "text"
        : overBubble ? "grab"
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
        case "bubble":
          currentAnnRef.current = { ...ann, tail: pos };
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
      } else if (ann.kind === "bubble") {
        const dx = pos.x - ann.x;
        const dy = pos.y - ann.y;
        const dist = Math.sqrt(dx * dx + dy * dy);
        // Si el drag fue corto es un click → sin cola; si fue largo → con cono
        finalAnn = dist < 8 ? { ...ann, tail: undefined } : { ...ann, tail: pos };
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
    const TOOLBAR_W = 624;
    const TOOLBAR_H = 88;
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
  const toolDefs: { tool: Tool; icon: React.ReactNode; title: string; key: string }[] = [
    { tool: "arrow",     icon: <ArrowUpRight size={15} />,   title: "Arrow [A]",                   key: "A" },
    { tool: "rect",      icon: <Square size={14} />,         title: "Rectangle [S] (Ctrl=square)", key: "S" },
    { tool: "circle",    icon: <CircleIcon size={14} />,     title: "Circle [C] (Ctrl=perfect)",   key: "C" },
    { tool: "marker",    icon: <Pencil size={14} />,         title: "Marker [P]",                  key: "P" },
    { tool: "highlight", icon: <Highlighter size={14} />,    title: "Highlighter [H]",             key: "H" },
    { tool: "text",      icon: <Type size={14} />,           title: "Text [T]",                    key: "T" },
    { tool: "blur",      icon: <Blend size={14} />,          title: "Blur [B]",                    key: "B" },
    { tool: "invert",    icon: <Contrast size={14} />,       title: "Invert colors [I]",           key: "I" },
    { tool: "bubble",    icon: <MessageCircle size={14} />,  title: "Numbered bubble [N]",         key: "N" },
    { tool: "ruler",     icon: <Ruler size={14} />,          title: "Ruler [L] (measure px)",      key: "L" },
  ];

  return (
    <div
      style={{ position: "fixed", inset: 0, overflow: "hidden", userSelect: "none" }}
      onContextMenu={(e) => e.preventDefault()}
      onMouseDown={(e) => {
        // Cerrar paleta de colores flotante al hacer click fuera de ella
        if (colorPickerPosRef.current !== null && e.button === 0) {
          colorPickerPosRef.current = null;
          setColorPickerPos(null);
        }
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

      {/* Annotation toolbar — two rows */}
      {displayPhase === "annotating" && (
        <div
          style={{
            position: "absolute",
            ...getToolbarStyle(),
            background: "rgba(16, 16, 18, 0.95)",
            border: "1px solid rgba(255,255,255,0.1)",
            borderRadius: 12,
            backdropFilter: "blur(20px)",
            zIndex: 100,
            boxShadow: "0 6px 32px rgba(0,0,0,0.55)",
            overflow: "hidden",
          }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {/* ── Row 1: Annotation tools ── */}
          <div style={{ display: "flex", alignItems: "center", gap: 3, padding: "7px 10px" }}>
            {toolDefs.map(({ tool, icon, title, key }) => {
              const isActive = activeTool === tool;
              const isHov = hoveredTool === tool && !isActive;
              return (
              <button
                key={tool}
                onClick={() => setActiveTool(isActive ? null : tool)}
                onMouseEnter={() => setHoveredTool(tool)}
                onMouseLeave={() => setHoveredTool(null)}
                title={title}
                style={{
                  background: isActive ? "rgba(74,222,128,0.18)" : isHov ? "rgba(255,255,255,0.07)" : "transparent",
                  border: isActive ? "1px solid #4ade80" : isHov ? "1px solid rgba(255,255,255,0.12)" : "1px solid transparent",
                  borderRadius: 6,
                  color: isActive ? "#4ade80" : isHov ? "#e5e7eb" : "#d1d5db",
                  cursor: "pointer",
                  width: 28,
                  height: 32,
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "center",
                  justifyContent: "center",
                  gap: 1,
                  fontSize: 14,
                  fontWeight: "bold",
                  flexShrink: 0,
                  position: "relative",
                  paddingBottom: 2,
                  transition: "background 0.1s, border-color 0.1s, color 0.1s",
                }}
              >
                <span style={{
                  width: 16, height: 16,
                  display: "flex", alignItems: "center", justifyContent: "center",
                  flexShrink: 0,
                }}>
                  {icon}
                </span>
                <span style={{
                  fontSize: 7,
                  fontWeight: "normal",
                  fontFamily: "monospace",
                  opacity: activeTool === tool ? 0.7 : 0.35,
                  lineHeight: 1,
                  display: "block",
                  width: "100%",
                  textAlign: "center",
                  marginTop: 2,
                }}>
                  {key}
                </span>
              </button>
              );
            })}

            <div style={{ width: 1, height: 20, background: "rgba(255,255,255,0.1)", margin: "0 4px", flexShrink: 0 }} />

            {/* Color presets */}
            {PRESET_COLORS.map((c) => (
              <button
                key={c}
                onClick={() => setStrokeColor(c)}
                className="ov-preset-btn"
                style={{
                  width: 15,
                  height: 15,
                  borderRadius: "50%",
                  background: c,
                  border: strokeColor === c ? "2px solid #fff" : "1px solid rgba(255,255,255,0.2)",
                  cursor: "pointer",
                  padding: 0,
                  flexShrink: 0,
                }}
              />
            ))}

            <button
              onMouseDown={(e) => {
                e.stopPropagation();
                if (colorPickerPosRef.current !== null) {
                  colorPickerPosRef.current = null;
                  setColorPickerPos(null);
                } else {
                  const rect = e.currentTarget.getBoundingClientRect();
                  setColorPickerHex(strokeColor);
                  const above = rect.top > window.innerHeight / 2;
                  const pos = { x: rect.left + rect.width / 2, y: above ? rect.top : rect.bottom, above };
                  colorPickerPosRef.current = pos;
                  setColorPickerPos(pos);
                }
              }}
              title="Custom color"
              className="ov-picker-btn"
              style={{
                width: 30,
                height: 30,
                borderRadius: 6,
                background: colorPickerPos !== null ? "rgba(255,255,255,0.12)" : "transparent",
                border: colorPickerPos !== null ? "1px solid rgba(255,255,255,0.5)" : "1px solid transparent",
                cursor: "pointer",
                padding: 0,
                flexShrink: 0,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 3,
              }}
            >
              <span style={{
                width: 12,
                height: 12,
                borderRadius: 3,
                background: strokeColor,
                border: "1.5px solid rgba(255,255,255,0.4)",
                display: "inline-block",
                flexShrink: 0,
              }} />
              <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="#e5e7eb" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
                <path d="M8.5 1.5l2 2-6 6H2.5v-2l6-6z"/>
              </svg>
            </button>

            <div style={{ width: 1, height: 20, background: "rgba(255,255,255,0.1)", margin: "0 4px", flexShrink: 0 }} />

            {/* Stroke size */}
            {([{ v: 2, label: "S" }, { v: 4, label: "M" }, { v: 6, label: "L" }] as const).map(({ v, label }) => {
              const isActive = strokeSize === v;
              const isHov = hoveredSize === v && !isActive;
              return (
              <button
                key={v}
                onClick={() => setStrokeSize(v)}
                onMouseEnter={() => setHoveredSize(v)}
                onMouseLeave={() => setHoveredSize(null)}
                style={{
                  background: isActive ? "rgba(255,255,255,0.15)" : isHov ? "rgba(255,255,255,0.07)" : "transparent",
                  border: isActive ? "1px solid rgba(255,255,255,0.45)" : isHov ? "1px solid rgba(255,255,255,0.18)" : "1px solid transparent",
                  borderRadius: 5,
                  color: isActive ? "#fff" : isHov ? "#d1d5db" : "#6b7280",
                  cursor: "pointer",
                  width: 22,
                  height: 22,
                  fontSize: 10,
                  fontWeight: "bold",
                  flexShrink: 0,
                  transition: "background 0.1s, border-color 0.1s, color 0.1s",
                }}
              >
                {label}
              </button>
              );
            })}
          </div>

          {/* ── Divider ── */}
          <div style={{ height: 1, background: "rgba(255,255,255,0.07)", margin: "0 10px" }} />

          {/* ── Row 2: Actions ── */}
          <div style={{ display: "flex", alignItems: "center", gap: 3, padding: "5px 10px" }}>
            {/* Selection size — info, left side */}
            <span style={{
              color: "#4b5563",
              fontSize: 10,
              fontFamily: "monospace",
              whiteSpace: "nowrap",
              userSelect: "none",
              letterSpacing: "0.02em",
            }}>
              {getSelectionSize()}
            </span>

            <div style={{ flex: 1 }} />

            {/* Undo */}
            <button
              onClick={() => {
                annotationsRef.current = annotationsRef.current.slice(0, -1);
                setAnnCount(annotationsRef.current.length);
                draw();
              }}
              disabled={annCount === 0}
              title="Undo (Ctrl+Z)"
              className="ov-action-btn"
              style={{
                background: "transparent",
                border: "1px solid transparent",
                borderRadius: 6,
                color: annCount === 0 ? "#374151" : "#9ca3af",
                cursor: annCount === 0 ? "default" : "pointer",
                width: 28,
                height: 28,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <Undo2 size={14} />
            </button>

            <div style={{ width: 1, height: 18, background: "rgba(255,255,255,0.08)", margin: "0 4px", flexShrink: 0 }} />

            {/* Save to file */}
            <button
              onClick={saveCapture}
              title="Save as PNG"
              className="ov-action-btn"
              style={{
                background: "transparent",
                border: "1px solid rgba(255,255,255,0.1)",
                borderRadius: 6,
                color: "#9ca3af",
                cursor: "pointer",
                width: 28,
                height: 28,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <Save size={14} />
            </button>

            {/* Pin */}
            <button
              onClick={pinCapture}
              title="Pin on screen"
              className="ov-action-btn"
              style={{
                background: "transparent",
                border: "1px solid rgba(255,255,255,0.1)",
                borderRadius: 6,
                color: "#9ca3af",
                cursor: "pointer",
                width: 28,
                height: 28,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <Pin size={14} />
            </button>

            <div style={{ width: 1, height: 18, background: "rgba(255,255,255,0.08)", margin: "0 4px", flexShrink: 0 }} />

            {/* Capture — primary action */}
            <button
              onClick={exportCapture}
              title="Copy to clipboard (Ctrl+C)"
              className="ov-capture-btn"
              style={{
                background: "rgba(74,222,128,0.15)",
                border: "1px solid rgba(74,222,128,0.6)",
                borderRadius: 7,
                color: "#4ade80",
                cursor: "pointer",
                padding: "0 14px",
                height: 28,
                fontSize: 12,
                fontWeight: "600",
                whiteSpace: "nowrap",
                letterSpacing: "0.01em",
              }}
            >
              Capture
            </button>

            {/* Esc */}
            <button
              onClick={async () => { reset(); await closeOverlay(); }}
              title="Cancel (Esc)"
              className="ov-esc-btn"
              style={{
                background: "transparent",
                border: "1px solid rgba(255,255,255,0.1)",
                borderRadius: 6,
                color: "#4b5563",
                cursor: "pointer",
                padding: "0 10px",
                height: 28,
                fontSize: 11,
                whiteSpace: "nowrap",
              }}
            >
              Esc
            </button>
          </div>
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

      {/* Paleta de colores flotante (click derecho) */}
      {colorPickerPos && (() => {
        const PAD = 8;
        const W = 160;
        const H = 148;
        const left = Math.min(colorPickerPos.x, window.innerWidth  - W - PAD);
        const goAbove = colorPickerPos.above !== undefined
          ? colorPickerPos.above
          : colorPickerPos.y + PAD + H > window.innerHeight;
        const top  = goAbove
          ? colorPickerPos.y - H - PAD
          : colorPickerPos.y + PAD;
        return (
          <div
            style={{
              position: "fixed",
              left,
              top,
              width: W,
              background: "rgba(18,18,20,0.97)",
              border: "1px solid rgba(255,255,255,0.12)",
              borderRadius: 10,
              padding: 10,
              boxShadow: "0 8px 32px rgba(0,0,0,0.6)",
              zIndex: 400,
            }}
            onMouseDown={(e) => e.stopPropagation()}
            onContextMenu={(e) => e.preventDefault()}
          >
            <div style={{ display: "grid", gridTemplateColumns: "repeat(6, 1fr)", gap: 5, marginBottom: 8 }}>
              {PALETTE.map((c) => (
                <button
                  key={c}
                  onClick={() => {
                    setStrokeColor(c);
                    colorPickerPosRef.current = null;
                    setColorPickerPos(null);
                  }}
                  style={{
                    width: 18,
                    height: 18,
                    borderRadius: 3,
                    background: c,
                    border: strokeColor === c ? "2px solid #fff" : "1px solid rgba(255,255,255,0.2)",
                    cursor: "pointer",
                    padding: 0,
                  }}
                />
              ))}
            </div>
            <div style={{ display: "flex", gap: 5, alignItems: "center" }}>
              <div style={{ width: 18, height: 18, borderRadius: 3, background: colorPickerHex, border: "1px solid rgba(255,255,255,0.2)", flexShrink: 0 }} />
              <input
                type="text"
                value={colorPickerHex}
                spellCheck={false}
                maxLength={7}
                onChange={(e) => {
                  const v = e.target.value;
                  setColorPickerHex(v);
                  if (/^#[0-9a-fA-F]{6}$/.test(v)) setStrokeColor(v);
                }}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") {
                    if (/^#[0-9a-fA-F]{6}$/.test(colorPickerHex)) setStrokeColor(colorPickerHex);
                    colorPickerPosRef.current = null;
                    setColorPickerPos(null);
                  }
                  if (e.key === "Escape") {
                    colorPickerPosRef.current = null;
                    setColorPickerPos(null);
                  }
                }}
                style={{
                  flex: 1,
                  background: "rgba(255,255,255,0.07)",
                  border: "1px solid rgba(255,255,255,0.15)",
                  borderRadius: 5,
                  color: "#e5e7eb",
                  fontSize: 11,
                  fontFamily: "monospace",
                  padding: "3px 6px",
                  outline: "none",
                  minWidth: 0,
                }}
              />
            </div>
          </div>
        );
      })()}

    </div>
  );
}

export default CaptureOverlay;
