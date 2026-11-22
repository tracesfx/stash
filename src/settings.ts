// Settings (SPEC §7): tabs in a glass segmented control and custom sliders. There is no Save button: every
// change is saved and applied as it is made (debounced), with a quiet "Saved" tick. Esc closes the window.
// Appearance is a hidden tab, revealed from the footer.
import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

interface Appearance {
  [k: string]: number | string | boolean;
  glass_translucency: number; glass_blur: number; glass_saturation: number; glass_tint: number; hover_color: string;
  win_translucency: number; win_blur: number; win_saturation: number;
  wheel_size: number; wheel_hole: number; wheel_gap: number; wheel_opacity: number; ui_font: number; ui_radius: number;
  popup_corner: string; animations: boolean; cursor_glow: number; cursor_glow_on: boolean; cursor_glow_lag: number; menu_anim: string; menu_anim_ms: number;
}
interface Config {
  library_path: string; hotkey: string; hold_ms: number; capture_url: boolean; tag_menu: boolean;
  autostart: boolean; glass: string[]; ocr: boolean; appearance: Appearance; rec_system_audio: boolean; rec_mic: boolean;
  rec_fps: number; rec_max_secs: number; wheel_actions: string[]; style_rev: number; settings_autoclose: boolean; trash_days: number;
}
interface Dto { config: Config; autostart_supported: boolean; }

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text; // textContent only (paths and names are untrusted)
  return e;
}

let draft: Config;
let opened = false; // entrance finished (click-outside-to-close is armed after it)
let saved: Config; // last config the app accepted
let autostartSupported = true;

// ---------- autosave ----------
let cTimer: number | undefined;
let pending = false;
/** Saves the whole draft (debounced while dragging sliders); now = flush immediately. */
function commit(now = false) {
  pending = true;
  refreshPreview();
  clearTimeout(cTimer);
  cTimer = window.setTimeout(flush, now ? 0 : 220);
}
async function flush() {
  clearTimeout(cTimer);
  if (!pending) return;
  pending = false;
  try {
    saved = await invoke<Config>("settings_save", { new: draft });
    say("Saved", false, true);
  } catch (e) {
    say(String(e), true);
    // a rejected shortcut goes back to the one that still works
    if (draft.hotkey !== saved.hotkey) { draft.hotkey = saved.hotkey; showKeys(draft.hotkey); }
  }
}

// ---------- status ----------
let sTimer: number | undefined;
function say(msg: string, err = false, ok = false) {
  const s = $("status");
  s.textContent = msg;
  s.className = msg ? "on" + (err ? " err" : "") + (ok ? " ok" : "") : "";
  clearTimeout(sTimer);
  if (msg) sTimer = window.setTimeout(() => s.classList.remove("on"), err ? 6000 : 1400);
}

// ---------- building blocks ----------
interface RowOpts { stack?: boolean; value?: HTMLElement }
/** Label + hint on the left, control on the right (or below, for sliders) so text never gets squished. */
function row(label: string, hint: string, control: HTMLElement, o: RowOpts = {}): HTMLElement {
  const r = el("div", "row" + (o.stack ? " stack" : ""));
  const t = el("div");
  t.append(el("div", "lab", label));
  if (hint) t.append(el("div", "hint", hint));
  r.append(t);
  if (o.stack) {
    if (o.value) r.append(o.value);
    else r.append(el("span"));
    control.classList.add("ctl");
    r.append(control);
  } else {
    r.append(control);
  }
  return r;
}

/** Marks a row as advanced: hidden while the footer's Advanced switch is off. */
function adv<T extends HTMLElement>(r: T): T { r.classList.add("adv"); return r; }

function group(title: string, ...rows: HTMLElement[]): HTMLElement[] {
  const g = el("div", "group");
  g.append(...rows);
  return title ? [el("div", "gt", title), g] : [g];
}

function toggle(checked: boolean, onChange: (v: boolean) => void, disabled = false): HTMLLabelElement {
  const l = el("label", "switch");
  const i = el("input");
  i.type = "checkbox";
  i.checked = checked;
  i.disabled = disabled;
  i.addEventListener("change", () => { onChange(i.checked); commit(true); });
  l.append(i, el("i"));
  return l;
}

interface SliderOpts { min: number; max: number; step?: number; stops?: number[]; value: number; fmt: (v: number) => string; onInput: (v: number) => void }
interface Slider { el: HTMLElement; val: HTMLElement; set: (v: number) => void; disable: (off: boolean) => void }

