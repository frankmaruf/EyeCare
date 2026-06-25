import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
  widgetSize: number;
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
// Main dashboard + settings view
// ---------------------------------------------------------------------------

async function renderDashboard() {
  const s = await invoke<Settings>("get_settings");
  let current = s; // last known full settings, so saves preserve unshown fields

  app.innerHTML = `
    <main class="dash">
      <header class="dash-head">
        <h1>EyeBreak</h1>
        <span class="tag" id="phase-tag">working</span>
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

      <section class="card">
        <h2>Settings</h2>
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

          <span class="section-label">Floating widget (shows when minimized)</span>
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
          <label>Widget size <span class="unit">(px)</span>
            <input type="number" id="f-wsize" min="80" max="320" />
          </label>
          <label>Widget opacity <span class="unit">(%)</span>
            <input type="number" id="f-wopacity" min="20" max="100" />
          </label>
        </div>
        <div class="save-row">
          <button class="btn" id="btn-save">Save settings</button>
          <span class="saved" id="saved-msg"></span>
        </div>
      </section>

      <footer class="dash-foot">
        Closing this window keeps EyeBreak running in the tray.
      </footer>
    </main>
  `;

  // --- element refs ---
  const ringFg = document.querySelector<SVGCircleElement>(".ring-fg")!;
  const ringTime = document.querySelector<HTMLDivElement>("#ring-time")!;
  const ringLabel = document.querySelector<HTMLDivElement>("#ring-label")!;
  const phaseTag = document.querySelector<HTMLSpanElement>("#phase-tag")!;
  const btnPause = document.querySelector<HTMLButtonElement>("#btn-pause")!;
  const btnTake = document.querySelector<HTMLButtonElement>("#btn-take")!;
  const btnSkip = document.querySelector<HTMLButtonElement>("#btn-skip")!;

  const fWork = document.querySelector<HTMLInputElement>("#f-work")!;
  const fBreak = document.querySelector<HTMLInputElement>("#f-break")!;
  const fWarn = document.querySelector<HTMLInputElement>("#f-warn")!;
  const fEsc = document.querySelector<HTMLSelectElement>("#f-esc")!;
  const fSnooze = document.querySelector<HTMLInputElement>("#f-snooze")!;
  const fMax = document.querySelector<HTMLInputElement>("#f-max")!;
  const fSound = document.querySelector<HTMLInputElement>("#f-sound")!;
  const fWMode = document.querySelector<HTMLSelectElement>("#f-wmode")!;
  const fWShape = document.querySelector<HTMLSelectElement>("#f-wshape")!;
  const fWSize = document.querySelector<HTMLInputElement>("#f-wsize")!;
  const fWOpacity = document.querySelector<HTMLInputElement>("#f-wopacity")!;
  const btnSave = document.querySelector<HTMLButtonElement>("#btn-save")!;
  const savedMsg = document.querySelector<HTMLSpanElement>("#saved-msg")!;

  // --- fill the form from current settings ---
  function fillForm(cfg: Settings) {
    current = cfg;
    fWork.value = String(Math.round(cfg.workIntervalSecs / 60));
    fBreak.value = String(cfg.breakLengthSecs);
    fWarn.value = String(cfg.preBreakWarningSecs);
    fEsc.value = cfg.escalation;
    fSnooze.value = String(Math.round(cfg.snoozeSecs / 60));
    fMax.value = String(cfg.maxPostpones);
    fSound.checked = cfg.soundEnabled;
    fWMode.value = cfg.widgetMode;
    fWShape.value = cfg.widgetShape;
    fWSize.value = String(cfg.widgetSize);
    fWOpacity.value = String(cfg.widgetOpacity);
  }
  fillForm(s);

  // --- timer controls ---
  let paused = false;
  btnPause.addEventListener("click", async () => {
    paused = !paused;
    await invoke("timer_set_paused", { paused });
  });
  btnTake.addEventListener("click", () => invoke("timer_take_break"));
  btnSkip.addEventListener("click", () => invoke("timer_skip"));

  // --- save settings ---
  btnSave.addEventListener("click", async () => {
    const next: Settings = {
      ...current, // preserve fields not shown here (e.g. widget position)
      workIntervalSecs: Number(fWork.value) * 60,
      breakLengthSecs: Number(fBreak.value),
      preBreakWarningSecs: Number(fWarn.value),
      escalation: fEsc.value as Escalation,
      snoozeSecs: Number(fSnooze.value) * 60,
      maxPostpones: Number(fMax.value),
      soundEnabled: fSound.checked,
      widgetMode: fWMode.value as WidgetMode,
      widgetShape: fWShape.value as WidgetShape,
      widgetSize: Number(fWSize.value),
      widgetOpacity: Number(fWOpacity.value),
    };
    const saved = await invoke<Settings>("set_settings", { settings: next });
    fillForm(saved); // reflect any clamping the backend applied
    savedMsg.textContent = "Saved ✓";
    setTimeout(() => (savedMsg.textContent = ""), 1800);
  });

  // --- live timer updates ---
  function applyTick(t: TimerSnapshot) {
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
  }

  applyTick(await invoke<TimerSnapshot>("get_timer"));
  await listen<TimerSnapshot>("timer:tick", (e) => applyTick(e.payload));
}

// ---------------------------------------------------------------------------
// Floating widget view (loaded at index.html#widget)
// ---------------------------------------------------------------------------

const WIDGET_CIRC = 2 * Math.PI * 44;

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
    <div class="widget" data-tauri-drag-region>
      <svg class="w-ring" viewBox="0 0 100 100" data-tauri-drag-region>
        <circle class="w-ring-bg" cx="50" cy="50" r="44"></circle>
        <circle class="w-ring-fg" cx="50" cy="50" r="44" transform="rotate(-90 50 50)"></circle>
      </svg>
      <div class="w-center" data-tauri-drag-region>
        <div class="w-time" id="w-time">--:--</div>
      </div>
      <button class="w-restore" id="w-restore" title="Open EyeBreak">⤢</button>
    </div>
  `;

  applyWidgetStyle(s);

  const card = document.querySelector<HTMLDivElement>(".widget")!;
  const ringFg = document.querySelector<SVGCircleElement>(".w-ring-fg")!;
  const timeEl = document.querySelector<HTMLDivElement>("#w-time")!;

  document
    .querySelector<HTMLButtonElement>("#w-restore")!
    .addEventListener("click", (e) => {
      e.stopPropagation();
      invoke("show_main");
    });

  const update = (t: TimerSnapshot) => {
    timeEl.textContent = fmt(t.remaining);
    setRing(ringFg, t.remaining, t.total, WIDGET_CIRC);
    card.dataset.phase = t.paused ? "paused" : t.phase;
  };

  update(await invoke<TimerSnapshot>("get_timer"));
  await listen<TimerSnapshot>("timer:tick", (e) => update(e.payload));
  await listen<Settings>("settings:changed", (e) => applyWidgetStyle(e.payload));
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
    renderDashboard();
  }
});
