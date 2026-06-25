import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
}

interface TimerSnapshot {
  phase: "working" | "break";
  remaining: number;
  total: number;
  paused: boolean;
  postponesUsed: number;
  maxPostpones: number;
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

const app = document.querySelector<HTMLDivElement>("#app")!;

// ---------------------------------------------------------------------------
// Break window view (loaded at index.html#break)
// ---------------------------------------------------------------------------

function renderBreak() {
  if (location.hash.includes("sound=1")) beep();

  app.innerHTML = `
    <div class="break-screen" data-tauri-drag-region>
      <p class="break-eyebrow">EyeBreak</p>
      <h1 class="break-title">Look ~20 feet away</h1>
      <p class="break-sub">Relax your eyes — let your focus drift to the distance.</p>
      <div class="break-count" id="break-count">00:20</div>
      <div class="break-actions">
        <button class="btn ghost" id="break-postpone">Postpone</button>
        <button class="btn" id="break-skip">Skip break</button>
      </div>
      <p class="break-note" id="break-note"></p>
    </div>
  `;

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

  listen<TimerSnapshot>("timer:tick", (e) => {
    const t = e.payload;
    if (t.phase === "break") {
      countEl.textContent = fmt(t.remaining);
    }
    if (t.maxPostpones > 0) {
      postBtn.disabled = t.postponesUsed >= t.maxPostpones;
    }
  });

  // The backend closes this window on break-end / skip / postpone.
}

// ---------------------------------------------------------------------------
// Floating widget view (loaded at index.html#widget)
// ---------------------------------------------------------------------------

const WIDGET_CIRC = 2 * Math.PI * 44;

// Monochrome inline icons (inherit currentColor) so the action bar looks
// consistent — no clashing colour emoji.
const ICON_PAUSE = `<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1.2"/><rect x="14" y="5" width="4" height="14" rx="1.2"/></svg>`;
const ICON_PLAY = `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.5l13 7.5-13 7.5z"/></svg>`;
const ICON_EYE = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>`;
const ICON_SKIP = `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M5 4.5l10 7.5-10 7.5z"/><rect x="16.5" y="5" width="3" height="14" rx="1.2"/></svg>`;
const ICON_EXPAND = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6"/><path d="M10 20H4v-6"/><path d="M20 4l-8 8"/><path d="M4 20l8-8"/></svg>`;

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
  const ringFg = document.querySelector<SVGCircleElement>(".w-ring-fg")!;
  const timeEl = document.querySelector<HTMLDivElement>("#w-time")!;
  const pauseBtn = document.querySelector<HTMLButtonElement>("#w-pause")!;

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

  // Resize grip (bottom-right corner) — drag to resize the frameless window.
  document
    .querySelector<HTMLDivElement>("#w-resize")!
    .addEventListener("pointerdown", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      try {
        await getCurrentWindow().startResizeDragging("SouthEast");
      } catch {
        /* ignore */
      }
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
        <button class="icon-btn" id="btn-settings" title="Settings">⚙</button>
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
    phaseTag.textContent = t.paused
      ? "paused"
      : t.phase === "break"
        ? "break"
        : "working";
    phaseTag.dataset.phase = t.paused ? "paused" : t.phase;
    ringTime.textContent = fmt(t.remaining);
    ringLabel.textContent =
      t.phase === "break" ? "break time left" : "until next break";
    setRing(ringFg, t.remaining, t.total);
  };

  applyTick(await invoke<TimerSnapshot>("get_timer"));
  unlistenMainTick = await listen<TimerSnapshot>("timer:tick", (e) =>
    applyTick(e.payload),
  );
}

