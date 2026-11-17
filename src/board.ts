// Whiteboard: infinite pan/zoom canvas with vector elements. Plain Canvas 2D, no dependencies.
import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";

type Tool = "sel" | "pen" | "line" | "arrow" | "rect" | "ellipse" | "text" | "note";
type Kind = Exclude<Tool, "sel"> | "image" | "video";
interface Seed { t: string; rel: string; text: string; w: number; h: number; }
interface E {
  id: number; k: Kind; x: number; y: number; w: number; h: number;
  c: string; f: string; s: number; pts?: number[]; ow?: number; oh?: number; t?: string; src?: string; vid?: string;
}

const cv = document.getElementById("cv") as HTMLCanvasElement;
const ctx = cv.getContext("2d")!;
const ta = document.getElementById("ta") as HTMLTextAreaElement;
const bar = document.getElementById("bar")!;
const props = document.getElementById("props")!;
const statusEl = document.getElementById("status")!;

let els: E[] = [];
let nextId = 1;
let tool: Tool = "sel";
let sel: number | null = null;
let color = "auto";
let fill = false;
let size = 3;
let pan = { x: 0, y: 0 }, zoom = 1;
let dirty = false;
let existing = false;
const undo: E[][] = [], redo: E[][] = [];
const imgs = new Map<number, HTMLImageElement>();
const vids = new Map<number, HTMLVideoElement>();
const mediaUrl = (rel: string) => `http://stash.localhost/media/${rel}`;
let exporting = false, skipVideo = false, raf = 0;
let saveTimer: number | undefined;

const dark = () => document.documentElement.dataset.theme !== "light";
const ink = (c: string) => (c === "auto" ? (dark() ? "#f2f2f6" : "#1b1b21") : c);
const bg = () => (dark() ? "#1b1b22" : "#fbfbfd");
const FONT = '"Segoe UI Variable Text","Segoe UI",system-ui,sans-serif';
const fsOf = (e: E) => 12 + e.s * 3;

// ---------- history ----------
const snap = () => els.map((e) => ({ ...e }));
function commit() {
  undo.push(snap());
  if (undo.length > 100) undo.shift();
  redo.length = 0;
  changed();
}
function changed() {
  dirty = true;
  statusEl.textContent = "Unsaved";
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => save(false), 2500);
  draw();
}
function restore(list: E[]) {
  els = list.map((e) => ({ ...e }));
  if (sel !== null && !els.some((e) => e.id === sel)) sel = null;
  els.forEach(ensureImg);
  els.forEach(ensureVid);
  changed();
}
function doUndo() { if (undo.length) { redo.push(snap()); restore(undo.pop()!); } }
function doRedo() { if (redo.length) { undo.push(snap()); restore(redo.pop()!); } }

// ---------- geometry ----------
const toWorld = (sx: number, sy: number) => ({ x: (sx - pan.x) / zoom, y: (sy - pan.y) / zoom });
function bounds(e: E) {
  return { x: Math.min(e.x, e.x + e.w), y: Math.min(e.y, e.y + e.h), w: Math.abs(e.w), h: Math.abs(e.h) };
}
function hit(e: E, p: { x: number; y: number }) {
  const b = bounds(e), m = 6 / zoom;
  if (e.k === "line" || e.k === "arrow") {
    const [x1, y1, x2, y2] = [e.x, e.y, e.x + e.w, e.y + e.h];
    const l2 = (x2 - x1) ** 2 + (y2 - y1) ** 2 || 1;
    const t = Math.max(0, Math.min(1, ((p.x - x1) * (x2 - x1) + (p.y - y1) * (y2 - y1)) / l2));
    return Math.hypot(p.x - (x1 + t * (x2 - x1)), p.y - (y1 + t * (y2 - y1))) < m + e.s;
  }
  return p.x >= b.x - m && p.x <= b.x + b.w + m && p.y >= b.y - m && p.y <= b.y + b.h + m;
}
const top = (p: { x: number; y: number }) => { for (let i = els.length - 1; i >= 0; i--) if (hit(els[i], p)) return els[i]; return null; };

