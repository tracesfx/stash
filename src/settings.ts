import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

interface Config { library_path: string; hotkey: string; hold_ms: number; capture_url: boolean; show_popup: boolean; autostart: boolean; theme: string; glass: string[]; ocr: boolean; appearance: Appearance; rec_system_audio: boolean; rec_mic: boolean; rec_fps: number; rec_max_secs: number; wheel_actions: string[]; }
interface Appearance { [k: string]: number | string | boolean; glass_translucency: number; glass_blur: number; glass_saturation: number; glass_tint: number; hover_color: string; wheel_size: number; wheel_hole: number; wheel_gap: number; wheel_opacity: number; ui_font: number; ui_radius: number; popup_corner: string; popup_ms: number; animations: boolean; }
interface Dto { config: Config; autostart_supported: boolean; version: string; }

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const keys = $("keys"), holdr = $<HTMLInputElement>("holdr"), holdv = $("holdv"), lib = $<HTMLInputElement>("lib");
const ocr = $<HTMLInputElement>("ocr"), autostart = $<HTMLInputElement>("autostart"), popup = $<HTMLInputElement>("popup"), url = $<HTMLInputElement>("url");
const status = $("status"), recBtn = $("rec");

const recsys = $<HTMLInputElement>("recsys"), recmic = $<HTMLInputElement>("recmic"), recfps = $<HTMLInputElement>("recfps"), recfpsv = $("recfpsv"), recmax = $<HTMLInputElement>("recmax");
recfps.addEventListener("input", () => (recfpsv.textContent = recfps.value));