/** Custom slider: thick glass track, bright fill, lens knob; pointer drag + full keyboard support. */
function slider(o: SliderOpts): Slider {
  const s = el("div", "slider");
  s.tabIndex = 0;
  s.setAttribute("role", "slider");
  const track = el("div", "track"), fill = el("div", "fill"), knob = el("div", "knob");
  track.append(fill);
  s.append(track);
  const val = el("span", "val");
  const n = o.stops ? o.stops.length : 0;
  if (o.stops) {
    const ticks = el("div", "ticks");
    o.stops.forEach((_, i) => { const t = el("i"); t.style.left = `${(i / (n - 1)) * 100}%`; ticks.append(t); });
    s.append(ticks);
  }
  s.append(knob);
  let v = o.value;
  const frac = (x: number) => {
    if (o.stops) {
      let best = 0;
      o.stops.forEach((st, i) => { if (Math.abs(st - x) < Math.abs(o.stops![best] - x)) best = i; });
      return best / (n - 1);
    }
    return (x - o.min) / (o.max - o.min);
  };
  const fromFrac = (f: number) => {
    f = Math.min(1, Math.max(0, f));
    if (o.stops) return o.stops[Math.round(f * (n - 1))];
    const step = o.step ?? 1;
    return Math.min(o.max, Math.max(o.min, Math.round((o.min + f * (o.max - o.min)) / step) * step));
  };
  const paint = () => {
    const f = frac(v);
    // the knob travels inside the track (13px = knob radius), the fill reaches its centre
    knob.style.left = `calc(13px + (100% - 26px) * ${f})`;
    fill.style.width = `calc(13px + (100% - 26px) * ${f})`;
    val.textContent = o.fmt(v);
    s.setAttribute("aria-valuenow", String(v));
    s.setAttribute("aria-valuetext", o.fmt(v));
  };
  const set = (nv: number) => { if (nv !== v) { v = nv; paint(); o.onInput(v); commit(); } };
  const at = (clientX: number) => {
    const r = s.getBoundingClientRect();
    return fromFrac((clientX - r.left - 13) / (r.width - 26));
  };
  s.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    s.setPointerCapture(e.pointerId);
    s.classList.add("drag");
    s.focus();
    set(at(e.clientX));
    const move = (m: PointerEvent) => set(at(m.clientX));
    const up = () => { s.classList.remove("drag"); s.removeEventListener("pointermove", move); };
    s.addEventListener("pointermove", move);
    s.addEventListener("pointerup", up, { once: true });
    s.addEventListener("pointercancel", up, { once: true });
  });
  s.addEventListener("keydown", (e) => {
    const idx = o.stops ? Math.round(frac(v) * (n - 1)) : 0;
    const step = o.step ?? 1;
    const big = o.stops ? 1 : Math.max(step, Math.round((o.max - o.min) / 10 / step) * step);
    let nv: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") nv = o.stops ? o.stops[Math.min(n - 1, idx + 1)] : Math.min(o.max, v + step);
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") nv = o.stops ? o.stops[Math.max(0, idx - 1)] : Math.max(o.min, v - step);
    else if (e.key === "PageUp") nv = o.stops ? o.stops[Math.min(n - 1, idx + 2)] : Math.min(o.max, v + big);
    else if (e.key === "PageDown") nv = o.stops ? o.stops[Math.max(0, idx - 2)] : Math.max(o.min, v - big);
    else if (e.key === "Home") nv = o.stops ? o.stops[0] : o.min;
    else if (e.key === "End") nv = o.stops ? o.stops[n - 1] : o.max;
    if (nv !== null) { e.preventDefault(); set(nv); }
  });
  paint();
  return { el: s, val, set: (x: number) => { v = x; paint(); }, disable: (off: boolean) => { s.classList.toggle("off", off); s.tabIndex = off ? -1 : 0; } };
}

/** A row with a full-width slider under its label and the value as a pill on the right. */
function sliderRow(label: string, hint: string, o: SliderOpts): { row: HTMLElement; s: Slider } {
  const s = slider(o);
  return { row: row(label, hint, s.el, { stack: true, value: s.val }), s };
}

const ms = (v: number) => `${v} ms`;
const clipLen = (v: number) => (v === 0 ? "No limit" : v < 60 ? `${v} s` : `${Math.round(v / 60)} min`);

// ---------- tabs ----------
const TABS: [string, string][] = [["capture", "Capture"], ["wheel", "Wheel"], ["recording", "Recording"], ["general", "General"], ["appearance", "Appearance"]];
const panes: Record<string, HTMLElement> = {};
const tabBtns: Record<string, HTMLButtonElement> = {};
const ind = el("span", "ind");
let current = "capture";

/** Advanced on: every setting plus the Appearance tab. Off: only the basic settings. */
function showAdvanced(on: boolean) {
  tabBtns.appearance.style.display = on ? "" : "none";
  document.body.classList.toggle("basic", !on);
  $("appBtn").classList.toggle("on", on);
  try { localStorage.setItem("stash.settings.advanced", on ? "1" : "0"); } catch { /* storage unavailable */ }
  if (!on && current === "appearance") select("capture");
  else moveIndicator();
}

function moveIndicator() {
  const b = tabBtns[current];
  if (!b || b.style.display === "none") return;
  ind.style.width = `${b.offsetWidth}px`;
  ind.style.transform = `translateX(${b.offsetLeft}px)`;
}

