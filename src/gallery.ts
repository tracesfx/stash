// Gallery (SPEC §6, M7): Notes / Visuals / All tabs, a day timeline with sticky headers and a scrubber, tag
// collections, chip tagging, a JS-positioned masonry whose changes animate in place (FLIP), and a Canvas view.
// User text is only ever set through textContent.
import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

interface Item {
  id: string; kind: string; created: string; method: string; caption: string; tags: string[];
  source_title: string; process: string; url: string | null; ext: string; name: string; bytes: number;
  mime: string; w: number | null; h: number | null; original_path: string | null;
  text: string | null; path: string; icon: string;
  palette: string[]; ocr: string | null;
}
type Tab = "notes" | "visuals" | "all";
type View = "grid" | "canvas";

const PAGE = 160; // cards added per step of infinite scroll
const GAP = 14, MINCOL = 220, LISTMAX = 860;
const EASE = "cubic-bezier(.2, .8, .2, 1)", SPRING = "cubic-bezier(.34, 1.4, .5, 1)";
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const grid = $("grid"), wrap = $("grid-wrap"), detail = $("detail-in"), q = $<HTMLInputElement>("q");
const empty = $("empty"), count = $("count"), toast = $("toast"), dropEl = $("drop"), rail = $("rail"), viewer = $("viewer");
const chipsEl = $("chips"), scrub = $("scrub"), canvasEl = $("canvas"), world = $("cv-world"), tagpop = $("tagpop");

// Static icon markup only (never user data).
const SVG = (d: string) => `<svg class="ico" viewBox="0 0 16 16">${d}</svg>`;
const ICON = {
  copy: SVG('<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M3 10.5V4.5A1.5 1.5 0 0 1 4.5 3h6"/>'),
  trash: SVG('<path d="M2.5 4.5h11M6 4.5V3h4v1.5M4 4.5l.6 8.5h6.8l.6-8.5"/>'),
  open: SVG('<path d="M7 3.5H4A1.5 1.5 0 0 0 2.5 5v7A1.5 1.5 0 0 0 4 13.5h7a1.5 1.5 0 0 0 1.5-1.5V9M9.5 2.5h4v4M13.5 2.5l-6 6"/>'),
  link: SVG('<path d="M6.5 9.5l3-3M7 4.5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11.5l-1 1A2.5 2.5 0 0 1 4.5 9l1-1"/>'),
};

let items: Item[] = [];
let shown: Item[] = [];
let limit = PAGE;
let selected = new Set<string>();
let anchor = -1;
let dragging = false;
let toastTimer: number | undefined;
const f = { date: "all", day: "", tag: "", app: "", color: "" };
let viewIdx = -1;
let shuffleJustStarted = false;
let opening = true;

const store = { get: (k: string) => { try { return localStorage.getItem(k); } catch { return null; } }, set: (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* storage unavailable */ } } };
let tab: Tab = (["notes", "visuals", "all"].includes(store.get("stash.gallery.tab") ?? "") ? store.get("stash.gallery.tab") : "all") as Tab;
let view: View = store.get("stash.gallery.view") === "canvas" ? "canvas" : "grid";

const enc = encodeURIComponent;
const thumb = (it: Item) => `http://stash.localhost/thumb/${enc(it.id)}`;
const full = (it: Item) => `http://stash.localhost/full/${enc(it.id)}`;
const isImg = (it: Item) => it.kind === "image";
const isNote = (it: Item) => it.kind === "text" || it.kind === "link";
const VIDEO = new Set(["mp4", "m4v", "webm", "mov"]);
const AUDIO = new Set(["mp3", "wav", "ogg", "oga", "m4a", "flac"]);
const ext = (it: Item) => it.ext.toLowerCase();
const isVideo = (it: Item) => it.kind !== "image" && VIDEO.has(ext(it));
const isAudio = (it: Item) => it.kind !== "image" && AUDIO.has(ext(it));
const isPdf = (it: Item) => ext(it) === "pdf";
const isTextFile = (it: Item) => it.kind === "file" && it.text != null;
const canCopy = (it: Item) => ext(it) === "png" || it.kind === "text" || it.kind === "link";
const reduced = () => document.documentElement.dataset.motion === "off" || matchMedia("(prefers-reduced-motion: reduce)").matches;
const byId = (id: string) => items.find((i) => i.id === id);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text; // textContent only: captions/titles are untrusted
  return e;
}

// ---------- colours ----------
const BUCKETS: [string, string][] = [
  ["red", "#e5484d"], ["orange", "#f08a24"], ["yellow", "#f5d90a"], ["green", "#3fb950"], ["teal", "#1fc2b5"], ["blue", "#3b82f6"],
  ["purple", "#8b5cf6"], ["pink", "#ec4899"], ["brown", "#8b5a3c"], ["gray", "#8a8f98"], ["black", "#16161a"], ["white", "#f4f4f6"],
];
function bucketOf(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, d = mx - mn;
  const sat = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1));
  if (l < 0.14) return "black";
  if (l > 0.9) return "white";
  if (sat < 0.14) return "gray";
  let h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
  h = (h * 60 + 360) % 360;
  if (h >= 12 && h < 48 && l < 0.4) return "brown";
  if (h < 12 || h >= 345) return "red";
  if (h < 40) return "orange";
  if (h < 68) return "yellow";
  if (h < 165) return "green";
  if (h < 200) return "teal";
  if (h < 255) return "blue";
  if (h < 295) return "purple";
  return "pink";
}
const hasColor = (it: Item, name: string) => it.palette.slice(0, 3).some((c) => bucketOf(c) === name);