// ---- appearance (applies live) ----
type Def = { k: string; label: string; hint?: string; min: number; max: number; step?: number; unit?: string; glass?: boolean };
const SLIDERS: Def[] = [
  { k: "glass_translucency", label: "Glass translucency", hint: "Frosted (left) to clear (right).", min: 0, max: 100, unit: "%", glass: true },
  { k: "glass_blur", label: "Glass blur", min: 0, max: 60, unit: "px", glass: true },
  { k: "glass_saturation", label: "Glass saturation", min: 100, max: 250, step: 5, unit: "%", glass: true },
  { k: "glass_tint", label: "Colour wash strength", min: 0, max: 100, unit: "%", glass: true },
  { k: "wheel_size", label: "Wheel size", min: 60, max: 160, step: 5, unit: "%" },
  { k: "wheel_hole", label: "Wheel centre size", hint: "Radius of the dead zone in the middle.", min: 20, max: 80, unit: "px" },
  { k: "wheel_gap", label: "Wheel slice gap", min: 0, max: 14, unit: "px" },
  { k: "wheel_opacity", label: "Wheel opacity", min: 30, max: 100, unit: "%" },
  { k: "ui_font", label: "Text size", min: 11, max: 18, unit: "px" },
  { k: "ui_radius", label: "Corner roundness", min: 0, max: 20, unit: "px" },
  { k: "popup_ms", label: "Popup duration", min: 600, max: 8000, step: 100, unit: "ms" },
];
const APP_PRESETS: [string, Partial<Appearance>][] = [
  ["Default", { glass_translucency: 75, glass_blur: 26, glass_saturation: 170, glass_tint: 50, wheel_size: 100, wheel_hole: 40, wheel_gap: 5, wheel_opacity: 100, ui_font: 13, ui_radius: 10 }],
  ["Crystal", { glass_translucency: 85, glass_blur: 40, glass_saturation: 200, glass_tint: 30 }],
  ["Frosted", { glass_translucency: 15, glass_blur: 50, glass_saturation: 130, glass_tint: 60 }],
  ["Compact", { wheel_size: 80, wheel_hole: 30, ui_font: 12, ui_radius: 6 }],
  ["Big & soft", { wheel_size: 130, wheel_gap: 8, ui_font: 15, ui_radius: 16 }],
];
let aTimer: number | undefined;
function pushAppearance() {
  clearTimeout(aTimer);
  aTimer = window.setTimeout(() => invoke("settings_set_appearance", { appearance: cfg.appearance }).catch((e) => say(String(e), true)), 80);
}
function row(label: string, hint: string | undefined, control: HTMLElement, cls = ""): HTMLElement {
  const r = document.createElement("div");
  r.className = "set " + cls;
  const g = document.createElement("div");
  g.className = "grow";
  const l = document.createElement("div");
  l.className = "lab";
  l.textContent = label;
  g.append(l);
  if (hint) { const h = document.createElement("div"); h.className = "hint"; h.textContent = hint; g.append(h); }
  r.append(g, control);
  return r;
}
function buildAppearance() {
  const host = $("appearance");
  host.replaceChildren();
  const title = document.createElement("div");
  title.className = "set col";
  title.style.border = "0";
  title.innerHTML = '<div><div class="lab">Appearance</div><div class="hint">Everything here applies instantly to every Stash window and the radial wheel.</div></div><div class="presets" id="apresets"></div>';
  host.append(title);
  const inputs: Record<string, HTMLInputElement> = {};
  const vals: Record<string, HTMLElement> = {};
  const refresh = () => {
    for (const d of SLIDERS) { inputs[d.k].value = String(cfg.appearance[d.k]); vals[d.k].textContent = `${cfg.appearance[d.k]} ${d.unit ?? ""}`; }
  };
  for (const d of SLIDERS) {
    const wrap = document.createElement("div");
    wrap.style.cssText = "display:flex;align-items:center;gap:10px";
    const inp = document.createElement("input");
    inp.type = "range"; inp.min = String(d.min); inp.max = String(d.max); inp.step = String(d.step ?? 1);
    const v = document.createElement("span");
    v.className = "val";
    inp.addEventListener("input", () => { cfg.appearance[d.k] = Number(inp.value); v.textContent = `${inp.value} ${d.unit ?? ""}`; pushAppearance(); });
    inputs[d.k] = inp; vals[d.k] = v;
    wrap.append(inp, v);
    const r = row(d.label, d.hint, wrap, d.glass ? "glassonly" : "");
    host.append(r);
  }
  // hover colour
  const hw = document.createElement("div");
  hw.style.cssText = "display:flex;align-items:center;gap:8px";
  const hc = document.createElement("input");
  hc.type = "color"; hc.style.cssText = "width:40px;height:30px;padding:0;border:1px solid var(--line);border-radius:8px;background:none;cursor:pointer";
  hc.value = cfg.appearance.hover_color || "#4a7dff";
  hc.addEventListener("input", () => { cfg.appearance.hover_color = hc.value; pushAppearance(); });
  const hr = document.createElement("button");
  hr.className = "btn"; hr.textContent = "Use Windows accent";
  hr.addEventListener("click", () => { cfg.appearance.hover_color = ""; pushAppearance(); });
  hw.append(hc, hr);
  host.append(row("Hover / selection colour", "Used for wheel highlight, hovered buttons and selection.", hw));
  // popup corner
  const sel = document.createElement("select");
  sel.className = "field"; sel.style.width = "150px";
  for (const [v, t] of [["br", "Bottom right"], ["bl", "Bottom left"], ["tr", "Top right"], ["tl", "Top left"]]) { const o = document.createElement("option"); o.value = v; o.textContent = t; sel.append(o); }
  sel.value = cfg.appearance.popup_corner;
  sel.addEventListener("change", () => { cfg.appearance.popup_corner = sel.value; pushAppearance(); });
  host.append(row("Popup and note position", "Screen corner for the confirmation popup and quick note.", sel));
  // animations
  const lab = document.createElement("label");
  lab.className = "switch";
  const an = document.createElement("input");
  an.type = "checkbox"; an.checked = cfg.appearance.animations;
  an.addEventListener("change", () => { cfg.appearance.animations = an.checked; pushAppearance(); });
  lab.append(an, document.createElement("i"));
  host.append(row("Animations", "Turn off for instant, motion-free UI.", lab));
  // presets
  for (const [name, p] of APP_PRESETS) {
    const b = document.createElement("button");
    b.className = "btn"; b.textContent = name;
    b.addEventListener("click", () => { Object.assign(cfg.appearance, p); refresh(); pushAppearance(); });
    $("apresets").append(b);
  }
  $("apresets").style.cssText = "display:flex;gap:8px;flex-wrap:wrap";
  refresh();
}