function select(id: string) {
  current = id;
  for (const [k] of TABS) {
    tabBtns[k].classList.toggle("on", k === id);
    tabBtns[k].setAttribute("aria-selected", String(k === id));
    panes[k].classList.toggle("on", k === id);
  }
  moveIndicator();
  $("panes").scrollTop = 0;
  if (id === "appearance") { refreshPreview(); setTimeout(() => animIntro(), 260); }
}

function buildTabs() {
  const nav = $("tabs");
  nav.append(ind);
  for (const [id, label] of TABS) {
    const b = el("button", "", label);
    b.setAttribute("role", "tab");
    b.onclick = () => select(id);
    tabBtns[id] = b;
    nav.append(b);
    tabsRO.observe(b);
    const p = el("section", "pane");
    panes[id] = p;
    $("panes").append(p);
  }
}

// ---------- capture ----------
let recording = false;
const keys = el("div", "keys");
const recBtn = el("button", "btn", "Change");

function showKeys(combo: string, rec = false) {
  keys.replaceChildren();
  if (rec) { keys.append(el("span", "rect", "Press the new shortcut…")); return; }
  for (const k of combo.split("+")) keys.append(el("kbd", "", k === "Super" ? "Win" : k));
}

function buildCapture() {
  const resetNote = el("button", "btn", "Reset to corner");
  resetNote.onclick = async () => { await invoke("note_reset_position"); say("The note will open in the corner again", false, true); };
  const ctl = el("div", "ctl inline");
  ctl.append(keys, recBtn);
  showKeys(draft.hotkey);
  const hold = sliderRow("Wheel hold delay", "How long to hold the shortcut before the radial wheel opens. A shorter press is a snip.", { min: 80, max: 1000, step: 10, value: draft.hold_ms, fmt: ms, onInput: (v) => (draft.hold_ms = v) });

  const tagRow = row("Tag menu after capture", "A ring of your recent tags at the cursor, with Star (add to Favourites) and Discard (do not keep the capture) in the middle. Click a tag or type a new one; Tab switches to a caption. It disappears when you move the pointer off it, then a toast confirms the save. When off, only the toast shows.", toggle(draft.tag_menu, (v) => (draft.tag_menu = v)));

  const corners = el("div", "corners");
  for (const c of ["tl", "tr", "bl", "br"]) {
    const b = el("button");
    b.title = { tl: "Top left", tr: "Top right", bl: "Bottom left", br: "Bottom right" }[c]!;
    b.classList.toggle("on", draft.appearance.popup_corner === c);
    b.onclick = () => { draft.appearance.popup_corner = c; corners.querySelectorAll("button").forEach((x) => x.classList.toggle("on", x === b)); commit(true); };
    corners.append(b);
  }

  panes.capture.append(
    ...group("Shortcut",
      row("Capture shortcut", "Tap to snip a region. Hold for the radial wheel. Needs a modifier (Ctrl, Alt, Shift or Win).", ctl),
      adv(hold.row)),
    ...group("After a capture", tagRow,
      adv(row("Toast position", "Screen corner for confirmation toasts. The quick note starts here too until you move it.", corners)),
      adv(row("Quick note position", "Drag the note bar anywhere; it reopens where you left it.", resetNote))),
  );
}

// ---------- wheel ----------
const ICONS: Record<string, string> = {
  snip: '<path d="M3 7V3h4M13 3h4v4M17 13v4h-4M7 17H3v-4"/>',
  window: '<rect x="2.5" y="4" width="15" height="12" rx="2"/><path d="M2.5 7.5h15"/>',
  screen: '<rect x="2" y="3.5" width="16" height="10" rx="1.5"/><path d="M10 13.5v3M6.5 16.5h7"/>',
  selection: '<path d="M3 5h14M3 9.5h8M3 14h10M14.5 9v8"/>',
  clipboard: '<rect x="4.5" y="4" width="11" height="13.5" rx="2"/><path d="M7.5 2.5h5v3h-5z"/>',
  note: '<path d="M4 16l.8-3.5L13 4.3l2.7 2.7-8.2 8.2L4 16z"/><path d="M11.6 5.7l2.7 2.7"/>',
  record: '<rect x="2.5" y="5.5" width="10" height="9" rx="2"/><path d="M12.5 9l5-3v8l-5-3"/>',
  pause: '<path d="M7 4.5v11M13 4.5v11"/>',
  gallery: '<rect x="3" y="3" width="6" height="6" rx="1.3"/><rect x="11" y="3" width="6" height="6" rx="1.3"/><rect x="3" y="11" width="6" height="6" rx="1.3"/><rect x="11" y="11" width="6" height="6" rx="1.3"/>',
  settings: '<circle cx="10" cy="10" r="2.6"/><path d="M10 2.5v2.2M10 15.3v2.2M2.5 10h2.2M15.3 10h2.2M4.7 4.7l1.5 1.5M13.8 13.8l1.5 1.5M4.7 15.3l1.5-1.5M13.8 6.2l1.5-1.5"/>',
};
const WHEEL: [string, string, string][] = [
  ["snip", "Region", "Drag a rectangle on the screen."],
  ["window", "Window", "The window you were in."],
  ["screen", "Screen", "The whole monitor under the cursor."],
  ["selection", "Selection", "What is selected in your app."],
  ["clipboard", "Clipboard", "Whatever you last copied."],
  ["note", "Note", "Type a quick note."],
  ["record", "Record", "Record a region as a clip."],
  ["pause", "Pause", "Pause or resume the shortcut."],
  ["gallery", "Gallery", "Open the gallery."],
  ["settings", "Settings", "Open these settings."],
];

