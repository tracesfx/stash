// Opening / animation timing for the webview pages. Cheap enough to stay on: a few marks, then a short rAF
// frame recorder for the entrance, summarised into one line in the Stash log (`perf <page>: ...`).
import { invoke } from "@tauri-apps/api/core";

const marks: string[] = [];
const t0 = performance.now();
let longFrames: { start: number; dur: number; block: number; src: string }[] = [];

/** Long Animation Frames: what blocked the main thread (script source + time), when it happened.
 *  STASH_PERF=1 only (the gallery page lives for the whole session). */
function observeLongFrames() {
  try {
    new PerformanceObserver((l) => {
      for (const e of l.getEntries() as (PerformanceEntry & { blockingDuration?: number; scripts?: readonly { sourceFunctionName?: string; invoker?: string; duration: number }[] })[]) {
        const top = [...(e.scripts ?? [])].sort((a, b) => b.duration - a.duration)[0]; // the entry list is frozen
        longFrames.push({ start: Math.round(e.startTime), dur: Math.round(e.duration), block: Math.round(e.blockingDuration ?? 0), src: top ? `${top.invoker || top.sourceFunctionName || "?"}:${Math.round(top.duration)}` : "-" });
      }
      if (longFrames.length > 200) longFrames = longFrames.slice(-100);
    }).observe({ type: "long-animation-frame", buffered: true });
  } catch { /* not supported */ }
}

/** Time since the page's navigation started (ms). */
export function mark(name: string) {
  marks.push(`${name}=${Math.round(performance.now())}`);
}

// The frame recorder runs a rAF loop (main-thread work of its own), so it is opt-in: STASH_PERF=1.
let enabled: boolean | null = null;
const perfOn = invoke<boolean>("perf_enabled").then((v) => { enabled = !!v; if (enabled) observeLongFrames(); return enabled; }).catch(() => (enabled = false));

/** Records frame intervals for `ms`, then logs marks + frame stats under `page` (STASH_PERF=1 only). */
export function recordFrames(page: string, ms: number) {
  if (enabled === null) { perfOn.then((v) => v && recordFrames(page, ms)); return; }
  if (!enabled) { invoke("perf_log", { page, line: marks.join(" ") }).catch(() => {}); marks.length = 0; return; }
  const ds: number[] = [], at: number[] = [];
  let last = 0;
  const start = performance.now();
  longFrames = longFrames.filter((f) => f.start >= start - 50);
  const tick = (t: number) => {
    if (last) { ds.push(t - last); at.push(t - start); }
    last = t;
    if (t - start < ms) requestAnimationFrame(tick);
    else report(page, ds, start, at);
  };
  requestAnimationFrame(tick);
}

function report(page: string, ds: number[], start: number, at: number[]) {
  if (!ds.length) return;
  const sorted = [...ds].sort((a, b) => a - b);
  const med = sorted[Math.floor(sorted.length / 2)];
  const p = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
  const dropped = ds.filter((d) => d > med * 1.5).length;
  const lf = longFrames.filter((f) => f.start >= start - 50).slice(0, 8).map((f) => `${f.start}+${f.dur}(${f.src})`).join(" ");
  const line = `${marks.join(" ")} | frames=${ds.length} med=${med.toFixed(1)} p95=${p(0.95).toFixed(1)} max=${sorted[sorted.length - 1].toFixed(1)} hitches=${dropped} [${ds.map((d, i) => (d > med * 1.5 ? `${Math.round(at[i])}:${d.toFixed(0)}` : "")).filter(Boolean).slice(0, 24).join(" ")}] | loaf: ${lf || "none"} | start=${Math.round(start - t0)}`;
  invoke("perf_log", { page, line }).catch(() => {});
  marks.length = 0;
}
