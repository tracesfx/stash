// Applies the chosen theme to <html data-theme>, plus the accent and Liquid Glass colours. "system" follows the
// Windows app mode. Values come from Rust so native and web windows always agree.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./theme.css";

interface Info { theme: string; accent: string; glass: string[]; }

const dark = window.matchMedia("(prefers-color-scheme: dark)");
let current = "system";

function apply(info: Info) {
  current = info.theme;
  const root = document.documentElement;
  root.dataset.theme = info.theme === "system" ? (dark.matches ? "dark" : "light") : info.theme;
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(info.accent.slice(i, i + 2), 16));
  root.style.setProperty("--accent", info.accent);
  root.style.setProperty("--on-accent", r * 0.299 + g * 0.587 + b * 0.114 > 160 ? "#111" : "#fff");
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