function buildWheel() {
  const tiles = el("div", "tiles");
  for (const [id, name, hint] of WHEEL) {
    const t = el("button", "tile" + (draft.wheel_actions.includes(id) ? " on" : ""));
    t.innerHTML = `<svg viewBox="0 0 20 20">${ICONS[id]}</svg><span class="ck"></span>`; // static markup
    t.append(el("b", "", name), el("span", "", hint));
    t.onclick = () => {
      const on = draft.wheel_actions.includes(id);
      if (on && draft.wheel_actions.length <= 2) {
        t.classList.remove("shake"); void t.offsetWidth; t.classList.add("shake");
        say("Keep at least two actions on the wheel.", true);
        return;
      }
      draft.wheel_actions = WHEEL.map((w) => w[0]).filter((w) => (w === id ? !on : draft.wheel_actions.includes(w)));
      t.classList.toggle("on", !on);
      commit(true);
    };
    tiles.append(t);
  }
  const g = el("div", "group");
  g.append(row("Wheel actions", "Hold the shortcut to open the wheel. Wedges keep this order, clockwise from the top. Changes apply right away.", el("span")), tiles);
  panes.wheel.append(g);
}

// ---------- recording ----------
function buildRecording() {
  const fps = sliderRow("Frame rate", "Lower means smaller files and less CPU.", { min: 10, max: 60, step: 5, value: draft.rec_fps, fmt: (v) => `${v} fps`, onInput: (v) => (draft.rec_fps = v) });
  const max = sliderRow("Maximum clip length", "Recording stops by itself after this long.", { min: 0, max: 3600, stops: [0, 30, 60, 120, 300, 600, 900, 1800, 3600], value: draft.rec_max_secs, fmt: clipLen, onInput: (v) => (draft.rec_max_secs = v) });
  panes.recording.append(
    ...group("Audio",
      row("Record system audio", "Clips include what you hear (desktop audio).", toggle(draft.rec_system_audio, (v) => (draft.rec_system_audio = v))),
      row("Record microphone", "Clips include your default microphone.", toggle(draft.rec_mic, (v) => (draft.rec_mic = v)))),
    ...group("Video", adv(fps.row), adv(max.row)),
  );
}

// ---------- general ----------
function buildGeneral() {
  const f = el("div", "folder ctl");
  const lib = el("input", "field");
  lib.type = "text";
  lib.value = draft.library_path;
  lib.spellcheck = false;
  // the folder is applied when editing is done (Enter or leaving the field), never per keystroke
  lib.addEventListener("change", () => { draft.library_path = lib.value.trim(); commit(true); });
  const browse = el("button", "btn", "Browse…");
  browse.onclick = async () => {
    modalUntilFocus();
    const dir = await open({ directory: true, defaultPath: lib.value || undefined });
    if (typeof dir === "string") { lib.value = dir; draft.library_path = dir; commit(true); }
  };
  f.append(lib, browse);
  const trashRow = sliderRow("Empty trash after", "Deleted items wait in the gallery's Trash tab, then are removed for good.",
    { min: 0, max: 365, stops: [1, 7, 14, 30, 60, 90, 365, 0], value: draft.trash_days ?? 30, fmt: (v) => (v === 0 ? "Never" : v === 1 ? "1 day" : `${v} days`), onInput: (v) => (draft.trash_days = v) }).row;
  panes.general.append(
    ...group("Library",
      row("Library folder", "A OneDrive or Dropbox folder keeps it synced. Only new captures go to a new folder; move old files yourself.", f, { stack: true }),
      trashRow,
      adv(row("Search text inside images", "Reads text in screenshots with the OCR built into Windows, on this PC only.", toggle(draft.ocr, (v) => (draft.ocr = v)))),
      adv(row("Record page URL from browsers", "Reads the address bar through Windows UI Automation. Chromium browsers may load pages slightly slower.", toggle(draft.capture_url, (v) => (draft.capture_url = v))))),
    ...group("System",
      adv(row("Close settings when clicking elsewhere", "Changes are saved as you make them, so nothing is lost.", toggle(draft.settings_autoclose, (v) => (draft.settings_autoclose = v)))),
      row("Start with Windows", autostartSupported ? "Runs quietly in the tray at sign-in." : "Only applies to the installed (release) build.", toggle(draft.autostart, (v) => (draft.autostart = v), !autostartSupported))),
  );
}