// ---------- rendering ----------
function ensureImg(e: E) {
  if (e.k !== "image" || !e.src || imgs.has(e.id)) return;
  const im = new Image();
  im.onload = () => draw();
  im.src = e.src;
  imgs.set(e.id, im);
}

function ensureVid(e: E) {
  if (e.k !== "video" || !e.vid || vids.has(e.id)) return;
  const v = document.createElement("video");
  v.crossOrigin = "anonymous"; v.muted = true; v.loop = true; v.preload = "auto"; v.playsInline = true;
  v.addEventListener("loadeddata", () => { v.currentTime = 0.05; draw(); });
  v.addEventListener("seeked", () => draw());
  v.src = mediaUrl(e.vid);
  vids.set(e.id, v);
}

function toggleVideo(e: E) {
  const v = vids.get(e.id);
  if (!v) return;
  if (v.paused) { v.play().catch(() => {}); draw(); } else { v.pause(); draw(); }
}

function wrap(c: CanvasRenderingContext2D, text: string, maxW: number): string[] {
  const out: string[] = [];
  for (const para of text.split("\n")) {
    let line = "";
    for (const word of para.split(" ")) {
      const t = line ? line + " " + word : word;
      if (line && c.measureText(t).width > maxW) { out.push(line); line = word; } else line = t;
    }
    out.push(line);
  }
  return out;
}

