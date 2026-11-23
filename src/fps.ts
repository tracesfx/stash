// Animation frame-rate cap for web windows (Settings > Appearance > Motion > Frame rate; 0 = Auto).
// When the cap is below the display's refresh rate, running Web Animations (CSS animations, transitions and
// element.animate) are paused and stepped by hand every 1/cap seconds, so they draw at the chosen rate and still
// take their real duration. Scrolling and the cursor glow are JS-driven and stay at the display rate.
// Nothing runs while no animation is playing: the loop starts on animation/transition events and on
// element.animate()/play(), and stops once every stepped animation has ended.

let cap = 0;
/** Animations we paused and step: last step time (performance.now()). */
const held = new Map<Animation, number>();
/** Animations the page itself paused: never stepped or resumed by us. */
const userPaused = new WeakSet<Animation>();
const origPause = Animation.prototype.pause;
const origPlay = Animation.prototype.play;
Animation.prototype.pause = function (this: Animation) {
  held.delete(this);
  userPaused.add(this);
  origPause.call(this);
};
Animation.prototype.play = function (this: Animation) {
  userPaused.delete(this);
  held.delete(this);
  origPlay.call(this);
  kick();
};
const origAnimate = Element.prototype.animate;
Element.prototype.animate = function (this: Element, ...args: Parameters<Element["animate"]>) {
  const a = origAnimate.apply(this, args);
  kick();
  return a;
};
addEventListener("animationstart", kick, true);
addEventListener("transitionrun", kick, true);

// display refresh period, measured from rAF deltas (median of the last 24); remembered across pages
let period = 1000 / 60;
try { const p = Number(localStorage.getItem("stash.refresh")); if (p > 2 && p < 70) period = p; } catch { /* none */ }
const deltas: number[] = [];
let raf = 0, lastFrame = 0, lastStep = 0;

/** Stepping only pays off when the cap is below the display rate (stepped animations run on the main thread). */
const active = () => cap > 0 && cap < (1000 / period) * 0.95;

export function setFpsCap(fps: number, animations: boolean) {
  cap = animations && fps >= 15 ? Math.min(240, fps) : 0;
  if (!active()) release();
  kick();
}

function release() {
  for (const a of held.keys()) if (a.playState === "paused") origPlay.call(a);
  held.clear();
}

function kick() {
  if (cap > 0 && !raf) { lastFrame = 0; raf = requestAnimationFrame(frame); }
}

function frame(t: number) {
  raf = 0;
  if (lastFrame) {
    const d = t - lastFrame;
    if (d > 2 && d < 70) {
      deltas.push(d);
      if (deltas.length > 24) deltas.shift();
      if (deltas.length >= 8) {
        const p = [...deltas].sort((x, y) => x - y)[deltas.length >> 1];
        if (Math.abs(p - period) > 0.3) { period = p; try { localStorage.setItem("stash.refresh", p.toFixed(2)); } catch { /* none */ } }
      }
    }
  }
  lastFrame = t;
  if (!active()) {
    release();
    if (cap > 0 && deltas.length < 8) raf = requestAnimationFrame(frame); // still measuring
    return;
  }
  // take over animations that started since the last frame (ones still pending start are taken next frame)
  let waiting = false;
  for (const a of document.getAnimations()) {
    if (held.has(a) || userPaused.has(a) || a.playState !== "running") continue;
    if (a.timeline !== document.timeline) continue; // scroll-driven
    if (a.pending || a.currentTime == null) { waiting = true; continue; }
    origPause.call(a);
    held.set(a, t);
  }
  // step on the first display frame at least one cap interval after the last step (half a frame of slack)
  if (t - lastStep >= 1000 / cap - period / 2) {
    lastStep = t;
    for (const [a, last] of held) {
      if (a.playState !== "paused" || a.currentTime == null) { held.delete(a); continue; } // cancelled / finished
      const rate = a.playbackRate || 1;
      const ct = Number(a.currentTime) + (t - last) * rate;
      const end = a.effect?.getComputedTiming().endTime;
      const endMs = typeof end === "number" ? end : Infinity;
      if ((rate > 0 && ct >= endMs) || (rate < 0 && ct <= 0)) {
        held.delete(a);
        a.finish();
      } else {
        a.currentTime = ct;
        held.set(a, t);
      }
    }
  }
  if (held.size || waiting) raf = requestAnimationFrame(frame);
}