// ---------- appearance ----------
type ADef = [keyof Appearance & string, string, string, number, number, number, string];
// Gallery and settings (backdrop.rs draws the blur behind the page, so this window shows changes live)
const WIN_LOOK: ADef[] = [
  ["win_blur", "Blur", "0 = clear. Up to 30 px the frost fades in over the sharp view behind; above 30 it gets softer.", 0, 60, 1, "px"],
  ["win_translucency", "Translucency", "How thin the dark tint is.", 0, 100, 1, "%"],
  ["win_saturation", "Saturation", "Colour boost of what shows through.", 100, 250, 5, "%"],
];
// Wheel, tag menu, quick note and toasts (drawn natively; see the preview)
const NATIVE_LOOK: ADef[] = [
  ["glass_blur", "Blur", "0 = the screen shows through sharp.", 0, 60, 1, "px"],
  ["glass_translucency", "Translucency", "How thin the dark tint is.", 0, 100, 1, "%"],
  ["glass_saturation", "Saturation", "Colour boost of the blurred background.", 100, 250, 5, "%"],
];
const GLOW: ADef = ["cursor_glow", "Glow strength", "", 0, 100, 1, "%"];
const GLOW_LAG: ADef = ["cursor_glow_lag", "Glow follow delay", "How far the glow trails behind the pointer. 0 = stuck to it.", 0, 400, 10, "ms"];
const SHEEN: ADef = ["glass_tint", "Colour sheen", "How strongly the glass colours tint every glass surface.", 0, 100, 1, "%"];
const WHEEL_LOOK: ADef[] = [
  ["wheel_size", "Size", "", 60, 160, 5, "%"],
  ["wheel_hole", "Centre size", "Radius of the dead zone in the middle.", 20, 80, 1, "px"],
  ["wheel_gap", "Slice gap", "", 0, 14, 1, "px"],
  ["wheel_opacity", "Opacity", "", 30, 100, 1, "%"],
];
const UI_LOOK: ADef[] = [
  ["ui_font", "Text size", "", 11, 18, 1, "px"],
  ["ui_radius", "Corner roundness", "", 0, 20, 1, "px"],
];
// ---------- menu animation picker ----------
// The curves mirror gfx::open_window / gfx::open_slice (Rust), so the mini wheels play what the real wheel and
// tag menu will do.
type MenuAnim = "petals" | "fade" | "zoom" | "bounce" | "swirl" | "fan";
const ANIMS: [MenuAnim, string, string][] = [
  ["petals", "Petals", "Slices pop in one by one"],
  ["fade", "Fade", "A soft, quiet fade"],
  ["zoom", "Zoom", "Grows in with a long glide"],
  ["bounce", "Bounce", "Springs past full size"],
  ["swirl", "Swirl", "Turns into place"],
  ["fan", "Fan", "Unfolds clockwise"],
];
const cl = (t: number) => Math.min(1, Math.max(0, t));
const easeOut = (t: number) => 1 - (1 - cl(t)) ** 3;
const easeQuart = (t: number) => 1 - (1 - cl(t)) ** 4;
const easeQuint = (t: number) => 1 - (1 - cl(t)) ** 5;
const smoothstep = (t: number) => { t = cl(t); return t * t * (3 - 2 * t); };
const back = (t: number, c1: number) => { const u = cl(t) - 1; return 1 + (c1 + 1) * u * u * u + c1 * u * u; };
const springC = (t: number) => (t >= 1 ? 1 : 1 - Math.exp(-6 * Math.max(0, t)) * Math.cos(3 * Math.PI * Math.max(0, t)));
function openWindow(a: MenuAnim, d: number, age: number): [number, number] {
  const t = (f: number) => cl(age / (d * f));
  switch (a) {
    case "petals": { const k = easeOut(t(0.55)); return [k, 0.9 + 0.1 * k]; }
    case "fade": { const k = smoothstep(t(1)); return [k, 0.985 + 0.015 * k]; }
    case "zoom": return [easeOut(t(0.6)), 0.72 + 0.28 * easeQuint(t(1))];
    case "bounce": return [easeOut(t(0.3)), 0.5 + 0.5 * springC(t(1))];
    case "swirl": return [easeOut(t(0.6)), 0.8 + 0.2 * easeQuart(t(1))];
    case "fan": return [easeOut(t(0.35)), 0.95 + 0.05 * easeOut(t(1))];
  }
}
function openSlice(a: MenuAnim, d: number, age: number, i: number, n: number): { k: number; a: number; rot: number } {
  const t = cl(age / d);
  if (a === "petals") {
    const step = Math.min((0.5 * d) / (Math.max(n, 2) - 1), (22 * d) / 240);
    const u = cl((age - i * step) / (0.5 * d));
    return u <= 0 ? { k: 0.7, a: 0, rot: 0 } : { k: 0.7 + 0.3 * back(u, 1.3), a: easeOut(u * 1.6), rot: 0 };
  }
  if (a === "swirl") return { k: 1, a: 1, rot: -50 * (1 - easeQuart(t)) };
  if (a === "fan") { const e = easeQuart(t); return { k: 1, a: i === 0 ? 1 : Math.min(1, e * 4), rot: -(i * (360 / n)) * (1 - e) }; }
  return { k: 1, a: 1, rot: 0 };
}