function drawEl(c: CanvasRenderingContext2D, e: E) {
  c.save();
  c.lineWidth = e.s; c.lineCap = "round"; c.lineJoin = "round";
  c.strokeStyle = ink(e.c); c.fillStyle = e.f ? ink(e.f) : "transparent";
  switch (e.k) {
    case "pen": {
      const p = e.pts!;
      if (p.length < 2) break;
      const sx = e.ow ? e.w / e.ow : 1, sy = e.oh ? e.h / e.oh : 1;
      c.beginPath();
      c.moveTo(e.x + p[0] * sx, e.y + p[1] * sy);
      for (let i = 2; i < p.length - 2; i += 2) {
        c.quadraticCurveTo(e.x + p[i] * sx, e.y + p[i + 1] * sy, e.x + ((p[i] + p[i + 2]) / 2) * sx, e.y + ((p[i + 1] + p[i + 3]) / 2) * sy);
      }
      c.lineTo(e.x + p[p.length - 2] * sx, e.y + p[p.length - 1] * sy);
      c.stroke();
      break;
    }
    case "line": case "arrow": {
      c.beginPath(); c.moveTo(e.x, e.y); c.lineTo(e.x + e.w, e.y + e.h); c.stroke();
      if (e.k === "arrow") {
        const a = Math.atan2(e.h, e.w), L = 10 + e.s * 2.5;
        c.beginPath();
        c.moveTo(e.x + e.w - L * Math.cos(a - 0.45), e.y + e.h - L * Math.sin(a - 0.45));
        c.lineTo(e.x + e.w, e.y + e.h);
        c.lineTo(e.x + e.w - L * Math.cos(a + 0.45), e.y + e.h - L * Math.sin(a + 0.45));
        c.stroke();
      }
      break;
    }
    case "rect": {
      const b = bounds(e);
      c.beginPath(); c.roundRect(b.x, b.y, b.w, b.h, 6); if (e.f) c.fill(); c.stroke();
      break;
    }
    case "ellipse": {
      const b = bounds(e);
      c.beginPath(); c.ellipse(b.x + b.w / 2, b.y + b.h / 2, b.w / 2, b.h / 2, 0, 0, Math.PI * 2); if (e.f) c.fill(); c.stroke();
      break;
    }
    case "text": {
      c.font = `${fsOf(e)}px ${FONT}`; c.fillStyle = ink(e.c); c.textBaseline = "top";
      (e.t ?? "").split("\n").forEach((l, i) => c.fillText(l, e.x, e.y + i * fsOf(e) * 1.3));
      break;
    }
    case "note": {
      const b = bounds(e);
      c.shadowColor = "rgba(0,0,0,.25)"; c.shadowBlur = 10; c.shadowOffsetY = 3;
      c.fillStyle = e.f ? ink(e.f) : "#ffd966"; c.beginPath(); c.roundRect(b.x, b.y, b.w, b.h, 4); c.fill();
      c.shadowColor = "transparent";
      c.font = `${fsOf(e)}px ${FONT}`; c.fillStyle = "#222"; c.textBaseline = "top";
      c.save(); c.beginPath(); c.rect(b.x, b.y, b.w, b.h); c.clip();
      wrap(c, e.t ?? "", b.w - 20).forEach((l, i) => c.fillText(l, b.x + 10, b.y + 10 + i * fsOf(e) * 1.3));
      c.restore();
      break;
    }
    case "image": {
      const im = imgs.get(e.id);
      if (im?.complete) c.drawImage(im, e.x, e.y, e.w, e.h);
      break;
    }
    case "video": {
      const v = vids.get(e.id);
      if (v && v.readyState >= 2 && !skipVideo) c.drawImage(v, e.x, e.y, e.w, e.h);
      else { c.fillStyle = "#111"; c.fillRect(e.x, e.y, e.w, e.h); }
      if (!exporting && (!v || v.paused)) {
        const r = 24 / zoom, cx = e.x + e.w / 2, cy = e.y + e.h / 2;
        c.fillStyle = "rgba(0,0,0,.55)"; c.beginPath(); c.arc(cx, cy, r, 0, Math.PI * 2); c.fill();
        c.fillStyle = "#fff"; c.beginPath(); c.moveTo(cx - r * 0.3, cy - r * 0.5); c.lineTo(cx + r * 0.55, cy); c.lineTo(cx - r * 0.3, cy + r * 0.5); c.fill();
      }
      break;
    }
  }
  c.restore();
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  if (cv.width !== Math.round(innerWidth * dpr) || cv.height !== Math.round(innerHeight * dpr)) {
    cv.width = Math.round(innerWidth * dpr); cv.height = Math.round(innerHeight * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = bg(); ctx.fillRect(0, 0, innerWidth, innerHeight);
  // dot grid
  const step = 32 * zoom > 12 ? 32 : 128;
  ctx.fillStyle = dark() ? "rgba(255,255,255,.09)" : "rgba(0,0,0,.1)";
  const x0 = Math.floor(-pan.x / zoom / step) * step, y0 = Math.floor(-pan.y / zoom / step) * step;
  for (let x = x0; x * zoom + pan.x < innerWidth; x += step)
    for (let y = y0; y * zoom + pan.y < innerHeight; y += step) ctx.fillRect(x * zoom + pan.x - 1, y * zoom + pan.y - 1, 2, 2);
  ctx.setTransform(dpr * zoom, 0, 0, dpr * zoom, dpr * pan.x, dpr * pan.y);
  els.forEach((e) => drawEl(ctx, e));
  if (draft) drawEl(ctx, draft);
  const s = els.find((e) => e.id === sel);
  if (s) {
    const b = bounds(s), accent = getComputedStyle(document.documentElement).getPropertyValue("--hover") || "#4a7dff";
    ctx.strokeStyle = accent.trim(); ctx.lineWidth = 1.5 / zoom; ctx.setLineDash([5 / zoom, 4 / zoom]);
    ctx.strokeRect(b.x - 4 / zoom, b.y - 4 / zoom, b.w + 8 / zoom, b.h + 8 / zoom); ctx.setLineDash([]);
    if (s.k !== "line" && s.k !== "arrow" && s.k !== "text") { ctx.fillStyle = accent.trim(); ctx.fillRect(b.x + b.w - 4 / zoom, b.y + b.h - 4 / zoom, 10 / zoom, 10 / zoom); }
  }
  document.getElementById("zoomv")!.textContent = `${Math.round(zoom * 100)}%`;
  if (!raf && [...vids.values()].some((v) => !v.paused)) raf = requestAnimationFrame(() => { raf = 0; draw(); });
}

// ---------- pointer ----------
let draft: E | null = null;
let drag: null | { mode: "pan" | "move" | "resize"; sx: number; sy: number; ox: number; oy: number; ow: number; oh: number } = null;
let spaceDown = false;

function add(e: Omit<E, "id">): E { const el = { ...e, id: nextId++ } as E; els.push(el); return el; }

cv.addEventListener("pointerdown", (ev) => {
  if (ta.style.display === "block") ta.blur();
  cv.setPointerCapture(ev.pointerId);
  const p = toWorld(ev.clientX, ev.clientY);
  if (ev.button === 1 || spaceDown) { drag = { mode: "pan", sx: ev.clientX, sy: ev.clientY, ox: pan.x, oy: pan.y, ow: 0, oh: 0 }; return; }
  if (ev.button !== 0) return;
  if (tool === "sel") {
    const s = els.find((e) => e.id === sel);
    if (s) {
      const b = bounds(s);
      if (Math.abs(p.x - (b.x + b.w)) < 10 / zoom && Math.abs(p.y - (b.y + b.h)) < 10 / zoom && s.k !== "line" && s.k !== "arrow" && s.k !== "text") {
        drag = { mode: "resize", sx: p.x, sy: p.y, ox: s.x, oy: s.y, ow: s.w, oh: s.h }; return;
      }
    }
    const h = top(p);
    sel = h?.id ?? null;
    syncProps();
    if (h) drag = { mode: "move", sx: p.x, sy: p.y, ox: h.x, oy: h.y, ow: 0, oh: 0 };
    else drag = { mode: "pan", sx: ev.clientX, sy: ev.clientY, ox: pan.x, oy: pan.y, ow: 0, oh: 0 };
    draw();
    return;
  }
  if (tool === "text" || tool === "note") {
    const e = add({ k: tool, x: p.x, y: p.y, w: tool === "note" ? 200 : 0, h: tool === "note" ? 150 : 0, c: color, f: "", s: size, t: "" });
    sel = e.id; edit(e, true); return;
  }
  const base = { x: p.x, y: p.y, w: 0, h: 0, c: color, f: fill && (tool === "rect" || tool === "ellipse") ? color === "auto" ? (dark() ? "#3a3a4a" : "#e4e4ee") : color + "55" : "", s: size };
  draft = { id: -1, k: tool, ...base, ...(tool === "pen" ? { pts: [0, 0], ow: 1, oh: 1 } : {}) };
});

cv.addEventListener("pointermove", (ev) => {
  const p = toWorld(ev.clientX, ev.clientY);
  if (drag) {
    if (drag.mode === "pan") { pan = { x: drag.ox + ev.clientX - drag.sx, y: drag.oy + ev.clientY - drag.sy }; draw(); return; }
    const s = els.find((e) => e.id === sel);
    if (!s) return;
    if (drag.mode === "move") { s.x = drag.ox + p.x - drag.sx; s.y = drag.oy + p.y - drag.sy; }
    else {
      const b = bounds(s);
      s.w = Math.max(8, drag.ow + p.x - drag.sx); s.h = Math.max(8, drag.oh + p.y - drag.sy);
      if ((s.k === "image" || s.k === "video") && ev.shiftKey === false && drag.oh) s.h = s.w * (drag.oh / drag.ow);
      void b;
    }
    draw(); return;
  }
  if (!draft) return;
  if (draft.k === "pen") {
    const px = p.x - draft.x, py = p.y - draft.y;
    draft.pts!.push(px, py);
    const xs = draft.pts!.filter((_, i) => i % 2 === 0), ys = draft.pts!.filter((_, i) => i % 2 === 1);
    draft.w = Math.max(...xs) - Math.min(...xs) || 1; draft.h = Math.max(...ys) - Math.min(...ys) || 1;
    draft.ow = draft.w; draft.oh = draft.h;
  } else {
    draft.w = p.x - draft.x; draft.h = p.y - draft.y;
    if (ev.shiftKey && (draft.k === "rect" || draft.k === "ellipse")) { const m = Math.max(Math.abs(draft.w), Math.abs(draft.h)); draft.w = Math.sign(draft.w || 1) * m; draft.h = Math.sign(draft.h || 1) * m; }
  }
  draw();
});

function normalizePen(e: E) {
  // translate points so (x, y) is the bbox top-left; keeps hit-testing and scaling simple
  const p = e.pts!;
  let minx = Infinity, miny = Infinity;
  for (let i = 0; i < p.length; i += 2) { minx = Math.min(minx, p[i]); miny = Math.min(miny, p[i + 1]); }
  for (let i = 0; i < p.length; i += 2) { p[i] -= minx; p[i + 1] -= miny; }
  e.x += minx; e.y += miny;
}

cv.addEventListener("pointerup", () => {
  if (drag) {
    const moved = drag.mode !== "pan";
    drag = null;
    if (moved) commit();
    return;
  }
  if (!draft) return;
  const d = draft; draft = null;
  const tiny = d.k === "pen" ? (d.pts!.length < 4) : Math.abs(d.w) < 3 && Math.abs(d.h) < 3;
  if (tiny) { draw(); return; }
  if (d.k === "pen") normalizePen(d);
  const e = add({ ...d } as E);
  sel = e.id; tool = "sel"; syncTools(); syncProps();
  commit();
});

cv.addEventListener("dblclick", (ev) => {
  const h = top(toWorld(ev.clientX, ev.clientY));
  if (h && (h.k === "text" || h.k === "note")) { sel = h.id; edit(h, false); }
  else if (h?.k === "video") { sel = h.id; toggleVideo(h); }
});

cv.addEventListener("wheel", (ev) => {
  ev.preventDefault();
  if (ev.ctrlKey) {
    const p = toWorld(ev.clientX, ev.clientY);
    zoom = Math.max(0.1, Math.min(8, zoom * Math.exp(-ev.deltaY * 0.0015)));
    pan = { x: ev.clientX - p.x * zoom, y: ev.clientY - p.y * zoom };
  } else {
    pan = { x: pan.x - (ev.shiftKey ? ev.deltaY : ev.deltaX), y: pan.y - (ev.shiftKey ? 0 : ev.deltaY) };
  }
  draw();
}, { passive: false });

// ---------- text editing ----------
let editing: { e: E; isNew: boolean } | null = null;
function edit(e: E, isNew: boolean) {
  editing = { e, isNew };
  const sx = e.x * zoom + pan.x, sy = e.y * zoom + pan.y;
  const noteBox = e.k === "note";
  Object.assign(ta.style, {
    display: "block", left: sx + "px", top: sy + "px", fontSize: fsOf(e) * zoom + "px", lineHeight: "1.3",
    color: noteBox ? "#222" : ink(e.c), background: noteBox ? "#ffd966" : "transparent",
    width: noteBox ? Math.abs(e.w) * zoom + "px" : "260px", height: noteBox ? Math.abs(e.h) * zoom + "px" : fsOf(e) * 1.3 * zoom * 3 + "px",
    whiteSpace: noteBox ? "pre-wrap" : "pre", padding: noteBox ? "10px" : "2px",
  });
  ta.value = e.t ?? "";
  ta.focus();
  draw();
}
function endEdit() {
  if (!editing) return;
  const { e, isNew } = editing; editing = null;
  e.t = ta.value; ta.style.display = "none";
  if (!e.t.trim()) { els = els.filter((x) => x !== e); sel = null; if (!isNew) commit(); draw(); return; }
  if (e.k === "text") {
    ctx.font = `${fsOf(e)}px ${FONT}`;
    const lines = e.t.split("\n");
    e.w = Math.max(...lines.map((l) => ctx.measureText(l).width)); e.h = lines.length * fsOf(e) * 1.3;
  }
  tool = "sel"; syncTools(); commit();
}
ta.addEventListener("blur", endEdit);
ta.addEventListener("keydown", (e) => { if (e.key === "Escape") { e.preventDefault(); ta.blur(); } e.stopPropagation(); });

// ---------- keyboard ----------
window.addEventListener("keydown", (e) => {
  if (e.code === "Space") { spaceDown = true; cv.style.cursor = "grab"; e.preventDefault(); return; }
  if (e.ctrlKey && e.key.toLowerCase() === "z") { e.shiftKey ? doRedo() : doUndo(); e.preventDefault(); return; }
  if (e.ctrlKey && e.key.toLowerCase() === "y") { doRedo(); e.preventDefault(); return; }
  if (e.ctrlKey && e.key.toLowerCase() === "s") { e.preventDefault(); save(true); return; }
  if (e.ctrlKey && e.key.toLowerCase() === "d" && sel !== null) {
    e.preventDefault();
    const s = els.find((x) => x.id === sel)!;
    const n = add({ ...s, x: s.x + 20, y: s.y + 20, pts: s.pts ? [...s.pts] : undefined });
    if (n.k === "image") imgs.set(n.id, imgs.get(s.id)!);
    ensureVid(n);
    sel = n.id; commit(); return;
  }
  if ((e.key === "Delete" || e.key === "Backspace") && sel !== null) { els = els.filter((x) => x.id !== sel); sel = null; commit(); return; }
  if (e.key === "Escape") { sel = null; tool = "sel"; syncTools(); draw(); return; }
  if (e.ctrlKey || e.altKey) return;
  const map: Record<string, Tool> = { v: "sel", p: "pen", l: "line", a: "arrow", r: "rect", o: "ellipse", t: "text", n: "note" };
  const t = map[e.key.toLowerCase()];
  if (t) { tool = t; syncTools(); }
  if (e.key === "0") fit();
});
window.addEventListener("keyup", (e) => { if (e.code === "Space") { spaceDown = false; cv.style.cursor = "crosshair"; } });
window.addEventListener("resize", draw);

// ---------- images ----------
function addImage(src: string) {
  const im = new Image();
  im.onload = () => {
    const k = Math.min(1, 480 / Math.max(im.width, im.height));
    const m = toWorld(innerWidth / 2, innerHeight / 2);
    const e = add({ k: "image", x: m.x - (im.width * k) / 2, y: m.y - (im.height * k) / 2, w: im.width * k, h: im.height * k, c: "auto", f: "", s: 1, src });
    imgs.set(e.id, im);
    sel = e.id; commit();
  };
  im.src = src;
}
window.addEventListener("paste", (e) => {
  const item = [...(e.clipboardData?.items ?? [])].find((i) => i.type.startsWith("image/"));
  if (!item) return;
  const f = item.getAsFile();
  if (!f) return;
  const r = new FileReader();
  r.onload = () => addImage(String(r.result));
  r.readAsDataURL(f);
});
getCurrentWebview().onDragDropEvent(async (ev) => {
  if (ev.payload.type !== "drop") return;
  for (const p of ev.payload.paths) {
    if (/\.(mp4|m4v|webm|mov)$/i.test(p)) {
      try { await addSeeds([{ t: "video", rel: await invoke<string>("board_add_video", { path: p }), text: "", w: 0, h: 0 }]); } catch (err) { statusEl.textContent = String(err); }
      continue;
    }
    try { addImage(await invoke<string>("board_read_image", { path: p })); } catch { /* not an image */ }
  }
});

// ---------- view / export / save ----------
function fit() {
  if (!els.length) { pan = { x: 0, y: 0 }; zoom = 1; draw(); return; }
  const r = extent();
  zoom = Math.min(2, Math.min(innerWidth / (r.w + 160), innerHeight / (r.h + 200)));
  pan = { x: innerWidth / 2 - (r.x + r.w / 2) * zoom, y: innerHeight / 2 - (r.y + r.h / 2) * zoom };
  draw();
}
function extent() {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const e of els) { const b = bounds(e); x0 = Math.min(x0, b.x); y0 = Math.min(y0, b.y); x1 = Math.max(x1, b.x + b.w); y1 = Math.max(y1, b.y + b.h); }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}
function render(): string {
  exporting = true;
  try { return renderOnce(); } catch {
    skipVideo = true; // a video that taints the canvas is exported as a black box instead of failing the save
    try { return renderOnce(); } finally { skipVideo = false; exporting = false; }
  } finally { exporting = false; }
}
function renderOnce(): string {
  const r = extent(), pad = 40;
  let k = 1;
  const big = Math.max(r.w, r.h) + pad * 2;
  if (big > 4096) k = 4096 / big;
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.round((r.w + pad * 2) * k)); c.height = Math.max(1, Math.round((r.h + pad * 2) * k));
  const g = c.getContext("2d")!;
  g.fillStyle = bg(); g.fillRect(0, 0, c.width, c.height);
  g.setTransform(k, 0, 0, k, (pad - r.x) * k, (pad - r.y) * k);
  els.forEach((e) => drawEl(g, e));
  return c.toDataURL("image/png");
}
async function save(notify: boolean) {
  clearTimeout(saveTimer);
  if (!els.length || (!dirty && !notify)) return;
  try {
    await invoke("board_save", { png: render(), scene: JSON.stringify({ v: 1, els, view: { pan, zoom }, next: nextId }), notify });
    dirty = false; existing = true;
    statusEl.textContent = "Saved";
  } catch (err) { statusEl.textContent = "Save failed: " + err; }
}
getCurrentWindow().onCloseRequested(async (ev) => {
  ev.preventDefault();
  await save(false);
  await invoke("board_close");
});