// ---------- days ----------
const pad = (n: number) => String(n).padStart(2, "0");
const keyOf = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const dayCache = new Map<string, string>();
function dayKey(it: Item): string {
  let k = dayCache.get(it.id);
  if (!k) { k = keyOf(new Date(it.created)); dayCache.set(it.id, k); }
  return k;
}
function dayLabel(key: string, long = false): string {
  const [y, m, d] = key.split("-").map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const diff = Math.round((today.getTime() - date.getTime()) / 86400000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  const opts: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short" };
  if (date.getFullYear() !== today.getFullYear() || long) opts.year = "numeric";
  return date.toLocaleDateString(undefined, opts);
}
const timeOf = (it: Item) => new Date(it.created).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

// ---------- data ----------
async function load(animate = false) {
  const next = await invoke<Item[]>("gallery_list");
  const sig = (l: Item[]) => l.map((i) => i.id + cardVer(i)).join();
  if (items.length && sig(next) === sig(items)) return; // nothing changed: keep scroll, selection and cards
  items = next;
  const ids = new Set(items.map((i) => i.id));
  for (const k of [...cardCache.keys()]) if (!ids.has(k.slice(k.indexOf(":") + 1))) cardCache.delete(k);
  selected = new Set([...selected].filter((id) => ids.has(id)));
  renderRail();
  filter(animate ? "flip" : "none");
  updateSelBar();
  backfill();
}

function haystack(it: Item) {
  return [it.caption, it.tags.join(" "), it.source_title, it.process, it.url ?? "", it.name, it.text ?? "", it.ocr ?? "", it.kind, it.method, it.created.slice(0, 10)].join("\n").toLowerCase();
}

function inDate(it: Item): boolean {
  if (f.day) return dayKey(it) === f.day;
  if (f.date === "all") return true;
  const age = Date.now() - new Date(it.created).getTime();
  const day = 86400000;
  return f.date === "today" ? dayKey(it) === keyOf(new Date()) : f.date === "week" ? age < 7 * day : age < 30 * day;
}
const inTab = (it: Item) => tab === "all" || (tab === "notes") === isNote(it);
const hasTag = (it: Item, t: string) => it.tags.some((x) => x.toLowerCase() === t.toLowerCase());

/** Recomputes the shown list and re-lays out. `anim`: "none" (instant), "flip" (keep cards, animate the
 *  difference), "switch" (as flip, and new cards wait briefly for their images so they enter complete). */
function filter(anim: Anim = "none") {
  const terms = q.value.toLowerCase().split(/\s+/).filter(Boolean);
  shown = items.filter((it) => {
    if (!inTab(it)) return false;
    if (f.tag && !hasTag(it, f.tag)) return false;
    if (f.app && it.process !== f.app) return false;
    if (f.color && !hasColor(it, f.color)) return false;
    if (!inDate(it)) return false;
    const h = haystack(it);
    return terms.every((t) => {
      if (t.startsWith("#")) return it.tags.some((x) => x.toLowerCase().includes(t.slice(1)));
      if (t.startsWith("type:")) return it.kind.startsWith(t.slice(5));
      if (t.startsWith("color:")) return it.palette.slice(0, 3).some((c) => bucketOf(c).startsWith(t.slice(6)));
      return h.includes(t);
    });
  });
  if (anim === "switch") limit = PAGE;
  limit = Math.max(PAGE, Math.min(limit, shown.length));
  renderChips();
  count.textContent = shown.length === items.length ? `${items.length} items` : `${shown.length} of ${items.length}`;
  renderEmpty();
  if (view === "canvas") openCanvas();
  else {
    if (anim === "switch") wrap.scrollTop = 0; // a new view starts at the top; edits and new captures keep the scroll
    layout(opening ? "none" : anim);
  }
  renderScrub();
}

function renderEmpty() {
  empty.classList.toggle("on", shown.length === 0);
  if (shown.length) return;
  empty.replaceChildren();
  empty.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5 9V5h4M15 5h4v4M19 15v4h-4M9 19H5v-4"/><circle cx="12" cy="12" r="1.4" fill="var(--accent)" stroke="none"/></svg>';
  if (items.length) {
    empty.append(el("h2", "", "Nothing here"), el("div", "", tab === "notes" ? "No notes match. Try All, or clear a filter." : "Try another tab, or clear a filter or the search."));
  } else {
    const hint = el("div");
    hint.append("Tap ", Object.assign(el("kbd"), { textContent: "Ctrl" }), " ", Object.assign(el("kbd"), { textContent: "Alt" }), " ", Object.assign(el("kbd"), { textContent: "S" }), " to snip, hold it for the wheel, or drop / paste (Ctrl+V) anything here.");
    empty.append(el("h2", "", "Your library is empty"), hint);
  }
}

// ---------- header: tabs, view toggle, filter chips ----------
function seg(host: HTMLElement, opts: [string, string][], cur: () => string, pick: (v: string) => void) {
  const ind = el("span", "ind");
  host.append(ind);
  const btns: HTMLButtonElement[] = [];
  const move = () => {
    const b = btns.find((x) => x.dataset.v === cur());
    if (!b) return;
    ind.style.width = `${b.offsetWidth}px`;
    ind.style.transform = `translateX(${b.offsetLeft}px)`;
    btns.forEach((x) => x.classList.toggle("on", x === b));
  };
  for (const [v, label] of opts) {
    const b = el("button", "", label);
    b.dataset.v = v;
    b.setAttribute("role", "tab");
    b.onclick = () => { if (cur() !== v) { pick(v); move(); } };
    btns.push(b);
    host.append(b);
  }
  new ResizeObserver(move).observe(host);
  return move;
}
seg($("tabs"), [["notes", "Notes"], ["visuals", "Visuals"], ["all", "All"]], () => tab, (v) => {
  tab = v as Tab;
  store.set("stash.gallery.tab", tab);
  renderRail();
  filter("switch");
});
seg($("views"), [["grid", "Grid"], ["canvas", "Canvas"]], () => view, (v) => setView(v as View));

function renderChips() {
  chipsEl.replaceChildren();
  const chip = (label: string, clear: () => void) => {
    const c = el("button", "chip", label);
    const x = el("b", "", "×");
    c.append(x);
    c.title = "Remove filter";
    c.onclick = () => { clear(); renderRail(); filter("flip"); };
    chipsEl.append(c);
  };
  if (f.day) chip(dayLabel(f.day, true), () => (f.day = ""));
  else if (f.date !== "all") chip({ today: "Today", week: "This week", month: "This month" }[f.date] ?? f.date, () => (f.date = "all"));
  if (f.tag) chip("#" + f.tag, () => (f.tag = ""));
  if (f.app) chip(f.app.replace(/\.exe$/i, ""), () => (f.app = ""));
  if (f.color) chip(f.color, () => (f.color = ""));
  if (chipsEl.childElementCount > 1) {
    const all = el("button", "chip", "Clear all");
    all.onclick = () => { Object.assign(f, { date: "all", day: "", tag: "", app: "", color: "" }); renderRail(); filter("flip"); };
    chipsEl.append(all);
  }
  chipsEl.classList.toggle("on", chipsEl.childElementCount > 0);
}

// ---------- rail: collections, dates, colours, apps ----------
function railItem(label: string, n: number | null, on: boolean, onclick: () => void) {
  const b = el("button", "ri" + (on ? " on" : ""));
  b.append(el("span", "", label));
  if (n !== null) b.append(el("span", "n", String(n)));
  b.onclick = onclick;
  return b;
}
let showAllTags = false;
function renderRail() {
  rail.replaceChildren();
  const section = (title: string, nodes: HTMLElement[]) => { if (nodes.length) rail.append(el("div", "rh", title), ...nodes); };
  const pool = items.filter(inTab);

  // collections: tags with a cover (newest image with that tag, else a text snippet)
  const tagN = new Map<string, number>(), cover = new Map<string, Item>();
  for (const it of pool) for (const t of it.tags) {
    const k = t.toLowerCase();
    tagN.set(k, (tagN.get(k) ?? 0) + 1);
    const c = cover.get(k);
    if (!c || (!isImg(c) && isImg(it))) cover.set(k, it);
  }
  const names = new Map<string, string>();
  for (const it of pool) for (const t of it.tags) if (!names.has(t.toLowerCase())) names.set(t.toLowerCase(), t);
  const ranked = [...tagN.entries()].sort((a, b) => b[1] - a[1]);
  if (ranked.length) {
    const tiles = el("div", "cols");
    for (const [k, n] of ranked.slice(0, 6)) {
      const it = cover.get(k)!;
      const name = names.get(k)!;
      const b = el("button", "col" + (isImg(it) ? "" : " txt") + (f.tag.toLowerCase() === k ? " on" : ""));
      if (isImg(it)) b.style.backgroundImage = `url("${thumb(it)}")`;
      else b.append(el("span", "sn", (it.text ?? it.caption ?? "").slice(0, 120)));
      b.append(el("b", "", "#" + name), el("i", "", `${n} item${n === 1 ? "" : "s"}`));
      b.title = `#${name}`;
      b.onclick = () => { f.tag = f.tag.toLowerCase() === k ? "" : name; renderRail(); filter("switch"); };
      tiles.append(b);
    }
    const rest = ranked.slice(6);
    const nodes: HTMLElement[] = [tiles];
    if (rest.length) {
      if (showAllTags) for (const [k, n] of rest) {
        const name = names.get(k)!;
        nodes.push(railItem("#" + name, n, f.tag.toLowerCase() === k, () => { f.tag = f.tag.toLowerCase() === k ? "" : name; renderRail(); filter("switch"); }));
      }
      const more = el("button", "ri more", showAllTags ? "Fewer tags" : `${rest.length} more tag${rest.length === 1 ? "" : "s"}`);
      more.onclick = () => { showAllTags = !showAllTags; renderRail(); };
      nodes.push(more);
    }
    section("Collections", nodes);
  }

  section("Library", [["all", "All time"], ["today", "Today"], ["week", "This week"], ["month", "This month"]].map(([v, l]) =>
    railItem(l, v === "all" ? pool.length : null, !f.day && f.date === v, () => { f.date = v; f.day = ""; renderRail(); filter("switch"); })));

  if (tab !== "notes" && items.some((i) => i.palette.length)) {
    const sw = el("div", "sws");
    for (const [name, hex] of BUCKETS) {
      const b = el("button", "sw" + (f.color === name ? " on" : ""));
      b.style.background = hex;
      b.title = name;
      b.onclick = () => { f.color = f.color === name ? "" : name; renderRail(); filter("switch"); };
      sw.append(b);
    }
    rail.append(el("div", "rh", "Colour"), sw);
  }
  const apps = new Map<string, number>();
  for (const it of pool) if (it.process) apps.set(it.process, (apps.get(it.process) ?? 0) + 1);
  section("Apps", [...apps.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8).map(([a, n]) =>
    railItem(a.replace(/\.exe$/i, ""), n, f.app === a, () => { f.app = f.app === a ? "" : a; renderRail(); filter("switch"); })));
  if (opening) Array.from(rail.children).forEach((c, i) => (c as HTMLElement).style.setProperty("--i", String(Math.min(i, 24))));
}

// ---------- cards ----------
function actBtn(icon: string, title: string, fn: () => void, cls = ""): HTMLButtonElement {
  const b = el("button", "ab " + cls);
  b.innerHTML = icon; // static markup
  b.title = title;
  b.addEventListener("click", (e) => { e.stopPropagation(); fn(); });
  b.addEventListener("mousedown", (e) => e.stopPropagation());
  b.draggable = false;
  return b;
}
function chkBtn(it: Item) {
  const chk = el("button", "chk");
  chk.title = "Select";
  chk.addEventListener("mousedown", (e) => e.stopPropagation());
  chk.addEventListener("click", (e) => {
    e.stopPropagation();
    selected.has(it.id) ? selected.delete(it.id) : selected.add(it.id);
    anchor = shown.findIndex((i) => i.id === it.id);
    refreshSel();
  });
  return chk;
}
function sourceLabel(it: Item): string {
  if (it.source_title) return it.source_title;
  if (it.url) { try { return new URL(it.url).hostname; } catch { return it.url; } }
  return it.process.replace(/\.exe$/i, "");
}

// Card elements are cached per layout kind and re-used, so their images stay decoded across view changes.
const cardCache = new Map<string, { el: HTMLElement; v: string }>();
const cardVer = (it: Item) => `${it.caption}|${it.tags.join(",")}|${it.palette.length}|${it.w ?? ""}`;
const cardKind = () => (tab === "notes" ? "note" : "card");
function cardFor(it: Item, eager: boolean): HTMLElement {
  const k = `${cardKind()}:${it.id}`;
  const v = cardVer(it);
  const hit = cardCache.get(k);
  if (hit && hit.v === v) {
    hit.el.classList.toggle("sel", selected.has(it.id));
    hit.el.classList.remove("leaving");
    return hit.el;
  }
  const c = cardKind() === "note" ? noteRow(it) : card(it, eager);
  if (hit) { const old = hit.el; if (old.isConnected) { c.style.cssText = old.style.cssText; old.replaceWith(c); } posOf.set(c, posOf.get(old)!); }
  cardCache.set(k, { el: c, v });
  if (cardCache.size > 1500) { const oldest = cardCache.keys().next().value; if (oldest) cardCache.delete(oldest); }
  return c;
}

function card(it: Item, eager = false): HTMLElement {
  const c = el("div", "card" + (selected.has(it.id) ? " sel" : ""));
  c.dataset.id = it.id;
  c.draggable = true;
  c.append(chkBtn(it));
  if (isImg(it)) {
    const img = el("img");
    img.loading = eager ? "eager" : "lazy";
    img.draggable = false;
    img.style.aspectRatio = it.w && it.h ? `${it.w} / ${it.h}` : "4 / 3";
    img.onload = () => {
      img.classList.add("ok");
      if (!it.w && img.naturalWidth) {
        // the sidecar had no size: take the thumbnail's and re-flow
        it.w = img.naturalWidth; it.h = img.naturalHeight;
        img.style.aspectRatio = `${it.w} / ${it.h}`;
        relayoutSoon();
      }
    };
    img.src = thumb(it);
    c.append(img);
  } else if (isVideo(it)) {
    const v = el("video");
    v.muted = true; v.loop = true; v.preload = "metadata"; v.draggable = false;
    v.style.aspectRatio = it.w && it.h ? `${it.w} / ${it.h}` : "16 / 9";
    v.src = full(it) + "#t=0.1";
    c.addEventListener("mouseenter", () => { v.play().catch(() => {}); });
    c.addEventListener("mouseleave", () => { v.pause(); v.currentTime = 0.1; });
    c.append(v, el("span", "vbadge", "▶"));
  } else if (it.kind === "text" || it.kind === "link" || isTextFile(it)) {
    const t = el("div", "t" + (it.kind === "link" ? " link" : ""), it.text ?? "");
    if (isTextFile(it)) { t.classList.add("code"); t.append(el("span", "badge", it.ext)); }
    c.append(t);
    if (isNote(it) && sourceLabel(it)) { const s = el("div", "src"); s.append(el("span", "", sourceLabel(it))); c.append(s); }
  } else {
    const fl = el("div", "f");
    fl.append(el("span", "badge", it.ext || "file"), el("span", "", it.original_path?.split(/[\\/]/).pop() ?? it.name));
    c.append(fl);
  }
  if (it.palette.length && isImg(it)) {
    const strip = el("div", "strip");
    for (const col of it.palette.slice(0, 5)) { const i = el("i"); i.style.background = col; strip.append(i); }
    c.append(strip);
  }
  if (it.caption) c.append(el("div", "cap", it.caption));
  const acts = el("div", "acts");
  if (canCopy(it)) acts.append(actBtn(ICON.copy, "Copy", () => copyItem(it)));
  if (it.url) acts.append(actBtn(ICON.open, "Open source page", () => invoke("gallery_open_url", { url: it.url })));
  acts.append(actBtn(ICON.trash, "Move to trash", () => trash([it.id]), "d"));
  c.append(acts);
  return c;
}

/** Notes tab row: the note itself, then where it came from (the page title, e.g. the YouTube video). */
function noteRow(it: Item): HTMLElement {
  const c = el("div", "card note" + (selected.has(it.id) ? " sel" : ""));
  c.dataset.id = it.id;
  c.draggable = true;
  c.append(chkBtn(it));
  c.append(el("div", "nt" + (it.kind === "link" ? " link" : ""), it.caption && it.kind === "link" ? `${it.caption}\n${it.text ?? ""}` : it.text ?? it.caption));
  const s = el("div", "src");
  const label = sourceLabel(it);
  if (label) {
    if (it.url) {
      const ico = el("span"); ico.innerHTML = ICON.link; // static markup
      const a = el("a", "", label);
      a.title = it.url;
      a.addEventListener("click", (e) => { e.stopPropagation(); invoke("gallery_open_url", { url: it.url }); });
      s.append(ico, a);
    } else s.append(el("span", "", label));
  }
  for (const t of it.tags.slice(0, 4)) s.append(el("span", "tg", "#" + t));
  s.append(el("span", "tm", timeOf(it)));
  c.append(s);
  return c;
}

// ---------- layout: day sections + JS masonry, animated with FLIP ----------
type Anim = "none" | "flip" | "switch" | "enter";
interface Pos { x: number; y: number; w: number }
const posOf = new WeakMap<HTMLElement, Pos>();
const sections = new Map<string, { el: HTMLElement; head: HTMLButtonElement; body: HTMLElement }>();
const liveCards = () => Array.from(grid.querySelectorAll<HTMLElement>(".day:not(.leaving) .card:not(.leaving)"));
const tf = (p: { x: number; y: number }, dx = 0, dy = 0, s = 1) => `translate(${p.x + dx}px, ${p.y + dy}px)${s !== 1 ? ` scale(${s})` : ""}`;

function makeSection(key: string) {
  const s = el("section", "day");
  s.dataset.day = key;
  const h = el("div", "dh");
  const head = el("button");
  head.title = "Show only this day (e.g. to lay it out on the canvas)";
  head.onclick = () => { f.day = f.day === key ? "" : key; renderRail(); filter("switch"); };
  h.append(head);
  const body = el("div", "dbody");
  s.append(h, body);
  const sec = { el: s, head, body };
  sections.set(key, sec);
  return sec;
}

function leave(node: HTMLElement, animate: boolean, from: Pos | null) {
  node.classList.add("leaving");
  if (!animate) { node.remove(); return; }
  const a = from
    ? node.animate([{ opacity: 1, transform: tf(from) }, { opacity: 0, transform: tf(from, 0, 0, 0.86) }], { duration: 180, easing: EASE, fill: "forwards" })
    : node.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 160, easing: EASE, fill: "forwards" });
  a.onfinish = () => { if (node.classList.contains("leaving")) node.remove(); };
}