/** Annular wedge with constant-width gaps (as wheel.rs draws it), centred on 0,0, clockwise from the top. */
function wedgeD(i: number, n: number, rIn: number, rOut: number, gap: number): string {
  const step = 360 / n, mid = i * step, half = step / 2, d = gap / 2;
  const off = (r: number) => (Math.asin(Math.min(1, d / r)) * 180) / Math.PI;
  const pt = (r: number, deg: number) => { const a = (deg * Math.PI) / 180; return `${(r * Math.sin(a)).toFixed(2)} ${(-r * Math.cos(a)).toFixed(2)}`; };
  const [a0, a1] = [mid - half + off(rOut), mid + half - off(rOut)];
  const [b0, b1] = [mid - half + off(rIn), mid + half - off(rIn)];
  return `M${pt(rOut, a0)} A${rOut} ${rOut} 0 0 1 ${pt(rOut, a1)} L${pt(rIn, b1)} A${rIn} ${rIn} 0 0 0 ${pt(rIn, b0)} Z`;
}

const SVGNS = "http://www.w3.org/2000/svg";
interface MiniWheel { el: SVGSVGElement; play: () => void; rest: () => void }
function miniWheel(anim: () => MenuAnim, ms: () => number): MiniWheel {
  const n = 5;
  const svg = document.createElementNS(SVGNS, "svg");
  svg.setAttribute("viewBox", "-50 -50 100 100");
  const whole = document.createElementNS(SVGNS, "g");
  const slices: SVGGElement[] = [];
  for (let i = 0; i < n; i++) {
    const g = document.createElementNS(SVGNS, "g");
    const p = document.createElementNS(SVGNS, "path");
    p.setAttribute("d", wedgeD(i, n, 17, 45, 3.2));
    if (i === 1) p.classList.add("hot");
    const dot = document.createElementNS(SVGNS, "circle");
    const a = ((i * 360) / n) * (Math.PI / 180);
    dot.setAttribute("cx", (31 * Math.sin(a)).toFixed(2));
    dot.setAttribute("cy", (-31 * Math.cos(a)).toFixed(2));
    dot.setAttribute("r", "3.2");
    g.append(p, dot);
    whole.append(g);
    slices.push(g);
  }
  const hub = document.createElementNS(SVGNS, "circle");
  hub.setAttribute("r", "13.5");
  hub.classList.add("hub");
  whole.append(hub);
  svg.append(whole);
  let raf = 0, t0 = 0;
  const pose = (age: number) => {
    const a = anim(), d = ms(), on = draft.appearance.animations;
    const [op, sc] = on ? openWindow(a, d, age) : [1, 1];
    whole.setAttribute("transform", `scale(${sc.toFixed(4)})`);
    whole.setAttribute("opacity", op.toFixed(3));
    slices.forEach((g, i) => {
      const s = on ? openSlice(a, d, age, i, n) : { k: 1, a: 1, rot: 0 };
      g.setAttribute("transform", `rotate(${s.rot.toFixed(2)}) scale(${s.k.toFixed(4)})`);
      g.setAttribute("opacity", s.a.toFixed(3));
    });
  };
  const frame = (t: number) => {
    if (!t0) t0 = t;
    const age = t - t0;
    pose(age);
    if (age < ms() + 40) raf = requestAnimationFrame(frame); else { raf = 0; pose(1e6); }
  };
  const play = () => { cancelAnimationFrame(raf); t0 = 0; pose(0); raf = requestAnimationFrame(frame); };
  const rest = () => { cancelAnimationFrame(raf); raf = 0; pose(1e6); };
  rest();
  return { el: svg, play, rest };
}

