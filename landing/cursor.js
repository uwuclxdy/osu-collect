/* ds-kit ambient layer, cursor v3: the jelly mark lit up with speed, its tube trail in
   the comet blue, leaving the mark 4px narrower than it is across the motion and stopping
   2px short of its keyline, the parting dot grid, droplets and click. Desktop fine-pointer
   only, silent under `prefers-reduced-motion`. Rules, mechanisms and anti-patterns: cursor.md.
   Load as `<script type="module" src="cursor.js"></script>` after cursor.css. */

import { SEMANTIC_POINTER, SEMANTIC_BLOCKED, collectCursorSelectors } from './cursor-rules.js';

const reducedQuery = matchMedia('(prefers-reduced-motion: reduce)');
// the gate is the media query outside which the sheet hides #cursor and stops hiding the
// native cursor, so no device arms the hidden cursor for a mark the stylesheet already hid.
// Both queries are watched: a visitor switching reduced motion on mid-session gets the
// native cursor back at once, from the module standing down and from the sheet alike, and
// the drawn one back on the next move after switching it off again (a page booted under
// reduced motion never builds it)
const fineQuery = matchMedia('(pointer: fine) and (hover: hover)');
// a list that carries a `change` listener is never read: in Chrome a `.matches` read can eat
// the list's next change event (cursor.md, the anti-patterns), so the gate reads through
// lists of its own that nothing listens on
const reducedNow = matchMedia(reducedQuery.media), fineNow = matchMedia(fineQuery.media);
const motionAllowed = () => fineNow.matches && !reducedNow.matches;
const onGateChange = fn => {
  reducedQuery.addEventListener('change', fn);
  fineQuery.addEventListener('change', fn);
};
const root = document.documentElement;

/* Tokens resolve through a 1x1 canvas: getComputedStyle serializes oklch back
   as oklch, while fillStyle takes any colour the engine knows. A value
   fillStyle rejects leaves the previous one standing, hence the sentinel. */
const swatch = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
const resolveColour = (prop, fallback) => {
  const css = getComputedStyle(root).getPropertyValue(prop).trim();
  if (!css) return fallback;
  swatch.fillStyle = '#000000';
  swatch.fillStyle = css;
  if (swatch.fillStyle === '#000000' && css !== '#000000') return fallback;
  swatch.clearRect(0, 0, 1, 1);
  swatch.fillRect(0, 0, 1, 1);
  const d = swatch.getImageData(0, 0, 1, 1).data;
  return [d[0], d[1], d[2]];
};
const onThemeChange = fn => new MutationObserver(fn).observe(root, { attributeFilter: ['data-theme'] });

/* Every spring here is underdamped (zeta < 1). With its target held over a step, its exact
   solution maps the offset from the target and the velocity through one matrix, so a frame
   of any length lands where the continuous spring would be: no step size, the same curve at
   every refresh rate. dt is in the unit omega's radians are per. */
const springMatrix = (omega, zeta, dt) => {
  const damping = zeta * omega, wd = omega * Math.sqrt(1 - zeta * zeta);
  const decay = Math.exp(-damping * dt), cos = Math.cos(wd * dt), sin = Math.sin(wd * dt) / wd;
  return {
    xx: decay * (cos + damping * sin), xv: decay * sin,
    vx: -decay * omega * omega * sin, vv: decay * (cos - damping * sin),
  };
};

// an input the page receives after a frame that passed its stamp is still played from its stamp,
// this far back at most; an older one is played from this far back
const MAX_LATE_MS = 64;