let layoutToken = 0;
function layout(anim: Anim) {
  const token = ++layoutToken;
  const animate = anim !== "none" && !reduced();
  const vh = wrap.clientHeight;
  // FIRST: where the cards on screen are now
  const first = new Map<HTMLElement, DOMRect>();
  if (animate && anim !== "enter") for (const c of liveCards()) { const r = c.getBoundingClientRect(); if (r.bottom > -100 && r.top < innerHeight + 100) first.set(c, r); }

  const W = grid.clientWidth;
  const list = tab === "notes";
  const cols = list ? 1 : Math.max(1, Math.floor((W + GAP) / (MINCOL + GAP)));
  const colW = list ? Math.min(W, LISTMAX) : Math.floor((W - GAP * (cols - 1)) / cols);

  // group the rendered part of the list by day (headers count the whole day)
  const totals = new Map<string, number>();
  for (const it of shown) totals.set(dayKey(it), (totals.get(dayKey(it)) ?? 0) + 1);
  const groups = new Map<string, Item[]>();
  shown.slice(0, limit).forEach((it) => { const k = dayKey(it); (groups.get(k) ?? groups.set(k, []).get(k)!).push(it); });

  // sections that go away leave the flow at once (so nothing below jumps) and fade out
  for (const [k, s] of sections) if (!groups.has(k)) {
    sections.delete(k);
    Object.assign(s.el.style, { top: `${s.el.offsetTop}px` });
    leave(s.el, animate, null);
  }
  let prev: HTMLElement | null = null;
  const placed: HTMLElement[] = [];
  let eager = 0;
  for (const [k, its] of groups) {
    const s = sections.get(k) ?? makeSection(k);
    if (prev ? prev.nextElementSibling !== s.el : grid.firstElementChild !== s.el) { if (prev) prev.after(s.el); else grid.prepend(s.el); }
    prev = s.el;
    s.head.replaceChildren(dayLabel(k), el("span", "", String(totals.get(k) ?? its.length)));
    const want = new Set(its.map((i) => i.id));
    for (const c of Array.from(s.body.children) as HTMLElement[]) {
      const kindOk = list === c.classList.contains("note");
      if (!c.classList.contains("leaving") && !(want.has(c.dataset.id!) && kindOk)) leave(c, animate && first.has(c), posOf.get(c) ?? null);
    }
    for (const it of its) {
      const c = cardFor(it, eager++ < 40);
      if (c.parentElement !== s.body) s.body.append(c);
      if (c.style.width !== `${colW}px`) c.style.width = `${colW}px`;
      placed.push(c);
    }
  }
  // MEASURE (one reflow), then POSITION per section
  const hs = placed.map((c) => c.offsetHeight);
  let i = 0;
  for (const [k, its] of groups) {
    const s = sections.get(k)!;
    const colH = new Array(cols).fill(0);
    for (let n = 0; n < its.length; n++, i++) {
      const c = placed[i];
      let col = 0;
      for (let j = 1; j < cols; j++) if (colH[j] < colH[col] - 1) col = j;
      const p = { x: col * (colW + GAP), y: colH[col], w: colW };
      colH[col] += hs[i] + (list ? 10 : GAP);
      posOf.set(c, p);
      c.style.transform = tf(p);
    }
    s.body.style.height = `${Math.max(0, Math.max(...colH) - (list ? 10 : GAP))}px`;
  }
  if (!animate) return;

  // LAST + PLAY: moved cards glide from where they were; new ones on screen cascade in by position
  const entering: { c: HTMLElement; r: DOMRect }[] = [];
  const wr = wrap.getBoundingClientRect();
  for (const c of placed) {
    const p = posOf.get(c)!;
    const r = c.getBoundingClientRect();
    const was = first.get(c);
    if (was) {
      const dx = was.left - r.left, dy = was.top - r.top;
      if (Math.abs(dx) + Math.abs(dy) > 0.5) c.animate([{ transform: tf(p, dx, dy) }, { transform: tf(p) }], { duration: 220, easing: EASE });
    } else if (r.bottom > wr.top && r.top < wr.top + vh + 40) entering.push({ c, r });
  }
  if (!entering.length) return;
  const lefts = [...new Set(entering.map((e) => Math.round(e.r.left)))].sort((a, b) => a - b);
  const play = () => {
    if (token !== layoutToken) return;
    for (const { c, r } of entering) {
      const p = posOf.get(c)!;
      const d = anim === "enter"
        ? Math.min(600, 120 + ((r.top - wr.top) / 100) * 55 + lefts.indexOf(Math.round(r.left)) * 45)
        : Math.min(360, ((r.top - wr.top) / 100) * 26 + lefts.indexOf(Math.round(r.left)) * 34);
      c.style.opacity = "";
      c.animate([{ opacity: 0, transform: tf(p, 0, anim === "enter" ? 26 : 14, anim === "enter" ? 0.9 : 0.95) }, { opacity: 1, transform: tf(p) }],
        { duration: anim === "enter" ? 600 : 420, easing: SPRING, delay: d, fill: "backwards" });
    }
  };
  // a switch waits a moment for the new pictures, so cards enter complete instead of as empty frames
  const waiting = anim === "switch" ? entering.map((e) => e.c.querySelector("img")).filter((im): im is HTMLImageElement => !!im && !im.complete) : [];
  if (!waiting.length) { play(); return; }
  for (const { c } of entering) c.style.opacity = "0";
  Promise.race([Promise.all(waiting.map((im) => im.decode().catch(() => {}))), new Promise((r) => setTimeout(r, 240))]).then(play);
}

