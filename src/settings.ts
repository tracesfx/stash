import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

interface Config { library_path: string; hotkey: string; hold_ms: number; capture_url: boolean; show_popup: boolean; autostart: boolean; theme: string; glass: string[]; }
interface Dto { config: Config; autostart_supported: boolean; version: string; }

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const keys = $("keys"), holdr = $<HTMLInputElement>("holdr"), holdv = $("holdv"), lib = $<HTMLInputElement>("lib");
const autostart = $<HTMLInputElement>("autostart"), popup = $<HTMLInputElement>("popup"), url = $<HTMLInputElement>("url");
const status = $("status"), recBtn = $("rec");

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
  popup.checked = cfg.show_popup;
  url.checked = cfg.capture_url;
  if (!d.autostart_supported) {
    autostart.disabled = true;
    $("autohint").textContent = "Only applies to the installed (release) build.";
  }
  markTheme(cfg.theme);
  setGlass(cfg.glass, false);
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