async function showSettings() {
  stopMainTick();

  app.innerHTML = `
    <main class="dash settings-page">
      <header class="dash-head">
        <button class="icon-btn" id="btn-back" title="Back">←</button>
        <h1>Settings</h1>
        <span class="icon-spacer"></span>
      </header>

      <section class="card">
        <h2>Timing</h2>
        <div class="grid">
          <label>Work interval <span class="unit">(minutes)</span>
            <input type="number" id="f-work" min="1" max="120" />
          </label>
          <label>Break length <span class="unit">(seconds)</span>
            <input type="number" id="f-break" min="5" max="600" />
          </label>
          <label>Pre-break warning <span class="unit">(seconds, 0=off)</span>
            <input type="number" id="f-warn" min="0" max="120" />
          </label>
        </div>
      </section>

      <section class="card">
        <h2>Reminders</h2>
        <div class="grid">
          <label>Reminder intensity
            <select id="f-esc">
              <option value="gentle">Gentle (notification)</option>
              <option value="standard">Standard (window)</option>
              <option value="forced">Forced (fullscreen)</option>
            </select>
          </label>
          <label>Snooze duration <span class="unit">(minutes)</span>
            <input type="number" id="f-snooze" min="1" max="60" />
          </label>
          <label>Max postpones <span class="unit">(0=unlimited)</span>
            <input type="number" id="f-max" min="0" max="10" />
          </label>
          <label class="check">
            <input type="checkbox" id="f-sound" /> Play a sound when a break starts
          </label>
        </div>
      </section>

      <section class="card">
        <h2>Floating widget</h2>
        <div class="grid">
          <label>Widget mode
            <select id="f-wmode">
              <option value="off">Off</option>
              <option value="minimized">When minimized</option>
              <option value="always">Always on top</option>
            </select>
          </label>
          <label>Widget shape
            <select id="f-wshape">
              <option value="round">Round</option>
              <option value="squircle">Squircle</option>
              <option value="square">Square</option>
            </select>
          </label>
          <label>Widget width <span class="unit">(px)</span>
            <input type="number" id="f-wwidth" min="80" max="480" />
          </label>
          <label>Widget height <span class="unit">(px)</span>
            <input type="number" id="f-wheight" min="80" max="480" />
          </label>
          <label>Widget opacity <span class="unit">(%)</span>
            <input type="number" id="f-wopacity" min="20" max="100" />
          </label>
        </div>
      </section>

      <div class="save-row">
        <button class="btn" id="btn-save">Save settings</button>
        <span class="saved" id="saved-msg"></span>
      </div>
    </main>
  `;

  const $ = <T extends HTMLElement>(sel: string) =>
    document.querySelector<T>(sel)!;

  const fWork = $<HTMLInputElement>("#f-work");
  const fBreak = $<HTMLInputElement>("#f-break");
  const fWarn = $<HTMLInputElement>("#f-warn");
  const fEsc = $<HTMLSelectElement>("#f-esc");
  const fSnooze = $<HTMLInputElement>("#f-snooze");
  const fMax = $<HTMLInputElement>("#f-max");
  const fSound = $<HTMLInputElement>("#f-sound");
  const fWMode = $<HTMLSelectElement>("#f-wmode");
  const fWShape = $<HTMLSelectElement>("#f-wshape");
  const fWWidth = $<HTMLInputElement>("#f-wwidth");
  const fWHeight = $<HTMLInputElement>("#f-wheight");
  const fWOpacity = $<HTMLInputElement>("#f-wopacity");
  const savedMsg = $<HTMLSpanElement>("#saved-msg");

  // fill from the last known settings
  const c = mainSettings;
  fWork.value = String(Math.round(c.workIntervalSecs / 60));
  fBreak.value = String(c.breakLengthSecs);
  fWarn.value = String(c.preBreakWarningSecs);
  fEsc.value = c.escalation;
  fSnooze.value = String(Math.round(c.snoozeSecs / 60));
  fMax.value = String(c.maxPostpones);
  fSound.checked = c.soundEnabled;
  fWMode.value = c.widgetMode;
  fWShape.value = c.widgetShape;
  fWWidth.value = String(c.widgetWidth);
  fWHeight.value = String(c.widgetHeight);
  fWOpacity.value = String(c.widgetOpacity);

  $<HTMLButtonElement>("#btn-back").addEventListener("click", () =>
    showDashboard(),
  );

  $<HTMLButtonElement>("#btn-save").addEventListener("click", async () => {
    const next: Settings = {
      ...mainSettings, // preserve fields not shown here (e.g. widget position)
      workIntervalSecs: Number(fWork.value) * 60,
      breakLengthSecs: Number(fBreak.value),
      preBreakWarningSecs: Number(fWarn.value),
      escalation: fEsc.value as Escalation,
      snoozeSecs: Number(fSnooze.value) * 60,
      maxPostpones: Number(fMax.value),
      soundEnabled: fSound.checked,
      widgetMode: fWMode.value as WidgetMode,
      widgetShape: fWShape.value as WidgetShape,
      widgetWidth: Number(fWWidth.value),
      widgetHeight: Number(fWHeight.value),
      widgetOpacity: Number(fWOpacity.value),
    };
    mainSettings = await invoke<Settings>("set_settings", { settings: next });
    // reflect any clamping the backend applied
    fWWidth.value = String(mainSettings.widgetWidth);
    fWHeight.value = String(mainSettings.widgetHeight);
    fWOpacity.value = String(mainSettings.widgetOpacity);
    savedMsg.textContent = "Saved ✓";
    setTimeout(() => (savedMsg.textContent = ""), 1800);
  });
}

async function renderMainWindow() {
  mainSettings = await invoke<Settings>("get_settings");
  showDashboard();
}

// ---------------------------------------------------------------------------
// Boot — pick the view from the URL hash
// ---------------------------------------------------------------------------

window.addEventListener("DOMContentLoaded", () => {
  if (location.hash.startsWith("#break")) {
    renderBreak();
  } else if (location.hash.startsWith("#widget")) {
    renderWidget();
  } else {
    renderMainWindow();
  }
});