let relayoutRaf = 0;
function relayoutSoon(anim: Anim = "none") {
  if (relayoutRaf) return;
  relayoutRaf = requestAnimationFrame(() => { relayoutRaf = 0; if (view === "grid") layout(anim); });
}
// width changes (window resize, the detail panel opening) re-flow after they settle, with a glide
let resizeTimer: number | undefined, lastW = 0;
new ResizeObserver(() => {
  const w = grid.clientWidth;
  if (w === lastW || view !== "grid") return;
  lastW = w;
  clearTimeout(resizeTimer);
  resizeTimer = window.setTimeout(() => layout("flip"), 90);
}).observe(wrap);

new IntersectionObserver((e) => {
  if (e[0].isIntersecting && limit < shown.length && view === "grid") { limit += PAGE; layout("none"); renderScrub(); }
}, { root: wrap, rootMargin: "800px" }).observe($("sentinel"));

// ---------- timeline scrubber ----------
let scrubDays: { key: string; at: number; n: number }[] = [];
function renderScrub() {
  scrub.replaceChildren();
  const days = new Map<string, number>();
  for (const it of shown) days.set(dayKey(it), (days.get(dayKey(it)) ?? 0) + 1);
  scrub.classList.toggle("off", view !== "grid" || days.size < 3);
  if (days.size < 3) return;
  // positions follow item counts, so busy days get more room
  let acc = 0;
  scrubDays = [...days.entries()].map(([key, n]) => { const d = { key, at: acc / shown.length, n }; acc += n; return d; });
  let lastMonth = "", lastAt = -1;
  for (const d of scrubDays) {
    const month = d.key.slice(0, 7);
    const tk = el("i", "tk" + (month !== lastMonth ? " m" : ""));
    tk.style.top = `${d.at * 100}%`;
    scrub.append(tk);
    if (month !== lastMonth && d.at - lastAt > 0.06) {
      const [y, m] = month.split("-").map(Number);
      const ml = el("span", "ml", new Date(y, m - 1, 1).toLocaleDateString(undefined, { month: "short", year: y === new Date().getFullYear() ? undefined : "2-digit" }));
      ml.style.top = `${d.at * 100}%`;
      scrub.append(ml);
      lastAt = d.at;
    }
    lastMonth = month;
  }
  scrub.append(el("span", "bub"), el("i", "pos"));
  syncScrubPos();
}
function dayAtFrac(fr: number) {
  let best = scrubDays[0];
  for (const d of scrubDays) if (d.at <= fr) best = d;
  return best;
}
function jumpToDay(key: string) {
  const idx = shown.findIndex((it) => dayKey(it) === key);
  if (idx < 0) return;
  if (idx >= limit - 20) { limit = Math.min(shown.length, idx + PAGE); layout("none"); }
  const s = sections.get(key);
  if (s) wrap.scrollTop = s.el.offsetTop;
}
function syncScrubPos() {
  const pos = scrub.querySelector<HTMLElement>(".pos");
  if (!pos || !scrubDays.length) return;
  const top = wrap.scrollTop;
  let cur = scrubDays[0];
  for (const [k, s] of sections) if (s.el.offsetTop <= top + 40) { const d = scrubDays.find((x) => x.key === k); if (d) cur = d; }
  pos.style.top = `${cur.at * 100}%`;
}
wrap.addEventListener("scroll", () => requestAnimationFrame(syncScrubPos), { passive: true });
scrub.addEventListener("pointermove", (e) => {
  const r = scrub.getBoundingClientRect();
  const fr = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
  const d = dayAtFrac(fr);
  const bub = scrub.querySelector<HTMLElement>(".bub");
  if (d && bub) { bub.textContent = dayLabel(d.key); bub.style.top = `${fr * 100}%`; }
  if (scrub.classList.contains("drag") && d) jumpToDay(d.key);
});
scrub.addEventListener("pointerdown", (e) => {
  scrub.setPointerCapture(e.pointerId);
  scrub.classList.add("drag");
  const r = scrub.getBoundingClientRect();
  const d = dayAtFrac(Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)));
  if (d) jumpToDay(d.key);
  const up = () => scrub.classList.remove("drag");
  scrub.addEventListener("pointerup", up, { once: true });
  scrub.addEventListener("pointercancel", up, { once: true });
});

// ---------- selection ----------
function markSel() {
  document.querySelectorAll<HTMLElement>(".card, .cv-item").forEach((c) => c.classList.toggle("sel", selected.has(c.dataset.id!)));
  updateSelBar();
}
function refreshSel() {
  markSel();
  showDetail();
}

const selbar = el("div");
selbar.id = "selbar";
document.body.append(selbar);
function updateSelBar() {
  document.body.classList.toggle("multi", selected.size > 0);
  selbar.replaceChildren();
  selbar.classList.toggle("on", selected.size > 0);
  if (!selected.size) { tagpop.classList.remove("on"); return; }
  const btn = (label: string, fn: () => void, cls = "") => { const b = el("button", "btn " + cls, label); b.onclick = fn; return b; };
  const ids = () => [...selected];
  selbar.append(
    el("span", "n", `${selected.size} selected`),
    btn("Tag…", () => openTagPop()),
    btn("Select all", () => { selected = new Set(shown.map((i) => i.id)); refreshSel(); }),
    btn("Export…", () => exportItems(ids())),
    btn("Trash", () => trash(ids()), "danger"),
    btn("✕", () => { selected.clear(); refreshSel(); }),
  );
}

// Rubber-band selection: drag on empty space; Ctrl/Shift adds to the current selection.
const marq = el("div");
marq.id = "marq";
document.body.append(marq);
wrap.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || (e.target as HTMLElement).closest(".card, .dh button")) return;
  if (e.clientX >= wrap.getBoundingClientRect().left + wrap.clientWidth) return; // scrollbar
  const [x0, y0] = [e.clientX, e.clientY];
  const base = e.ctrlKey || e.shiftKey ? new Set(selected) : new Set<string>();
  let active = false;
  const move = (m: PointerEvent) => {
    if (!active && Math.hypot(m.clientX - x0, m.clientY - y0) < 5) return;
    active = true;
    const [l, t, r, b] = [Math.min(x0, m.clientX), Math.min(y0, m.clientY), Math.max(x0, m.clientX), Math.max(y0, m.clientY)];
    Object.assign(marq.style, { display: "block", left: l + "px", top: t + "px", width: r - l + "px", height: b - t + "px" });
    const hits = new Set(base);
    liveCards().forEach((c) => {
      const q = c.getBoundingClientRect();
      if (q.left < r && q.right > l && q.top < b && q.bottom > t) hits.add(c.dataset.id!);
    });
    selected = hits;
    markSel();
    const wr = wrap.getBoundingClientRect();
    if (m.clientY > wr.bottom - 30) wrap.scrollTop += 18; else if (m.clientY < wr.top + 30) wrap.scrollTop -= 18;
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    marq.style.display = "none";
    if (!active && !base.size) selected.clear();
    refreshSel();
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up, { once: true });
});

function clickSelect(id: string, e: MouseEvent) {
  const idx = shown.findIndex((i) => i.id === id);
  if (e.shiftKey && anchor >= 0) {
    const [a, b] = [Math.min(anchor, idx), Math.max(anchor, idx)];
    selected = new Set(shown.slice(a, b + 1).map((i) => i.id));
  } else if (e.ctrlKey || e.metaKey) {
    selected.has(id) ? selected.delete(id) : selected.add(id);
    anchor = idx;
  } else {
    selected = new Set([id]);
    anchor = idx;
  }
  refreshSel();
}
grid.addEventListener("click", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (c) clickSelect(c.dataset.id!, e);
});
grid.addEventListener("dblclick", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (c) openViewer(shown.findIndex((i) => i.id === c.dataset.id));
});

/** Arrow keys: left/right step through the list, up/down go to the nearest card above/below. */
function moveSel(key: string) {
  const cur = selected.size === 1 ? [...selected][0] : null;
  const idx = cur ? shown.findIndex((i) => i.id === cur) : -1;
  let next = -1;
  if (idx < 0) next = 0;
  else if (key === "ArrowRight") next = Math.min(shown.length - 1, idx + 1);
  else if (key === "ArrowLeft") next = Math.max(0, idx - 1);
  else if (view === "grid") {
    const from = document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(cur!)}"]:not(.leaving)`)?.getBoundingClientRect();
    if (!from) next = idx;
    else {
      const down = key === "ArrowDown";
      let best: HTMLElement | null = null, bd = Infinity;
      for (const c of liveCards()) {
        const r = c.getBoundingClientRect();
        const dy = down ? r.top - from.bottom : from.top - r.bottom;
        if (dy < -4) continue;
        const d = dy + Math.abs(r.left + r.width / 2 - (from.left + from.width / 2)) * 2;
        if (d < bd && c.dataset.id !== cur) { bd = d; best = c; }
      }
      next = best ? shown.findIndex((i) => i.id === best!.dataset.id) : idx;
    }
  } else next = Math.min(shown.length - 1, Math.max(0, idx + (key === "ArrowDown" ? 1 : -1)));
  if (next < 0 || !shown[next]) return;
  if (next >= limit) { limit = next + PAGE; layout("none"); }
  selected = new Set([shown[next].id]);
  anchor = next;
  refreshSel();
  document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(shown[next].id)}"]:not(.leaving), .cv-item[data-id="${CSS.escape(shown[next].id)}"]`)?.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
}

grid.addEventListener("dragstart", async (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (!c) return;
  e.preventDefault(); // replace the HTML5 drag with a native file drag
  const id = c.dataset.id!;
  const ids = selected.has(id) ? [...selected] : [id];
  const picked = ids.map((i) => byId(i)!).filter(Boolean);
  dragging = true;
  try {
    await startDrag({ item: picked.map((p) => p.path), icon: picked[0].icon });
  } catch (err) {
    console.error(err);
  } finally {
    setTimeout(() => (dragging = false), 400);
  }
});