let animSync: () => void = () => {};
let animIntro: () => void = () => {};
/** Animation tiles (each a mini wheel that plays on hover and when picked) + the duration slider. */
function animRows(): HTMLElement[] {
  const ap = draft.appearance;
  const cur = () => (ANIMS.some(([k]) => k === ap.menu_anim) ? ap.menu_anim : "petals") as MenuAnim;
  const dur = () => Number(ap.menu_anim_ms) || 260;
  const tiles = el("div", "atiles ctl");
  const minis: Record<string, MiniWheel> = {};
  for (const [k, name, hint] of ANIMS) {
    const t = el("button", "atile" + (cur() === k ? " on" : ""));
    const m = miniWheel(() => k, dur);
    minis[k] = m;
    t.append(m.el, el("b", "", name), el("span", "", hint));
    t.addEventListener("pointerenter", () => m.play());
    t.onclick = () => {
      ap.menu_anim = k;
      tiles.querySelectorAll(".atile").forEach((x) => x.classList.toggle("on", x === t));
      m.play();
      commit(true);
    };
    tiles.append(t);
  }
  let replay: number | undefined;
  const len = sliderRow("Duration", "How long the wheel and tag menu take to open. Short feels snappy; long shows off the motion.",
    { min: 100, max: 600, step: 10, value: dur(), fmt: ms, onInput: (v) => { ap.menu_anim_ms = v; clearTimeout(replay); replay = window.setTimeout(() => minis[cur()].play(), 160); } });
  const pick = row("Opening animation", "How the wheel and the tag menu appear. Hover a tile to preview it (needs Animations).", tiles, { stack: true });
  const sync = () => { const off = !ap.animations; tiles.classList.toggle("off", off); len.s.disable(off); pick.classList.toggle("dis", off); };
  sync();
  animSync = sync;
  animIntro = () => minis[cur()].play();
  return [pick, len.row];
}

