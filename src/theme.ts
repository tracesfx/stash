// Applies the chosen theme to <html data-theme>, plus the accent and Liquid Glass colours. "system" follows the
// Windows app mode. Values come from Rust so native and web windows always agree.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./theme.css";

interface Info { theme: string; accent: string; glass: string[]; hover: string; appearance: { glass_translucency: number; glass_blur: number; glass_saturation: number; glass_tint: number; ui_font: number; ui_radius: number; animations: boolean }; }

function luma(h: string) { const [r, g, b] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)); return r * 0.299 + g * 0.587 + b * 0.114; }

const dark = window.matchMedia("(prefers-color-scheme: dark)");
let current = "system";

function apply(info: Info) {
  current = info.theme;
  const root = document.documentElement;
  root.dataset.theme = info.theme === "system" ? (dark.matches ? "dark" : "light") : info.theme;
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(info.accent.slice(i, i + 2), 16));
  root.style.setProperty("--accent", info.accent);
  root.style.setProperty("--on-accent", r * 0.299 + g * 0.587 + b * 0.114 > 160 ? "#111" : "#fff");
  const ap = info.appearance;
  root.style.setProperty("--hover", info.hover);
  root.style.setProperty("--on-hover", luma(info.hover) > 160 ? "#111" : "#fff");
  root.style.setProperty("--fs", `${ap.ui_font}px`);
  root.style.setProperty("--r-m", `${ap.ui_radius}px`);
  root.style.setProperty("--r-s", `${Math.round(ap.ui_radius * 0.6)}px`);
  root.style.setProperty("--r-l", `${Math.round(ap.ui_radius * 1.4)}px`);
  const tr = ap.glass_translucency / 100; // 0 frosted .. 1 clear
  root.style.setProperty("--g-blur", `blur(${ap.glass_blur}px) saturate(${ap.glass_saturation / 100})`);
  root.style.setProperty("--g-body", (0.36 - tr * 0.345).toFixed(3)); // page tint alpha
  root.style.setProperty("--g-card", (0.025 + (1 - tr) * 0.15).toFixed(3)); // surface alpha
  root.style.setProperty("--g-wash", ((ap.glass_tint / 50) * (1 - tr * 0.35)).toFixed(2)); // gradient wash strength
  root.dataset.motion = ap.animations ? "on" : "off";
  (["a", "b", "c"] as const).forEach((k, i) => info.glass[i] && root.style.setProperty(`--glass-${k}`, info.glass[i]));
}

async function refresh() {
  try {
    apply(await invoke<Info>("theme_info"));
  } catch {
    /* keep CSS defaults */
  }
}

export function initTheme() {
  refresh();
  dark.addEventListener("change", () => current === "system" && refresh());
  listen("theme-changed", refresh);
}

initTheme();