// ---------- viewer: zooms open from its card ----------
function openViewer(i: number) {
  if (i < 0 || i >= shown.length) return;
  const wasOpen = viewer.classList.contains("on");
  if (!wasOpen && !history.length && viewIdx === -1 && !shuffleJustStarted) shuffle = false;
  shuffleJustStarted = false;
  viewIdx = i;
  const it = shown[i];
  viewer.replaceChildren();
  let media: HTMLElement | null = null;
  if (isImg(it)) {
    const img = el("img");
    img.src = thumb(it); // already decoded in the grid: the zoom starts from a real picture
    if (ext(it) === "png" || ext(it) === "jpg" || ext(it) === "jpeg") {
      const hi = new Image();
      hi.onload = () => { if (viewIdx === i) img.src = hi.src; };
      hi.src = full(it);
    }
    if (it.w && it.h) img.style.aspectRatio = `${it.w} / ${it.h}`;
    img.style.width = it.w && it.h ? `min(92vw, ${it.w}px, calc(84vh * ${it.w / it.h}))` : "";
    viewer.append(img);
    media = img;
  } else if (isVideo(it)) {
    const v = el("video");
    v.src = full(it); v.controls = true; v.autoplay = true; v.loop = true;
    v.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(v);
  } else if (isAudio(it)) {
    const box = el("div", "vt aud", it.original_path?.split(/[\\/]/).pop() ?? it.name);
    const a = el("audio");
    a.src = full(it); a.controls = true; a.autoplay = true;
    a.addEventListener("click", (e) => e.stopPropagation());
    box.append(a);
    viewer.append(box);
  } else if (isPdf(it)) {
    const fr = el("iframe");
    fr.src = full(it);
    fr.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(fr);
  } else if (isTextFile(it)) {
    const pre = el("div", "vt code", "Loading…");
    pre.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(pre);
    fetch(full(it)).then((r) => r.text()).then((t) => { if (viewIdx === i) pre.textContent = t.length > 300000 ? t.slice(0, 300000) + "\n\n… (truncated)" : t; }).catch(() => (pre.textContent = it.text ?? ""));
  } else {
    const t = el("div", "vt", it.text ?? it.name);
    viewer.append(t);
    media = t;
  }
  if (it.caption) viewer.append(el("div", "vc", it.caption));
  if (shuffle) viewer.append(el("div", "vi", "Shuffle · Space or → next · ← back · Esc to exit"));
  for (const [dir, label, side] of [[-1, "‹", "left"], [1, "›", "right"]] as const) {
    const b = el("button", "vn", label);
    b.style[side] = "22px";
    b.onclick = (e) => { e.stopPropagation(); stepViewer(dir); };
    viewer.append(b);
  }
  viewer.classList.add("on");
  // zoom from the card (FLIP on the media element)
  const src = document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(it.id)}"]:not(.leaving), .cv-item[data-id="${CSS.escape(it.id)}"]`);
  if (!wasOpen && media && src && !reduced()) {
    const a = src.getBoundingClientRect(), b = media.getBoundingClientRect();
    if (a.width && b.width) {
      const sx = a.width / b.width, sy = a.height / b.height;
      media.animate([
        { transformOrigin: "0 0", transform: `translate(${a.left - b.left}px, ${a.top - b.top}px) scale(${sx}, ${sy})`, borderRadius: "12px" },
        { transformOrigin: "0 0", transform: "none" },
      ], { duration: 240, easing: EASE });
    }
  }
}
function closeViewer() {
  viewer.classList.remove("on");
  viewIdx = -1;
  shuffle = false;
}
viewer.addEventListener("click", () => { if (shuffle) { stepViewer(1); return; } closeViewer(); });

// ---------- tag chip input with autocomplete ----------
function allTags(): [string, number][] {
  const m = new Map<string, [string, number]>();
  for (const it of items) for (const t of it.tags) { const k = t.toLowerCase(); const e = m.get(k); m.set(k, [e?.[0] ?? t, (e?.[1] ?? 0) + 1]); }
  return [...m.values()].sort((a, b) => b[1] - a[1]);
}
/** Chips for `tags` plus a text field: Enter/comma/Tab adds (or picks the highlighted suggestion), Backspace on an
 *  empty field removes the last chip, ↑/↓ move through suggestions. */
function tagInput(tags: string[], onAdd: (t: string) => void, onRemove: (t: string) => void, placeholder = "Add a tag…"): HTMLElement {
  const box = el("div", "tagin");
  const inp = el("input");
  inp.placeholder = placeholder;
  inp.spellcheck = false;
  const sugg = el("div", "sugg");
  let hi = -1;
  let cur = [...tags];
  const chips = () => {
    box.querySelectorAll(".tc").forEach((c) => c.remove());
    for (const t of cur) {
      const c = el("span", "tc", "#" + t);
      const x = el("button", "", "×");
      x.title = `Remove #${t}`;
      x.onclick = (e) => { e.stopPropagation(); cur = cur.filter((y) => y !== t); chips(); onRemove(t); };
      c.append(x);
      box.insertBefore(c, inp);
    }
  };
  const options = () => {
    const v = inp.value.trim().replace(/^#/, "").toLowerCase();
    return allTags().filter(([t]) => !cur.some((c) => c.toLowerCase() === t.toLowerCase()) && (!v || t.toLowerCase().includes(v))).slice(0, 6);
  };
  const paint = () => {
    const o = options();
    sugg.replaceChildren(...o.map(([t, n], i) => {
      const b = el("button", i === hi ? "hi" : "", "#" + t);
      b.append(el("span", "", String(n)));
      b.onmousedown = (e) => { e.preventDefault(); add(t); };
      return b;
    }));
    sugg.classList.toggle("on", document.activeElement === inp && o.length > 0);
  };
  const add = (raw: string) => {
    const t = raw.trim().replace(/^#+/, "").trim();
    inp.value = "";
    hi = -1;
    if (t && !cur.some((c) => c.toLowerCase() === t.toLowerCase())) { cur.push(t); chips(); onAdd(t); }
    paint();
  };
  inp.addEventListener("input", () => {
    if (/[,\s]$/.test(inp.value) && inp.value.trim()) { add(inp.value.replace(/[,\s]+$/, "")); return; }
    hi = -1;
    paint();
  });
  inp.addEventListener("keydown", (e) => {
    const o = options();
    if (e.key === "ArrowDown" && o.length) { e.preventDefault(); hi = (hi + 1) % o.length; paint(); }
    else if (e.key === "ArrowUp" && o.length) { e.preventDefault(); hi = (hi - 1 + o.length) % o.length; paint(); }
    else if (e.key === "Enter" || (e.key === "Tab" && (inp.value.trim() || hi >= 0))) {
      e.preventDefault();
      if (hi >= 0 && o[hi]) add(o[hi][0]); else if (inp.value.trim()) add(inp.value); else inp.blur();
    } else if (e.key === "Backspace" && !inp.value && cur.length) { const t = cur.pop()!; chips(); onRemove(t); paint(); }
    else if (e.key === "Escape") { e.stopPropagation(); inp.value = ""; inp.blur(); }
    e.stopPropagation(); // gallery shortcuts stay off while typing
  });
  inp.addEventListener("focus", paint);
  inp.addEventListener("blur", () => { sugg.classList.remove("on"); if (inp.value.trim()) add(inp.value); });
  box.addEventListener("click", () => inp.focus());
  box.append(inp, sugg);
  chips();
  return box;
}

/** Re-renders an item's card in place after an edit, keeping its position. */
function refreshItem(it: Item) {
  for (const kind of ["card", "note"]) {
    const hit = cardCache.get(`${kind}:${it.id}`);
    if (!hit) continue;
    cardCache.delete(`${kind}:${it.id}`);
    if (hit.el.isConnected && kind === cardKind()) {
      const c = cardFor(it, true);
      c.style.cssText = hit.el.style.cssText;
      posOf.set(c, posOf.get(hit.el)!);
      hit.el.replaceWith(c);
    }
  }
  relayoutSoon();
}

async function saveMeta(it: Item, caption: string, tags: string[]) {
  await invoke("gallery_set_meta", { id: it.id, caption, tags });
  it.caption = caption.trim();
  it.tags = tags;
  refreshItem(it);
  renderRail();
}

// ---------- detail panel ----------
function fmtBytes(n: number) { return n > 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(n / 1e3)) + " KB"; }

function showDetail() {
  detail.replaceChildren();
  detail.classList.remove("fresh");
  void detail.offsetWidth;
  detail.classList.add("fresh");
  const sel = [...selected].map((id) => byId(id)!).filter(Boolean);
  document.body.classList.toggle("detail", sel.length > 0);
  if (!sel.length) return;
  if (sel.length > 1) {
    detail.append(el("h3", "", `${sel.length} selected`));
    detail.append(el("label", "l", "Tags on all of them"), multiTagInput(sel));
    const row = el("div", "row");
    const ex = el("button", "btn", "Export…");
    ex.onclick = () => exportItems(sel.map((s) => s.id));
    const del = el("button", "btn danger", "Move to trash");
    del.onclick = () => trash(sel.map((s) => s.id));
    row.append(ex, del);
    detail.append(row);
    return;
  }
  const it = sel[0];
  if (isImg(it)) {
    const pv = el("img", "pv");
    pv.src = ext(it) === "png" ? full(it) : thumb(it);
    pv.onclick = () => openViewer(shown.findIndex((x) => x.id === it.id));
    detail.append(pv);
  } else if (it.text) {
    detail.append(el("div", "tx", it.text));
  }

  const cap = el("input", "field"); cap.type = "text"; cap.value = it.caption; cap.placeholder = "Add a caption";
  cap.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); cap.blur(); }
    else if (e.key === "Escape") { cap.value = it.caption; cap.blur(); e.stopPropagation(); }
  });
  cap.addEventListener("blur", () => { if (cap.value.trim() !== it.caption) saveMeta(it, cap.value, it.tags); });
  const tags = tagInput(it.tags,
    (t) => saveMeta(it, it.caption, [...it.tags, t]),
    (t) => saveMeta(it, it.caption, it.tags.filter((x) => x !== t)));
  detail.append(el("label", "l", "Caption"), cap, el("label", "l", "Tags"), tags);
  if (it.palette.length) {
    const pal = el("div", "pal");
    for (const col of it.palette) {
      const b = el("button", "dot");
      b.style.background = col;
      b.title = `${col} — click to filter by this colour`;
      b.onclick = () => { f.color = bucketOf(col); renderRail(); filter("switch"); };
      pal.append(b);
    }
    detail.append(el("label", "l", "Palette"), pal);
  }
  if (it.ocr) {
    const d = el("details", "ocr");
    d.append(el("summary", "", "Text found in image"), el("div", "", it.ocr));
    detail.append(d);
  }

  const row = el("div", "row");
  const mk = (label: string, fn: () => void, cls = "") => { const b = el("button", "btn " + cls, label); b.onclick = fn; row.append(b); };
  if (canCopy(it)) mk("Copy", () => copyItem(it));
  mk("Show in folder", () => invoke("gallery_reveal", { id: it.id }));
  if (it.url) mk("Open source", () => invoke("gallery_open_url", { url: it.url }));
  mk("Export…", () => exportItems([it.id]));
  mk("Trash", () => trash([it.id]), "danger");
  detail.append(row);

  const meta = el("div", "meta");
  const add = (k: string, v: string | HTMLElement) => { const b = el("b"); typeof v === "string" ? (b.textContent = v) : b.append(v); meta.append(el("span", "", k), b); };
  add("Saved", new Date(it.created).toLocaleString());
  add("How", it.method);
  if (it.w && it.h) add("Size", `${it.w} × ${it.h}`);
  add("File", `${it.ext.toUpperCase()} · ${fmtBytes(it.bytes)}`);
  if (it.process) add("App", it.process);
  if (it.source_title) add("Window", it.source_title);
  if (it.url) { const a = el("a", "", it.url); a.onclick = () => invoke("gallery_open_url", { url: it.url }); add("URL", a); }
  if (it.original_path) add("From", it.original_path);
  detail.append(meta);
}

