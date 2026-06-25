import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";

// ---------------------------------------------------------------------------
// Shared types (mirror the Rust structs)
// ---------------------------------------------------------------------------

type Escalation = "gentle" | "standard" | "forced";
type WidgetMode = "off" | "minimized" | "always";
type WidgetShape = "round" | "squircle" | "square";

interface Settings {
  workIntervalSecs: number;
  breakLengthSecs: number;
  preBreakWarningSecs: number;
  escalation: Escalation;
  snoozeSecs: number;
  maxPostpones: number;
  soundEnabled: boolean;
  widgetMode: WidgetMode;
  widgetShape: WidgetShape;
  widgetWidth: number;
  widgetHeight: number;
  widgetOpacity: number;
  widgetX: number | null;
  widgetY: number | null;
  autostart: boolean;
  globalShortcutsEnabled: boolean;
  scPause: string;
  scSkip: string;
  scTake: string;
  scPostpone: string;
  scToggleWidget: string;
  workHoursEnabled: boolean;
  workStart: string;
  workEnd: string;
  workDays: boolean[];
  idlePauseEnabled: boolean;
  idleThresholdSecs: number;
  longBreakEnabled: boolean;
  longBreakEvery: number;
  longBreakSecs: number;
  blinkEnabled: boolean;
  blinkIntervalSecs: number;
  reduceMotion: boolean;
  highContrast: boolean;
  suppressOnFullscreen: boolean;
  respectDnd: boolean;
  hydrationEnabled: boolean;
  hydrationIntervalSecs: number;
  postureEnabled: boolean;
  postureIntervalSecs: number;
  eveningNudgeEnabled: boolean;
  eveningHour: number;
  tipsEnabled: boolean;
  exercisesEnabled: boolean;
  calmVisualsEnabled: boolean;
  accent: string;
  statsEnabled: boolean;
}

interface DayBar {
  date: string;
  taken: number;
  skipped: number;
}

interface StatsSummary {
  todayTaken: number;
  todaySkipped: number;
  totalTaken: number;
  streak: number;
  last7: DayBar[];
}