// ---------- seeds (items sent from the gallery, or videos dropped in) ----------
function blobToDataUrl(url: string): Promise<string> {
  return fetch(url).then((r) => r.blob()).then((b) => new Promise<string>((res, rej) => { const f = new FileReader(); f.onload = () => res(String(f.result)); f.onerror = rej; f.readAsDataURL(b); }));
}
function natural(src: string): Promise<{ w: number; h: number }> {
  return new Promise((res) => { const im = new Image(); im.onload = () => res({ w: im.width, h: im.height }); im.onerror = () => res({ w: 320, h: 240 }); im.src = src; });
}
async function addSeeds(seeds: Seed[]) {
  const o = toWorld(90, 110);
  let cx = o.x, cy = o.y, rowH = 0;
  const place = (w: number, h: number) => {
    const k = Math.min(1, 360 / w, 300 / h);
    const [pw, ph] = [w * k, h * k];
    if (cx > o.x && cx + pw > o.x + 1500) { cx = o.x; cy += rowH + 30; rowH = 0; }
    const pos = { x: cx, y: cy, w: pw, h: ph };
    cx += pw + 30; rowH = Math.max(rowH, ph);
    return pos;
  };
  for (const s of seeds) {
    try {
      if (s.t === "image") {
        const src = await blobToDataUrl(mediaUrl(s.rel));
        const n = await natural(src);
        const e = add({ k: "image", ...place(n.w, n.h), c: "auto", f: "", s: 1, src });
        ensureImg(e);
      } else if (s.t === "video") {
        const e = add({ k: "video", ...place(s.w || 640, s.h || 360), c: "auto", f: "", s: 1, vid: s.rel });
        ensureVid(e);
      } else {
        add({ k: "note", ...place(220, 160), c: "auto", f: "", s: 3, t: s.text.slice(0, 400) });
      }
    } catch { /* skip unreadable item */ }
  }
  commit();
  fit();
}
listen<Seed[]>("board-seed", (ev) => addSeeds(ev.payload));