// ---- radial wheel: which actions are shown (applies live; shown in this fixed order) ----
const WHEEL: [string, string, string][] = [
  ["snip", "Region", "Drag a rectangle on the screen."],
  ["window", "Window", "The window you were in, as a screenshot."],
  ["screen", "Screen", "The whole monitor under the cursor."],
  ["selection", "Selection", "Text, link or image selected in the app you were using."],
  ["clipboard", "Clipboard", "Whatever you last copied (text, image or files)."],
  ["note", "Note", "Type a quick note."],
  ["record", "Record", "Record a region as a video clip."],
  ["board", "Whiteboard", "Open a new whiteboard."],
  ["pause", "Pause", "Pause or resume the capture hotkey."],
  ["gallery", "Gallery", "Open the gallery."],
  ["settings", "Settings", "Open these settings."],
];
function buildWheel() {
  const host = $("wheelcard");
  host.replaceChildren();
  const title = document.createElement("div");
  title.className = "set col";
  title.style.border = "0";
  title.innerHTML = '<div><div class="lab">Radial wheel</div><div class="hint">Choose what appears when you hold the shortcut. Wedges keep this order, clockwise from the top. Keep at least two.</div></div>';
  host.append(title);
  for (const [id, name, hint] of WHEEL) {
    const lab = document.createElement("label");
    lab.className = "switch";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = cfg.wheel_actions.includes(id);
    cb.addEventListener("change", async () => {
      const next = WHEEL.map((w) => w[0]).filter((w) => (w === id ? cb.checked : cfg.wheel_actions.includes(w)));
      try {
        cfg.wheel_actions = await invoke<string[]>("settings_set_wheel", { actions: next });
        say("");
      } catch (e) {
        cb.checked = !cb.checked;
        say(String(e), true);
      }
    });
    lab.append(cb, document.createElement("i"));
    host.append(row(name, hint, lab));
  }
}

let cfg: Config;
let hotkey = "";
let recording = false;

function say(msg: string, err = false) { status.textContent = msg; status.className = err ? "err" : ""; }

/** Shows "Ctrl+Alt+S" as keycap chips. */
function showKeys(combo: string, rec = false) {
  keys.replaceChildren();
  keys.classList.toggle("rec", rec);
  if (rec) { const s = document.createElement("span"); s.className = "rect"; s.textContent = "Press the new shortcut…"; keys.append(s); return; }
  for (const k of combo.split("+")) {
    const e = document.createElement("kbd");
    e.textContent = k === "Super" ? "Win" : k;
    keys.append(e);
  }
}

async function init() {
  const d = await invoke<Dto>("settings_get");
  cfg = d.config;
  hotkey = cfg.hotkey;
  showKeys(hotkey);
  holdr.value = String(Math.min(600, cfg.hold_ms));
  holdv.textContent = holdr.value;
  lib.value = cfg.library_path;
  autostart.checked = cfg.autostart;
  ocr.checked = cfg.ocr;
  popup.checked = cfg.show_popup;
  url.checked = cfg.capture_url;
  if (!d.autostart_supported) {
    autostart.disabled = true;
    $("autohint").textContent = "Only applies to the installed (release) build.";
  }
  markTheme(cfg.theme);
  setGlass(cfg.glass, false);
  recsys.checked = cfg.rec_system_audio;
  recmic.checked = cfg.rec_mic;
  recfps.value = String(cfg.rec_fps);
  recfpsv.textContent = recfps.value;
  recmax.value = String(cfg.rec_max_secs);
  buildAppearance();
  buildWheel();
  $("ver").textContent = `Stash ${d.version}`;
}
// ---- theme (applies immediately) ----
function markTheme(t: string) {
  document.body.classList.toggle("is-glass", t === "glass"); document.querySelectorAll<HTMLElement>(".th").forEach((b) => b.classList.toggle("on", b.dataset.t === t)); }
$("themes").addEventListener("click", async (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>(".th");
  if (!b) return;
  cfg.theme = b.dataset.t!;
  markTheme(cfg.theme);
  await invoke("settings_set_theme", { theme: cfg.theme });
});

