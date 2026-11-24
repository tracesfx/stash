// First-run tutorial (tutorial.rs): three steps the user does for real. The app reports each one as it happens
// ("saved", "wheel", "gallery"), so the card ticks it off and moves on; Next skips a step, Skip ends it.
import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Info { hotkey: string; library: string; hold_ms: number }
type Step = "saved" | "wheel" | "gallery";

const $ = (id: string) => document.getElementById(id)!;
function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}
function keys(combo: string): HTMLElement[] {
  const out: HTMLElement[] = [];
  combo.split("+").forEach((k, i) => {
    if (i) out.push(el("span", "", "+") as HTMLElement);
    out.push(el("kbd", "", k === "Super" ? "Win" : k));
  });
  out.forEach((e) => { if (e.tagName === "SPAN") { e.style.display = "inline"; e.style.margin = "0 3px"; } });
  return out;
}

const order: Step[] = ["saved", "wheel", "gallery"];
const done = new Set<Step>();
const rows: Record<Step, HTMLLIElement> = {} as Record<Step, HTMLLIElement>;

function render() {
  const cur = order.find((s) => !done.has(s));
  order.forEach((s) => {
    rows[s].classList.toggle("done", done.has(s));
    rows[s].classList.toggle("cur", s === cur);
  });
  const all = !cur;
  document.body.classList.toggle("all", all);
  $("foot").textContent = all ? "That's it. Change the shortcut any time in Settings." : "Try each one now. This card moves on by itself.";
  if (all) ($("done") as HTMLButtonElement).focus();
}

function mark(s: Step) {
  if (done.has(s)) return;
  // a step done out of order also counts
  done.add(s);
  render();
}

async function finish() {
  document.body.classList.add("pre");
  await new Promise((r) => setTimeout(r, 120));
  invoke("tutorial_finish");
}

async function init() {
  const info = await invoke<Info>("tutorial_info");
  const hk = info.hotkey;
  const steps: [Step, string, (HTMLElement | string)[]][] = [
    ["saved", "Snip anything", ["Tap ", ...keys(hk), ", then drag a box. It's saved right away; tag it or just move on."]],
    ["wheel", "Open the wheel", ["Hold ", ...keys(hk), " for notes, clips, recording and more. Let go on a slice to pick it."]],
    ["gallery", "See your stash", ["Click the Stash icon in the taskbar tray (or the wheel's Gallery slice)."]],
  ];
  steps.forEach(([id, title, body], i) => {
    const li = el("li");
    const t = el("div");
    const sp = el("span");
    sp.append(...body);
    t.append(el("b", "", title), sp);
    li.append(el("div", "n", String(i + 1)), t);
    rows[id] = li;
    $("steps").append(li);
  });
  render();
  await listen<string>("tutorial", (e) => { if (order.includes(e.payload as Step)) mark(e.payload as Step); });
  $("later").onclick = () => { const cur = order.find((s) => !done.has(s)); if (cur) mark(cur); };
  $("done").onclick = finish;
  $("skip").onclick = finish;
  window.addEventListener("keydown", (e) => { if (e.key === "Escape") finish(); });
  invoke("window_ready").catch(() => {});
  requestAnimationFrame(() => {
    document.body.classList.remove("pre");
    document.body.classList.add("opening");
  });
}

init();
