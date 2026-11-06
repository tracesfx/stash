import "./theme";
import { invoke } from "@tauri-apps/api/core";

const SHOW_MS = 1500; // visible time before auto-close
const RESUME_MS = 800; // after hover ends
const NOTICE_MS = 1800;

const card = document.getElementById("card")!;
const img = document.getElementById("img") as HTMLImageElement;
const txt = document.getElementById("txt")!;
const meta = document.getElementById("meta")!;
const bar = document.getElementById("bar")!;
const cap = document.getElementById("cap") as HTMLInputElement;

interface PopupData {
  kind: "image" | "text" | "notice";
  data: string;
  captionable: boolean;
  meta: string;
}

let timer: number | undefined;
let editing = false;
let captionable = true;
let showMs = SHOW_MS;

async function close() {
  card.classList.add("out");
  await new Promise((r) => setTimeout(r, 150));
  invoke("close_popup");
}

/** Auto-close after `ms`, with the countdown bar draining over the same time. */
function arm(ms: number) {
  clearTimeout(timer);
  bar.classList.remove("run");
  if (editing) return;
  bar.style.setProperty("--ms", `${ms}ms`);
  void bar.offsetWidth; // restart the CSS animation
  bar.classList.add("run");
  timer = window.setTimeout(close, ms);
}

invoke<PopupData | null>("popup_data").then((d) => {
  if (!d) return;
  captionable = d.captionable;
  meta.textContent = d.meta;
  if (d.kind === "image") img.src = d.data;
  else {
    document.body.classList.add(d.kind);
    txt.textContent = d.data;
  }
  if (d.kind === "notice") showMs = NOTICE_MS;
  arm(showMs);
});

card.addEventListener("mouseenter", () => clearTimeout(timer));
card.addEventListener("mouseleave", () => arm(RESUME_MS));

card.addEventListener("click", async () => {
  if (editing || !captionable) return;
  editing = true;
  clearTimeout(timer);
  document.body.classList.add("editing");
  await invoke("popup_focus");
  cap.focus();
});

// Native <input>: Ctrl+Backspace, Ctrl+A, Ctrl+Z etc. work out of the box.
cap.addEventListener("keydown", async (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    await invoke("set_caption", { caption: cap.value });
    close();
  } else if (e.key === "Escape") {
    close();
  }
});