/** Tags shared by every selected item: removing one removes it from all, adding adds to all. */
function multiTagInput(sel: Item[]): HTMLElement {
  const common = sel[0].tags.filter((t) => sel.every((s) => hasTag(s, t)));
  const apply = async (add: string[], remove: string[]) => {
    const ids = sel.map((s) => s.id);
    await invoke("gallery_tag", { ids, add, remove });
    for (const s of sel) {
      s.tags = s.tags.filter((t) => !remove.some((r) => r.toLowerCase() === t.toLowerCase()));
      for (const a of add) if (!hasTag(s, a)) s.tags.push(a);
      refreshItem(s);
    }
    renderRail();
    flash(add.length ? `Tagged ${sel.length} with #${add[0]}` : `Removed #${remove[0]} from ${sel.length}`);
  };
  return tagInput(common, (t) => apply([t], []), (t) => apply([], [t]), "Tag all selected…");
}

function openTagPop() {
  const sel = [...selected].map((id) => byId(id)!).filter(Boolean);
  if (!sel.length) return;
  tagpop.replaceChildren(el("h4", "", `Tag ${sel.length} item${sel.length === 1 ? "" : "s"}`), multiTagInput(sel));
  tagpop.classList.add("on");
  tagpop.querySelector("input")?.focus();
}
document.addEventListener("pointerdown", (e) => {
  if (tagpop.classList.contains("on") && !(e.target as HTMLElement).closest("#tagpop, #selbar")) tagpop.classList.remove("on");
});

// ---------- export ----------
async function exportItems(ids?: string[]) {
  const list = ids ?? (selected.size ? [...selected] : shown.map((i) => i.id));
  if (!list.length) { flash("Nothing to export"); return; }
  const dest = await pickFolder({ directory: true, title: `Export ${list.length} item${list.length === 1 ? "" : "s"} to…` });
  if (typeof dest !== "string") return;
  try {
    const out = await invoke<string>("gallery_export", { ids: list, dest });
    flash(`Exported ${list.length} item${list.length === 1 ? "" : "s"}`, { label: "Open folder", fn: () => invoke("gallery_open_path", { path: out }) });
  } catch (e) {
    flash(String(e));
  }
}
$("export").addEventListener("click", () => exportItems());

// ---------- shuffle ----------
let shuffle = false;
let history: number[] = [];
function randomIdx(): number {
  if (shown.length < 2) return 0;
  let i = viewIdx;
  while (i === viewIdx) i = Math.floor(Math.random() * shown.length);
  return i;
}
function startShuffle() {
  if (!shown.length) { flash("Nothing to shuffle"); return; }
  shuffle = true;
  shuffleJustStarted = true;
  history = [];
  viewIdx = -1;
  openViewer(randomIdx());
}
function stepViewer(dir: number) {
  if (!shuffle) { openViewer(viewIdx + dir); return; }
  if (dir > 0) { history.push(viewIdx); openViewer(randomIdx()); }
  else if (history.length) openViewer(history.pop()!);
}
$("shuffle").addEventListener("click", startShuffle);

// ---------- canvas view ----------
// Auto-arranged board of the current scope: notes as sticky notes, images at their real aspect ratio,
// clustered by source page. Pan (drag empty space / wheel), zoom (Ctrl+wheel, buttons), drag items to move them.
// Positions persist per scope in <library>/.views/<scope>.json; new items go to free space below.
const CANVAS_MAX = 500;
interface CvSaved { v: 1; pos: Record<string, [number, number]>; labels: { t: string; x: number; y: number }[]; pan: [number, number]; zoom: number }
const cv = { scope: "", pos: new Map<string, [number, number]>(), size: new Map<string, [number, number]>(), labels: [] as CvSaved["labels"], pan: [40, 40] as [number, number], zoom: 1, list: [] as Item[] };
const cvBar = $("cv-bar"), cvNote = $("cv-note");

function scopeKey(): string {
  const raw = [tab, f.day || f.date, f.tag, f.app, f.color].join("|");
  let h = 5381;
  for (const ch of raw) h = ((h * 33) ^ ch.charCodeAt(0)) >>> 0;
  return `${tab}-${f.day || f.date}-${(f.tag || "all").slice(0, 24)}-${h.toString(16)}`;
}

function setView(v: View) {
  view = v;
  store.set("stash.gallery.view", v);
  wrap.style.display = v === "grid" ? "" : "none";
  canvasEl.classList.toggle("on", v === "canvas");
  if (v === "grid") { lastW = 0; layout("switch"); renderScrub(); } else { openCanvas(); scrub.classList.add("off"); }
}

function cvApply() {
  world.style.transform = `translate(${cv.pan[0]}px, ${cv.pan[1]}px) scale(${cv.zoom})`;
  const z = cvBar.querySelector(".z");
  if (z) z.textContent = `${Math.round(cv.zoom * 100)}%`;
}
let cvSaveTimer: number | undefined;
function cvSave() {
  clearTimeout(cvSaveTimer);
  cvSaveTimer = window.setTimeout(() => {
    const data: CvSaved = { v: 1, pos: Object.fromEntries(cv.pos), labels: cv.labels, pan: cv.pan, zoom: cv.zoom };
    invoke("gallery_view_save", { scope: cv.scope, data }).catch(() => {});
  }, 400);
}

function cvItem(it: Item): HTMLElement {
  const d = el("div", "cv-item" + (selected.has(it.id) ? " sel" : ""));
  d.dataset.id = it.id;
  if (isImg(it)) {
    const img = el("img");
    img.draggable = false;
    img.src = thumb(it);
    d.append(img);
  } else if (isVideo(it)) {
    const v = el("video");
    v.muted = true; v.preload = "metadata";
    v.src = full(it) + "#t=0.1";
    d.append(v);
  } else if (isNote(it) || isTextFile(it)) {
    d.classList.add("cv-note");
    d.append(el("div", "nt", it.text ?? it.caption));
    const src = sourceLabel(it);
    if (src) d.append(el("div", "src", src));
  } else {
    d.classList.add("cv-file");
    d.append(el("span", "badge", it.ext || "file"), el("span", "", it.original_path?.split(/[\\/]/).pop() ?? it.name));
  }
  return d;
}