function buildAppearance() {
  const mk = (d: ADef) => {
    const [k, label, hint, min, max, step, unit] = d;
    const r = sliderRow(label, hint, { min, max, step, value: Number(draft.appearance[k] ?? 0), fmt: (v) => `${v} ${unit}`, onInput: (v) => { draft.appearance[k] = v; } });
    return r.row;
  };
  // cursor glow: on/off, then strength and follow delay (dimmed while off)
  function glowRows(): HTMLElement[] {
    const sl = [GLOW, GLOW_LAG].map((d) => {
      const [k, label, hint, min, max, step, unit] = d;
      return sliderRow(label, hint, { min, max, step, value: Number(draft.appearance[k] ?? 0), fmt: (v) => `${v} ${unit}`, onInput: (v) => { draft.appearance[k] = v; } });
    });
    const sync = () => sl.forEach((r) => r.s.disable(draft.appearance.cursor_glow_on === false));
    const on = row("Cursor glow", "A subtle light in the glass colours that follows the pointer (windows, wheel and tag menu).",
      toggle(draft.appearance.cursor_glow_on !== false, (v) => { draft.appearance.cursor_glow_on = v; sync(); }));
    sync();
    return [on, ...sl.map((r) => r.row)];
  }
  // glass colours: three round swatches + a gradient preview
  const sws = el("div", "swatches ctl");
  const inputs: HTMLInputElement[] = [];
  const gprev = el("div", "gprev");
  const paintGlass = () => {
    draft.glass.forEach((c, i) => { inputs[i].value = c; (inputs[i].parentElement as HTMLElement).style.background = c; });
    gprev.style.background = `linear-gradient(90deg, ${draft.glass.join(", ")})`;
  };
  for (let i = 0; i < 3; i++) {
    const w = el("label", "sw");
    const inp = el("input");
    inp.type = "color";
    inp.addEventListener("click", modalUntilFocus);
    inp.addEventListener("input", () => { draft.glass[i] = inp.value; paintGlass(); commit(); });
    inputs.push(inp);
    w.append(inp);
    sws.append(w);
  }
  sws.append(gprev);
  paintGlass();

  // hover colour
  const hov = el("div", "ctl inline");
  const hw = el("label", "sw");
  const hc = el("input");
  hc.type = "color";
  hc.addEventListener("click", modalUntilFocus);
  hc.value = draft.appearance.hover_color || "#4a7dff";
  hw.style.background = hc.value;
  hc.addEventListener("input", () => { draft.appearance.hover_color = hc.value; hw.style.background = hc.value; commit(); });
  hw.append(hc);
  const acc = el("button", "btn", "Use Windows accent");
  acc.onclick = async () => {
    draft.appearance.hover_color = "";
    commit(true);
    await flush();
    // show the colour now in use (theme.ts sets --accent from the Windows accent)
    setTimeout(() => {
      const a = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
      if (/^#[0-9a-f]{6}$/i.test(a)) { hc.value = a; hw.style.background = a; }
    }, 120);
  };
  hov.append(hw, acc);

  const pv = el("div", "pv");
  pv.append(previewImg);
  panes.appearance.append(
    ...group("Everywhere",
      row("Glass colours", "The sheen on panels, the wheel and the tag menu.", sws, { stack: true }),
      mk(SHEEN), ...glowRows(),
      row("Hover and selection colour", "Wheel highlight, selected items and switches.", hov),
      row("Animations", "Turn off for instant, motion-free UI.", toggle(draft.appearance.animations, (v) => { draft.appearance.animations = v; animSync(); }))),
    ...group("Menu animation", ...animRows()),
    ...group("Gallery and settings windows", ...WIN_LOOK.map(mk), ...UI_LOOK.map(mk)),
    ...group("Wheel, tag menu and note", pv, ...NATIVE_LOOK.map(mk)),
    ...group("Wheel shape", ...WHEEL_LOOK.map(mk)),
  );
}

// ---------- shortcut recorder ----------
const NAMED: Record<string, string> = { Space: "Space", ArrowUp: "ArrowUp", ArrowDown: "ArrowDown", ArrowLeft: "ArrowLeft", ArrowRight: "ArrowRight", Enter: "Enter", Tab: "Tab", Backquote: "`", Minus: "-", Equal: "=" };
function keyName(code: string): string | null {
  let m;
  if ((m = /^Key([A-Z])$/.exec(code))) return m[1];
  if ((m = /^Digit(\d)$/.exec(code))) return m[1];
  if (/^F\d{1,2}$/.test(code)) return code;
  return NAMED[code] ?? null;
}
async function stopRecording() {
  if (!recording) return;
  recording = false;
  showKeys(draft.hotkey);
  recBtn.textContent = "Change";
  await invoke("hotkey_suspend", { suspend: false });
}
recBtn.addEventListener("click", async () => {
  if (recording) { await stopRecording(); return; }
  recording = true;
  await invoke("hotkey_suspend", { suspend: true }); // so pressing the live shortcut doesn't trigger a snip
  showKeys("", true);
  recBtn.textContent = "Cancel";
});

// ---------- close ----------
async function closeWindow() {
  await stopRecording();
  await flush(); // never lose the last change
  $("app").classList.add("out");
  await new Promise((r) => setTimeout(r, 140));
  invoke("settings_close");
}

window.addEventListener("keydown", async (e) => {
  if (recording) {
    e.preventDefault();
    if (e.key === "Escape") { await stopRecording(); return; }
    const k = keyName(e.code);
    if (!k) return; // a bare modifier or unsupported key: keep waiting
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
    if (!mods.length) { say("Include at least one modifier (Ctrl, Alt, Shift or Win).", true); return; }
    draft.hotkey = [...mods, k].join("+");
    await stopRecording();
    commit(true);
    return;
  }
  if (e.key === "Escape") { e.preventDefault(); closeWindow(); }
});
// Clicking outside the window closes it (changes are already saved). Our own native dialogs (folder picker,
// colour pickers) also take focus, so they suspend this until the window has focus again.
let modal = false;
function modalUntilFocus() { modal = true; }
window.addEventListener("focus", () => (modal = false));
window.addEventListener("blur", () => {
  if (recording) { stopRecording(); return; }
  setTimeout(() => { if (!document.hasFocus() && !modal && opened && draft?.settings_autoclose !== false) closeWindow(); }, 120);
});
window.addEventListener("resize", moveIndicator);
// the Text size knob resizes the tabs without a window resize: keep the indicator under the selected tab
const tabsRO = new ResizeObserver(() => moveIndicator());

// ---------- Appearance preview (the native glass, rendered by Rust over a desktop snapshot) ----------
const previewImg = el("img");
previewImg.alt = "Glass preview";
let pvTimer: number | undefined;
let pvUrl = "";
function refreshPreview() {
  if (!draft || current !== "appearance") return;
  clearTimeout(pvTimer);
  pvTimer = window.setTimeout(async () => {
    try {
      const buf = await invoke<ArrayBuffer>("appearance_preview", { appearance: draft.appearance, glass: draft.glass });
      const url = URL.createObjectURL(new Blob([buf], { type: "image/png" }));
      previewImg.onload = () => { if (pvUrl) URL.revokeObjectURL(pvUrl); pvUrl = url; };
      previewImg.src = url;
    } catch { /* preview is best-effort */ }
  }, 90);
}

window.addEventListener("beforeunload", () => { if (pending) invoke("settings_save", { new: draft }); });
$("appBtn").addEventListener("click", () => showAdvanced(!$("appBtn").classList.contains("on")));

async function init() {
  const d = await invoke<Dto>("settings_get");
  draft = structuredClone(d.config);
  saved = structuredClone(d.config);
  autostartSupported = d.autostart_supported;
  buildTabs();
  buildCapture();
  buildWheel();
  buildRecording();
  buildGeneral();
  buildAppearance();
  let showAdv = false;
  try { showAdv = (localStorage.getItem("stash.settings.advanced") ?? localStorage.getItem("stash.settings.appearance")) === "1"; } catch { /* storage unavailable */ }
  showAdvanced(showAdv);
  select("capture");
  // the window is created hidden: show it once this first frame is ready, then play the entrance
  // (title drops in, the tab bar pops, groups cascade, footer rises)
  document.querySelectorAll<HTMLElement>(".pane.on > *").forEach((e, i) => e.style.setProperty("--i", String(Math.min(i, 10))));
  // (a hidden page gets no animation frames: reveal first, animate on the first visible frame)
  moveIndicator();
  invoke("window_ready").catch(() => {});
  requestAnimationFrame(() => {
    moveIndicator();
    $("app").classList.add("in");
    document.body.classList.remove("pre");
    document.body.classList.add("opening");
    setTimeout(() => { document.body.classList.remove("opening"); opened = true; }, 900);
  });
  document.fonts?.ready.then(moveIndicator);
}

init().catch((e) => say(String(e), true));