// ---------- toolbar ----------
const ICONS: Record<string, string> = {
  sel: '<path d="M5 3l12 8-5.5 1.5L9 18z"/>',
  pen: '<path d="M4 20l1-4L16 5l3 3L8 19z"/><path d="M14 7l3 3"/>',
  line: '<path d="M5 19L19 5"/>',
  arrow: '<path d="M5 19L19 5M10 5h9v9"/>',
  rect: '<rect x="4" y="6" width="16" height="12" rx="2"/>',
  ellipse: '<ellipse cx="12" cy="12" rx="8" ry="6"/>',
  text: '<path d="M6 6h12M12 6v13M9 19h6"/>',
  note: '<path d="M5 4h14v11l-5 5H5z"/><path d="M14 20v-5h5"/>',
};
const NAMES: Record<string, string> = { sel: "Select (V)", pen: "Pen (P)", line: "Line (L)", arrow: "Arrow (A)", rect: "Rectangle (R)", ellipse: "Ellipse (O)", text: "Text (T)", note: "Sticky note (N)" };
function mk(tag: string, cls = "", html = ""): HTMLElement { const e = document.createElement(tag); if (cls) e.className = cls; if (html) e.innerHTML = html; return e; } // html is static markup only
for (const t of Object.keys(ICONS)) {
  const b = mk("button", "tb", `<svg viewBox="0 0 24 24">${ICONS[t]}</svg>`);
  b.title = NAMES[t]; b.dataset.t = t;
  b.addEventListener("click", () => { tool = t as Tool; syncTools(); });
  bar.append(b);
}
bar.append(mk("div", "sep"));
const undoB = mk("button", "tb", '<svg viewBox="0 0 24 24"><path d="M9 7L4 12l5 5M4 12h10a6 6 0 010 12" transform="translate(0 -4)"/></svg>');
undoB.title = "Undo (Ctrl+Z)"; undoB.onclick = doUndo;
const fitB = mk("button", "tb", '<svg viewBox="0 0 24 24"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/></svg>');
fitB.title = "Zoom to fit (0)"; fitB.onclick = fit;
const saveB = mk("button", "tb", '<svg viewBox="0 0 24 24"><path d="M5 4h11l3 3v13H5z"/><path d="M8 4v6h8V4M8 20v-6h8v6"/></svg>');
saveB.title = "Save (Ctrl+S)"; saveB.onclick = () => save(true);
const z = mk("span"); z.id = "zoomv"; z.id = "zoomv";
const zw = mk("div"); zw.id = "zoom"; zw.append(z);
bar.append(undoB, fitB, saveB, zw);