// ---- the dot grid: a still texture, a spotlight at the pointer, and a wake ----
// the grid's move handler, built only when motion is allowed at boot. It rides the one
// module-level pointermove listener that also looks for a late sheet, so a boot during a move
// still pushes the grid on that move
let gridTrack = null;
const bootGrid = () => {
  let canvas = document.getElementById('bg-dots');
  if (!canvas) {
    canvas = document.createElement('canvas');
    canvas.id = 'bg-dots';
    canvas.setAttribute('aria-hidden', 'true');
    document.body.prepend(canvas);
  }
  const ctx = canvas.getContext('2d');
  const gap = 40, R = 150;
  // `fromX`/`fromY`: where the next push measures from; `slack`: the speed cap's unspent allowance
  const mouse = { x: 0, y: 0, t: 0, active: false, fromX: 0, fromY: 0, slack: 0 };
  let w, h, dots = [], cols = 0, rows = 0, originX = 0, originY = 0;
  let scheduled = false, flowing = false, lastFrame = 0;
  let base = [54, 56, 71], lit = [104, 187, 239];
  // the lift keeps its peak whatever the floor, so a raised --dot-rest shortens
  // the spotlight's travel instead of pushing it past the top of the ramp
  const REST = 0.24, PEAK = 0.80;
  let rest = REST;
  const readTokens = () => {
    base = resolveColour('--line', base);
    lit = resolveColour('--accent', lit);
    const raw = parseFloat(getComputedStyle(root).getPropertyValue('--dot-rest'));
    rest = Number.isFinite(raw) ? Math.min(Math.max(raw, 0), PEAK) : REST;
  };
  root.style.setProperty('--dot-gap', gap + 'px');

  /* THE WAKE. A moving pointer pushes every dot inside WAKE_R away from itself, by the
     distance it moved since its last move (speed times the interval, so a stroke's whole
     push follows its length rather than the event rate), and each dot rides a damped spring
     back to its home: omega 0.02 rad/ms at zeta 0.5. A dot that settles is snapped to zero,
     so the resting frame is byte-identical to the one before the wake. A parted dot also
     lights up by how far it sits from home, so the wake glows while it swings back. */
  const WAKE_R = 110, WAKE_K = 0.012, WAKE_MAX = 14, OMEGA = 0.02, ZETA = 0.5, WAKE_SPEED = 3;
  const EPS = 0.02;
  // the dots play this long at most past the last frame or move: a quiet stall is skipped, and a
  // stall the pointer moves through plays its real time
  const MAX_FLOW_MS = 50;
  // the speed cap is an allowance of WAKE_SPEED per ms that carries this much of it unspent, so
  // stamps rounded to whole ms, which can put two samples' travel in one interval, push in full
  const WAKE_SLACK_MS = 1;

  const build = () => {
    const dpr = Math.min(devicePixelRatio || 1, 2);
    w = innerWidth; h = innerHeight;
    canvas.width = w * dpr; canvas.height = h * dpr;
    canvas.style.width = w + 'px'; canvas.style.height = h + 'px';
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    dots = [];
    cols = Math.ceil(w / gap) + 1; rows = Math.ceil(h / gap) + 1;
    originX = (w - (cols - 1) * gap) / 2; originY = (h - (rows - 1) * gap) / 2;
    for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
      dots.push({ bx: originX + x * gap, by: originY + y * gap, ox: 0, oy: 0, vx: 0, vy: 0, pushed: false });
    }
  };
  const draw = () => {
    ctx.clearRect(0, 0, w, h);
    const on = motionAllowed() && mouse.active;
    const shx = on ? (mouse.x - w / 2) / w * 10 : 0, shy = on ? (mouse.y - h / 2) / h * 10 : 0;
    for (const d of dots) {
      const px = d.bx + d.ox + shx, py = d.by + d.oy + shy;
      let f = 0;
      if (on) {
        const dist = Math.hypot(px - mouse.x, py - mouse.y);
        if (dist < R) f = (1 - dist / R) ** 2;
      }
      if (d.ox || d.oy) f = Math.max(f, Math.min(1, Math.hypot(d.ox, d.oy) / WAKE_MAX) * 0.7);
      const a = rest + f * (PEAK - rest), r = 1.15 + f * 1.45;
      const c = f ? base.map((b, i) => Math.round(b + (lit[i] - b) * f)) : base;
      ctx.beginPath(); ctx.arc(px, py, r, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(${c[0]},${c[1]},${c[2]},${a})`; ctx.fill();
    }
  };
  const schedule = () => { if (!scheduled && !flowing) { scheduled = true; requestAnimationFrame(() => { scheduled = false; draw(); }); } };

  const resting = d => !d.ox && !d.oy && !d.vx && !d.vy;
  const stepDot = (d, k) => {
    const { ox, oy, vx, vy } = d;
    d.ox = k.xx * ox + k.xv * vx; d.vx = k.vx * ox + k.vv * vx;
    d.oy = k.xx * oy + k.xv * vy; d.vy = k.vx * oy + k.vv * vy;
    // the cap is a wall: a dot that reaches it stops there with its outward speed gone, so
    // a scribble held in one place cannot fling it further
    const off = Math.hypot(d.ox, d.oy);
    if (off > WAKE_MAX) {
      const outward = (d.vx * d.ox + d.vy * d.oy) / off;
      if (outward > 0) { d.vx -= outward * d.ox / off; d.vy -= outward * d.oy / off; }
      d.ox *= WAKE_MAX / off; d.oy *= WAKE_MAX / off;
    }
  };

  // the instant the dots' clock stops at until the next frame or move
  const flowsUntil = () => Math.max(lastFrame, mouse.t) + MAX_FLOW_MS;

  // the spring loop runs only while a dot is off its home, and parks on the resting frame.
  // `lastFrame` is the instant the dots stand at: the push that woke the loop, then each
  // frame, or a move landing more than MAX_FLOW_MS past it (catchUp). A frame stamped before
  // that push (an input landing after the vsync it is dispatched in) steps nothing.
  const flow = now => {
    const dt = Math.max(0, Math.min(now, flowsUntil()) - lastFrame);
    lastFrame = Math.max(lastFrame, now);
    const k = springMatrix(OMEGA, ZETA, dt);
    let moving = false;
    for (const d of dots) {
      if (resting(d)) continue;
      stepDot(d, k);
      // a dot pushed since the last frame is never snapped on it: a weak push at the wake's
      // edge adds up with the next one at every refresh rate, not only where frames are rare
      if (!d.pushed && Math.abs(d.ox) < EPS && Math.abs(d.oy) < EPS && Math.abs(d.vx) < EPS / 16 && Math.abs(d.vy) < EPS / 16) {
        d.ox = d.oy = d.vx = d.vy = 0;
      } else moving = true;
      d.pushed = false;
    }
    draw();
    if (moving) requestAnimationFrame(flow);
    else flowing = false;
  };
  // a move landing more than MAX_FLOW_MS after the instant the dots stand at first brings them
  // to it on their own clock, skipping the stall between as a frame would, so a push is never
  // folded back more than MAX_FLOW_MS nor across a stall the clock skipped
  const catchUp = at => {
    if (!flowing || at - lastFrame <= MAX_FLOW_MS) return;
    const k = springMatrix(OMEGA, ZETA, Math.min(at, flowsUntil()) - lastFrame);
    for (const d of dots) if (!resting(d)) stepDot(d, k);
    lastFrame = at;
  };
  const part = (x, y, travel, at) => {
    // the dots are drawn under the parallax shift, so the wake is centred where they paint
    x -= (mouse.x - w / 2) / w * 10; y -= (mouse.y - h / 2) / h * 10;
    const push = travel * WAKE_K;
    // only the grid cells the wake can reach are visited: the pitch is regular, so the
    // index window is arithmetic, and a parted dot sits at most WAKE_MAX from its cell
    const reach = WAKE_R + WAKE_MAX;
    const x0 = Math.max(0, Math.floor((x - reach - originX) / gap)), x1 = Math.min(cols - 1, Math.ceil((x + reach - originX) / gap));
    const y0 = Math.max(0, Math.floor((y - reach - originY) / gap)), y1 = Math.min(rows - 1, Math.ceil((y + reach - originY) / gap));
    // a push wakes the loop at its own instant; one landing while the loop runs meets each
    // dot where it stands at `at`, and is folded by the spring's own motion to the instant
    // the dots stand at, back to it, or forward from a stamp up to MAX_LATE_MS before it, so
    // the dots move from `at` on as if the push had landed then
    if (!flowing) { flowing = true; lastFrame = at; requestAnimationFrame(flow); }
    const since = Math.max(at - lastFrame, -MAX_LATE_MS);
    const ahead = springMatrix(OMEGA, ZETA, since), back = springMatrix(OMEGA, ZETA, -since);
    for (let row = y0; row <= y1; row++) for (let col = x0; col <= x1; col++) {
      const d = dots[row * cols + col];
      const dx = d.bx + ahead.xx * d.ox + ahead.xv * d.vx - x, dy = d.by + ahead.xx * d.oy + ahead.xv * d.vy - y;
      const dist = Math.hypot(dx, dy);
      if (dist >= WAKE_R || dist < 0.5) continue;
      const f = (1 - dist / WAKE_R) ** 2 * push;
      const kx = dx / dist * f, ky = dy / dist * f;
      d.ox += back.xv * kx; d.vx += back.vv * kx;
      d.oy += back.xv * ky; d.vy += back.vv * ky;
      d.pushed = true;
    }
  };

  readTokens(); build(); draw();
  if (motionAllowed()) {
    gridTrack = e => {
      if (!motionAllowed()) return;
      // one push per move the pointer made, coalesced into this event or not, each at the
      // move's own instant held inside [the last move's, now], so the pushes are the same
      // however the browser batches the moves into dispatches
      const batch = e.getCoalescedEvents ? e.getCoalescedEvents() : [];
      for (const c of batch.length ? batch : [e]) {
        const at = Math.min(Math.max(c.timeStamp, mouse.t), performance.now());
        catchUp(at);
        // a gap past 80ms is a new stroke, never a 1000px jump to push with
        const stroke = mouse.active && mouse.t && at - mouse.t <= 80;
        // a sample sharing the instant of the one before it moves the pointer and pushes
        // nothing: the next later one pushes for both, so no travel is lost to stamps in whole ms
        if (stroke && at === mouse.t) { mouse.x = c.clientX; mouse.y = c.clientY; continue; }
        if (stroke) {
          // the speed cap as a distance, so it binds the same at every event rate
          const allowance = mouse.slack + WAKE_SPEED * (at - mouse.t);
          const travel = Math.min(Math.hypot(c.clientX - mouse.fromX, c.clientY - mouse.fromY), allowance);
          mouse.slack = Math.min(allowance - travel, WAKE_SPEED * WAKE_SLACK_MS);
          part(c.clientX, c.clientY, travel, at);
        } else mouse.slack = 0;
        mouse.x = mouse.fromX = c.clientX; mouse.y = mouse.fromY = c.clientY; mouse.t = at; mouse.active = true;
      }
      schedule();
    };
    root.addEventListener('mouseleave', () => { mouse.active = false; schedule(); });
    // a gate closing mid-session puts every dot home now, not after its swing
    onGateChange(() => {
      if (motionAllowed()) return;
      for (const d of dots) d.ox = d.oy = d.vx = d.vy = 0;
      mouse.active = false;
      schedule();
    });
  }
  addEventListener('resize', () => { build(); draw(); });
  onThemeChange(() => { readTokens(); schedule(); });
};

// ---- the jelly drop (replaces the native cursor) ----
const bootMark = () => {
  const host = document.createElement('div');
  host.id = 'cursor';
  host.setAttribute('aria-hidden', 'true');
  const canvas = document.createElement('canvas');
  canvas.id = 'cursor-paint';
  host.append(canvas);
  document.body.appendChild(host);
  const ctx = canvas.getContext('2d');
  // the bloom is its own element under the host: a canvas blend mode composites only
  // against the canvas, so the one way to lighten the page without shading it is the
  // halo's, a CSS mix-blend-mode on a node the page composites
  const bloom = document.createElement('div');
  bloom.id = 'cursor-glow';
  bloom.setAttribute('aria-hidden', 'true');
  document.body.appendChild(bloom);

  const rootStyle = getComputedStyle(root);
  const token = (name, fallback) => parseFloat(rootStyle.getPropertyValue(name)) || fallback;
  const SIZE = {
    idle: token('--cursor-size-idle', 18),
    hover: token('--cursor-size-hover', 24),
    press: token('--cursor-size-press', 19),
    pressIdle: token('--cursor-size-press-idle', 14),
    blocked: token('--cursor-size-blocked', 24),
  };
  const RIPPLE_MS = token('--dur-far', 260);
  const RIPPLE_R0 = 10, RIPPLE_R1 = 36;   // a 20px ring opening to 72px
  const SHOW_MS = token('--dur-near', 180), HIDE_MS = SHOW_MS * 0.6;   // the mark's own fade, an exit at 0.6x
  const STROKE = 2, DOT_R = 2, KEYLINE = 2.6;
  // the trail stops GAP_PX short of the keyline's outer edge, so a dark ring of one width
  // always separates the mark from its trail: the hole cut under the trail is the mark's
  // outline grown by half the keyline plus the gap
  const GAP_PX = 2, HOLE_PX = KEYLINE / 2 + GAP_PX;
  const TAU = Math.PI * 2;

  // the system's --ease-out, cubic-bezier(0.16, 1, 0.3, 1), solved for x by Newton's method
  const cubicBezier = (x1, y1, x2, y2) => {
    const at = (a, b, t) => ((1 - 3 * b + 3 * a) * t * t + (3 * b - 6 * a) * t + 3 * a) * t;
    const slope = (a, b, t) => 3 * (1 - 3 * b + 3 * a) * t * t + 2 * (3 * b - 6 * a) * t + 3 * a;
    return x => {
      let t = x;
      for (let i = 0; i < 8; i++) {
        const d = slope(x1, x2, t);
        if (Math.abs(d) < 1e-6) break;
        t -= (at(x1, x2, t) - x) / d;
      }
      return at(y1, y2, Math.min(1, Math.max(0, t)));
    };
  };
  const easeOut = cubicBezier(0.16, 1, 0.3, 1);

  // The trail is a time window, as long as the last TAIL_MS of travel, that drains
  // into the ring when the pointer stops and is dropped once that drain has run the window
  // out: nothing is left at rest. Stillness past DRAIN_MS is not travel: the window is
  // measured on a clock that stops with the pointer, while the path it holds ages in real
  // time, so a pointer that stops between steps leaves a shorter tail than its path alone.
  // At a crawl the window stretches until it holds the mark's larger half size of path
  // past the hole, so a short tail shows behind the mark; CRAWL_MS caps it at a second of
  // travel. A run of travel begins from rest or from a stop, and its tail grows from
  // nothing as path accrues past the hole (see drawTrail).
  const TAIL_MS = 240, CRAWL_MS = 1000;
  // the centre line gets a point at least every ARC_PX of path and every STEP_MS of time.
  // MAX_STEPS holds about 8000px of path in one window that way; a window wanting more is
  // laid proportionally coarser, never cut short
  const ARC_PX = 2, STEP_MS = 2, MAX_STEPS = 4096;
  // a span at least GAP_MIN_PX long and over GAP_RATIO times the chords on both sides of it
  // is samples the browser lost; each side's direction is read over GAP_TANGENT_PX of path,
  // and the fill lays points about a neighbouring chord apart: at least GAP_SPACING_PX, at
  // most GAP_MAX_POINTS of them
  const GAP_MIN_PX = 24, GAP_RATIO = 3, GAP_TANGENT_PX = 8, GAP_SPACING_PX = 4, GAP_MAX_POINTS = 64;
  // the drawn centre line and the recorded half sizes are low-passed over this much arc,
  // resampled every RESAMPLE_PX
  const SMOOTH_PX = 8, RESAMPLE_PX = 2;
  // a pointer quiet this long has stopped and the tube starts to drain; a mouse creeping a
  // pixel every 80ms is still travelling
  const DRAIN_MS = 90;
  const TRAIL_PEAK = 0.9;       // effective alpha at the head
  const TEXT_PEAK = 0.6;        // over text, so the glyph just behind the caret shows through
  const TRIM_PX = 4;            // the trail starts this much narrower than the mark is across the motion
  const TANGENT_PX = 1;         // a grid point's direction is read over at least this much path
  const HEAD_TANGENT_PX = 16;   // and the head's over this much path behind it
  const EXIT_TANGENT_PX = 2;    // and the line's where it leaves the hole over this much each side
  // past where the line leaves the hole the tube eases from the mark's own width across the
  // motion to the narrow axis its samples recorded over this much path
  const START_EASE_PX = 16;
  // the start reads across the line, not the head, once the exit lies this far past the
  // hole's edge and the turn there clears the hole by as much (startWidth). A smaller scale
  // reads the line sooner at a hairpin, and sooner on a fast shake's turns that barely clear
  // the hole, where the line read jumps from frame to frame
  const START_RAMP_PX = 3.3;
  const PIXEL_ALPHA = 0.5 / 255;   // a stamp fainter than this rounds to nothing on the cleared canvas
  const gridX = new Float32Array(MAX_STEPS + 1), gridY = new Float32Array(MAX_STEPS + 1);
  const gridAge = new Float32Array(MAX_STEPS + 1);
  const gridDist = new Float32Array(MAX_STEPS + 1);   // path length back from the head
  const gridEnd = new Float32Array(MAX_STEPS + 1);    // where that point's run of travel ends, as path length back from the head
  const gridPause = new Float32Array(MAX_STEPS + 1);  // stillness past DRAIN_MS, summed over the gaps since that point was laid
  const pauseAfter = [];                              // the same per path point, filled each frame
  const knots = [];                                   // where each run of path points at one position starts, filled each frame
  const spanX = [0, 0, 0, 0], spanY = [0, 0, 0, 0];   // the four control points of the span being resampled
  // the mark's half sizes under its squeeze, less the trim, as each point's samples recorded
  // them, low-passed along the path; the tube's half width there (before the age fade and the
  // run caps); and the trail's peak alpha there (lower over text)
  const gridW = new Float32Array(MAX_STEPS + 1), gridH = new Float32Array(MAX_STEPS + 1);
  const gridAcross = new Float32Array(MAX_STEPS + 1);
  const gridPeak = new Float32Array(MAX_STEPS + 1);
  const gridTx = new Float32Array(MAX_STEPS + 1), gridTy = new Float32Array(MAX_STEPS + 1);
  const gridT = new Float64Array(MAX_STEPS + 1);      // the instant each point was laid at
  const gridArc = new Float64Array(MAX_STEPS + 1);    // path length from the tail, for the low-pass
  // the centre line and the half sizes resampled evenly by arc, before and after the
  // low-pass, and its full-width kernel
  let evenX = new Float64Array(4096), evenY = new Float64Array(4096), evenW = new Float64Array(4096), evenH = new Float64Array(4096);
  let lowX = new Float64Array(4096), lowY = new Float64Array(4096), lowW = new Float64Array(4096), lowH = new Float64Array(4096);
  const kernel = new Float64Array(Math.floor(6 * SMOOTH_PX / RESAMPLE_PX) + 1);

  // Jelly: stretch along the velocity, area held, through a spring loose enough
  // to swing past round into a squash when the pointer stops.
  const JELLY_MAX = 0.42, JELLY_OMEGA = 20, JELLY_ZETA = 0.28, VEL_WINDOW_MS = 24, STILL_MS = 40;
  const JELLY_GAIN = 0.28, JELLY_CURVE = 0.75;   // stretch at 1000px/s, and how it grows past that
  const SHAKE_KICK = 9;
  // the motion between two frames of a running loop plays this long at most, however many
  // inputs split it: a stalled tab plays on from where it was instead of jumping ahead
  const MAX_STEP_S = 0.064;
  const SHAPE_OMEGA = 30, SHAPE_ZETA = 0.55;

  // The light-up: the mark's ink brightens and a bloom grows under it with speed, both on
  // an eased tone rather than the raw interval, so a stop drifts down instead of popping.
  // The bloom's box is its largest radius; it scales down from there.
  const TONE_MS = 120, FULL_SPEED = 1.4, GLOW_R0 = 14, GLOW_R1 = 20, GLOW_PEAK = 0.105;
  const GLOW_BOX = GLOW_R0 + GLOW_R1;

  // Droplets shed off the head only past SPRAY_MIN px/ms: SPRAY_RATE * x^2 / (x + SPRAY_EASE)
  // droplets a ms, x the speed past SPRAY_MIN, so a stroke just past it sheds a few and the
  // rate's slope nears SPRAY_RATE per px/ms further up; their orange share ramps from none at
  // SPRAY_MIN to ORANGE_MAX at SPRAY_FULL. The cap bounds the shed and refuses a new droplet
  // rather than evicting a live one: every particle plays its whole life, and a click's
  // splash is never refused.
  const SPRAY_MIN = 2.5, SPRAY_EASE = 1, SPRAY_FULL = 4, SPRAY_RATE = 0.08, SPRAY_CAP = 96, ORANGE_MAX = 0.5;

  let dpr = 1;
  let ink = [98, 186, 241], litInk = [190, 225, 250], keyline = [43, 45, 60], refuse = [233, 120, 130];
  let spray = [190, 225, 250], sprayHot = [240, 170, 120], rippleInk = [160, 215, 250];
  let sprite = null;
  // the light-up moves the ink by its tone times the widest channel gap to the lit ink, and
  // the bloom shows nothing under 0.02 of it: a tone below this rounds to the plain ink
  let toneRest = 0.003;

  const rgb = c => `rgb(${c[0]},${c[1]},${c[2]})`;
  const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * t));

  // one soft disc, stamped along the path: overlapping stamps shade the trail
  // like a tube, bright in the middle and soft at the edge, with no seams
  const buildSprite = (core, edge) => {
    const s = document.createElement('canvas');
    s.width = s.height = 64;
    const g = s.getContext('2d');
    const grad = g.createRadialGradient(32, 32, 0, 32, 32, 32);
    grad.addColorStop(0, `rgba(${core[0]},${core[1]},${core[2]},1)`);
    grad.addColorStop(0.35, `rgba(${core[0]},${core[1]},${core[2]},1)`);
    grad.addColorStop(0.75, `rgba(${edge[0]},${edge[1]},${edge[2]},0.85)`);
    grad.addColorStop(1, `rgba(${edge[0]},${edge[1]},${edge[2]},0)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, 64, 64);
    return s;
  };
  const readColours = () => {
    ink = resolveColour('--cursor-ink', ink);
    litInk = resolveColour('--cursor-lit', litInk);
    keyline = resolveColour('--cursor-keyline', keyline);
    refuse = resolveColour('--cursor-refuse', refuse);
    spray = resolveColour('--cursor-spray', spray);
    sprayHot = resolveColour('--cursor-spray-hot', sprayHot);
    rippleInk = resolveColour('--cursor-ripple', rippleInk);
    sprite = buildSprite(resolveColour('--cursor-trail-core', ink), resolveColour('--cursor-trail-edge', ink));
    toneRest = Math.min(0.02, 0.5 / Math.max(1, ...ink.map((c, i) => Math.abs(litInk[i] - c))));
  };
  const sizeCanvas = () => {
    dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = Math.round(innerWidth * dpr);
    canvas.height = Math.round(innerHeight * dpr);
    canvas.style.width = innerWidth + 'px';
    canvas.style.height = innerHeight + 'px';
    dirty = null;
    request();
  };
  // a window dragged to another monitor changes the ratio with no resize event
  const watchRatio = () => {
    matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => { sizeCanvas(); watchRatio(); }, { once: true });
  };

  // ---- state ----
  const usable = list => list.filter(sel => {
    try { host.matches(sel); return true; } catch { return false; }
  }).join(', ');
  const found = collectCursorSelectors(document.styleSheets);
  const hitPointer = usable([...found.pointer, SEMANTIC_POINTER]);
  const hitInert = usable(found.inert);
  const hitBlocked = usable([...found.blocked, SEMANTIC_BLOCKED]);

  /* A field is its resolved type, asked of an input only: `el.type` answers
     `text` for a missing or unknown attribute, which is what renders, while
     `<a>`, `<li>` and `<link>` reflect a `type="text"` attribute verbatim. */
  const TEXT_TYPES = ['text', 'search', 'email', 'password', 'url', 'tel', 'number'];
  const isField = el => (el instanceof HTMLInputElement ? TEXT_TYPES.includes(el.type)
    : el instanceof HTMLTextAreaElement) || el.matches('[contenteditable="true"]');

  const range = document.createRange();
  // the runs that are the element's own: its text nodes and those of its inline
  // descendants, never a block child's, whose glyphs retarget the event to it
  // anyway. A hovered page or body therefore costs its direct children, not
  // every run beneath them.
  let walkRoot = null;
  const walker = document.createTreeWalker(document, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: n => n.nodeType === Node.TEXT_NODE ? NodeFilter.FILTER_ACCEPT
      : n === walkRoot || getComputedStyle(n).display.startsWith('inline') ? NodeFilter.FILTER_SKIP : NodeFilter.FILTER_REJECT,
  });
  // the caret promises glyphs under the pointer, so hit-test text runs, never a
  // box: a block heading's box is mostly empty space. A run's rect is the glyph
  // box, shorter than its line box, so the leading above and below a run counts
  // as the run's, and so does the gap between two of the element's own runs
  // stacked (an inline child taller than its line opens one wider than the
  // leading), or a caret moving down through prose would swell back into the
  // ring at every line; a block child's margin is no such gap
  const overText = (el, x, y) => {
    if (isField(el)) return true;
    // CodeMirror-style editors hide glyphs behind pointer-events:none spans
    if (el.closest('[contenteditable="true"]')) return true;
    const line = parseFloat(getComputedStyle(el).lineHeight);
    let above = -Infinity, below = Infinity;
    walkRoot = el;
    walker.currentNode = el;
    for (let n = walker.nextNode(); n && el.contains(n); n = walker.nextNode()) {
      if (!n.textContent.trim()) continue;
      range.selectNodeContents(n);
      for (const r of range.getClientRects()) {
        if (x < r.left || x > r.right) continue;
        const leading = Number.isFinite(line) ? Math.max(0, (line - r.height) / 2) : 0;
        if (y >= r.top - leading && y <= r.bottom + leading) return true;
        if (r.bottom <= y) above = Math.max(above, r.bottom);
        else if (r.top >= y) below = Math.min(below, r.top);
      }
    }
    return Number.isFinite(line) && below - above <= line * 1.5;
  };

  function stateFor(el, x, y) {
    if (!el) return '';
    if (hitBlocked && el.closest(hitBlocked)) return 'blocked';
    const click = hitPointer && el.closest(hitPointer);
    if (click) {
      // a pointer switched back off outranks the clickable rule only from its own element or below
      const off = hitInert && el.closest(hitInert);
      if (!off || !(off === click || click.contains(off))) return 'hover';
    }
    return overText(el, x, y) ? 'text' : '';
  }

  const pointer = { x: 0, y: 0 };
  let state = '', caretSource = null, caretH = 20;
  let placed = false, proven = false, detached = false, bailed = false, gated = false;
  let pressedState = null, rootRetries = 0;
  let frame = 0, applyQueued = false, queuedTarget = null, dirty = null;
  // `last` is the instant the motion stands at, `frameAt` that of the last frame or wake. A
  // parked loop requests no frames and keeps the goal its springs were heading for: a hidden
  // mark parks with them still moving
  let last = 0, frameAt = 0, parked = true, parkedGoal = null;

  const inViewTransition = () => document.getAnimations().some(a =>
    a.effect && a.effect.pseudoElement === '::view-transition-new(root)');

  function apply(el) {
    const target = el || document.elementFromPoint(pointer.x, pointer.y);
    // mid view transition every hit test answers the root; hold the state and
    // re-derive, and a root answer outliving the retries with no swap is honest
    if (target === root) {
      if (rootRetries++ < 5 || inViewTransition()) {
        setTimeout(() => { if (!host.classList.contains('press')) queueApply(null); }, 80);
        return;
      }
    }
    rootRetries = 0;
    state = stateFor(target, pointer.x, pointer.y);
    host.classList.toggle('hover', state === 'hover');
    host.classList.toggle('blocked', state === 'blocked');
    host.classList.toggle('text', state === 'text');
    if (state === 'text' && target && target !== caretSource) {
      caretSource = target;
      // past 20px type a proportional caret reads as a rule drawn on the page
      const size = parseFloat(getComputedStyle(target).fontSize);
      if (size) caretH = Math.min(Math.round(size * 1.2), 24);
    } else if (state !== 'text') caretSource = null;
  }
  // overText reads client rects, which forces layout: once a frame, never once a move
  function queueApply(el) {
    queuedTarget = el;
    applyQueued = true;
    request();
  }

  // ---- motion ----
  const samples = [];   // { x, y, t, pause, net, w, h, peak, fill?, start? }, oldest first
  let behind = null;    // the path's point before the oldest sample kept: the last one dropped at another position, or the end of its fill
  const ripples = [];   // { x, y, t0 }
  const drops = [];     // { x, y, vx, vy, born, life, r, hot }
  const spring = () => ({ x: 0, v: 0 });
  const hw = spring(), hh = spring(), fill = spring(), dot = spring(), slash = spring(), red = spring();
  const jp = spring(), jq = spring();   // stretch as a traceless tensor: amount and axis in one vector
  hw.x = hh.x = SIZE.idle / 2; dot.x = 1;
  let tone = 0;                          // the eased speed the light-up answers
  // the mark's and trail's own fade, in and out with `visible`: droplets and ripples keep
  // their own alpha, so a pointer leaving the window never cuts one short
  let shown = 0;
  let headX = 1, headY = 0;              // the last direction the head travelled
  let left = false;                      // the pointer left the window: the next move starts a new stroke
  let lastEvent = null, sprayDebt = 0, hotDebt = 0, lastSpeed = 0;

  // k: a springMatrix over the step
  const step = (s, target, k) => {
    const off = s.x - target;
    s.x = target + k.xx * off + k.xv * s.v;
    s.v = k.vx * off + k.vv * s.v;
  };
  const rest = (s, target, eps) => {
    if (Math.abs(s.x - target) < eps && Math.abs(s.v) < eps * 20) { s.x = target; s.v = 0; return true; }
    return false;
  };

  // the pointer's position at time t, linear between samples
  const posAt = t => {
    const n = samples.length;
    if (t >= samples[n - 1].t) return [samples[n - 1].x, samples[n - 1].y];
    if (t <= samples[0].t) return [samples[0].x, samples[0].y];
    let i = n - 1;
    while (i > 0 && samples[i - 1].t > t) i--;
    const a = samples[i - 1], b = samples[i];
    const u = b.t > a.t ? (t - a.t) / (b.t - a.t) : 1;
    return [a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * u];
  };

  // a pointer quiet for STILL_MS counts as stopped, and stays stopped until its next move
  const pointerStill = now => samples.length < 2 || now - samples[samples.length - 1].t > STILL_MS;

  // the pointer's velocity in px/ms over the last VEL_WINDOW_MS of samples, reaching back no
  // earlier than `from`. The window ends at the newest sample, never at `now`, and spans
  // samples rather than events, so the read is the same at 125Hz and 1000Hz, however the
  // moves were batched, and a fast mouse's whole-pixel steps average out over it
  function velocity(from) {
    const head = samples[samples.length - 1];
    const span = Math.min(VEL_WINDOW_MS, head.t - Math.max(from, samples[0].t));
    if (span <= 1) return [0, 0];
    const [ox, oy] = posAt(head.t - span);
    return [(head.x - ox) / span, (head.y - oy) / span];
  }

  function jellyTarget(now) {
    if (pointerStill(now)) return [0, 0, 0];
    const [mx, my] = velocity(-Infinity);
    const vx = mx * 1000, vy = my * 1000;
    const v = Math.hypot(vx, vy);
    const amount = Math.min(JELLY_MAX, JELLY_GAIN * Math.pow(v / 1000, JELLY_CURVE));
    const theta = Math.atan2(vy, vx);
    return [amount * Math.cos(2 * theta), amount * Math.sin(2 * theta), v / 1000];
  }

  function targets() {
    if (state === 'text') return { w: 1, h: caretH / 2, f: 1, d: 0, sl: 0, r: 0 };
    const pressed = host.classList.contains('press');
    let size = SIZE.idle, f = 0, d = 1, sl = 0, r = 0;
    // the hover fill sits over the text under the pointer: 12% of --cursor-ink, 26% pressed
    if (state === 'hover') { size = pressed ? SIZE.press : SIZE.hover; f = pressed ? 0.26 : 0.12; d = 0; }
    else if (state === 'blocked') { size = SIZE.blocked; d = 0; sl = 1; r = 1; }
    else if (pressed) size = SIZE.pressIdle;
    return { w: size / 2, h: size / 2, f, d, sl, r };
  }

  // what a sample records of the mark: its half sizes under the jelly's squeeze across its
  // stretch (a ring at speed is narrower across than its size, so a fast stroke draws a
  // narrower tube), less half the trim, floored at the caret's own half width so its line
  // moving down stays that line; and the trail's peak alpha there. The squeeze is the narrow
  // axis, so the recorded width does not depend on the direction the head had
  const record = () => {
    const squeeze = 1 / jellyStretch(shapeRoundness());
    return {
      w: Math.max(1, hw.x * squeeze - TRIM_PX / 2), h: Math.max(1, hh.x * squeeze - TRIM_PX / 2),
      peak: state === 'text' ? TEXT_PEAK : TRAIL_PEAK,
    };
  };

  // samples[i]'s direction of travel, read over GAP_TANGENT_PX of path on the side `dir`
  // points to (-1 behind it, +1 ahead), and the sample that reach ends at; null with none,
  // and null when the reach would cross a pause: a hand that stopped may leave another way
  function travelAt(i, dir) {
    const p = samples[i];
    for (let j = i + dir; j >= 0 && j < samples.length; j += dir) {
      if (samples[dir < 0 ? j + 1 : j].pause > 0) return null;
      const dx = (samples[j].x - p.x) * dir, dy = (samples[j].y - p.y) * dir, d = Math.hypot(dx, dy);
      if (d >= GAP_TANGENT_PX) return { x: dx / d, y: dy / d, at: j };
    }
    return null;
  }

  // the first chord from samples[i] toward `dir` that moves at all; 0 with none
  function chordAt(i, dir) {
    for (let j = i; j + dir >= 0 && j + dir < samples.length; j += dir) {
      const c = Math.hypot(samples[j + dir].x - samples[j].x, samples[j + dir].y - samples[j].y);
      if (c > 0) return c;
    }
    return 0;
  }

  // points along a cubic Hermite from a to b, leaving a along `from` and arriving at b along
  // `to` (a missing one is the other mirrored across the chord, close to the circular arc
  // through both for a gentle turn), about `near` apart; each is timed and sized at its
  // share of the way from a to b
  function hermite(a, b, chord, from, to, near) {
    const ex = (b.x - a.x) / chord, ey = (b.y - a.y) / chord;
    const mirror = d => { const along = 2 * (d.x * ex + d.y * ey); return { x: along * ex - d.x, y: along * ey - d.y }; };
    const ta = from || mirror(to), tb = to || mirror(from);
    const m = Math.min(GAP_MAX_POINTS, Math.ceil(chord / Math.max(near, GAP_SPACING_PX)) - 1);
    const points = [];
    for (let k = 1; k <= m; k++) {
      const u = k / (m + 1), u2 = u * u, u3 = u2 * u;
      const h00 = 2 * u3 - 3 * u2 + 1, h10 = (u3 - 2 * u2 + u) * chord, h01 = 3 * u2 - 2 * u3, h11 = (u3 - u2) * chord;
      points.push({
        x: h00 * a.x + h10 * ta.x + h01 * b.x + h11 * tb.x, y: h00 * a.y + h10 * ta.y + h01 * b.y + h11 * tb.y,
        t: a.t + (b.t - a.t) * u, pause: 0,
        w: a.w + (b.w - a.w) * u, h: a.h + (b.h - a.h) * u, peak: a.peak + (b.peak - a.peak) * u,
      });
    }
    return points;
  }

  // The points filling the span into samples[i] when it is a stretch the browser lost
  // samples over (GAP_MIN_PX, and GAP_RATIO times the chords on both sides): drawn as one
  // chord, the spline would put a whole turn at each end of it. Times and sizes run evenly
  // across the fill as they would across the bare span, so only positions change. A span
  // ending in a pause passes the same test only once a chord ahead of it is known: a start
  // from rest has a first chord short beside the next and fails it, while a stall that lost
  // samples mid-travel is a long chord between short ones. The result is kept on the
  // sample once the direction ahead is read off samples that can no longer change; from
  // then on the fill never reshapes as the window drops the samples behind it.
  function fillAt(i) {
    const b = samples[i];
    if (b.fill !== undefined) return b.fill;
    const a = samples[i - 1], newest = samples.length - 1;
    const chord = Math.hypot(b.x - a.x, b.y - a.y);
    if (chord < GAP_MIN_PX) {
      if (i < newest) b.fill = null;
      return null;
    }
    const ahead = chordAt(i, 1);
    if (b.pause > 0 && !ahead) return null;
    const near = Math.max(chordAt(i - 1, -1), ahead);
    const from = travelAt(i - 1, -1), to = travelAt(i, 1);
    const lost = near > 0 && chord > GAP_RATIO * near && (from || to);
    const points = lost ? hermite(a, b, chord, from, to, near) : null;
    if (to && to.at < newest) b.fill = points;
    return points;
  }

  // the samples with their fills, then the pointer now. A filled span is taken for travel
  // whose samples were lost rather than a stop, so the pause at its far end ends no run of
  // travel: the tube runs through the fill unpinched
  function pathPoints(now) {
    const pts = [];
    for (let i = 0; i < samples.length; i++) {
      const fill = i ? fillAt(i) : null;
      if (fill) pts.push(...fill);
      pts.push(fill && samples[i].pause > 0 ? { ...samples[i], pause: 0 } : samples[i]);
    }
    pts.push({ x: pointer.x, y: pointer.y, t: now, pause: 0, ...record() });
    return pts;
  }

  // the grid's instants from t0 to now: inside each span, evenly in time, as many as its
  // chord asks for at ARC_PX and its time at STEP_MS, carried across spans so a span asking
  // for less than one point shares one with the next
  function layGrid(pts, t0, now) {
    const wanted = (a, b) => {
      const lo = Math.max(a.t, t0), hi = Math.min(b.t, now);
      if (hi <= lo) return 0;
      const share = (hi - lo) / (b.t - a.t);
      return Math.max(Math.hypot(b.x - a.x, b.y - a.y) * share / ARC_PX, (hi - lo) / STEP_MS);
    };
    let total = 0;
    for (let i = 1; i < pts.length; i++) total += wanted(pts[i - 1], pts[i]);
    const scale = Math.min(1, (MAX_STEPS - 1) / total);
    let count = 0, laid = 0;
    gridT[0] = t0;
    for (let i = 1; i < pts.length; i++) {
      const a = pts[i - 1], b = pts[i];
      const want = wanted(a, b) * scale;
      if (want <= 0) continue;
      const lo = Math.max(a.t, t0), hi = Math.min(b.t, now);
      for (let k = Math.floor(laid) + 1; k < laid + want; k++) gridT[++count] = lo + (hi - lo) * (k - laid) / want;
      laid += want;
    }
    gridT[++count] = now;
    return count;
  }

  /* The centre line. The path is a uniform cubic B-spline over the samples: its bend is
     continuous along the whole path, which passes a third of the way from each sample
     toward the midpoint of its two neighbours, so a fast stroke's sparse samples draw one
     curve rather than straight runs joined at bends. Each span runs over the time between
     its two samples, so ages stay true to when the pointer was there, and over a run of
     samples at one position the path holds still for as long as the run lasts. A stretch
     where the browser lost samples is filled first (fillAt), so its spacing changes
     gradually and the spline has no turn to pile up at either end. The grid lays a point at
     least every ARC_PX of each span's chord and every STEP_MS of its time (layGrid), so a
     fast stretch draws in short chords and a jump that took a fraction of a millisecond
     still gets its points. The positions and the recorded sizes then pass through one
     low-pass along the path (smoothAlongPath), and the tube's widths are laid on the result
     (layWidths). A new sample reshapes the last two spans and, through the low-pass, the
     line and its width up to 3 x SMOOTH_PX of path behind them, and an unsettled fill
     (fillAt) across its whole gap; the tail's last 3 x SMOOTH_PX moves as the window
     drains it. */
  function buildGrid(now, win) {
    const pts = pathPoints(now);
    const n = pts.length;
    const t0 = Math.max(pts[0].t, now - win);
    if (n < 2 || now <= t0) return 0;
    // the still time the pointer has spent since each sample, summed from the head back
    pauseAfter.length = n;
    pauseAfter[n - 1] = 0;
    for (let i = n - 2; i >= 0; i--) pauseAfter[i] = pauseAfter[i + 1] + pts[i + 1].pause;
    knots.length = 0;
    for (let i = 0; i < n; i++) if (!i || pts[i].x !== pts[i - 1].x || pts[i].y !== pts[i - 1].y) knots.push(i);
    const lastKnot = knots.length - 1;
    // span k's four control points: the first point of each run at one position. Before the
    // first stands `behind`, so a sample leaving the window never reshapes what is still
    // drawn; with none dropped yet, the mirror of the second, which starts the curve on the
    // first sample. Past the head stands the mirror of the one before it, which ends the
    // curve on the head along the last chord with no bend
    const loadSpan = k => {
      for (let c = 0; c < 4; c++) {
        const i = k - 1 + c;
        if (i < 0 || i > lastKnot) continue;
        const p = pts[knots[i]];
        spanX[c] = p.x; spanY[c] = p.y;
      }
      if (k === 0) {
        spanX[0] = behind ? behind.x : 2 * spanX[1] - spanX[2];
        spanY[0] = behind ? behind.y : 2 * spanY[1] - spanY[2];
      }
      if (k + 1 === lastKnot) { spanX[3] = 2 * spanX[2] - spanX[1]; spanY[3] = 2 * spanY[2] - spanY[1]; }
    };
    const count = layGrid(pts, t0, now);
    let k = 0, loaded = -1, b = 0;
    for (let j = 0; j <= count; j++) {
      const t = gridT[j];
      while (k < lastKnot && t >= pts[knots[k + 1]].t) k++;
      let x, y;
      if (k === lastKnot) {
        x = pts[knots[k]].x; y = pts[knots[k]].y;
      } else {
        if (k !== loaded) { loaded = k; loadSpan(k); }
        const leave = pts[knots[k + 1] - 1].t, arrive = pts[knots[k + 1]].t;
        const u = t > leave ? (t - leave) / (arrive - leave) : 0;
        const v = 1 - u, u2 = u * u, u3 = u2 * u;
        const b0 = v * v * v / 6, b1 = (3 * u3 - 6 * u2 + 4) / 6, b2 = (-3 * u3 + 3 * u2 + 3 * u + 1) / 6, b3 = u3 / 6;
        x = b0 * spanX[0] + b1 * spanX[1] + b2 * spanX[2] + b3 * spanX[3];
        y = b0 * spanY[0] + b1 * spanY[1] + b2 * spanY[2] + b3 * spanY[3];
      }
      gridX[j] = x; gridY[j] = y; gridAge[j] = now - t;
      // the recorded sizes ride the samples that bracket this instant
      while (b < n - 2 && t > pts[b + 1].t) b++;
      const s0 = pts[b], s1 = pts[b + 1];
      const f = s1.t > s0.t ? Math.min(1, Math.max(0, (t - s0.t) / (s1.t - s0.t))) : 1;
      gridW[j] = s0.w + (s1.w - s0.w) * f;
      gridH[j] = s0.h + (s1.h - s0.h) * f;
      gridPeak[j] = s0.peak + (s1.peak - s0.peak) * f;
      gridPause[j] = pauseAfter[b + 1] + Math.min(s1.pause, s1.t - t);
    }
    smoothAlongPath(count);
    // each grid point's direction, read over a chord of at least TANGENT_PX around it, so a
    // slow pointer's sub-pixel jitter cannot swing the width the recorded sizes give it
    let tx = headX, ty = headY;
    for (let j = 0; j <= count; j++) {
      let a = j, c = j, dx = 0, dy = 0, len = 0;
      while (len < TANGENT_PX && (a > 0 || c < count) && c - a < 16) {
        a = Math.max(0, a - 1); c = Math.min(count, c + 1);
        dx = gridX[c] - gridX[a]; dy = gridY[c] - gridY[a]; len = Math.hypot(dx, dy);
      }
      if (len >= TANGENT_PX / 2) { tx = dx / len; ty = dy / len; }
      gridTx[j] = tx; gridTy[j] = ty;
    }
    readHeadDirection(count);
    gridDist[count] = 0;
    for (let j = count - 1; j >= 0; j--) gridDist[j] = gridDist[j + 1] + Math.hypot(gridX[j + 1] - gridX[j], gridY[j + 1] - gridY[j]);
    // a run of travel ends where the pointer stopped (a pause was recorded): the path
    // before that stop is a drained tube of its own, and the run's tail is the path since
    let stop = count;
    for (let j = count; j >= 0; j--) {
      while (stop >= 0 && gridPause[stop] <= gridPause[j] + 1e-3) stop--;
      gridEnd[j] = stop >= 0 ? gridDist[stop] : gridDist[0];
    }
    layWidths(count);
    return count;
  }

  // the head's direction, read over HEAD_TANGENT_PX of path behind it: the hole's edge and
  // the window's floor are the mark's extent along it, so a slow hand's per-sample wobble,
  // which the low-pass leaves at the head, must not swing it. Past half the reach, it stops
  // where the path turned back on itself (a point whose own direction opposes the chord
  // read so far), so once the new leg is half the reach long a hairpin's old leg no longer
  // pulls the head's direction off it; the first half is never cut, since a slow hand's
  // wobble there turns single points sideways
  function readHeadDirection(count) {
    let a = count, dx = 0, dy = 0, len = 0;
    const turnsBack = j => len >= HEAD_TANGENT_PX / 2 && gridTx[j] * dx + gridTy[j] * dy < 0;
    while (len < HEAD_TANGENT_PX && a > 0 && !turnsBack(a - 1)) {
      a--;
      dx = gridX[count] - gridX[a]; dy = gridY[count] - gridY[a]; len = Math.hypot(dx, dy);
    }
    if (len >= TANGENT_PX / 2) { gridTx[count] = dx / len; gridTy[count] = dy / len; }
    headX = gridTx[count]; headY = gridTy[count];
  }

  // the tube's half width at each grid point: the recorded half sizes across the path, and
  // from the head back to where the line leaves the hole the start, so the visible tube
  // leaves the ring at the ring's width less the trim even where the jelly lags a turn or
  // the line doubles back inside the ring at a hairpin. The start is the mark's extent
  // across the tube where it leaves the hole, less half the trim (startWidth), read on the
  // first frame that draws the newest sample and kept on it: a tube draining after a stop
  // or a stall holds the width the hand left it with while the jelly relaxes or the ring
  // grows over a target. Past that point the width eases to the recorded one over
  // START_EASE_PX along a smoothstep, so it has no kink where the two meet, and the narrow
  // axis behind keeps a turn from swelling the tube
  function layWidths(count) {
    const edge = holeEdge(), out = holeExit(count);
    const leaves = Math.max(edge, gridDist[out]);
    const newest = samples[samples.length - 1];
    if (newest.start === undefined) newest.start = startWidth(count, edge, out);
    const start = newest.start;
    for (let j = 0; j <= count; j++) {
      const recorded = Math.hypot(gridW[j] * gridTy[j], gridH[j] * gridTx[j]);
      const u = Math.min(1, Math.max(0, (gridDist[j] - leaves) / START_EASE_PX));
      gridAcross[j] = recorded + (start - recorded) * (1 - u * u * (3 - 2 * u));
    }
  }

  // where the line, back from the head, first leaves the hole (outsideHole): the index of the
  // first grid point outside it, so the start reaches every point that could show past the
  // ring; the line's far end when it never leaves
  function holeExit(count) {
    for (let j = count - 1; j >= 0; j--) if (outsideHole(gridX[j], gridY[j])) return j;
    return 0;
  }

  // the start: the mark's half extent, less half the trim, across the direction the tube
  // takes where it leaves the hole, at grid point `out`. A line leaving at the hole's edge
  // runs on from the head, and its own direction there bends with whatever turn the low-pass
  // rounds just outside the hole, so the start reads across the head's direction; a line
  // leaving past the edge has turned inside the hole, and the start reads across the line
  // where it leaves, once that turn clears the hole. The line's read takes over along a
  // smoothstep as the exit lies up to START_RAMP_PX past the edge and as the turn clears the
  // hole by up to START_RAMP_PX. The exit is a grid point, so a drifting exit still moves the
  // width a grid step at a time, and a turn dipping back inside the hole moves the exit to
  // the leg beyond it at once, which can switch the start from the head's read to that leg's
  function startWidth(count, edge, out) {
    const past = Math.min(1, Math.max(0, (gridDist[out] - edge) / START_RAMP_PX));
    const head = trimmedAcross(-headY, headX);
    if (past === 0) return head;
    const u = past * Math.min(1, clearance(out) / START_RAMP_PX);
    return head + (startAcross(count, out) - head) * u * u * (3 - 2 * u);
  }

  // how far the line clears the hole past its exit at grid point `out`: the farthest it lies
  // outside the hole while it stays outside, over the START_EASE_PX of path the start eases
  // across. The scan stops at the first point START_RAMP_PX or more out and returns its
  // distance, so the result can pass START_RAMP_PX; the caller clamps it
  function clearance(out) {
    let most = 0;
    for (let k = out; k >= 0 && gridDist[k] - gridDist[out] <= START_EASE_PX && most < START_RAMP_PX; k--) {
      const gap = outlineDistance(gridX[k] - pointer.x, gridY[k] - pointer.y) - HOLE_PX;
      if (gap <= 0) break;
      most = Math.max(most, gap);
    }
    return most;
  }

  // the start across the line where it leaves the hole, its direction read over
  // EXIT_TANGENT_PX of path each side of grid point `out`: at a hairpin the line curves
  // within a few px of that point, and a longer read takes in the turn
  function startAcross(count, out) {
    let a = out, c = Math.min(count, out + 1);
    while (a > 0 && gridDist[a] - gridDist[out] < EXIT_TANGENT_PX) a--;
    while (c < count && gridDist[out] - gridDist[c] < EXIT_TANGENT_PX) c++;
    const dx = gridX[c] - gridX[a], dy = gridY[c] - gridY[a], len = Math.hypot(dx, dy);
    return len > 1e-6 ? trimmedAcross(-dy / len, dx / len) : trimmedAcross(-headY, headX);
  }

  // the mark's half extent along a direction less half the trim, floored at a pixel so the
  // caret's line moving down stays a line
  const trimmedAcross = (nx, ny) => Math.max(1, support(nx, ny) - TRIM_PX / 2);

  const fit = (a, n) => a.length >= n ? a : new Float64Array(Math.max(n, 2 * a.length));

  // A Gaussian over the centre line and the half sizes by arc length, sigma SMOOTH_PX,
  // narrowed to a third of the path left to the head and to the tail, so both ends stay
  // where they are: the head stays on the pointer at the mark's own trimmed size while a
  // slow stroke's per-sample wobble, which the spline passes a good share of, averages out,
  // and a burst after a stall widens or narrows the tube over the arc the line bends over,
  // not over the few px between its samples. The filter runs on an even resample of the
  // path, read back at each grid point's own arc length through a cubic: a linear read-back
  // would bend the line only at the resample points.
  function smoothAlongPath(count) {
    gridArc[0] = 0;
    for (let j = 1; j <= count; j++) gridArc[j] = gridArc[j - 1] + Math.hypot(gridX[j] - gridX[j - 1], gridY[j] - gridY[j - 1]);
    const n = Math.ceil(gridArc[count] / RESAMPLE_PX) + 1;
    if (n < 3) return;
    const h = gridArc[count] / (n - 1);
    evenX = fit(evenX, n); evenY = fit(evenY, n); evenW = fit(evenW, n); evenH = fit(evenH, n);
    lowX = fit(lowX, n); lowY = fit(lowY, n); lowW = fit(lowW, n); lowH = fit(lowH, n);
    resampleEvenly(count, n, h);
    lowPass(n, h);
    readBack(count, n, h);
  }

  function resampleEvenly(count, n, h) {
    let seg = 0;
    for (let k = 0; k < n; k++) {
      const at = k === n - 1 ? gridArc[count] : k * h;
      while (seg < count - 1 && gridArc[seg + 1] < at) seg++;
      const len = gridArc[seg + 1] - gridArc[seg], u = len > 0 ? Math.min(1, (at - gridArc[seg]) / len) : 1;
      evenX[k] = gridX[seg] + (gridX[seg + 1] - gridX[seg]) * u;
      evenY[k] = gridY[seg] + (gridY[seg + 1] - gridY[seg]) * u;
      evenW[k] = gridW[seg] + (gridW[seg + 1] - gridW[seg]) * u;
      evenH[k] = gridH[seg] + (gridH[seg + 1] - gridH[seg]) * u;
    }
  }

  function lowPass(n, h) {
    const reach = Math.floor(3 * SMOOTH_PX / h);
    for (let q = 0; q <= reach; q++) kernel[q] = Math.exp(-0.5 * (q * h / SMOOTH_PX) ** 2);
    for (let k = 0; k < n; k++) {
      const sigma = Math.min(SMOOTH_PX, (n - 1 - k) * h / 3, k * h / 3);
      if (sigma < h / 2) { lowX[k] = evenX[k]; lowY[k] = evenY[k]; lowW[k] = evenW[k]; lowH[k] = evenH[k]; continue; }
      const full = sigma === SMOOTH_PX;
      const taps = full ? reach : Math.floor(3 * sigma / h), c = -0.5 * (h / sigma) ** 2;
      let sx = evenX[k], sy = evenY[k], sw = evenW[k], sh = evenH[k], total = 1;
      for (let q = 1; q <= taps; q++) {
        const weight = full ? kernel[q] : Math.exp(c * q * q);
        sx += weight * (evenX[k - q] + evenX[k + q]); sy += weight * (evenY[k - q] + evenY[k + q]);
        sw += weight * (evenW[k - q] + evenW[k + q]); sh += weight * (evenH[k - q] + evenH[k + q]);
        total += 2 * weight;
      }
      lowX[k] = sx / total; lowY[k] = sy / total; lowW[k] = sw / total; lowH[k] = sh / total;
    }
  }

  // Catmull-Rom through the filtered resample, at each grid point's arc length
  function readBack(count, n, h) {
    for (let j = 0; j <= count; j++) {
      const at = Math.min(n - 1, gridArc[j] / h), k1 = Math.min(n - 2, Math.floor(at)), g = at - k1;
      const k0 = Math.max(0, k1 - 1), k2 = k1 + 1, k3 = Math.min(n - 1, k1 + 2);
      const g2 = g * g, g3 = g2 * g;
      const c0 = -0.5 * g3 + g2 - 0.5 * g, c1 = 1.5 * g3 - 2.5 * g2 + 1, c2 = -1.5 * g3 + 2 * g2 + 0.5 * g, c3 = 0.5 * g3 - 0.5 * g2;
      gridX[j] = c0 * lowX[k0] + c1 * lowX[k1] + c2 * lowX[k2] + c3 * lowX[k3];
      gridY[j] = c0 * lowY[k0] + c1 * lowY[k1] + c2 * lowY[k2] + c3 * lowY[k3];
      gridW[j] = c0 * lowW[k0] + c1 * lowW[k1] + c2 * lowW[k2] + c3 * lowW[k3];
      gridH[j] = c0 * lowH[k0] + c1 * lowH[k1] + c2 * lowH[k2] + c3 * lowH[k3];
    }
  }

  // the path where the trail's front edge sits, `edge` px behind the head: its age, and
  // that age less the time the pointer has stood still since it was laid (past DRAIN_MS,
  // the pauses recorded on the samples plus the current one). The taper's clock starts
  // at the second, so travel time alone never dims the trail at the hole however slowly
  // it goes, a stop drains the tube instead of holding it until the window empties, and a
  // move after a pause keeps the old path at the level it drained to rather than
  // restarting its clock
  function edgeAge(count, edge, still) {
    let j = count;
    while (j > 0 && gridDist[j] < edge) j--;
    const span = gridDist[j] - gridDist[j + 1];
    const u = span > 0 ? (edge - gridDist[j + 1]) / span : 1;
    const age = gridAge[j + 1] + (gridAge[j] - gridAge[j + 1]) * Math.min(1, u);
    return { age, age0: Math.max(0, age - gridPause[j] - Math.max(0, still - DRAIN_MS)) };
  }

  const grow = (b, x, y, r) => {
    if (!b) return [x - r, y - r, x + r, y + r];
    b[0] = Math.min(b[0], x - r); b[1] = Math.min(b[1], y - r);
    b[2] = Math.max(b[2], x + r); b[3] = Math.max(b[3], y + r);
    return b;
  };

  // the jelly's stretch along its axis, `a`; it squeezes to 1/a across it
  function jellyStretch(roundness) {
    return 1 + Math.min(JELLY_MAX, Math.hypot(jp.x, jq.x)) * roundness;
  }
  // the jelly's axis, as an angle
  const jellyAngle = () => Math.atan2(jq.x, jp.x) / 2;
  // the jelly as a symmetric tensor: stretch `a` along the axis, 1/a across it
  function jellyTensor(roundness) {
    const phi = jellyAngle();
    const a = jellyStretch(roundness), b = 1 / a;
    const c = Math.cos(phi), s = Math.sin(phi);
    return [a * c * c + b * s * s, (a - b) * c * s, a * s * s + b * c * c];
  }
  const shapeRoundness = () => Math.min(hw.x, hh.x) / Math.max(hw.x, hh.x, 1e-3);
  // the jelly's matrix, centred on the pointer, in device pixels
  function jellyMatrix(x, y, T) {
    return [T[0] * dpr, T[1] * dpr, T[1] * dpr, T[2] * dpr, x * dpr, y * dpr];
  }
  const shape = (w, h, inset) => {
    const iw = Math.max(0, w - inset), ih = Math.max(0, h - inset);
    ctx.beginPath();
    ctx.roundRect(-iw, -ih, 2 * iw, 2 * ih, Math.min(iw, ih));
  };
  // the live mark's half extent along a direction: the support function of the drawn
  // rounded rect (corner radius = its smaller half size) under its jelly tensor
  function support(vx, vy) {
    const T = jellyTensor(shapeRoundness());
    const mx = T[0] * vx + T[1] * vy, my = T[1] * vx + T[2] * vy;
    const rho = Math.min(hw.x, hh.x);
    return (hw.x - rho) * Math.abs(mx) + (hh.x - rho) * Math.abs(my) + rho * Math.hypot(mx, my);
  }
  // the hole's reach from the pointer along a direction: the support of the mark's outline
  // grown by HOLE_PX, which equals the outline's radius on its axes and overstates it
  // between them
  const holeReach = (vx, vy) => support(vx, vy) + HOLE_PX;
  // how far behind the head the trail's front edge sits
  const holeEdge = () => holeReach(headX, headY);
  // whether a point lies outside the hole: farther than HOLE_PX from the mark's drawn
  // outline. Within the jelly's narrow half size plus HOLE_PX of the pointer it is inside,
  // and past the hole's reach toward it outside; between the two the distance is solved
  function outsideHole(x, y) {
    const dx = x - pointer.x, dy = y - pointer.y, d = Math.hypot(dx, dy);
    if (d <= Math.min(hw.x, hh.x) / jellyStretch(shapeRoundness()) + HOLE_PX) return false;
    if (d > holeReach(dx / d, dy / d)) return true;
    return outlineDistance(dx, dy) > HOLE_PX;
  }

  // the distance from a point, given from the pointer, to the mark's drawn outline (zero
  // inside). The outline is the jelly's image of a rounded rect whose corner radius is its
  // smaller half size: an ellipse swept along a segment, none when the mark is round, so the
  // distance is the least distance to that ellipse over the sweep, convex along the segment
  function outlineDistance(dx, dy) {
    // the drawn half sizes: a spring overshooting toward the caret can pass zero
    const w = Math.max(1e-3, hw.x), h = Math.max(1e-3, hh.x);
    const rho = Math.min(w, h), a = jellyStretch(shapeRoundness());
    const phi = jellyAngle(), c = Math.cos(phi), s = Math.sin(phi);
    const lx = w - rho, ly = h - rho;
    // the point and the sweep's half length along the jelly's axis and across it
    const pu = dx * c + dy * s, pv = dy * c - dx * s;
    const su = a * (lx * c + ly * s), sv = (ly * c - lx * s) / a;
    const gap = t => ellipseGap(rho * a, rho / a, Math.abs(pu - t * su), Math.abs(pv - t * sv));
    return su || sv ? convexMin(gap) : gap(0);
  }

  // the distance from a point to a filled ellipse with semi-axes e0, e1 > 0, the point
  // given in the ellipse's first quadrant: zero inside, else the root of a function of one
  // variable that falls monotonically, bisected (Eberly, "Distance from a Point to an
  // Ellipse, an Ellipsoid, or a Hyperellipsoid"). The bisection's bracket holds only with
  // the major axis first, so swapped axes are mirrored across the diagonal, point included
  function ellipseGap(e0, e1, y0, y1) {
    if (e0 < e1) return ellipseGap(e1, e0, y1, y0);
    const z0 = y0 / e0, z1 = y1 / e1;
    if (z0 * z0 + z1 * z1 <= 1) return 0;
    const r = (e0 / e1) ** 2, n = r * z0;
    let lo = z1 - 1, hi = Math.hypot(n, z1) - 1, s = lo;
    for (let i = 0; i < 64; i++) {
      s = (lo + hi) / 2;
      if (s === lo || s === hi) break;
      const q0 = n / (s + r), q1 = z1 / (s + 1);
      if (q0 * q0 + q1 * q1 > 1) lo = s; else hi = s;
    }
    return Math.hypot(r * y0 / (s + r) - y0, y1 / (s + 1) - y1);
  }

  // the least value of a convex function over [-1, 1], by golden-section search to 1e-6
  function convexMin(f) {
    const g = (Math.sqrt(5) - 1) / 2;
    let lo = -1, hi = 1, p = hi - g * (hi - lo), q = lo + g * (hi - lo), fp = f(p), fq = f(q);
    while (hi - lo > 1e-6) {
      if (fp <= fq) { hi = q; q = p; fq = fp; p = hi - g * (hi - lo); fp = f(p); }
      else { lo = p; p = q; fp = fq; q = lo + g * (hi - lo); fq = f(q); }
    }
    return Math.min(fp, fq);
  }

  // the net clock: sample time less the stillness past DRAIN_MS. Its head runs DRAIN_MS past
  // the newest sample and stops there, so a longer stop neither stretches the window nor drops
  // the samples the tube is still draining from, and the first move after it finds the window
  // as it was
  const netHead = now => {
    const newest = samples[samples.length - 1];
    return newest.net + Math.min(now - newest.t, DRAIN_MS);
  };
  // a pointer still for DRAIN_MS plus the window has drained the tube: its stroke is over
  const strokeOver = (t, win) => t - samples[samples.length - 1].t > DRAIN_MS + win;

  // how far back the window reaches now, on the net clock: TAIL_MS, or the time it takes the
  // path to reach the mark's larger half size past the hole when that is longer, CRAWL_MS at
  // most. The reach ends inside the segment that completes it, so a flick that settles
  // into a crawl keeps only that much of its path past TAIL_MS, never its whole length.
  // The span leaves the pauses out, so the sample that ends a pause does not lengthen the
  // window by it and swell the tube that was draining
  function windowMs(now) {
    const floor = holeEdge() + Math.max(hw.x, hh.x);
    let d = 0, net = samples[samples.length - 1].net;
    for (let i = samples.length - 1; i > 0 && d < floor; i--) {
      const seg = Math.hypot(samples[i].x - samples[i - 1].x, samples[i].y - samples[i - 1].y);
      const need = floor - d;
      net = seg > need ? samples[i].net - (samples[i].net - samples[i - 1].net) * need / seg : samples[i - 1].net;
      d += seg;
    }
    return Math.min(CRAWL_MS, Math.max(TAIL_MS, netHead(now) - net));
  }

  // the tube: soft discs stamped along the grid, spaced by distance so the density,
  // and with it the alpha, holds at every speed, each stamp's alpha solved from the
  // overlap. A stamp's radius is the grid's half width there (layWidths: the start where
  // the line leaves the hole, the ring's width less the trim as the hand last moved,
  // easing to the recorded half sizes read across the path; the caret's trimmed half
  // height moving sideways, its own half width moving down), so over text the trail is a
  // line downward and a caret-tall tube sideways, and a turn keeps the past its width. The
  // stamping runs up to the pointer; the hole cut afterwards removes the mark's own share.
  // Two caps keep the tail from showing more than the path it has. A stamp is never
  // brighter than the front edge, so the part of the path that stood hidden in the hole
  // at a stop cannot come out brighter than the tube that drained beside it. And a
  // stamp's radius is held to the path left behind it in its run of travel (gridEnd) and,
  // inside the hole, to the hypotenuse of its depth in the hole and the run's path past
  // the hole, so the tail is a half disc of that path's radius at the hole's edge,
  // tapering to a point at the run's old end, and a run begun after a stop grows from
  // nothing like a stroke from rest: a full-radius disc a pixel past the hole's edge is
  // a crescent hugging the ring. It reports whether any stamp could set a pixel.
  function drawTrail(count, edge, win, front, box) {
    let carry = 0, drawn = false;
    const life = Math.max(1, win - front.age0);
    const frontP = Math.max(0, 1 - (front.age - front.age0) / life);
    for (let j = 0; j < count; j++) {
      const ax = gridX[j], ay = gridY[j], bx = gridX[j + 1], by = gridY[j + 1];
      const seg = Math.hypot(bx - ax, by - ay);
      if (seg < 1e-3) continue;
      const across = gridAcross[j];
      let at = carry;
      while (at <= seg) {
        const u = at / seg;
        const age = gridAge[j] + (gridAge[j + 1] - gridAge[j]) * u;
        const p = Math.max(0, Math.min(frontP, 1 - (age - front.age0) / life));
        const dist = gridDist[j] + (gridDist[j + 1] - gridDist[j]) * u;
        const runLeft = gridEnd[j] - dist;
        const r = Math.min(across * Math.pow(p, 0.6), runLeft, Math.hypot(Math.max(0, edge - dist), Math.max(0, gridEnd[j] - edge)));
        const spacing = Math.max(0.7, 0.32 * r);
        if (r > 0.3) {
          const overlap = Math.max(1, (2 * r) / spacing);
          const A = (gridPeak[j] + (gridPeak[j + 1] - gridPeak[j]) * u) * Math.pow(p, 0.9);
          ctx.globalAlpha = shown * (1 - Math.pow(1 - A, 1 / overlap));
          const x = ax + (bx - ax) * u, y = ay + (by - ay) * u;
          ctx.drawImage(sprite, (x - r) * dpr, (y - r) * dpr, 2 * r * dpr, 2 * r * dpr);
          box.b = grow(box.b, x, y, r + 1);
          if (ctx.globalAlpha >= PIXEL_ALPHA) drawn = true;
        }
        at += spacing;
      }
      carry = at - seg;
    }
    ctx.globalAlpha = 1;
    return drawn;
  }

  // the hole: the mark's live outline, grown under the unit transform as the keyline is
  // drawn, so the trail's front edge is the mark's back outline GAP_PX outside its keyline
  // whatever the jelly does to the shape
  function cutHole(x, y, T) {
    ctx.globalCompositeOperation = 'destination-out';
    ctx.setTransform(...jellyMatrix(x, y, T));
    shape(hw.x, hh.x, 0);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fill();
    ctx.lineWidth = 2 * HOLE_PX;
    ctx.stroke();
    ctx.globalCompositeOperation = 'source-over';
  }

  function placeGlow(x, y) {
    const r = GLOW_R0 + GLOW_R1 * tone;
    bloom.style.transform = `translate3d(${x - GLOW_BOX}px, ${y - GLOW_BOX}px, 0) scale(${(r / GLOW_BOX).toFixed(4)})`;
    bloom.style.opacity = tone < 0.02 ? '0' : (GLOW_PEAK * tone * shown).toFixed(4);
  }

  function drawMark(x, y, T, box) {
    const w = hw.x, h = hh.x;
    const M = jellyMatrix(x, y, T);
    const refusal = Math.min(1, Math.max(0, red.x));
    // the stroke and the dot light up with speed; the fill stays the plain ink, so the text
    // under the pointer sits under the same 12% or 26% of --accent at every speed
    const colour = rgb(mix(mix(ink, litInk, tone), refuse, refusal));
    const fillColour = rgb(mix(ink, refuse, refusal));
    const unit = () => ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    // strokes are laid under the unit transform, so the line keeps its width
    // however far the path is stretched
    // the ink path is inset before the stretch, so across a full stretch its
    // outer edge creeps 0.3px past the shape; KEYLINE still leaves a 1px keyline there
    ctx.globalAlpha = shown;
    ctx.setTransform(...M); shape(w, h, 0); unit();
    ctx.lineWidth = KEYLINE;
    ctx.strokeStyle = rgb(keyline);
    ctx.stroke();
    const f = Math.min(1, Math.max(0, fill.x));
    if (f > 0.004) {
      ctx.globalAlpha = shown * f;
      ctx.fillStyle = fillColour;
      ctx.setTransform(...M); shape(w, h, 0); unit();
      ctx.fill();
      ctx.globalAlpha = shown;
    }
    if (w > STROKE / 2 + 0.05 && h > STROKE / 2 + 0.05 && f < 0.999) {
      ctx.setTransform(...M); shape(w, h, STROKE / 2); unit();
      ctx.lineWidth = STROKE;
      ctx.strokeStyle = colour;
      ctx.stroke();
    }
    const sl = Math.min(1, Math.max(0, slash.x));
    if (sl > 0.004) {
      const k = Math.max(0, Math.min(w, h) - STROKE) * Math.SQRT1_2;
      ctx.globalAlpha = shown * sl;
      ctx.setTransform(...M);
      ctx.beginPath(); ctx.moveTo(-k, -k); ctx.lineTo(k, k);
      unit();
      ctx.lineCap = 'butt';
      ctx.lineWidth = STROKE + 2; ctx.strokeStyle = rgb(keyline); ctx.stroke();
      ctx.lineWidth = STROKE; ctx.strokeStyle = colour; ctx.stroke();
      ctx.globalAlpha = shown;
    }
    const d = Math.min(1, Math.max(0, dot.x));
    if (d > 0.004) {
      ctx.globalAlpha = shown * d;
      ctx.beginPath(); ctx.arc(x, y, DOT_R + 1, 0, TAU);
      ctx.fillStyle = rgb(keyline); ctx.fill();
      ctx.beginPath(); ctx.arc(x, y, DOT_R, 0, TAU);
      ctx.fillStyle = colour; ctx.fill();
    }
    ctx.globalAlpha = 1;
    const extent = Math.max(w, h) * (1 + JELLY_MAX) + 3;
    box.b = grow(box.b, x, y, extent);
  }

  // the click: one 1px ring leaving the release point on --ease-out, its page
  // keyline outside it, opening from 20px to 72px as it fades
  function drawRipples(now, box) {
    for (let i = ripples.length - 1; i >= 0; i--) {
      const rp = ripples[i];
      const u = (now - rp.t0) / RIPPLE_MS;
      if (u >= 1) { ripples.splice(i, 1); continue; }
      const e = easeOut(u);
      const r = RIPPLE_R0 + (RIPPLE_R1 - RIPPLE_R0) * e;
      ctx.globalAlpha = 0.9 * (1 - e);
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.arc(rp.x, rp.y, r + 1, 0, TAU);
      ctx.strokeStyle = rgb(keyline); ctx.stroke();
      ctx.beginPath(); ctx.arc(rp.x, rp.y, r, 0, TAU);
      ctx.strokeStyle = rgb(rippleInk); ctx.stroke();
      box.b = grow(box.b, rp.x, rp.y, r + 3);
    }
    ctx.globalAlpha = 1;
  }

  // each droplet lives its own life and fades as the square; its speed keeps DROP_DRAG of
  // itself per millisecond and it travels the exact integral of that decay on its own clock,
  // from its birth, so the spray goes the same distance at every refresh rate and an input
  // that moves the springs' clock never shortens a droplet already flying. A stalled tab
  // plays on, as the springs do, from where it was
  const DROP_DRAG = 0.998, DROP_DRAG_LOG = Math.log(DROP_DRAG);
  const fly = (d, ms) => {
    const drag = DROP_DRAG ** ms, travel = (drag - 1) / DROP_DRAG_LOG;
    d.x += d.vx * travel; d.y += d.vy * travel;
    d.vx *= drag; d.vy *= drag;
  };
  const dropExpired = now => {
    for (let i = drops.length - 1; i >= 0; i--) if ((now - drops[i].born) / drops[i].life >= 1) drops.splice(i, 1);
  };
  function drawDrops(now, box) {
    dropExpired(now);
    ctx.strokeStyle = rgb(keyline);
    ctx.lineWidth = 1;
    for (let i = drops.length - 1; i >= 0; i--) {
      const d = drops[i];
      const u = (now - d.born) / d.life;
      fly(d, Math.min(MAX_STEP_S * 1000, now - d.at));
      d.at = now;
      ctx.globalAlpha = (1 - u) * (1 - u);
      ctx.fillStyle = rgb(d.hot ? sprayHot : spray);
      ctx.beginPath(); ctx.arc(d.x, d.y, d.r * (1 - u * 0.5), 0, TAU);
      ctx.fill();
      ctx.stroke();
      box.b = grow(box.b, d.x, d.y, d.r + 2);
    }
    ctx.globalAlpha = 1;
  }
  // a droplet is born at the instant of the pointer sample that sheds it, not when its
  // handler runs, so neither when the browser dispatches that sample (up to MAX_LATE_MS after
  // a frame passed its stamp) nor how many samples it coalesces into one event moves the spray,
  // short of the cap
  const spawn = (x, y, vx, vy, life, r, share, born) => {
    // the share is paid as a running debt, so the orange count is exact, never a coin flip
    hotDebt += share;
    const hot = hotDebt >= 1;
    if (hot) hotDebt -= 1;
    const d = { x, y, vx, vy, born, at: born, life, r, hot };
    // one born before the motion's last step flies to it, as that step's frame would have flown it
    if (born < last) { fly(d, last - born); d.at = last; }
    drops.push(d);
  };
  const orangeShare = v => ORANGE_MAX * Math.min(1, Math.max(0, (v - SPRAY_MIN) / (SPRAY_FULL - SPRAY_MIN)));
  // droplets shed off the head when the pointer is fast: backwards along the motion with
  // a sideways scatter, the way spray leaves a stone skipped across water. It runs once per
  // recorded sample and reads the pointer's velocity over the samples of its stroke, so the
  // spray depends on neither how the moves were batched nor a single whole-pixel step
  const shed = (x, y, t) => {
    const prev = lastEvent;
    // a gap past 80ms is a new stroke, not a speed; the caret sheds nothing
    const fresh = !prev || t - prev.t > 80 || state === 'text';
    lastEvent = { x, y, t, from: fresh ? t : prev.from };
    if (fresh) { sprayDebt = 0; lastSpeed = 0; return; }
    const dt = Math.max(0, t - prev.t);
    const [vx, vy] = velocity(lastEvent.from);
    const v = Math.hypot(vx, vy);
    lastSpeed = v;
    if (v <= SPRAY_MIN) { sprayDebt = 0; return; }
    const past = v - SPRAY_MIN;
    sprayDebt += SPRAY_RATE * past * past / (past + SPRAY_EASE) * dt;
    const share = orangeShare(v);
    const ux = vx / v, uy = vy / v;
    const born = instant(t);
    while (sprayDebt >= 1) {
      sprayDebt -= 1;
      // a droplet whose life ended by this one's birth holds no slot, drawn yet or not
      if (drops.length >= SPRAY_CAP) dropExpired(born);
      if (drops.length >= SPRAY_CAP) continue;
      const back = 0.05 + Math.random() * 0.12, side = (Math.random() - 0.5) * 0.5;
      spawn(x - ux * Math.random() * 6, y - uy * Math.random() * 6,
        -ux * back - uy * side, -uy * back + ux * side,
        320 + Math.random() * 220, 1.4 + Math.random() * 1.4, share, born);
    }
  };
  // a click drops a stone: a ring of droplets leaves the release point under the ripple
  const splash = (x, y, at) => {
    // a click at rest is a slow click: the last speed counts only within the stroke gap
    const share = lastEvent && at - lastEvent.t <= 80 ? orangeShare(lastSpeed) : 0;
    for (let i = 0; i < 10; i++) {
      const a = (i + Math.random() * 0.6) / 10 * TAU;
      const v = 0.12 + Math.random() * 0.1;
      spawn(x, y, Math.cos(a) * v, Math.sin(a) * v, 320 + Math.random() * 120, 1.5 + Math.random() * 1.1, share, at);
    }
  };

  // standing down for a finger or a closed gate is temporary, a thrown error is not. Arming
  // restores `cursor-v3` too, in case a page script rewrote <html>'s class since boot
  const arm = () => { if (!bailed && !gated) root.classList.add('cursor-v3', 'custom-cursor'); };
  addEventListener('error', () => { bailed = true; root.classList.remove('custom-cursor'); });
  onGateChange(e => {
    gated = !motionAllowed();
    if (!gated) return;
    respond(e.timeStamp, () => { host.classList.remove('visible', 'press'); samples.length = 0; });
    // reduced motion asked for mid-flight outranks a droplet's life; the stand-down is
    // the finger's, so the first mouse move after the gate reopens re-arms
    detached = true;
    root.classList.remove('custom-cursor');
    drops.length = 0; ripples.length = 0;
    lastEvent = null;
  });

  // a frame wanted with no input behind it (a resize, a theme, a retry)
  function request() {
    if (parked) reach(performance.now());
    if (!frame) frame = requestAnimationFrame(tick);
  }
  // a move at `at` wakes a parked loop at its instant; on a running loop it lands on the
  // frame's goal, as every move of a stroke does, so a stroke's moves never split a frame
  function moved(at) {
    if (parked) reach(at);
    request();
  }
  // a press, a release, a leave, a scroll, a gate: what it changes starts at its own instant,
  // whether the loop runs or sleeps. The first statement of its handler, so the springs it
  // reads stand at that instant; `change` is what it does to the goal. Stamped before the
  // motion's last step, the springs stay there and gain the change's motion since its instant.
  // Returns that instant
  function respond(at, change = () => {}) {
    const t = instant(at);
    reach(t);
    const before = t < last && goalAt(last);
    change();
    if (before) retarget(before, goalAt(last), last - t);
    request();
    return t;
  }

  // an input's instant: its stamp, MAX_LATE_MS before the motion's last step at the earliest,
  // and never past now (a stamp on another clock)
  function instant(at) {
    return Math.min(Math.max(at, last - MAX_LATE_MS), performance.now());
  }
  // Brings the motion from `last` to an input's instant on the goal it had until then, or
  // leaves it at `last` for one stamped before it (respond then plays its change). A
  // parked loop's springs stood at their goal, or moved on unseen behind a hidden mark toward
  // a goal no input changed, so one exact step, snapped where a ticking loop's frames would
  // have snapped it, lands them where that loop would be. A running loop steps like a frame
  // ending at the input, inside the frame's clamp; the next frame steps from there.
  function reach(at) {
    const t = Math.max(instant(at), last);
    if (parked) {
      if (parkedGoal && t > last) {
        advance((t - last) / 1000, parkedGoal);
        settle(parkedGoal);
      }
      frameAt = t;
    } else if (t > last) {
      advance(stepTo(t), goalAt(t));
    }
    last = t;
    parked = false;
  }
  // the seconds from `last` to t that fall inside the first MAX_STEP_S after the last frame,
  // so the steps between two frames add up to MAX_STEP_S at most
  const stepTo = t => {
    const end = frameAt + MAX_STEP_S * 1000;
    return (Math.min(t, end) - Math.min(last, end)) / 1000;
  };
  const park = goal => { parked = true; parkedGoal = goal; };

  const goalAt = now => {
    const [tp, tq, v] = jellyTarget(now);
    return { ...targets(), jp: tp, jq: tq, tone: Math.min(1, v / FULL_SPEED), shown: host.classList.contains('visible') ? 1 : 0 };
  };

  // each spring, the goal field it heads for, and its matrix in `matrices`
  const springs = [[hw, 'w', 'size'], [hh, 'h', 'size'], [fill, 'f', 'layer'], [dot, 'd', 'layer'],
    [slash, 'sl', 'layer'], [red, 'r', 'layer'], [jp, 'jp', 'wobble'], [jq, 'jq', 'wobble']];
  const matrices = dt => ({
    size: springMatrix(SHAPE_OMEGA, SHAPE_ZETA, dt), layer: springMatrix(SHAPE_OMEGA, 0.9, dt),
    wobble: springMatrix(JELLY_OMEGA, JELLY_ZETA, dt),
  });
  const fade = (from, goal, ms) => goal ? Math.min(1, from + ms / SHOW_MS) : Math.max(0, from - ms / HIDE_MS);

  // every spring and ease over dt seconds, the goal held across the step
  function advance(dt, goal) {
    const k = matrices(dt);
    for (const [s, field, kind] of springs) step(s, goal[field], k[kind]);
    tone = goal.tone + (tone - goal.tone) * Math.exp(-dt * 1000 / TONE_MS);
    shown = fade(shown, goal.shown, dt * 1000);
  }

  // adds to a spring what one resting at zero does under k heading for `target` from speed `v`:
  // the springs are linear, so that is what a goal change or a kick that long ago adds now
  const superpose = (s, target, v, k) => {
    const r = { x: 0, v };
    step(r, target, k);
    s.x += r.x; s.v += r.v;
  };
  // a goal change `ms` before the motion's last step, played from its own instant: the springs
  // and the light-up's ease are linear in their goal
  function retarget(from, to, ms) {
    const k = matrices(ms / 1000);
    for (const [s, field, kind] of springs) superpose(s, to[field] - from[field], 0, k[kind]);
    tone += (to.tone - from.tone) * (1 - Math.exp(-ms / TONE_MS));
    if (from.shown === to.shown) return;
    // the fade is linear short of its ends: rewound to the change on the old goal and replayed on
    // the new; one standing at its end is taken to have stood there since the change
    const atChange = shown === from.shown ? shown : Math.min(1, Math.max(0, fade(shown, from.shown, -ms)));
    shown = fade(atChange, to.shown, ms);
  }

  // snaps whatever has come to rest onto its goal; true once nothing that shows still moves
  function settle(goal) {
    const toneSettled = Math.abs(tone - goal.tone) < (goal.tone ? 0.003 : toneRest);
    if (toneSettled) tone = goal.tone;
    // a mark drawing no toned ink (the caret's filled line, the refusal red) shows the
    // light-up only through the bloom, which shows nothing under 0.02: there the ease runs on
    // unseen, still for the park, and a wake's exact step carries it
    const tonedInk = goal.r < 1 && (goal.f < 0.999 || goal.d > 0.004 || goal.sl > 0.004);
    const toneStill = toneSettled || (!goal.tone && !tonedInk && tone < 0.02);
    // 0.003 of stretch is 0.03px on the idle ring: below that the wobble is done. The stretch
    // moves an outline by about itself times the mark's smaller half size (the tensor scales
    // it by the roundness), so a thinner mark coming to rest, the caret's line, is done at
    // the stretch that moves it 0.03px
    const thin = 0.003 * (SIZE.idle / 2) / Math.max(Math.min(goal.w, goal.h), 1);
    const wobbleRest = goal.jp || goal.jq ? 0.003 : Math.max(0.003, thin);
    return [rest(hw, goal.w, 0.01), rest(hh, goal.h, 0.01), rest(fill, goal.f, 0.002), rest(dot, goal.d, 0.002),
      rest(slash, goal.sl, 0.002), rest(red, goal.r, 0.002), rest(jp, goal.jp, wobbleRest), rest(jq, goal.jq, wobbleRest),
      toneStill, shown === goal.shown]
      .every(Boolean);
  }

  function tick() {
    frame = 0;
    const now = performance.now();
    const dt = stepTo(now);
    last = frameAt = now;
    if (applyQueued) { applyQueued = false; apply(queuedTarget); }

    // drop samples the window has passed, keeping one to interpolate the tail from
    const win = samples.length ? windowMs(now) : TAIL_MS;
    while (samples.length > 1 && samples[1].net < netHead(now) - win) {
      const gone = samples.shift(), lead = samples[0].fill;
      if (gone.x !== samples[0].x || gone.y !== samples[0].y) behind = lead ? lead[lead.length - 1] : gone;
    }
    if (samples.length && strokeOver(now, win)) samples.length = 0;
    const goal = goalAt(now);
    advance(dt, goal);
    const settled = settle(goal);

    if (dirty) {
      const pad = 2;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(Math.floor((dirty[0] - pad) * dpr), Math.floor((dirty[1] - pad) * dpr),
        Math.ceil((dirty[2] - dirty[0] + 2 * pad) * dpr), Math.ceil((dirty[3] - dirty[1] + 2 * pad) * dpr));
    }
    const box = { b: null };
    if (!placed) { dirty = null; park(goal); return; }

    const x = pointer.x, y = pointer.y;
    const T = jellyTensor(shapeRoundness());
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    const count = samples.length ? buildGrid(now, win) : 0;
    let trailShows = false;
    if (count && sprite) {
      const edge = holeEdge();
      const still = now - samples[samples.length - 1].t;
      const front = edgeAge(count, edge, still);
      trailShows = drawTrail(count, edge, win, front, box);
      if (trailShows) cutHole(x, y, T);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    drawDrops(now, box);
    drawRipples(now, box);
    placeGlow(x, y);
    drawMark(x, y, T, box);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    dirty = box.b;
    host.dataset.x = x;
    host.dataset.y = y;
    // the hidden cursor arms only once the mark has drawn, so a mark that dies
    // at boot never leaves the page cursorless
    if (!proven) { proven = true; arm(); }

    // the loop parks on the first frame that will repeat. A still pointer's goal stays put
    // until the next input and its tube never grows back, so with droplets and ripples
    // (their own alpha) gone, that frame is one whose mark is hidden (shown 0, the trail and
    // the bloom gone with it), or one with every spring at rest and no stamp that shows.
    const hidden = shown === 0 && goal.shown === 0;
    const unchanging = !ripples.length && !drops.length && pointerStill(now) && (hidden || (settled && !trailShows));
    if (unchanging) park(goal);
    else frame = requestAnimationFrame(tick);
  }

  readColours();
  sizeCanvas();
  watchRatio();
  addEventListener('resize', sizeCanvas);
  onThemeChange(() => { readColours(); request(); });

  // the native cursor hides on the first real mouse move, never at boot: a
  // touchscreen laptop matches (pointer: fine) too
  addEventListener('pointermove', e => {
    if (e.pointerType !== 'mouse' || gated) return;
    const batch = e.getCoalescedEvents ? e.getCoalescedEvents() : [];
    const moves = batch.length ? batch : [e];
    const at = moves[0].timeStamp;
    moved(at);
    // a return from outside the window starts a new stroke: joining it to the old one
    // would draw a tube across a stretch the pointer never travelled. A stroke also ends at
    // its own instant, whether or not a frame ran past it
    if (left) { left = false; samples.length = 0; }
    else if (samples.length && strokeOver(at, windowMs(at))) samples.length = 0;
    for (const c of moves) {
      const tail = samples[samples.length - 1];
      if (tail && c.timeStamp <= tail.t) { tail.x = c.clientX; tail.y = c.clientY; }
      else {
        if (!tail) behind = null;
        const pause = tail ? Math.max(0, c.timeStamp - tail.t - DRAIN_MS) : 0;
        samples.push({ x: c.clientX, y: c.clientY, t: c.timeStamp, pause, net: tail ? tail.net + c.timeStamp - tail.t - pause : 0, ...record() });
      }
      shed(c.clientX, c.clientY, c.timeStamp);
    }
    pointer.x = e.clientX; pointer.y = e.clientY;
    if (!placed || detached) {
      placed = true; detached = false;
      if (proven) arm();
    }
    host.classList.add('visible');
    queueApply(e.target instanceof Element ? e.target : null);
  }, { passive: true });

  // the trail fades with the mark and ages out; the next move drops it
  root.addEventListener('mouseleave', e => {
    respond(e.timeStamp, () => host.classList.remove('visible'));
    left = true;
    lastEvent = null;
  });
  // not while a press is held: the pressed element owns the pointer through a
  // capture, and a drag's own scroll would re-derive the state off the raw point
  addEventListener('scroll', e => {
    if (host.classList.contains('press')) return;
    respond(e.timeStamp);
    queueApply(null);
  }, { passive: true, capture: true });

  // capture on the press pair, so a handler that stops propagation cannot strand the mark
  addEventListener('pointerdown', e => {
    if (e.pointerType !== 'mouse') {
      respond(e.timeStamp, () => host.classList.remove('visible'));
      detached = true;
      root.classList.remove('custom-cursor');
      return;
    }
    if (e.button !== 0 || gated) return;
    respond(e.timeStamp, () => host.classList.add('press'));
    pressedState = state;
  }, { capture: true });

  // a press lost to blur, cancel or a drag must not leave its click feedback for the next button
  const release = e => {
    pressedState = null;
    if (!host.classList.contains('press')) return;
    respond(e.timeStamp, () => host.classList.remove('press'));
  };
  addEventListener('pointerup', e => {
    if (e.pointerType !== 'mouse') return;
    const at = respond(e.timeStamp);
    const was = pressedState;
    release(e);
    if (was === 'blocked') {
      // a refused click shakes its head: a sideways kick into the jelly, centre unmoved
      superpose(jp, 0, SHAKE_KICK, matrices((last - at) / 1000).wobble);
    } else if (was === 'hover') {
      ripples.push({ x: e.clientX, y: e.clientY, t0: at });
      splash(e.clientX, e.clientY, at);
    }
    queueApply(null);   // the click's handler may have removed or moved the target
  }, { capture: true });
  addEventListener('pointercancel', release, { capture: true });
  addEventListener('blur', release);

  // a native drag brings the OS cursor and a ghost back, and no `cursor: none` outranks either
  document.querySelectorAll('a[href], img').forEach(el => { el.draggable = false; });
  addEventListener('dragstart', e => {
    release(e);
    if (root.classList.contains('custom-cursor')) e.preventDefault();
  }, { capture: true });
  /* A press held on `a[href]` hands the cursor to the link-drag machinery before
     any dragstart exists, and refusing mousedown is already too late, so the
     pointerdown default is refused and focus set by hand. `focusVisible: false`
     or every click paints the keyboard ring. Primary mouse only: the middle
     button opens a tab and the right one the context menu. */
  addEventListener('pointerdown', e => {
    if (e.pointerType !== 'mouse' || e.button !== 0) return;
    if (!root.classList.contains('custom-cursor')) return;
    const link = e.target instanceof Element && e.target.closest('a[href]');
    if (!link) return;
    e.preventDefault();
    link.focus({ preventScroll: true, focusVisible: false });
  }, { capture: true });
};