interface TimerSnapshot {
  phase: "working" | "break";
  remaining: number;
  total: number;
  paused: boolean;
  postponesUsed: number;
  maxPostpones: number;
  isLong: boolean;
  idle: boolean;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fmt(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const mm = Math.floor(s / 60);
  const ss = s % 60;
  return `${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
}

const RADIUS = 120;
const CIRC = 2 * Math.PI * RADIUS;

function setRing(
  el: SVGCircleElement,
  remaining: number,
  total: number,
  circ: number = CIRC,
) {
  const frac = total > 0 ? remaining / total : 0;
  el.style.strokeDasharray = `${circ}`;
  el.style.strokeDashoffset = `${circ * (1 - frac)}`;
}

function beep() {
  try {
    const ctx = new AudioContext();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = 660;
    gain.gain.setValueAtTime(0.0001, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.25, ctx.currentTime + 0.05);
    gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.6);
    osc.connect(gain).connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.65);
  } catch {
    /* audio not available — ignore */
  }
}

const EYE_TIPS = [
  "20-20-20: every 20 min, look ~20 ft away for 20 sec.",
  "Blink fully and often — screens cut your blink rate in half.",
  "Keep your screen about an arm's length away.",
  "Put the top of your screen at or just below eye level.",
  "Cut glare: avoid bright lights or windows behind your screen.",
  "Match screen brightness to the room — not too bright in the dark.",
  "Sip water through the day — hydration eases dry eyes.",
  "Book a yearly eye check-up.",
];

const EYE_EXERCISES = [
  "Follow the dot with your eyes — keep your head still.",
  "Near → far: focus on your thumb, then something distant. Repeat.",
  "Slow eye rolls: circle clockwise, then counter-clockwise.",
  "Palming: rub palms warm, cup over closed eyes, breathe.",
];

const app = document.querySelector<HTMLDivElement>("#app")!;

// Accessibility: reflected on <html> so it applies to every window/view.
function applyAppearance(s: Settings) {
  const root = document.documentElement;
  root.classList.toggle("reduce-motion", s.reduceMotion);
  root.classList.toggle("high-contrast", s.highContrast);
  if (s.accent && !s.highContrast) {
    root.style.setProperty("--accent", s.accent);
  } else {
    root.style.removeProperty("--accent");
  }
}

// ---------------------------------------------------------------------------
// Break window view (loaded at index.html#break)
// ---------------------------------------------------------------------------

async function renderBreak() {
  if (location.hash.includes("sound=1")) beep();
  const s = await invoke<Settings>("get_settings").catch(() => null);

  const calmOn = s?.calmVisualsEnabled !== false;
  app.innerHTML = `
    <div class="break-screen" data-tauri-drag-region>
      ${calmOn ? `<div class="break-bg"><span></span><span></span></div>` : ""}
      <p class="break-eyebrow">EyeBreak</p>
      <h1 class="break-title" id="break-title">Look ~20 feet away</h1>
      <p class="break-sub" id="break-sub">Relax your eyes — let your focus drift to the distance.</p>
      <div class="break-exercise" id="break-exercise" hidden>
        <div class="ex-stage"><span class="ex-dot"></span></div>
        <p class="ex-label" id="ex-label"></p>
      </div>
      <div class="break-count" id="break-count">00:20</div>
      <div class="break-actions">
        <button class="btn ghost" id="break-postpone">Postpone</button>
        <button class="btn" id="break-skip">Skip break</button>
      </div>
      <p class="break-note" id="break-note"></p>
      <p class="break-tip" id="break-tip"></p>
    </div>
  `;

  if (s?.tipsEnabled !== false) {
    const tip = EYE_TIPS[Math.floor(Math.random() * EYE_TIPS.length)];
    document.querySelector<HTMLParagraphElement>("#break-tip")!.textContent =
      `💡 ${tip}`;
  }

  // Guided eye-exercise (long breaks only)
  const exEl = document.querySelector<HTMLDivElement>("#break-exercise")!;
  const exLabel = document.querySelector<HTMLParagraphElement>("#ex-label")!;
  let exTimer = 0;
  let exIdx = 0;
  const showExercise = (on: boolean) => {
    if (on && exTimer === 0) {
      exEl.hidden = false;
      const step = () => {
        exLabel.textContent = EYE_EXERCISES[exIdx % EYE_EXERCISES.length];
        exIdx++;
      };
      step();
      exTimer = window.setInterval(step, 7000);
    } else if (!on && exTimer !== 0) {
      exEl.hidden = true;
      clearInterval(exTimer);
      exTimer = 0;
    }
  };

  const titleEl = document.querySelector<HTMLHeadingElement>("#break-title")!;
  const subEl = document.querySelector<HTMLParagraphElement>("#break-sub")!;
  const countEl = document.querySelector<HTMLDivElement>("#break-count")!;
  const noteEl = document.querySelector<HTMLParagraphElement>("#break-note")!;
  const skipBtn = document.querySelector<HTMLButtonElement>("#break-skip")!;
  const postBtn = document.querySelector<HTMLButtonElement>("#break-postpone")!;

  skipBtn.addEventListener("click", () => invoke("timer_skip"));
  postBtn.addEventListener("click", async () => {
    const ok = await invoke<boolean>("timer_postpone");
    if (!ok) {
      postBtn.disabled = true;
      noteEl.textContent = "No postpones left — finish the break 🙂";
    }
  });

  const update = (t: TimerSnapshot) => {
    if (t.phase !== "break") return;
    countEl.textContent = fmt(t.remaining);
    if (t.isLong) {
      titleEl.textContent = "Stand up & move";
      subEl.textContent =
        "Walk around, stretch, and look out a window at the distance.";
    } else {
      titleEl.textContent = "Look ~20 feet away";
      subEl.textContent =
        "Relax your eyes — let your focus drift to the distance.";
    }
    showExercise(!!s?.exercisesEnabled && t.isLong);
    if (t.maxPostpones > 0) {
      postBtn.disabled = t.postponesUsed >= t.maxPostpones;
    }
  };

  update(await invoke<TimerSnapshot>("get_timer"));
  listen<TimerSnapshot>("timer:tick", (e) => update(e.payload));

  // The backend closes this window on break-end / skip / postpone.
}

// ---------------------------------------------------------------------------
// Floating widget view (loaded at index.html#widget)
// ---------------------------------------------------------------------------

const WIDGET_CIRC = 2 * Math.PI * 44;

// 12 reference dots around the ring (clock-style) so the depleting arc reads
// clearly as a countdown.
const WIDGET_TICKS = Array.from({ length: 12 }, (_, i) => {
  const a = (i * 30 * Math.PI) / 180;
  const x = (50 + 37 * Math.sin(a)).toFixed(2);
  const y = (50 - 37 * Math.cos(a)).toFixed(2);
  return `<circle cx="${x}" cy="${y}" r="1.15"></circle>`;
}).join("");

// Monochrome inline icons (inherit currentColor) so the action bar looks
// consistent — no clashing colour emoji.
const ICON_PAUSE = `<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1.2"/><rect x="14" y="5" width="4" height="14" rx="1.2"/></svg>`;
const ICON_PLAY = `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.5l13 7.5-13 7.5z"/></svg>`;
const ICON_EYE = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>`;
const ICON_SKIP = `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M5 4.5l10 7.5-10 7.5z"/><rect x="16.5" y="5" width="3" height="14" rx="1.2"/></svg>`;
const ICON_EXPAND = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6"/><path d="M10 20H4v-6"/><path d="M20 4l-8 8"/><path d="M4 20l8-8"/></svg>`;
const ICON_GEAR = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`;
const ICON_BACK = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 18l-6-6 6-6"/></svg>`;
const ICON_CHEVRON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>`;

function applyWidgetStyle(s: Settings) {
  const card = document.querySelector<HTMLDivElement>(".widget");
  if (!card) return;
  const radius =
    s.widgetShape === "round"
      ? "50%"
      : s.widgetShape === "square"
        ? "8px"
        : "30px";
  card.style.borderRadius = radius;
  card.style.setProperty("--w-fill", String(s.widgetOpacity / 100));
}

async function renderWidget() {
  const s = await invoke<Settings>("get_settings");

  app.innerHTML = `
    <div class="widget">
      <div class="w-dial">
        <svg class="w-ring" viewBox="0 0 100 100">
          <circle class="w-ring-bg" cx="50" cy="50" r="44"></circle>
          <g class="w-ticks">${WIDGET_TICKS}</g>
          <circle class="w-ring-fg" cx="50" cy="50" r="44" transform="rotate(-90 50 50)"></circle>
        </svg>
        <div class="w-center">
          <div class="w-time" id="w-time">--:--</div>
        </div>
      </div>
      <div class="w-actions">
        <button id="w-pause" title="Pause / Resume">${ICON_PAUSE}</button>
        <button id="w-take" title="Take a break now">${ICON_EYE}</button>
        <button id="w-skip" title="Skip">${ICON_SKIP}</button>
      </div>
      <button class="w-restore" id="w-restore" title="Open EyeBreak">${ICON_EXPAND}</button>
      <div class="w-resize" id="w-resize" title="Drag to resize"></div>
    </div>
  `;

  applyWidgetStyle(s);

  const card = document.querySelector<HTMLDivElement>(".widget")!;
  const dial = document.querySelector<HTMLDivElement>(".w-dial")!;
  const ringFg = document.querySelector<SVGCircleElement>(".w-ring-fg")!;
  const timeEl = document.querySelector<HTMLDivElement>("#w-time")!;
  const pauseBtn = document.querySelector<HTMLButtonElement>("#w-pause")!;

  // Size the time to the actual circle (≈ the dial's smaller side) so it always
  // fits inside the ring at any widget size or aspect ratio.
  const fitTime = () => {
    const r = dial.getBoundingClientRect();
    const d = Math.min(r.width, r.height);
    if (d > 0) timeEl.style.fontSize = `${Math.max(8, d * 0.27)}px`;
  };
  new ResizeObserver(fitTime).observe(dial);
  fitTime();

  // Reveal the controls on mouse activity, auto-hide ~2s after it stops.
  // Frameless webviews don't reliably fire pointerleave, so a CSS :hover would
  // get stuck "on"; drive it from JS with a timeout instead.
  let hideTimer = 0;
  const poke = () => {
    card.classList.add("hovered");
    clearTimeout(hideTimer);
    hideTimer = window.setTimeout(
      () => card.classList.remove("hovered"),
      2200,
    );
  };
  card.addEventListener("pointermove", poke);
  card.addEventListener("pointerdown", poke);
  card.addEventListener("pointerleave", () => {
    clearTimeout(hideTimer);
    card.classList.remove("hovered");
  });

  // Drag the frameless window: data-tauri-drag-region doesn't fire reliably
  // when the click lands on the SVG circles, so start the drag from JS.
  card.addEventListener("pointerdown", async (e) => {
    if (e.button !== 0) return;
    // let the buttons and the resize grip handle their own clicks
    if ((e.target as HTMLElement).closest("button, .w-resize")) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* ignore */
    }
  });

  // Resize grip (bottom-right). KWin refuses the WM's native resize for this
  // borderless always-on-top window, so drive setSize from JS (needs the
  // allow-set-size capability). Throttled to one resize per frame so it's smooth.
  const grip = document.querySelector<HTMLDivElement>("#w-resize")!;
  grip.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    const win = getCurrentWindow();
    const startX = e.screenX;
    const startY = e.screenY;
    const startW = window.innerWidth;
    const startH = window.innerHeight;
    grip.setPointerCapture(e.pointerId);

    const clamp = (v: number) => Math.max(120, Math.min(480, Math.round(v)));
    let raf = 0;
    let tw = startW;
    let th = startH;
    const apply = () => {
      raf = 0;
      win.setSize(new LogicalSize(tw, th));
    };
    const onMove = (ev: PointerEvent) => {
      tw = clamp(startW + (ev.screenX - startX));
      th = clamp(startH + (ev.screenY - startY));
      if (!raf) raf = requestAnimationFrame(apply);
    };
    const onUp = () => {
      grip.releasePointerCapture(e.pointerId);
      grip.removeEventListener("pointermove", onMove);
      grip.removeEventListener("pointerup", onUp);
      if (raf) cancelAnimationFrame(raf);
      win.setSize(new LogicalSize(tw, th));
    };
    grip.addEventListener("pointermove", onMove);
    grip.addEventListener("pointerup", onUp);
  });

  let paused = false;
  pauseBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("timer_set_paused", { paused: !paused });
  });
  document.querySelector<HTMLButtonElement>("#w-take")!.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("timer_take_break");
  });
  document.querySelector<HTMLButtonElement>("#w-skip")!.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("timer_skip");
  });
  document.querySelector<HTMLButtonElement>("#w-restore")!.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("show_main");
  });

  const update = (t: TimerSnapshot) => {
    paused = t.paused;
    pauseBtn.innerHTML = t.paused ? ICON_PLAY : ICON_PAUSE;
    timeEl.textContent = fmt(t.remaining);
    setRing(ringFg, t.remaining, t.total, WIDGET_CIRC);
    card.dataset.phase = t.paused ? "paused" : t.phase;
  };

  update(await invoke<TimerSnapshot>("get_timer"));
  await listen<TimerSnapshot>("timer:tick", (e) => update(e.payload));
  await listen<Settings>("settings:changed", (e) => applyWidgetStyle(e.payload));
}

// ---------------------------------------------------------------------------
// Main window — two views: timer dashboard <-> settings page
// ---------------------------------------------------------------------------

let mainSettings: Settings;
let unlistenMainTick: UnlistenFn | null = null;

function stopMainTick() {
  if (unlistenMainTick) {
    unlistenMainTick();
    unlistenMainTick = null;
  }
}

async function showDashboard() {
  stopMainTick();

  app.innerHTML = `
    <main class="dash">
      <header class="dash-head">
        <div class="head-left">
          <h1>EyeBreak</h1>
          <span class="tag" id="phase-tag">working</span>
        </div>
        <button class="icon-btn" id="btn-settings" title="Settings">${ICON_GEAR}</button>
      </header>

      <section class="ring-wrap">
        <svg class="ring" viewBox="0 0 280 280" width="220" height="220">
          <circle class="ring-bg" cx="140" cy="140" r="${RADIUS}"></circle>
          <circle class="ring-fg" cx="140" cy="140" r="${RADIUS}"
            transform="rotate(-90 140 140)"></circle>
        </svg>
        <div class="ring-center">
          <div class="ring-time" id="ring-time">--:--</div>
          <div class="ring-label" id="ring-label">until next break</div>
        </div>
      </section>

      <section class="controls">
        <button class="btn" id="btn-pause">Pause</button>
        <button class="btn ghost" id="btn-take">Take break now</button>
        <button class="btn ghost" id="btn-skip">Skip</button>
      </section>

      <footer class="dash-foot">
        Closing this window keeps EyeBreak running in the tray and (if enabled) shows the floating widget.
      </footer>
    </main>
  `;

  const ringFg = document.querySelector<SVGCircleElement>(".ring-fg")!;
  const ringTime = document.querySelector<HTMLDivElement>("#ring-time")!;
  const ringLabel = document.querySelector<HTMLDivElement>("#ring-label")!;
  const phaseTag = document.querySelector<HTMLSpanElement>("#phase-tag")!;
  const btnPause = document.querySelector<HTMLButtonElement>("#btn-pause")!;

  document
    .querySelector<HTMLButtonElement>("#btn-settings")!
    .addEventListener("click", () => showSettings());
  document
    .querySelector<HTMLButtonElement>("#btn-take")!
    .addEventListener("click", () => invoke("timer_take_break"));
  document
    .querySelector<HTMLButtonElement>("#btn-skip")!
    .addEventListener("click", () => invoke("timer_skip"));

  let paused = false;
  btnPause.addEventListener("click", () =>
    invoke("timer_set_paused", { paused: !paused }),
  );

  const applyTick = (t: TimerSnapshot) => {
    paused = t.paused;
    btnPause.textContent = t.paused ? "Resume" : "Pause";
    phaseTag.textContent = t.idle
      ? "idle"
      : t.paused
        ? "paused"
        : t.phase === "break"
          ? "break"
          : "working";
    phaseTag.dataset.phase = t.idle || t.paused ? "paused" : t.phase;
    ringTime.textContent = fmt(t.remaining);
    ringLabel.textContent = t.idle
      ? "paused — you're away"
      : t.phase === "break"
        ? "break time left"
        : "until next break";
    setRing(ringFg, t.remaining, t.total);
  };

  applyTick(await invoke<TimerSnapshot>("get_timer"));
  unlistenMainTick = await listen<TimerSnapshot>("timer:tick", (e) =>
    applyTick(e.payload),
  );
}

// A styled, animated dropdown replacing the native <select>, whose option
// popup is drawn by GTK on Linux and ignores our CSS (unreadable colours).
interface Opt {
  value: string;
  label: string;
}

function customSelect(mount: HTMLElement, options: Opt[], initial: string) {
  let current = initial;
  mount.classList.add("cselect");
  mount.innerHTML = `
    <button type="button" class="cselect-btn">
      <span class="cselect-value"></span>
      <span class="cselect-chev">${ICON_CHEVRON}</span>
    </button>
    <ul class="cselect-list" role="listbox"></ul>
  `;
  const valueEl = mount.querySelector<HTMLSpanElement>(".cselect-value")!;
  const list = mount.querySelector<HTMLUListElement>(".cselect-list")!;

  const render = () => {
    valueEl.textContent = options.find((o) => o.value === current)?.label ?? "";
    list.innerHTML = options
      .map(
        (o) =>
          `<li class="cselect-opt${o.value === current ? " sel" : ""}" data-v="${o.value}" role="option">${o.label}</li>`,
      )
      .join("");
  };
  render();

  mount
    .querySelector<HTMLButtonElement>(".cselect-btn")!
    .addEventListener("click", (e) => {
      e.stopPropagation();
      document
        .querySelectorAll(".cselect.open")
        .forEach((el) => el !== mount && el.classList.remove("open"));
      mount.classList.toggle("open");
    });
  list.addEventListener("click", (e) => {
    const li = (e.target as HTMLElement).closest<HTMLElement>(".cselect-opt");
    if (!li) return;
    current = li.dataset.v!;
    render();
    mount.classList.remove("open");
  });

  return { value: () => current };
}

async function showSettings() {
  stopMainTick();

  app.innerHTML = `
    <main class="dash settings-page">
      <header class="dash-head settings-head">
        <button class="icon-btn" id="btn-back" title="Back">${ICON_BACK}</button>
        <h1>Settings</h1>
        <span class="icon-spacer"></span>
      </header>

      <div class="settings-scroll">
        <section class="card s-card">
          <h2><span class="s-dot"></span> Timing</h2>
          <div class="grid">
            <label>Work interval <span class="unit">(min)</span>
              <input type="number" id="f-work" min="1" max="120" />
            </label>
            <label>Break length <span class="unit">(sec)</span>
              <input type="number" id="f-break" min="5" max="600" />
            </label>
            <label>Pre-break warning <span class="unit">(sec, 0=off)</span>
              <input type="number" id="f-warn" min="0" max="120" />
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Reminders</h2>
          <div class="grid">
            <label>Reminder intensity<div id="sel-esc"></div></label>
            <label>Snooze duration <span class="unit">(min)</span>
              <input type="number" id="f-snooze" min="1" max="60" />
            </label>
            <label>Max postpones <span class="unit">(0=∞)</span>
              <input type="number" id="f-max" min="0" max="10" />
            </label>
            <label class="toggle-row">
              <span>Play a sound when a break starts</span>
              <span class="switch">
                <input type="checkbox" id="f-sound" />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Floating widget</h2>
          <div class="grid">
            <label>Widget mode<div id="sel-wmode"></div></label>
            <label>Widget shape<div id="sel-wshape"></div></label>
            <label>Width <span class="unit">(px)</span>
              <input type="number" id="f-wwidth" min="120" max="480" />
            </label>
            <label>Height <span class="unit">(px)</span>
              <input type="number" id="f-wheight" min="120" max="480" />
            </label>
            <label>Opacity <span class="unit">(%)</span>
              <input type="number" id="f-wopacity" min="20" max="100" />
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Startup</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Launch EyeBreak at login</span>
              <span class="switch">
                <input type="checkbox" id="f-autostart" />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Global shortcuts</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Enable system-wide hotkeys</span>
              <span class="switch">
                <input type="checkbox" id="f-gsc" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Pause / resume<input type="text" id="f-scpause" spellcheck="false" /></label>
            <label>Skip break<input type="text" id="f-scskip" spellcheck="false" /></label>
            <label>Take a break now<input type="text" id="f-sctake" spellcheck="false" /></label>
            <label>Postpone<input type="text" id="f-scpostpone" spellcheck="false" /></label>
            <label>Hide / show widget<input type="text" id="f-sctoggle" spellcheck="false" /></label>
          </div>
          <p class="hint">e.g. <code>CmdOrControl+Alt+P</code> — applied on Save.</p>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Work hours</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Only remind during work hours</span>
              <span class="switch">
                <input type="checkbox" id="f-wh" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Start<input type="time" id="f-wstart" /></label>
            <label>End<input type="time" id="f-wend" /></label>
            <label class="span-row">Active days
              <div class="daypicker" id="daypicker"></div>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Long breaks</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Add a longer break periodically</span>
              <span class="switch">
                <input type="checkbox" id="f-long" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Every <span class="unit">(breaks)</span>
              <input type="number" id="f-longevery" min="1" max="20" />
            </label>
            <label>Long length <span class="unit">(min)</span>
              <input type="number" id="f-longlen" min="1" max="60" />
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Eye health</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Blink reminders</span>
              <span class="switch">
                <input type="checkbox" id="f-blink" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Blink every <span class="unit">(min)</span>
              <input type="number" id="f-blinkint" min="1" max="60" />
            </label>
            <label class="toggle-row">
              <span>Hydration reminders</span>
              <span class="switch">
                <input type="checkbox" id="f-hydration" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Hydrate every <span class="unit">(min)</span>
              <input type="number" id="f-hydrationint" min="5" max="240" />
            </label>
            <label class="toggle-row">
              <span>Posture / distance reminders</span>
              <span class="switch">
                <input type="checkbox" id="f-posture" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Posture every <span class="unit">(min)</span>
              <input type="number" id="f-postureint" min="5" max="240" />
            </label>
            <label class="toggle-row">
              <span>Evening warm-screen nudge</span>
              <span class="switch">
                <input type="checkbox" id="f-evening" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Evening after <span class="unit">(hour 0–23)</span>
              <input type="number" id="f-eveninghour" min="0" max="23" />
            </label>
            <label class="toggle-row">
              <span>Show tips on the break screen</span>
              <span class="switch">
                <input type="checkbox" id="f-tips" />
                <span class="slider"></span>
              </span>
            </label>
            <label class="toggle-row">
              <span>Guided eye-exercises (long breaks)</span>
              <span class="switch">
                <input type="checkbox" id="f-exercises" />
                <span class="slider"></span>
              </span>
            </label>
            <label class="toggle-row">
              <span>Calming break visuals</span>
              <span class="switch">
                <input type="checkbox" id="f-calm" />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Idle &amp; presentation</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Pause when I'm away (idle)</span>
              <span class="switch">
                <input type="checkbox" id="f-idle" />
                <span class="slider"></span>
              </span>
            </label>
            <label>Idle after <span class="unit">(sec)</span>
              <input type="number" id="f-idlethresh" min="30" max="600" />
            </label>
            <label class="toggle-row">
              <span>Suppress break over fullscreen apps</span>
              <span class="switch">
                <input type="checkbox" id="f-suppress" />
                <span class="slider"></span>
              </span>
            </label>
            <label class="toggle-row">
              <span>Respect OS Do-Not-Disturb (hides widget &amp; break during screen-share)</span>
              <span class="switch">
                <input type="checkbox" id="f-dnd" />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Accessibility</h2>
          <div class="grid">
            <label class="toggle-row">
              <span>Reduce motion (no animations)</span>
              <span class="switch">
                <input type="checkbox" id="f-reduce" />
                <span class="slider"></span>
              </span>
            </label>
            <label class="toggle-row">
              <span>High contrast</span>
              <span class="switch">
                <input type="checkbox" id="f-contrast" />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Updates</h2>
          <div class="update-row">
            <button class="btn ghost" id="btn-checkupdate">Check for updates</button>
            <span class="update-msg" id="update-msg"></span>
          </div>
          <p class="hint">
            EyeBreak 0.1.0. Auto-update applies to AppImage / Windows / macOS
            builds; the apt-installed <code>.deb</code> updates via your package
            manager.
          </p>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Backup</h2>
          <div class="update-row">
            <button class="btn ghost" id="btn-export">Export settings</button>
            <button class="btn ghost" id="btn-import">Import settings</button>
            <input
              type="file"
              id="import-file"
              accept="application/json,.json"
              style="display: none"
            />
            <span class="update-msg" id="backup-msg"></span>
          </div>
          <p class="hint">Save your config to a JSON file, or load it on another machine.</p>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Theme</h2>
          <div class="grid">
            <label>Accent colour
              <input type="color" id="f-accent" />
            </label>
            <div class="swatches" id="swatches"></div>
          </div>
        </section>

        <section class="card s-card">
          <h2><span class="s-dot"></span> Habit stats</h2>
          <div class="stats-grid">
            <div class="stat"><div class="stat-num" id="st-streak">–</div><div class="stat-lbl">day streak</div></div>
            <div class="stat"><div class="stat-num" id="st-today">–</div><div class="stat-lbl">today</div></div>
            <div class="stat"><div class="stat-num" id="st-total">–</div><div class="stat-lbl">total breaks</div></div>
          </div>
          <div class="bars" id="st-bars"></div>
          <label class="toggle-row">
            <span>Track habit stats (local only, no telemetry)</span>
            <span class="switch">
              <input type="checkbox" id="f-stats" />
              <span class="slider"></span>
            </span>
          </label>
        </section>
      </div>

      <div class="save-row">
        <button class="btn" id="btn-save">Save settings</button>
        <span class="saved" id="saved-msg"></span>
      </div>
    </main>
  `;

  const $ = <T extends HTMLElement>(sel: string) =>
    document.querySelector<T>(sel)!;

  const c = mainSettings;
  const fWork = $<HTMLInputElement>("#f-work");
  const fBreak = $<HTMLInputElement>("#f-break");
  const fWarn = $<HTMLInputElement>("#f-warn");
  const fSnooze = $<HTMLInputElement>("#f-snooze");
  const fMax = $<HTMLInputElement>("#f-max");
  const fSound = $<HTMLInputElement>("#f-sound");
  const fWWidth = $<HTMLInputElement>("#f-wwidth");
  const fWHeight = $<HTMLInputElement>("#f-wheight");
  const fWOpacity = $<HTMLInputElement>("#f-wopacity");
  const fAutostart = $<HTMLInputElement>("#f-autostart");
  const fGsc = $<HTMLInputElement>("#f-gsc");
  const fScPause = $<HTMLInputElement>("#f-scpause");
  const fScSkip = $<HTMLInputElement>("#f-scskip");
  const fScTake = $<HTMLInputElement>("#f-sctake");
  const fScPostpone = $<HTMLInputElement>("#f-scpostpone");
  const fScToggle = $<HTMLInputElement>("#f-sctoggle");
  const fWh = $<HTMLInputElement>("#f-wh");
  const fWStart = $<HTMLInputElement>("#f-wstart");
  const fWEnd = $<HTMLInputElement>("#f-wend");
  const fLong = $<HTMLInputElement>("#f-long");
  const fLongEvery = $<HTMLInputElement>("#f-longevery");
  const fLongLen = $<HTMLInputElement>("#f-longlen");
  const fBlink = $<HTMLInputElement>("#f-blink");
  const fBlinkInt = $<HTMLInputElement>("#f-blinkint");
  const fIdle = $<HTMLInputElement>("#f-idle");
  const fIdleThresh = $<HTMLInputElement>("#f-idlethresh");
  const fSuppress = $<HTMLInputElement>("#f-suppress");
  const fDnd = $<HTMLInputElement>("#f-dnd");
  const fReduce = $<HTMLInputElement>("#f-reduce");
  const fContrast = $<HTMLInputElement>("#f-contrast");
  const fHydration = $<HTMLInputElement>("#f-hydration");
  const fHydrationInt = $<HTMLInputElement>("#f-hydrationint");
  const fPosture = $<HTMLInputElement>("#f-posture");
  const fPostureInt = $<HTMLInputElement>("#f-postureint");
  const fEvening = $<HTMLInputElement>("#f-evening");
  const fEveningHour = $<HTMLInputElement>("#f-eveninghour");
  const fTips = $<HTMLInputElement>("#f-tips");
  const fExercises = $<HTMLInputElement>("#f-exercises");
  const fCalm = $<HTMLInputElement>("#f-calm");
  const fAccent = $<HTMLInputElement>("#f-accent");
  const fStats = $<HTMLInputElement>("#f-stats");
  const savedMsg = $<HTMLSpanElement>("#saved-msg");

  // animated custom dropdowns (readable, unlike the native popup)
  const selEsc = customSelect(
    $("#sel-esc"),
    [
      { value: "gentle", label: "Gentle (notification)" },
      { value: "standard", label: "Standard (window)" },
      { value: "forced", label: "Forced (fullscreen)" },
    ],
    c.escalation,
  );
  const selWMode = customSelect(
    $("#sel-wmode"),
    [
      { value: "off", label: "Off" },
      { value: "minimized", label: "When minimized" },
      { value: "always", label: "Always on top" },
    ],
    c.widgetMode,
  );
  const selWShape = customSelect(
    $("#sel-wshape"),
    [
      { value: "round", label: "Round" },
      { value: "squircle", label: "Squircle" },
      { value: "square", label: "Square" },
    ],
    c.widgetShape,
  );

  // fill numeric/toggle fields
  fWork.value = String(Math.round(c.workIntervalSecs / 60));
  fBreak.value = String(c.breakLengthSecs);
  fWarn.value = String(c.preBreakWarningSecs);
  fSnooze.value = String(Math.round(c.snoozeSecs / 60));
  fMax.value = String(c.maxPostpones);
  fSound.checked = c.soundEnabled;
  fWWidth.value = String(c.widgetWidth);
  fWHeight.value = String(c.widgetHeight);
  fWOpacity.value = String(c.widgetOpacity);
  fAutostart.checked = c.autostart;
  fGsc.checked = c.globalShortcutsEnabled;
  fScPause.value = c.scPause;
  fScSkip.value = c.scSkip;
  fScTake.value = c.scTake;
  fScPostpone.value = c.scPostpone;
  fScToggle.value = c.scToggleWidget;
  fWh.checked = c.workHoursEnabled;
  fWStart.value = c.workStart;
  fWEnd.value = c.workEnd;
  fLong.checked = c.longBreakEnabled;
  fLongEvery.value = String(c.longBreakEvery);
  fLongLen.value = String(Math.round(c.longBreakSecs / 60));
  fBlink.checked = c.blinkEnabled;
  fBlinkInt.value = String(Math.round(c.blinkIntervalSecs / 60));
  fIdle.checked = c.idlePauseEnabled;
  fIdleThresh.value = String(c.idleThresholdSecs);
  fSuppress.checked = c.suppressOnFullscreen;
  fDnd.checked = c.respectDnd;
  fReduce.checked = c.reduceMotion;
  fContrast.checked = c.highContrast;
  fHydration.checked = c.hydrationEnabled;
  fHydrationInt.value = String(Math.round(c.hydrationIntervalSecs / 60));
  fPosture.checked = c.postureEnabled;
  fPostureInt.value = String(Math.round(c.postureIntervalSecs / 60));
  fEvening.checked = c.eveningNudgeEnabled;
  fEveningHour.value = String(c.eveningHour);
  fTips.checked = c.tipsEnabled;
  fExercises.checked = c.exercisesEnabled;
  fCalm.checked = c.calmVisualsEnabled;
  fAccent.value = c.accent || "#4cc6c0";
  fStats.checked = c.statsEnabled;

  // active-days picker (Mon..Sun)
  const DAY_LABELS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
  const dayState =
    c.workDays && c.workDays.length === 7
      ? c.workDays.slice()
      : [true, true, true, true, true, true, true];
  const dp = $<HTMLDivElement>("#daypicker");
  DAY_LABELS.forEach((lbl, i) => {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "daychip" + (dayState[i] ? " on" : "");
    chip.textContent = lbl;
    chip.addEventListener("click", () => {
      dayState[i] = !dayState[i];
      chip.classList.toggle("on", dayState[i]);
    });
    dp.appendChild(chip);
  });

  $<HTMLButtonElement>("#btn-back").addEventListener("click", () =>
    showDashboard(),
  );

  $<HTMLButtonElement>("#btn-save").addEventListener("click", async () => {
    const next: Settings = {
      ...mainSettings, // preserve fields not shown here (e.g. widget position)
      workIntervalSecs: Number(fWork.value) * 60,
      breakLengthSecs: Number(fBreak.value),
      preBreakWarningSecs: Number(fWarn.value),
      escalation: selEsc.value() as Escalation,
      snoozeSecs: Number(fSnooze.value) * 60,
      maxPostpones: Number(fMax.value),
      soundEnabled: fSound.checked,
      widgetMode: selWMode.value() as WidgetMode,
      widgetShape: selWShape.value() as WidgetShape,
      widgetWidth: Number(fWWidth.value),
      widgetHeight: Number(fWHeight.value),
      widgetOpacity: Number(fWOpacity.value),
      autostart: fAutostart.checked,
      globalShortcutsEnabled: fGsc.checked,
      scPause: fScPause.value.trim(),
      scSkip: fScSkip.value.trim(),
      scTake: fScTake.value.trim(),
      scPostpone: fScPostpone.value.trim(),
      scToggleWidget: fScToggle.value.trim(),
      workHoursEnabled: fWh.checked,
      workStart: fWStart.value || "09:00",
      workEnd: fWEnd.value || "17:00",
      workDays: dayState,
      longBreakEnabled: fLong.checked,
      longBreakEvery: Number(fLongEvery.value),
      longBreakSecs: Number(fLongLen.value) * 60,
      blinkEnabled: fBlink.checked,
      blinkIntervalSecs: Number(fBlinkInt.value) * 60,
      idlePauseEnabled: fIdle.checked,
      idleThresholdSecs: Number(fIdleThresh.value),
      suppressOnFullscreen: fSuppress.checked,
      respectDnd: fDnd.checked,
      reduceMotion: fReduce.checked,
      highContrast: fContrast.checked,
      hydrationEnabled: fHydration.checked,
      hydrationIntervalSecs: Number(fHydrationInt.value) * 60,
      postureEnabled: fPosture.checked,
      postureIntervalSecs: Number(fPostureInt.value) * 60,
      eveningNudgeEnabled: fEvening.checked,
      eveningHour: Number(fEveningHour.value),
      tipsEnabled: fTips.checked,
      exercisesEnabled: fExercises.checked,
      calmVisualsEnabled: fCalm.checked,
      accent: fAccent.value,
      statsEnabled: fStats.checked,
    };
    mainSettings = await invoke<Settings>("set_settings", { settings: next });
    fWWidth.value = String(mainSettings.widgetWidth);
    fWHeight.value = String(mainSettings.widgetHeight);
    fWOpacity.value = String(mainSettings.widgetOpacity);
    savedMsg.textContent = "Saved ✓";
    savedMsg.classList.remove("show");
    void savedMsg.offsetWidth; // restart the animation
    savedMsg.classList.add("show");
    setTimeout(() => (savedMsg.textContent = ""), 2000);
  });

  // --- updates ---
  const updBtn = $<HTMLButtonElement>("#btn-checkupdate");
  const updMsg = $<HTMLSpanElement>("#update-msg");
  updBtn.addEventListener("click", async () => {
    updMsg.textContent = "Checking…";
    updBtn.disabled = true;
    try {
      const ver = await invoke<string>("check_update");
      if (ver) {
        updMsg.innerHTML = `Update available: <b>${ver}</b>`;
        updBtn.textContent = "Download & install";
        updBtn.disabled = false;
        updBtn.onclick = async () => {
          updMsg.textContent = "Downloading…";
          updBtn.disabled = true;
          try {
            await invoke("install_update");
          } catch (err) {
            updMsg.textContent = `Failed: ${err}`;
            updBtn.disabled = false;
          }
        };
      } else {
        updMsg.textContent = "You're up to date ✓";
        updBtn.disabled = false;
      }
    } catch {
      updMsg.textContent = "Update check failed (no release published yet)";
      updBtn.disabled = false;
    }
  });

  // --- backup: export / import ---
  const backupMsg = $<HTMLSpanElement>("#backup-msg");
  $<HTMLButtonElement>("#btn-export").addEventListener("click", async () => {
    const cfg = await invoke<Settings>("get_settings");
    const blob = new Blob([JSON.stringify(cfg, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "eyebreak-settings.json";
    a.click();
    URL.revokeObjectURL(url);
    backupMsg.textContent = "Exported ✓";
    setTimeout(() => (backupMsg.textContent = ""), 1800);
  });
  const importFile = $<HTMLInputElement>("#import-file");
  $<HTMLButtonElement>("#btn-import").addEventListener("click", () =>
    importFile.click(),
  );
  importFile.addEventListener("change", async () => {
    const file = importFile.files?.[0];
    importFile.value = "";
    if (!file) return;
    try {
      const incoming = JSON.parse(await file.text()) as Partial<Settings>;
      const merged = { ...mainSettings, ...incoming } as Settings;
      mainSettings = await invoke<Settings>("set_settings", {
        settings: merged,
      });
      showSettings(); // re-render with the imported values
    } catch {
      backupMsg.textContent = "Invalid settings file";
    }
  });

  // --- theme swatches (live preview) ---
  const PRESETS = [
    "#4cc6c0",
    "#5b8def",
    "#e2725b",
    "#a78bfa",
    "#34d399",
    "#f59e0b",
  ];
  const sw = $<HTMLDivElement>("#swatches");
  for (const hex of PRESETS) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "swatch";
    b.style.background = hex;
    b.addEventListener("click", () => {
      fAccent.value = hex;
      document.documentElement.style.setProperty("--accent", hex);
    });
    sw.appendChild(b);
  }
  fAccent.addEventListener("input", () =>
    document.documentElement.style.setProperty("--accent", fAccent.value),
  );

  // --- habit stats ---
  try {
    const stats = await invoke<StatsSummary>("get_stats");
    $("#st-streak").textContent = String(stats.streak);
    $("#st-today").textContent = String(stats.todayTaken);
    $("#st-total").textContent = String(stats.totalTaken);
    const max = Math.max(1, ...stats.last7.map((d) => d.taken));
    $("#st-bars").innerHTML = stats.last7
      .map((d) => {
        const h = Math.round((d.taken / max) * 100);
        return `<div class="bar" title="${d.date}: ${d.taken} taken, ${d.skipped} skipped"><div class="bar-fill" style="height:${h}%"></div><span>${d.date.slice(5)}</span></div>`;
      })
      .join("");
  } catch {
    /* stats unavailable */
  }
}

async function renderMainWindow() {
  mainSettings = await invoke<Settings>("get_settings");
  showDashboard();
}

// ---------------------------------------------------------------------------
// Boot — pick the view from the URL hash
// ---------------------------------------------------------------------------

// close any open custom dropdown when clicking elsewhere
document.addEventListener("click", (e) => {
  if (!(e.target as HTMLElement).closest(".cselect")) {
    document
      .querySelectorAll(".cselect.open")
      .forEach((el) => el.classList.remove("open"));
  }
});

window.addEventListener("DOMContentLoaded", () => {
  // Apply accessibility prefs to every window, and keep them in sync.
  invoke<Settings>("get_settings").then(applyAppearance).catch(() => {});
  listen<Settings>("settings:changed", (e) => applyAppearance(e.payload));

  if (location.hash.startsWith("#break")) {
    renderBreak();
  } else if (location.hash.startsWith("#widget")) {
    renderWidget();
  } else {
    renderMainWindow();
  }
});