/** Cluster key: the source page (URL without its fragment), else the window title, else the app. */
function clusterOf(it: Item): [string, string] {
  if (it.url) return [it.url.replace(/#.*$/, ""), sourceLabel(it)];
  if (it.source_title) return [it.source_title, it.source_title];
  if (it.process) return [it.process, it.process.replace(/\.exe$/i, "")];
  return ["", "Other"];
}

/** Shelf-packs items into clusters, and clusters into rows. Returns positions and cluster labels. */
function arrange(list: Item[], origin: [number, number] = [0, 0]) {
  const pos = new Map<string, [number, number]>();
  const labels: CvSaved["labels"] = [];
  const clusters = new Map<string, { label: string; its: Item[] }>();
  for (const it of list) { const [k, label] = clusterOf(it); (clusters.get(k) ?? clusters.set(k, { label, its: [] }).get(k)!).its.push(it); }
  const G = 16, CG = 70, LABEL = 34;
  const boxes = [...clusters.values()].map((c) => {
    const area = c.its.reduce((a, it) => { const [w, h] = cv.size.get(it.id)!; return a + (w + G) * (h + G); }, 0);
    const maxW = Math.min(1100, Math.max(260, Math.sqrt(area) * 1.25));
    let x = 0, y = 0, rowH = 0, bw = 0;
    const local: [Item, number, number][] = [];
    for (const it of c.its) {
      const [w, h] = cv.size.get(it.id)!;
      if (x > 0 && x + w > maxW) { x = 0; y += rowH + G; rowH = 0; }
      local.push([it, x, y]);
      x += w + G;
      rowH = Math.max(rowH, h);
      bw = Math.max(bw, x - G);
    }
    return { label: c.label, local, w: bw, h: y + rowH + LABEL };
  });
  const total = boxes.reduce((a, b) => a + (b.w + CG) * (b.h + CG), 0);
  const worldW = Math.max(1400, Math.sqrt(total) * 1.5);
  let x = 0, y = 0, rowH = 0;
  for (const b of boxes) {
    if (x > 0 && x + b.w > worldW) { x = 0; y += rowH + CG; rowH = 0; }
    labels.push({ t: b.label, x: origin[0] + x, y: origin[1] + y });
    for (const [it, lx, ly] of b.local) pos.set(it.id, [origin[0] + x + lx, origin[1] + y + LABEL + ly]);
    x += b.w + CG;
    rowH = Math.max(rowH, b.h);
  }
  return { pos, labels };
}

function cvBounds(ids: Iterable<string>) {
  let [l, t, r, b] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const id of ids) {
    const p = cv.pos.get(id), s = cv.size.get(id);
    if (!p || !s) continue;
    l = Math.min(l, p[0]); t = Math.min(t, p[1]); r = Math.max(r, p[0] + s[0]); b = Math.max(b, p[1] + s[1]);
  }
  for (const lb of cv.labels) { l = Math.min(l, lb.x); t = Math.min(t, lb.y); }
  return l === Infinity ? { l: 0, t: 0, r: 0, b: 0 } : { l, t, r, b };
}

function cvFit(animate = true) {
  const bb = cvBounds(cv.list.map((i) => i.id));
  const W = canvasEl.clientWidth, H = canvasEl.clientHeight;
  if (!W || bb.r <= bb.l) return;
  const z = Math.min(1.25, Math.max(0.08, Math.min((W - 80) / (bb.r - bb.l), (H - 100) / (bb.b - bb.t))));
  cv.zoom = z;
  cv.pan = [(W - (bb.r - bb.l) * z) / 2 - bb.l * z, (H - (bb.b - bb.t) * z) / 2 - bb.t * z];
  if (animate && !reduced()) world.animate([{ transform: world.style.transform || "none" }, { transform: `translate(${cv.pan[0]}px, ${cv.pan[1]}px) scale(${z})` }], { duration: 260, easing: EASE });
  cvApply();
  cvSave();
}

let cvToken = 0;
async function openCanvas() {
  const token = ++cvToken;
  const scope = scopeKey();
  const fresh = scope !== cv.scope;
  cv.scope = scope;
  let saved: CvSaved | null = null;
  if (fresh) {
    saved = await invoke<CvSaved | null>("gallery_view_load", { scope }).catch(() => null);
    if (token !== cvToken) return;
    cv.pos = new Map(Object.entries(saved?.pos ?? {}));
    cv.labels = saved?.labels ?? [];
    if (saved) { cv.pan = saved.pan; cv.zoom = saved.zoom; }
  }
  cv.list = shown.slice(0, CANVAS_MAX);
  cvNote.textContent = shown.length > CANVAS_MAX ? `Showing the newest ${CANVAS_MAX} of ${shown.length}: narrow the scope (a day or a tag) to see the rest` : "";
  cvNote.classList.toggle("on", shown.length > CANVAS_MAX);

  // build and measure
  world.replaceChildren();
  const els: HTMLElement[] = [];
  for (const it of cv.list) {
    const d = cvItem(it);
    let w = 240;
    if ((isImg(it) || isVideo(it)) && it.w && it.h) {
      const ar = it.w / it.h;
      w = Math.round(Math.min(380, Math.max(140, Math.sqrt(52000 * ar))));
      d.style.height = `${Math.round(w / ar)}px`;
    } else if (isImg(it)) d.style.height = "180px";
    d.style.width = `${w}px`;
    world.append(d);
    els.push(d);
  }
  els.forEach((d, i) => cv.size.set(cv.list[i].id, [d.offsetWidth, d.offsetHeight]));

  // positions: saved ones stay; first visit auto-arranges; later additions go below what is there
  const missing = cv.list.filter((it) => !cv.pos.has(it.id));
  if (missing.length) {
    const placedIds = cv.list.filter((it) => cv.pos.has(it.id)).map((i) => i.id);
    const origin: [number, number] = placedIds.length ? [cvBounds(placedIds).l, cvBounds(placedIds).b + 90] : [0, 0];
    const r = arrange(missing, origin);
    r.pos.forEach((p, id) => cv.pos.set(id, p));
    cv.labels.push(...r.labels);
    cvSave();
  }
  for (const lb of cv.labels) {
    const l = el("div", "cv-label", lb.t);
    l.style.transform = `translate(${lb.x}px, ${lb.y}px)`;
    world.append(l);
  }
  els.forEach((d, i) => { const p = cv.pos.get(cv.list[i].id)!; d.style.transform = `translate(${p[0]}px, ${p[1]}px)`; });
  if (!saved && fresh) cvFit(false); else cvApply();
  if (!reduced() && fresh) world.animate([{ opacity: 0, transform: `${world.style.transform} scale(.98)` }, { opacity: 1, transform: world.style.transform }], { duration: 260, easing: EASE });
}

function rearrange() {
  cv.pos.clear();
  cv.labels = [];
  const r = arrange(cv.list);
  cv.pos = r.pos;
  cv.labels = r.labels;
  const els = Array.from(world.querySelectorAll<HTMLElement>(".cv-item"));
  world.querySelectorAll(".cv-label").forEach((l) => l.remove());
  for (const lb of cv.labels) { const l = el("div", "cv-label", lb.t); l.style.transform = `translate(${lb.x}px, ${lb.y}px)`; world.append(l); }
  for (const d of els) {
    const p = cv.pos.get(d.dataset.id!)!;
    const to = `translate(${p[0]}px, ${p[1]}px)`;
    if (!reduced()) d.animate([{ transform: d.style.transform }, { transform: to }], { duration: 320, easing: EASE });
    d.style.transform = to;
  }
  cvFit();
}

// canvas toolbar
{
  const b = (label: string, title: string, fn: () => void) => { const x = el("button", "btn", label); x.title = title; x.onclick = fn; cvBar.append(x); };
  const zoomBy = (k: number) => {
    const W = canvasEl.clientWidth / 2, H = canvasEl.clientHeight / 2;
    const z = Math.min(3, Math.max(0.08, cv.zoom * k));
    cv.pan = [W - (W - cv.pan[0]) * (z / cv.zoom), H - (H - cv.pan[1]) * (z / cv.zoom)];
    cv.zoom = z;
    cvApply();
    cvSave();
  };
  b("−", "Zoom out", () => zoomBy(1 / 1.25));
  cvBar.append(el("span", "z", "100%"));
  b("+", "Zoom in", () => zoomBy(1.25));
  b("Fit", "Show everything", () => cvFit());
  b("Re-arrange", "Arrange again by source page (forgets moved positions)", rearrange);
  b("Save as image", "Save this canvas as a PNG in your library", exportCanvas);
}

// pan / select / drag items
canvasEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  if ((e.target as HTMLElement).closest("#cv-bar")) return;
  const item = (e.target as HTMLElement).closest<HTMLElement>(".cv-item");
  const [x0, y0] = [e.clientX, e.clientY];
  let moved = false;
  canvasEl.setPointerCapture(e.pointerId);
  if (item && e.button === 0) {
    const id = item.dataset.id!;
    if (!selected.has(id) || e.ctrlKey || e.shiftKey) clickSelect(id, e);
    const group = [...selected].filter((s) => cv.pos.has(s));
    const start = new Map(group.map((g) => [g, [...cv.pos.get(g)!] as [number, number]]));
    const nodes = new Map(group.map((g) => [g, world.querySelector<HTMLElement>(`.cv-item[data-id="${CSS.escape(g)}"]`)]));
    const move = (m: PointerEvent) => {
      const dx = (m.clientX - x0) / cv.zoom, dy = (m.clientY - y0) / cv.zoom;
      if (!moved && Math.hypot(m.clientX - x0, m.clientY - y0) < 4) return;
      moved = true;
      for (const g of group) {
        const s = start.get(g)!, n = nodes.get(g);
        const p: [number, number] = [Math.round(s[0] + dx), Math.round(s[1] + dy)];
        cv.pos.set(g, p);
        if (n) { n.classList.add("drag"); n.style.transform = `translate(${p[0]}px, ${p[1]}px)`; }
      }
    };
    const up = () => {
      canvasEl.removeEventListener("pointermove", move);
      nodes.forEach((n) => n?.classList.remove("drag"));
      if (moved) cvSave();
    };
    canvasEl.addEventListener("pointermove", move);
    canvasEl.addEventListener("pointerup", up, { once: true });
    return;
  }
  const p0 = [...cv.pan];
  canvasEl.classList.add("pan");
  const move = (m: PointerEvent) => {
    if (!moved && Math.hypot(m.clientX - x0, m.clientY - y0) < 3) return;
    moved = true;
    cv.pan = [p0[0] + m.clientX - x0, p0[1] + m.clientY - y0];
    cvApply();
  };
  const up = () => {
    canvasEl.removeEventListener("pointermove", move);
    canvasEl.classList.remove("pan");
    if (moved) cvSave(); else if (selected.size) { selected.clear(); refreshSel(); }
  };
  canvasEl.addEventListener("pointermove", move);
  canvasEl.addEventListener("pointerup", up, { once: true });
});
canvasEl.addEventListener("dblclick", (e) => {
  const item = (e.target as HTMLElement).closest<HTMLElement>(".cv-item");
  if (item) openViewer(shown.findIndex((i) => i.id === item.dataset.id));
});
canvasEl.addEventListener("wheel", (e) => {
  e.preventDefault();
  if (e.ctrlKey) {
    const r = canvasEl.getBoundingClientRect();
    const [mx, my] = [e.clientX - r.left, e.clientY - r.top];
    const z = Math.min(3, Math.max(0.08, cv.zoom * Math.exp(-e.deltaY * 0.0015)));
    cv.pan = [mx - (mx - cv.pan[0]) * (z / cv.zoom), my - (my - cv.pan[1]) * (z / cv.zoom)];
    cv.zoom = z;
  } else {
    cv.pan = [cv.pan[0] - (e.shiftKey ? e.deltaY : e.deltaX), cv.pan[1] - (e.shiftKey ? 0 : e.deltaY)];
  }
  cvApply();
  cvSave();
}, { passive: false });