const COLORS = ["auto", "#ff5a5f", "#ff9f43", "#ffd43b", "#3ddc97", "#38bdf8", "#7c8cff", "#d28bff"];
const colBtns: HTMLElement[] = [];
for (const c of COLORS) {
  const b = mk("button", "col"); b.style.background = c === "auto" ? "linear-gradient(135deg,#fff 50%,#222 50%)" : c; b.title = c === "auto" ? "Theme ink" : c;
  b.addEventListener("click", () => { color = c; const s = els.find((e) => e.id === sel); if (s) { s.c = c; commit(); } syncProps(); });
  colBtns.push(b); props.append(b);
}
const fillB = mk("button", "tb", '<svg viewBox="0 0 24 24"><rect x="5" y="6" width="14" height="12" rx="2" fill="currentColor" fill-opacity=".4"/></svg>');
fillB.title = "Fill shapes"; fillB.onclick = () => { fill = !fill; const s = els.find((e) => e.id === sel); if (s && (s.k === "rect" || s.k === "ellipse")) { s.f = fill ? (s.c === "auto" ? (dark() ? "#3a3a4a" : "#e4e4ee") : s.c + "55") : ""; commit(); } syncProps(); };
const sz = document.createElement("input"); sz.type = "range"; sz.min = "1"; sz.max = "12"; sz.title = "Stroke / text size";
sz.addEventListener("input", () => { size = Number(sz.value); const s = els.find((e) => e.id === sel); if (s) { s.s = size; changed(); } });
sz.addEventListener("change", () => { if (sel !== null) commit(); });
props.append(mk("div", "sep"), fillB, sz);

function syncTools() { bar.querySelectorAll<HTMLElement>("[data-t]").forEach((b) => b.classList.toggle("on", b.dataset.t === tool)); cv.style.cursor = tool === "sel" ? "default" : "crosshair"; draw(); }
function syncProps() {
  const s = els.find((e) => e.id === sel);
  const c = s?.c ?? color;
  colBtns.forEach((b, i) => b.classList.toggle("on", COLORS[i] === c));
  fillB.classList.toggle("on", s ? !!s.f : fill);
  sz.value = String(s?.s ?? size);
  draw();
}

// ---------- start ----------
(async () => {
  try {
    const init = await invoke<{ scene: string | null; existing: boolean; seeds: Seed[] }>("board_init");
    existing = init.existing;
    if (init.scene) {
      const d = JSON.parse(init.scene);
      els = d.els ?? []; nextId = d.next ?? els.reduce((m: number, e: E) => Math.max(m, e.id), 0) + 1;
      els.forEach(ensureImg);
      els.forEach(ensureVid);
      fit();
    }
    if (init.seeds.length) await addSeeds(init.seeds);
  } catch { /* new board */ }
  syncTools(); syncProps();
  statusEl.textContent = existing ? "Saved" : "New board";
  draw();
})();