// ---- liquid glass colours (apply live) ----
const PRESETS: [string, string[]][] = [
  ["Aurora", ["#5a78ff", "#be5aff", "#3cdcc8"]],
  ["Sunset", ["#ff7a59", "#ff3d81", "#ffc65a"]],
  ["Ocean", ["#2d9cff", "#1fd1c1", "#5a5cff"]],
  ["Rose", ["#ff8fb1", "#c27bff", "#ffd0a8"]],
  ["Mono", ["#9aa4b8", "#6b7388", "#c9d1e3"]],
];
const gIn = [$<HTMLInputElement>("ga"), $<HTMLInputElement>("gb"), $<HTMLInputElement>("gc")];
let gTimer: number | undefined;
function setGlass(cols: string[], save = true) {
  cols.forEach((c, i) => (gIn[i].value = c));
  $("gprev").style.background = `linear-gradient(90deg, ${cols[0]}, ${cols[1]}, ${cols[2]})`;
  if (!save) return;
  cfg.glass = cols;
  clearTimeout(gTimer);
  gTimer = window.setTimeout(() => invoke("settings_set_glass", { colors: cols }).catch((e) => say(String(e), true)), 150);
}
gIn.forEach((el) => el.addEventListener("input", () => setGlass(gIn.map((x) => x.value))));
$("greset").addEventListener("click", () => setGlass(PRESETS[0][1]));
for (const [name, cols] of PRESETS) {
  const b = document.createElement("button");
  b.className = "pr";
  b.style.background = `linear-gradient(135deg, ${cols[0]}, ${cols[1]}, ${cols[2]})`;
  b.title = name;
  const sp = document.createElement("span");
  sp.textContent = name;
  b.append(sp);
  b.addEventListener("click", () => setGlass(cols));
  $("gpresets").append(b);
}

holdr.addEventListener("input", () => (holdv.textContent = holdr.value));

// ---- shortcut recorder ----
const NAMED: Record<string, string> = { Space: "Space", ArrowUp: "ArrowUp", ArrowDown: "ArrowDown", ArrowLeft: "ArrowLeft", ArrowRight: "ArrowRight", Enter: "Enter", Tab: "Tab", Backquote: "`", Minus: "-", Equal: "=" };
function keyName(code: string): string | null {
  let m;
  if ((m = /^Key([A-Z])$/.exec(code))) return m[1];
  if ((m = /^Digit(\d)$/.exec(code))) return m[1];
  if (/^F\d{1,2}$/.test(code)) return code;
  return NAMED[code] ?? null;
}

async function stopRecording() {
  recording = false;
  showKeys(hotkey);
  recBtn.textContent = "Change";
  await invoke("hotkey_suspend", { suspend: false });
}

recBtn.addEventListener("click", async () => {
  if (recording) { await stopRecording(); return; }
  recording = true;
  await invoke("hotkey_suspend", { suspend: true }); // so pressing the live shortcut doesn't trigger a snip
  showKeys("", true);
  recBtn.textContent = "Cancel";
  say("");
});

window.addEventListener("keydown", async (e) => {
  if (!recording) return;
  e.preventDefault();
  if (e.key === "Escape") { await stopRecording(); return; }
  const k = keyName(e.code);
  if (!k) return; // a bare modifier or unsupported key: keep waiting
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
  if (!mods.length) { say("Include at least one modifier (Ctrl, Alt, Shift or Win).", true); return; }
  hotkey = [...mods, k].join("+");
  say("");
  await stopRecording();
});
window.addEventListener("blur", () => { if (recording) stopRecording(); });

// ---- folder + save ----
$("browse").addEventListener("click", async () => {
  const dir = await open({ directory: true, defaultPath: lib.value || undefined });
  if (typeof dir === "string") lib.value = dir;
});

$("save").addEventListener("click", async () => {
  if (recording) return;
  const next: Config = {
    library_path: lib.value.trim(),
    hotkey,
    hold_ms: Math.round(Number(holdr.value) || cfg.hold_ms),
    capture_url: url.checked,
    show_popup: popup.checked,
    autostart: autostart.checked,
    theme: cfg.theme,
    glass: cfg.glass,
    ocr: ocr.checked,
    appearance: cfg.appearance,
    rec_system_audio: recsys.checked,
    rec_mic: recmic.checked,
    rec_fps: Math.round(Number(recfps.value) || 30),
    rec_max_secs: Math.max(0, Math.round(Number(recmax.value) || 0)),
    wheel_actions: cfg.wheel_actions,
  };
  if (!next.library_path) { say("Choose a library folder.", true); return; }
  try {
    cfg = await invoke<Config>("settings_save", { new: next });
    hotkey = cfg.hotkey;
    showKeys(hotkey);
    say("Saved.");
  } catch (err) {
    say(String(err), true);
  }
});

init();