/** Renders the canvas to a PNG (thumbnails, notes, labels) and saves it into the library. */
async function exportCanvas() {
  const ids = cv.list.map((i) => i.id);
  const bb = cvBounds(ids);
  const m = 40;
  const W = bb.r - bb.l + 2 * m, H = bb.b - bb.t + 2 * m;
  const s = Math.min(2, 8000 / Math.max(W, H));
  const c = document.createElement("canvas");
  c.width = Math.round(W * s); c.height = Math.round(H * s);
  const g = c.getContext("2d")!;
  g.scale(s, s);
  g.translate(m - bb.l, m - bb.t);
  g.fillStyle = "#15161c";
  g.fillRect(bb.l - m, bb.t - m, W, H);
  const font = getComputedStyle(document.body).fontFamily;
  const rr = (x: number, y: number, w: number, h: number, r: number) => { g.beginPath(); g.roundRect(x, y, w, h, r); };
  g.fillStyle = "rgba(255,255,255,.7)";
  g.font = `600 15px ${font}`;
  for (const lb of cv.labels) g.fillText(lb.t.slice(0, 90), lb.x, lb.y + 18);
  const loadImg = (src: string) => new Promise<HTMLImageElement | null>((res) => { const im = new Image(); im.crossOrigin = "anonymous"; im.onload = () => res(im); im.onerror = () => res(null); im.src = src; });
  for (const it of cv.list) {
    const p = cv.pos.get(it.id), sz = cv.size.get(it.id);
    if (!p || !sz) continue;
    const [x, y] = p, [w, h] = sz;
    g.save();
    rr(x, y, w, h, 12);
    g.clip();
    if (isImg(it)) {
      const im = await loadImg(thumb(it));
      if (im) g.drawImage(im, x, y, w, h); else { g.fillStyle = "#2a2b33"; g.fill(); }
    } else {
      g.fillStyle = isNote(it) ? "#2c2d38" : "#24252e";
      g.fillRect(x, y, w, h);
      g.fillStyle = "#f2f2f5";
      g.font = `13px ${font}`;
      // wrap the text into the note
      const text = (it.text ?? it.caption ?? it.name).replace(/\r/g, "");
      let ty = y + 26;
      for (const para of text.split("\n")) {
        let line = "";
        for (const word of para.split(/\s+/)) {
          const t = line ? line + " " + word : word;
          if (g.measureText(t).width > w - 28 && line) { g.fillText(line, x + 14, ty); ty += 19; line = word; } else line = t;
          if (ty > y + h - 30) break;
        }
        if (ty > y + h - 30) break;
        g.fillText(line, x + 14, ty);
        ty += 19;
      }
      const src = sourceLabel(it);
      if (src) { g.fillStyle = "rgba(255,255,255,.5)"; g.font = `11px ${font}`; g.fillText(src.slice(0, 40), x + 14, y + h - 12); }
    }
    g.restore();
    g.strokeStyle = "rgba(255,255,255,.12)";
    rr(x + 0.5, y + 0.5, w - 1, h - 1, 12);
    g.stroke();
  }
  try {
    const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
    if (!blob) throw new Error("could not render the canvas");
    const name = `canvas-${new Date().toISOString().slice(0, 10)}.png`;
    await invoke("gallery_import_data", new Uint8Array(await blob.arrayBuffer()), { headers: { "x-name": encodeURIComponent(name) } });
    flash("Saved the canvas as an image in your library");
  } catch (e) {
    flash(String(e));
  }
}

// ---------- background palette + OCR indexing ----------
let backfilling = false;
async function backfill() {
  if (backfilling) return;
  backfilling = true;
  try {
    for (;;) {
      const r = await invoke<{ done: { id: string; palette: string[]; ocr: string | null }[]; remaining: number }>("gallery_backfill", { limit: 6 });
      for (const d of r.done) {
        const it = byId(d.id);
        if (it) { if (d.palette.length) it.palette = d.palette; if (d.ocr) it.ocr = d.ocr; }
      }
      if (r.done.length) renderRail();
      if (!r.remaining || !r.done.length) break;
    }
  } catch { /* window closed or indexing unavailable */ }
  backfilling = false;
}

// ---------- actions ----------
function flash(msg: string, action?: { label: string; fn: () => void }) {
  toast.replaceChildren(el("span", "", msg));
  if (action) { const a = el("a", "", action.label); a.onclick = () => { action.fn(); toast.classList.remove("on"); }; toast.append(a); }
  toast.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("on"), action ? 7000 : 2200);
}

const copyItem = (it: Item) => invoke("gallery_copy", { id: it.id }).then(() => flash("Copied")).catch((e) => flash(String(e)));

/** Trash: cards shrink out and neighbours glide in; Undo brings them back where they were. */
async function trash(ids: string[]) {
  const moved = await invoke<string[]>("gallery_delete", { ids });
  selected.clear();
  items = items.filter((i) => !moved.includes(i.id));
  renderRail();
  filter("flip");
  updateSelBar();
  showDetail();
  flash(`Moved ${moved.length} to trash`, { label: "Undo", fn: async () => { await invoke("gallery_restore", { ids: moved }); await load(true); } });
}

// ---------- events ----------
q.addEventListener("input", () => filter("none")); // typing stays instant

window.addEventListener("keydown", (e) => {
  const inField = (e.target as HTMLElement).tagName === "INPUT";
  if (viewer.classList.contains("on")) {
    if (e.key === "Escape") closeViewer();
    else if (e.key === "ArrowRight" || e.key === " " || e.key === "Enter") { e.preventDefault(); stepViewer(1); }
    else if (e.key === "ArrowLeft") stepViewer(-1);
    return;
  }
  if (e.ctrlKey && e.key.toLowerCase() === "f") { e.preventDefault(); q.focus(); q.select(); }
  else if (e.key === "/" && !inField) { e.preventDefault(); q.focus(); }
  else if (!inField && e.key.toLowerCase() === "s" && !e.ctrlKey) startShuffle();
  else if (!inField && e.key.toLowerCase() === "t" && !e.ctrlKey && selected.size) { e.preventDefault(); openTagPop(); }
  else if (e.key === "Escape") { if (inField) (e.target as HTMLElement).blur(); else if (tagpop.classList.contains("on")) tagpop.classList.remove("on"); else { selected.clear(); refreshSel(); } }
  else if (!inField && e.key === "Delete" && selected.size) trash([...selected]);
  else if (!inField && (e.key === " " || e.key === "Enter") && selected.size === 1) { e.preventDefault(); openViewer(shown.findIndex((i) => i.id === [...selected][0])); }
  else if (!inField && e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); selected = new Set(shown.map((i) => i.id)); refreshSel(); }
  else if (!inField && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) { e.preventDefault(); moveSel(e.key); }
});

// ---------- add content: drop and paste ----------
// Drops are handled as HTML5 events (the native handler only understands files): files from Explorer or a
// browser, images, text and links. Our own drag-out coming back in is ignored.
const hasPayload = (dt: DataTransfer | null) => !!dt && ["Files", "text/plain", "text/uri-list", "text/html"].some((t) => dt.types.includes(t));
let dragDepth = 0;
window.addEventListener("dragenter", (e) => { if (dragging || !hasPayload(e.dataTransfer)) return; e.preventDefault(); dragDepth++; dropEl.classList.add("on"); });
window.addEventListener("dragover", (e) => { if (dragging || !hasPayload(e.dataTransfer)) return; e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; });
window.addEventListener("dragleave", () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) dropEl.classList.remove("on"); });
window.addEventListener("drop", async (e) => {
  e.preventDefault();
  dragDepth = 0;
  dropEl.classList.remove("on");
  const dt = e.dataTransfer;
  if (dragging || !dt) return;
  let n = 0;
  try {
    if (dt.files.length) {
      for (const fl of Array.from(dt.files)) {
        await invoke("gallery_import_data", new Uint8Array(await fl.arrayBuffer()), { headers: { "x-name": encodeURIComponent(fl.name || "dropped") } });
        n++;
      }
    } else {
      const text = (dt.getData("text/uri-list") || dt.getData("text/plain")).split(/\r?\n/).find((l) => l.trim() && !l.startsWith("#"))?.trim() ?? dt.getData("text/plain").trim();
      if (text) { await invoke("gallery_import_text", { text }); n++; }
    }
  } catch (err) {
    flash(String(err));
    return;
  }
  if (n) flash(`Added ${n} item${n === 1 ? "" : "s"}`); // the library-changed event brings them in
});

// Ctrl+V pastes the clipboard (files, an image, text or a link) into the library.
window.addEventListener("keydown", async (e) => {
  if (!(e.ctrlKey && e.key.toLowerCase() === "v") || (e.target as HTMLElement).tagName === "INPUT" || viewer.classList.contains("on")) return;
  e.preventDefault();
  try {
    flash(await invoke<string>("gallery_paste"));
  } catch (err) {
    flash(String(err));
  }
});

// New captures, tags from the tag radial, imports: Rust emits library-changed and they slide into place.
listen("library-changed", () => load(true).catch(() => {}));

// Opening: the window is created hidden and the page stays invisible (body.pre) while the first layout runs;
// the window is then shown and the entrance (body.opening + the card cascade) plays on the first visible frame.
document.body.classList.add("pre");
wrap.style.display = view === "grid" ? "" : "none";
canvasEl.classList.toggle("on", view === "canvas");
load()
  .catch((e) => console.error(e))
  .finally(() => {
    invoke("window_ready").catch(() => {});
    q.focus();
    requestAnimationFrame(() => {
      document.body.classList.remove("pre");
      document.body.classList.add("opening");
      opening = false;
      if (view === "grid") layout("enter");
      setTimeout(() => {
        document.body.classList.remove("opening");
        document.querySelectorAll<HTMLElement>("#rail > *").forEach((e) => e.style.removeProperty("--i"));
      }, 1500);
    });
  });
