// Liquid Glass look for every web window: accent, glass colours and the Appearance knobs come from Rust
// (theme_info), so native and web windows always agree. Re-applied live on "theme-changed".
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./theme.css";

interface Info { accent: string; glass: string[]; hover: string; appearance: { win_translucency: number; win_blur: number; win_saturation: number; glass_tint: number; ui_font: number; ui_radius: number; animations: boolean }; }

function luma(h: string) { const [r, g, b] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)); return r * 0.299 + g * 0.587 + b * 0.114; }

function apply(info: Info) {
  const root = document.documentElement;
  root.dataset.theme = "glass";
  const set = (k: string, v: string) => root.style.setProperty(k, v);
  set("--accent", info.accent);
  set("--on-accent", luma(info.accent) > 160 ? "#111" : "#fff");
  set("--hover", info.hover);
  set("--on-hover", luma(info.hover) > 160 ? "#111" : "#fff");
  const ap = info.appearance;
  set("--fs", `${ap.ui_font}px`);
  set("--r-m", `${ap.ui_radius}px`);
  set("--r-s", `${Math.round(ap.ui_radius * 0.6)}px`);
  set("--r-l", `${Math.round(ap.ui_radius * 1.4)}px`);
  // Window glass: Rust draws the blurred backdrop behind the page (backdrop.rs); the page adds the smoky tint.
  // Frost fades in over the first few px of blur (mirrors theme::frost_of), so a low blur gives a light veil
  // and 0 is fully transparent; Translucency thins the tint either way.
  const tr = ap.win_translucency / 100;
  const frost = Math.min(1, ap.win_blur / 12);
  const lerp = (x: number, y: number) => x + (y - x) * frost;
  const body = lerp(0.04 + (1 - tr) * 0.3, 0.42 - tr * 0.36);
  const card = lerp(0.02, 0.035) + (1 - tr) * 0.06;
  const clear = frost < 0.5;
  set("--g-blur", `blur(${ap.win_blur}px) saturate(${ap.win_saturation / 100})`);
  set("--g-body", body.toFixed(3));
  set("--g-card", card.toFixed(3));
  set("--g-hi", lerp(0.04, 0.11).toFixed(3));
  set("--g-wash", (ap.glass_tint / 100).toFixed(2));
  root.dataset.clear = clear ? "1" : "0";
  root.dataset.motion = ap.animations ? "on" : "off";
  (["a", "b", "c"] as const).forEach((k, i) => info.glass[i] && set(`--glass-${k}`, info.glass[i]));
}

async function refresh() {
  try {
    apply(await invoke<Info>("theme_info"));
  } catch {
    /* keep CSS defaults */
  }
}

refresh();
listen("theme-changed", refresh);