/* A browser cache can serve another version's cursor.css beside this file, and a page can
   load the sheet after the module runs (a script-inserted link, a media or preload swap, an
   async module). The layer boots only once its own sheet applies, told by a marker no v1 or
   v2 sheet declares, and marks the page `cursor-v3`, which the sheet's hidden-cursor rule
   needs beside `custom-cursor`. Until then it looks again on window load and on every
   mouse move, so a late sheet still gets the whole layer and a legacy one never does. An
   async module can also run before `<body>` exists; the layer then boots on load or on the
   first mouse move once the body is there. */
const ownSheet = () => getComputedStyle(root).getPropertyValue('--cursor-v3') !== '';
let booted = false;
const boot = () => {
  if (booted || !document.body || !ownSheet()) return;
  booted = true;
  root.classList.add('cursor-v3');
  bootGrid();
  if (motionAllowed()) bootMark();
};
// the grid's track, and the late look for the sheet. pointermove, never mousemove: a refused
// pointerdown default (the link guard, the drawn scrollbar's thumb) suppresses every
// compatibility mouse event until the release, which froze the grid for a press-drag.
// Registered before the ring's, so on each move the grid reads the pointer first
addEventListener('pointermove', e => {
  if (e.pointerType !== 'mouse') return;
  boot();
  if (gridTrack) gridTrack(e);
}, { passive: true });
addEventListener('load', boot, { once: true });
boot();
