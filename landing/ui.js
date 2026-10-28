/* ds-kit controllers. Every component that needs script, in one module.
   Load as `<script type="module" src="ui.js"></script>`. It wires itself on load
   against whatever markup is present and does nothing where a component is absent,
   so one script tag is correct for every page in the system.

   Contract, and the reason this file exists rather than a set of paste-in blocks:
   - NOTHING here throws on a page that lacks a component. The pasted controllers
     this replaces crashed on a null `#navbar-nav`, a null `#modal-btn` and three
     null table ids, and the cursor's `window.onerror` handler reads any thrown
     error as a bail: one missing modal took the page's ring down with it.
   - `init()` is idempotent. Every element it wires carries a `data-ui-*` mark, so
     calling it again after rendering new markup wires only what is new. The one
     step outside that is the header label, which re-runs on every call by design
     (a table may re-render its head) and is idempotent because the span it builds
     is the thing it looks for.
   - The data half of a table is the page's. This file owns sort cycling, selection
     sync, paging clicks and the filter menu, and announces each as an event.

   Named exports let a page wire one piece by hand; the auto-init at the bottom is
   the default and covers every page in this system. */

import { collectPropSelectors } from './cursor-rules.js';

/* ============================ helpers ============================ */

// a component is wired once, whatever calls init(). The mark is on the element the
// listener is attached to, never on an ancestor, or a re-render silently skips it.
const once = (el, key) => {
  if (!el || el.dataset[key]) return false;
  el.dataset[key] = '1';
  return true;
};

// an exit costume's own clock, read off the element, so a token change or reduced
// motion retimes every hide without a number in this file
const animMs = el => (parseFloat(getComputedStyle(el).animationDuration) || 0) * 1000 + 20;
const transMs = el => Math.max(...getComputedStyle(el).transitionDuration.split(',').map(parseFloat)) * 1000;

// the control a row-level click must not answer for, because its own handler already will
const isControl = t => t.closest?.('button, a, input, select, textarea, label');

const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

// keeps a row inside its own scroll box without scrollIntoView, which would also scroll
// the page when the box sits near a viewport edge
const scrollIntoBox = (box, top, bottom) => {
  if (top < box.scrollTop) box.scrollTop = top;
  else if (bottom > box.scrollTop + box.clientHeight) box.scrollTop = bottom - box.clientHeight;
};

/* The query rule both filtering lists run on: every word has to land. The tokenizer is
   its own step because the palette's empty message names the fix from the COUNT:
   another word for a one-word query, fewer for the rest. */
const queryWords = typed => typed.toLowerCase().split(/\s+/).filter(Boolean);
const matchesQuery = (hay, words) => words.every(w => hay.includes(w));

/* The exit every overlay in this file runs: the `.closing` costume, held for `clock`,
   then hidden with the costume dropped on the same frame. `clock` is a function because
   the duration belongs to the costume, so it can only be read once the class is on: a
   modal panel's own exit is declared at `.modal-backdrop.closing .modal`, which needs
   the class on the BACKDROP too. The timer comes back to the caller, since a reopen
   mid-exit has to clear it. */
const runExit = (el, panel, clock) => {
  el.classList.add('closing');
  panel?.classList.add('closing');
  return setTimeout(() => {
    el.hidden = true;
    el.classList.remove('closing');
    panel?.classList.remove('closing');
  }, clock());
};

// a press inside a controller belongs to that controller, so only the others close.
// Controllers are dropped when their root leaves the DOM: a re-render that replaces
// a tip or a select leaves a dead entry here, and every outside press then walks it.
const closeOnOutside = controllers => document.addEventListener('pointerdown', e => {
  for (let i = controllers.length - 1; i >= 0; i--) {
    if (!controllers[i].root.isConnected) { controllers.splice(i, 1); continue; }
    if (!controllers[i].root.contains(e.target)) controllers[i].close();
  }
});

/* ====================== sliding inks (tabs, navbar) ====================== */

/* First placement lands in place: with the transform transition live, an ink
   fading in would slide from x=0 behind its own fade, reading as a teleport.
   Kill the transition once, flush, restore.

   An ink declaring --ink-base (.tab-ink, .navbar-ink) is laid out at that width
   and scaled, so its length rides the same transform as its travel and cannot
   snap to the target while the bar is still moving. The boxed pill has no
   --ink-base: scaleX would distort its 11px corners, so it resizes instead.

   Both ride translate3d: the z holds the ink's layer for the life of the page,
   so the sub-pixel geometry painted during the slide is the same geometry
   painted after it. A layer dropped at transitionend re-rasterizes the
   fractional size and position at whole pixels, which is the visible snap
   after landing. */
export function placeInk(inkEl, width, x) {
  const first = !inkEl.dataset.placed;
  if (first) {
    inkEl.dataset.placed = '1';
    inkEl.style.transition = 'none';
  }
  const base = parseFloat(getComputedStyle(inkEl).getPropertyValue('--ink-base'));
  const next = base > 0
    ? `translate3d(${x}px, 0, 0) scaleX(${width / base})`
    : `translate3d(${x}px, 0, 0)`;
  if (!(base > 0)) inkEl.style.width = width + 'px';
  inkEl.style.transform = next;
  if (first) {
    inkEl.getBoundingClientRect();   // flush: the jump happens un-animated
    requestAnimationFrame(() => { inkEl.style.transition = ''; });
  }
  inkEl.style.opacity = '1';
}

// one geometry read for every ink on the page: the active child's box against its rail
function placeFrom(wrapEl, inkEl, activeSel) {
  if (!wrapEl || !inkEl) return;
  const active = wrapEl.querySelector(activeSel);
  if (!active) { inkEl.style.opacity = '0'; return; }
  const wRect = wrapEl.getBoundingClientRect();
  const aRect = active.getBoundingClientRect();
  placeInk(inkEl, aRect.width, aRect.left - wRect.left);
}

/* A MARKER A CONSUMER HAS TO WRITE IS THE COMPONENT DEFINED TWICE, and the copy
   that gets forgotten is the one a reader meets. Four of them used to be markup:
   `.tab-ink`, `.tabs-boxed-ink`, `.navbar-ink` and the sidebar row's `.nav-dot`,
   61 hand-written spans on the pages in this repo alone. They are built here, so
   a page writes the wrap and gets the whole component. Where a marker sits is
   part of it: these are positioned with no z-index, so paint order is document
   order, and one that paints UNDER its siblings goes first. Existing markup is
   reused, so a page that still writes one never gets a second. `svg` builds in
   the SVG namespace, where `className` is read-only, so the class is an
   attribute for both. */
const SVG_NS = 'http://www.w3.org/2000/svg';
function ownChild(parent, cls, { tag = 'div', first = false, svg = false } = {}) {
  const sel = `:scope > .${cls.trim().split(/\s+/).join('.')}`;
  let el = parent.querySelector(sel);
  if (!el) {
    el = svg ? document.createElementNS(SVG_NS, tag) : document.createElement(tag);
    el.setAttribute('class', cls);
    el.setAttribute('aria-hidden', 'true');
    if (first) parent.prepend(el); else parent.append(el);
  }
  return el;
}

/* Same reading, one step further: a marker is not the only internal a consumer can
   be left to write. A table header's label needs a box of its own to centre in,
   because a cell centres its LINE BOX and all-caps ink rides high in one. The
   measurement and the levers that do not work are in `components.css` beside
   `.th-label`. This one carries the column's accessible name, so unlike `ownChild`
   it is never `aria-hidden`. It wraps the cell's text into one span placed where
   the text starts, which is why it REFUSES a cell whose text is split by an
   element: joining `P50 <span class=tag>ms</span> avg` would move the tag to the
   end and read the column out as `P50 avg ms`. Nothing on disk writes that shape,
   and a design system is consumed by markup nobody here wrote. The shapes it does
   take are a sort button (text, then the glyph) and a plain cell; the select-all
   cell holds a checkbox and no text, so it is left alone. */
function ownLabel(parent, cls) {
  const have = parent.querySelector(`:scope > .${cls}`);
  if (have) return have;
  const kids = [...parent.childNodes];
  const texts = kids.filter(n => n.nodeType === 3 && n.nodeValue.trim());
  if (!texts.length) return null;
  const span = kids.slice(kids.indexOf(texts[0]), kids.indexOf(texts[texts.length - 1]) + 1);
  if (span.some(n => n.nodeType === 1)) return null;   // an element splits the text
  const el = document.createElement('span');
  el.className = cls;
  el.textContent = texts.map(n => n.nodeValue).join('').trim();
  texts.slice(1).forEach(n => n.remove());
  parent.replaceChild(el, texts[0]);
  return el;
}

// a wrapper is not a child: the component's own node has to end up inside it
function ownWrap(el, cls) {
  const have = el.parentElement;
  if (have && cls.trim().split(/\s+/).every(t => have.classList.contains(t))) return have;
  const wrap = document.createElement('div');
  wrap.className = cls;
  el.parentNode.insertBefore(wrap, el);
  wrap.appendChild(el);
  return wrap;
}

// a resize or a scroll moves the ink without animating the slide: the indicator is
// following its tab, not travelling to a new one
function replace(inkEl, run) {
  if (!inkEl) return;
  inkEl.style.transition = 'none';
  run();
  requestAnimationFrame(() => { inkEl.style.transition = ''; });
}

/* ============================== tabs ============================== */

/* Underline tabs with a sliding ink, and boxed segments with a sliding pill.

   `data-tabs="group-id"` groups tabs that live in different wraps and `data-panel`
   names each one's panel. BOTH ARE OPTIONAL: with neither, the wrap is the group
   and the control just moves its own marker, which is what a segmented control
   with no panels needs. Keying only on `data-tabs` is why every page carrying a
   panel-less boxed control had a hand-written copy of this. */
const TAB_WRAP = '.tabs-wrap, .tabs-boxed-wrap';

export function initTabs() {
  /* examples/dashboard.html is what this cost: its wrap held no pill, so the
     segments cross-faded their own backgrounds instead. The two read identical
     AT REST, which is why no screenshot could fail on it; measured 60ms into a
     click, the sheet's pill at translate 24.55 on its way to 42.42 against the
     dashboard's nothing. The boxed pill paints under its segments, so it goes
     first. */
  /* The underline ink rides the TAB ROW, never the wrap: a wrap may hold its
     panels too (the sheet's specimen nests them), and the row is the box the
     ink underlines whatever else the wrap contains. The boxed pill's track is
     the wrap itself, so the two branch here. */
  const inkFor = wrap => {
    if (!wrap) return null;
    if (wrap.classList.contains('tabs-boxed-wrap')) return ownChild(wrap, 'tabs-boxed-ink', { first: true });
    // no row, no ink: an ink with no rail resolves against an ancestor the
    // component does not own
    const tabsRow = wrap.querySelector(':scope > .tabs');
    if (!tabsRow) return null;
    // the rail is hoisted, never queried inline: the builder args are parsed with
    // a flat scan that closes at the first `)`
    return ownChild(tabsRow, 'tab-ink');
  };
  const wraps = () => document.querySelectorAll(TAB_WRAP);
  const placeWrap = wrap => {
    const ink = inkFor(wrap);
    if (!ink) return;
    const boxed = ink.classList.contains('tabs-boxed-ink');
    // the rail is the ink's own parent, so geometry and paint cannot disagree
    placeFrom(ink.parentElement, ink, boxed ? '.tab-box.active' : '.tab.active');
    if (boxed) wrap.classList.add('has-ink');
  };

  document.querySelectorAll('.tab, .tab-box').forEach(btn => {
    if (!once(btn, 'uiTab')) return;
    const wrap = btn.closest(TAB_WRAP);
    const kind = btn.classList.contains('tab-box') ? '.tab-box' : '.tab';
    btn.addEventListener('click', () => {
      const group = btn.dataset.tabs;
      const peers = group
        ? [...document.querySelectorAll(`[data-tabs="${group}"]`)]
        : [...(wrap ? wrap.querySelectorAll(kind) : [btn])];
      peers.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');

      new Set(peers.map(b => b.dataset.panel)).forEach(id => {
        const el = id && document.getElementById(id);
        if (el) el.classList.remove('active');
      });
      const panel = btn.dataset.panel && document.getElementById(btn.dataset.panel);
      if (panel) panel.classList.add('active');

      // a data-tabs group can span wraps, so every wrap holding a peer re-places
      new Set(peers.map(b => b.closest(TAB_WRAP)).filter(Boolean))
        .forEach(placeWrap);
    });
  });

  requestAnimationFrame(() => wraps().forEach(placeWrap));   // first placement, after layout
  if (once(document.body, 'uiTabsResize')) {
    // a resize moves the ink without animating the slide: it is following its tab,
    // not travelling to a new one
    addEventListener('resize', () => wraps().forEach(w => replace(inkFor(w), () => placeWrap(w))));
  }
}

/* ============================= navbar ============================= */

/* The tab ink's physics on the horizontal navbar, plus the bar's docking state.

   Every `.navbar-nav` holding a `.navbar-ink` is wired, and each one owns only its
   own links: a page carrying a second navbar (a narrow variant, a docs sub-bar)
   used to need a hand-written copy of this controller, and the copy drifted.
   Nothing here is required to exist; a page with no navbar wires nothing rather
   than throwing on a null lookup, which is what the pasted version did. */
export function initNavbar() {
  document.querySelectorAll('.navbar-nav').forEach(nav => {
    if (!once(nav, 'uiNavbar')) return;
    const ink = ownChild(nav, 'navbar-ink');
    const place = () => placeFrom(nav, ink, '.navbar-link.active');

    nav.querySelectorAll('.navbar-link').forEach(link => {
      link.addEventListener('click', e => {
        if (link.getAttribute('href') === '#') e.preventDefault();
        nav.querySelectorAll('.navbar-link').forEach(l => l.classList.remove('active'));
        link.classList.add('active');
        place();
      });
    });

    addEventListener('resize', () => replace(ink, place));
    // under 768px the band scrolls and the ink rides the content: the same contract
    nav.addEventListener('scroll', () => replace(ink, place), { passive: true });
    requestAnimationFrame(place);
  });

  /* The docking state. scrollY is the whole state: `.is-top` drops the moment the
     page moves and returns at the top, so the bar is a floating card at rest and a
     band once scrolled. Below 768px the CSS ignores it. */
  document.querySelectorAll('.navbar').forEach(bar => {
    if (!once(bar, 'uiNavbarDock')) return;
    const sync = () => bar.classList.toggle('is-top', scrollY <= 0);
    sync();
    addEventListener('scroll', sync, { passive: true });
  });
}

/* ============================= sidebar ============================ */

/* The marker is the active row's own dot, so nothing is measured or positioned:
   the controller moves one class and the CSS does the rest.

   Every `.nav-item` is wired, each owning only the rows in its own sidebar.
   Keying on `.nav-item[data-section]` left a plain sidebar inert, so the worked
   dashboard example carried a hand-written copy of these six lines. */
export function initSidebar() {
  document.querySelectorAll('.nav-item').forEach(item => {
    if (!once(item, 'uiNav')) return;
    // the row's own dot is the marker, so it belongs to the row, not to the page
    ownChild(item, 'nav-dot', { tag: 'span', first: true });
    const scope = item.closest('.sidebar') || document;
    item.addEventListener('click', () => {
      scope.querySelectorAll('.nav-item').forEach(n => {
        n.classList.remove('active');
        n.removeAttribute('aria-current');
      });
      item.classList.add('active');
      item.setAttribute('aria-current', 'true');
    });
  });

  // the button is optional, and its own sidebar is the one it folds
  document.querySelectorAll('.sidebar-collapse').forEach(collapse => {
    const column = collapse.closest('.sidebar');
    if (!column || !once(collapse, 'uiCollapse')) return;
    const sync = () => {
      const folded = column.classList.contains('rail');
      collapse.setAttribute('aria-expanded', String(!folded));
      collapse.setAttribute('aria-label', folded ? 'Expand sidebar' : 'Collapse sidebar');
    };
    sync();                                     // a sidebar may ship folded
    collapse.addEventListener('click', () => { column.classList.toggle('rail'); sync(); });
  });
}

/* ========================== mobile drawer ========================= */

export function initDrawer() {
  const sidebar = document.getElementById('sidebar');
  const overlay = document.getElementById('sidebar-overlay');
  const hamburger = document.getElementById('hamburger-btn');
  if (!sidebar) return;

  const setOpen = open => {
    sidebar.classList.toggle('open', open);
    overlay?.classList.toggle('open', open);
    hamburger?.setAttribute('aria-expanded', String(open));
  };
  if (hamburger && once(hamburger, 'uiDrawer')) hamburger.addEventListener('click', () => setOpen(true));
  if (overlay && once(overlay, 'uiDrawer')) overlay.addEventListener('click', () => setOpen(false));
  if (once(sidebar, 'uiDrawerRows')) {
    document.querySelectorAll('.nav-item, .navbar-link').forEach(item => {
      item.addEventListener('click', () => { if (innerWidth <= 768) setOpen(false); });
    });
    addEventListener('keydown', e => { if (e.key === 'Escape') setOpen(false); });
  }
}

/* ====================== switches and their rows ====================== */

/* A `.toggle` is a real button carrying `aria-pressed`, so the state and the
   semantics move together and never disagree.

   THE ROW IS THE TARGET, her call 2026-08-29: a coarse pointer gets its 44px on
   the row and the control keeps its proportions, so the row has to be clickable
   or that floor buys nothing. The guard skips a click that landed on another
   control, so the control's own handler runs once and the row cannot double-toggle
   it. Every page carrying a settings panel had written this by hand. */
export function initToggles() {
  document.querySelectorAll('.toggle').forEach(t => {
    if (t.disabled || !once(t, 'uiToggle')) return;
    t.addEventListener('click', () => {
      t.setAttribute('aria-pressed', String(t.classList.toggle('on')));
    });
  });

  document.querySelectorAll('.toggle-wrap, .setting-row').forEach(row => {
    if (!once(row, 'uiRowTarget')) return;
    row.addEventListener('click', e => {
      if (isControl(e.target)) return;
      row.querySelector('.toggle:not(:disabled)')?.click();
    });
  });
}

/* ============================ disclosure ========================== */

/* `details` has no disabled state, so a row that cannot act says so with
   aria-disabled and the summary stays focusable to announce why. Its click IS
   the toggle (keyboard Enter and Space ride the same event), so cancelling the
   click cancels the row; the costume lives in components.css beside the
   accordion. */
export function initDisclosures() {
  document.querySelectorAll('.disclosure[aria-disabled="true"] > summary').forEach(s => {
    if (!once(s, 'uiDisclosure')) return;
    s.addEventListener('click', e => e.preventDefault());
  });
}

/* ============================== select ============================ */

/* The trigger keeps DOM focus for the whole interaction and the panel is a listbox,
   so the arrow-key row is named by aria-activedescendant instead of being focused.
   Escape therefore always lands back on the control that opened. */
const selectControllers = [];

export function initSelect() {
  document.querySelectorAll('.select').forEach((root, i) => {
    const trigger = root.querySelector('.select-trigger');
    const menu = root.querySelector('.select-menu');
    const options = [...root.querySelectorAll('[role="option"]')];
    const empty = root.querySelector('.palette-empty');

    /* A div trigger stays focusable under aria-disabled (that is the pattern:
    it can announce why it cannot act), and an input's disabled state stops its
    own events but not a clear sitting beside it, so every activation path
    reads the state here rather than trusting the pointer's `none`. It has to
    exist before the adapter builds: the multi adapter wires chip listeners at
    construction, and a root the guards below return early for (its menu not
    rendered yet) would leave this in its dead zone and throw at the first
    chip click. */
    const dead = () => trigger.disabled || trigger.getAttribute('aria-disabled') === 'true';
    // the same dead zone: a chip's remove announces, and it is wired at construction
    const announce = () => root.dispatchEvent(
      new CustomEvent('change', { bubbles: true, detail: { value: root.dataset.value } }));
    const selectOnly = opt => options.forEach(o => o.setAttribute('aria-selected', String(o === opt)));

    // one value surface per shape: the shipped .select-value, an <input>
    // trigger's value (combobox), or the chips in a multi-select trigger
    const valueAdapter = valueEl => valueEl && ({
      read: () => valueEl.textContent.trim(),
      value: () => valueEl.textContent.trim(),
      pick(opt) {
        selectOnly(opt);
        valueEl.textContent = opt.textContent.trim();
        return true;
      },
      clear() {},
    });
    const comboAdapter = (root, trigger) => {
      let committed = trigger.value.trim();
      const showAll = () => { options.forEach(o => { o.hidden = false; }); if (empty) empty.hidden = true; };
      return {
        read: () => committed,
        value: () => committed,
        // every open starts from the committed value with the full list, so a
        // query a blur left behind cannot pin the panel to one match
        onOpen: () => { trigger.value = committed; showAll(); },
        pick(opt) {
          selectOnly(opt);
          committed = opt.textContent.trim();
          trigger.value = committed;
          return true;
        },
        clear: () => { committed = ''; trigger.value = ''; showAll(); },
        restore: () => { trigger.value = committed; showAll(); },
      };
    };
    const multiAdapter = (root, trigger) => {
      const chips = root.querySelector('.select-chips');
      const picked = () => options.filter(o => o.getAttribute('aria-selected') === 'true')
        .map(o => o.textContent.trim()).join(', ');
      const sync = () => { root.dataset.value = picked(); };
      const removeChip = chip => {
        if (dead()) return;
        const label = chip.querySelector('span')?.textContent ?? '';
        const opt = options.find(o => o.textContent.trim() === label);
        if (opt) opt.setAttribute('aria-selected', 'false');
        chip.remove();
        sync();
        announce();
      };
      // one wiring path for a chip whatever created it: the root's once mark
      // never covers one, since construction wires before that guard runs
      const wireChip = chip => {
        if (!once(chip, 'uiSelectChip')) return;
        chip.addEventListener('click', e => { e.stopPropagation(); removeChip(chip); });
      };
      const addChip = label => {
        if (!chips) return;
        const chip = document.createElement('button');
        chip.type = 'button';
        chip.className = 'tag filter-chip select-chip';
        chip.setAttribute('aria-label', `remove ${label}`);
        chip.innerHTML = '<span></span><svg class="icon close" aria-hidden="true"><use href="#i-x"/></svg>';
        chip.querySelector('span').textContent = label;
        wireChip(chip);
        chips.appendChild(chip);
      };
      // the fold looks like a chip, but it is authored summary text: wiring
      // it as removable deletes the indicator on the click that should open
      chips?.querySelectorAll('.select-chip:not(.select-chip-more)').forEach(wireChip);
      return {
        read: picked,
        value: () => root.dataset.value,
        pick(opt) {
          const on = opt.getAttribute('aria-selected') === 'true';
          opt.setAttribute('aria-selected', String(!on));
          const label = opt.textContent.trim();
          if (on) {
            const chip = [...(chips?.querySelectorAll('.select-chip') ?? [])]
              .find(c => c.querySelector('span')?.textContent === label);
            chip?.remove();
          } else {
            addChip(label);
          }
          sync();
          return false;   // aria-multiselectable: a toggle does not close the panel
        },
        clear() {
          options.forEach(o => o.setAttribute('aria-selected', 'false'));
          chips?.querySelectorAll('.select-chip').forEach(c => c.remove());
          sync();
        },
      };
    };

    const adapter = !trigger ? null
      : root.classList.contains('select-multi') ? multiAdapter(root, trigger)
      : trigger.tagName === 'INPUT'             ? comboAdapter(root, trigger)
      : valueAdapter(root.querySelector('.select-value'));
    if (!trigger || !menu || !options.length || !adapter) return;
    if (!once(root, 'uiSelect')) return;

    menu.id ||= `select-${i}-menu`;
    trigger.setAttribute('aria-controls', menu.id);
    options.forEach((o, j) => { o.id ||= `select-${i}-opt-${j}`; });

    // the width floor: the trigger's text is only the selected value, so left
    // to itself a select in a flex row sizes to it and the open panel outgrows
    // the shut state. Measured after the row reaches the document (a detached
    // tree has no layout, so offsetWidth reads 0) and again through an explicit
    // fonts.load, since fonts.ready can resolve before a lazy face even starts.
    // Pinned at the panel's corner so its box never extends the scroller's
    // overflow. The floor is the widest option plus the menu's gutter and border.
    const sizer = document.createElement('span');
    sizer.className = 'menu-item';
    sizer.style.cssText = 'position:absolute;visibility:hidden;white-space:nowrap;top:0;left:0;';
    menu.appendChild(sizer);
    const fitLongest = () => {
      if (!root.isConnected) return;
      let widest = 0;
      options.forEach(opt => {
        sizer.textContent = opt.textContent;
        widest = Math.max(widest, sizer.offsetWidth);
      });
      root.style.minWidth = (widest + 10) + 'px';
    };
    requestAnimationFrame(fitLongest);
    document.fonts.ready.then(() => {
      document.fonts.load(getComputedStyle(sizer).font).then(fitLongest);
    });

    const visible = () => options.filter(o => !o.hidden);
    const selectedVisible = () => {
      const vis = visible();
      const at = vis.findIndex(o => o.getAttribute('aria-selected') === 'true');
      return at >= 0 ? at : 0;
    };
    let focusIdx = selectedVisible();
    // the value surface exists from load, or a consumer reading it before the
    // first pick disagrees with what aria-selected already announces
    root.dataset.value = adapter.read();
    // the surface is written before the event, or a consumer reading it from the
    // change handler still sees the previous pick
    const commit = () => { root.dataset.value = adapter.value(); announce(); };
    let closingTimer = 0;
    const isOpen = () => root.classList.contains('open');
    // read off --fold, the move itself, so a token change or reduced motion
    // retimes the hold without touching this number
    const exitMs = () => transMs(root);

    const revealFocused = opt => {
      const top = opt.offsetTop;
      scrollIntoBox(menu, top, top + opt.offsetHeight);
    };

    const markFocus = idx => {
      const vis = visible();
      if (!vis.length) {
        trigger.removeAttribute('aria-activedescendant');
        options.forEach(o => o.classList.remove('focus'));
        return;
      }
      focusIdx = ((idx % vis.length) + vis.length) % vis.length;
      const opt = vis[focusIdx];
      options.forEach(o => o.classList.toggle('focus', o === opt));
      trigger.setAttribute('aria-activedescendant', opt.id);
      revealFocused(opt);
    };

    const close = ({ focus = false } = {}) => {
      if (!isOpen()) return;
      root.classList.remove('open');
      // .closing keeps the body's rules applying while --fold runs back to 1;
      // it drops on a frame where every value already equals the closed one
      root.classList.add('closing');
      clearTimeout(closingTimer);
      closingTimer = setTimeout(() => root.classList.remove('closing'), exitMs());
      trigger.setAttribute('aria-expanded', 'false');
      trigger.removeAttribute('aria-activedescendant');
      options.forEach(o => o.classList.remove('focus'));
      if (focus) trigger.focus();
      // .select-up stays: dropping it mid-exit flips the panel across the trigger
    };

    const open = () => {
      if (dead() || isOpen()) return;
      clearTimeout(closingTimer);
      root.classList.remove('closing');
      selectControllers.forEach(c => { if (c.root !== root) c.close(); });
      adapter.onOpen?.();
      // the side with room wins; visibility:hidden still lays out, so this measures
      const r = trigger.getBoundingClientRect();
      const below = innerHeight - r.bottom;
      root.classList.toggle('select-up', below < menu.offsetHeight + 8 && r.top > below);
      root.classList.add('open');
      trigger.setAttribute('aria-expanded', 'true');
      markFocus(selectedVisible());
    };

    const pick = opt => {
      if (!opt) return;
      if (adapter.pick(opt)) close({ focus: true });
      commit();
    };

    const filter = () => {
      const words = queryWords(trigger.value.trim());
      options.forEach(opt => {
        opt.hidden = !matchesQuery(opt.textContent.trim().toLowerCase(), words);
      });
      if (empty) empty.hidden = visible().length > 0;
      markFocus(selectedVisible());
    };

    // an <input> trigger edits the query: focus opens, Space types instead of
    // committing, and clicking never closes. A button or div trigger toggles.
    if (trigger.tagName === 'INPUT') {
      trigger.addEventListener('focus', open);
      trigger.addEventListener('input', filter);
    }
    trigger.addEventListener('click', () => {
      if (trigger.tagName === 'INPUT') open();
      else isOpen() ? close() : open();
    });
    trigger.addEventListener('blur', () => close());
    trigger.addEventListener('keydown', e => {
      // a key on a chip or the clear belongs to that control: the trigger's
      // own handling would swallow the activation it should not answer for
      if (e.target !== trigger) return;
      switch (e.key) {
        case 'ArrowDown': e.preventDefault(); isOpen() ? markFocus(focusIdx + 1) : open(); break;
        case 'ArrowUp':   e.preventDefault(); isOpen() ? markFocus(focusIdx - 1) : open(); break;
        case 'Home':      if (isOpen()) { e.preventDefault(); markFocus(0); } break;
        case 'End':       if (isOpen()) { e.preventDefault(); markFocus(visible().length - 1); } break;
        case 'Enter':     e.preventDefault(); isOpen() ? pick(visible()[focusIdx]) : open(); break;
        case ' ':         if (trigger.tagName !== 'INPUT') { e.preventDefault(); isOpen() ? pick(visible()[focusIdx]) : open(); } break;
        case 'Escape':    if (isOpen()) { e.preventDefault(); adapter.restore?.(); close({ focus: true }); } break;
        case 'Tab':       close(); break;
      }
    });

    // the panel must not take focus off the trigger, or the blur above closes
    // it before the option's own click can land
    menu.addEventListener('mousedown', e => e.preventDefault());
    options.forEach(opt => {
      opt.addEventListener('click', () => pick(opt));
      opt.addEventListener('pointermove', () => {
        const pos = visible().indexOf(opt);
        if (pos >= 0 && pos !== focusIdx) markFocus(pos);
      });
    });

    root.querySelector('.select-clear')?.addEventListener('click', e => {
      e.stopPropagation();
      if (dead()) return;
      adapter.clear();
      commit();
    });

    selectControllers.push({ root, close });
  });

  if (once(document.body, 'uiSelectOutside')) closeOnOutside(selectControllers);
}

/* ============================= palette ============================ */

/* The query field keeps DOM focus for the whole interaction and the list is a
   listbox, so the arrow-key row is named by aria-activedescendant. The controller
   owns nothing a command does: it dispatches `run` on the backdrop with the row's
   data-command. Ctrl/Cmd+K opens it whether or not a trigger button exists. */
const VISIBLE_ROW = '.palette-row:not([hidden])';

export function initSlider() {
  document.querySelectorAll('.slider').forEach(root => {
    if (!once(root, 'uiSlider')) return;
    const track = root.querySelector('.slider-track');
    const fill = root.querySelector('.slider-fill');
    if (!track || !fill) return;   // the controller never throws on half-built markup
    const min = parseFloat(root.getAttribute('aria-valuemin') ?? '0');
    const max = parseFloat(root.getAttribute('aria-valuemax') ?? '100');
    let val = parseFloat(root.getAttribute('aria-valuenow') ?? String(min));
    const commit = () => {
      root.setAttribute('aria-valuenow', String(val));
      fill.style.width = ((val - min) / (max - min)) * 100 + '%';
    };
    const setVal = (v, fire) => {
      val = Math.min(max, Math.max(min, v));
      commit();
      if (fire) root.dispatchEvent(new Event('input', { bubbles: true }));
    };
    const valFromX = clientX => {
      const r = track.getBoundingClientRect();
      return min + ((clientX - r.left) / r.width) * (max - min);
    };
    root.addEventListener('pointerdown', e => {
      e.preventDefault();
      root.focus();
      setVal(valFromX(e.clientX), true);
      // a synthetic event carries no active pointer and the capture throws,
      // which would also bail the ring through its error listener
      try { root.setPointerCapture(e.pointerId); } catch {}
      root.dataset.dragging = '1';
    });
    root.addEventListener('pointermove', e => {
      if (root.dataset.dragging) setVal(valFromX(e.clientX), true);
    });
    const end = e => {
      if (!root.dataset.dragging) return;
      delete root.dataset.dragging;
      try { if (root.hasPointerCapture(e.pointerId)) root.releasePointerCapture(e.pointerId); } catch {}
    };
    root.addEventListener('pointerup', end);
    root.addEventListener('pointercancel', end);
    root.addEventListener('keydown', e => {
      const step =
        e.key === 'ArrowUp' || e.key === 'ArrowRight' ? 1 :
        e.key === 'ArrowDown' || e.key === 'ArrowLeft' ? -1 :
        e.key === 'PageUp' ? 10 :
        e.key === 'PageDown' ? -10 :
        e.key === 'Home' ? min - val :
        e.key === 'End' ? max - val : 0;
      if (!step) return;
      e.preventDefault();
      setVal(val + step, true);
    });
    Object.defineProperty(root, 'value', { get: () => String(val) });
    commit();
  });
}

export function initPalette() {
  // the sheet's own catalogue anchor is named `palette`, so an id lookup finds
  // the section that documents the component instead of the live palette
  const backdrop = document.querySelector('.palette-backdrop');
  if (!backdrop || !once(backdrop, 'uiPalette')) return;
  const panel = backdrop.querySelector('.palette');
  const input = backdrop.querySelector('.palette-input');
  const list = backdrop.querySelector('.palette-list');
  const empty = backdrop.querySelector('.palette-empty');
  const announcer = backdrop.querySelector('[role="status"]');
  const trigger = document.getElementById('palette-trigger');
  if (!panel || !input || !list) return;
  const rows = [...backdrop.querySelectorAll('.palette-row')];
  const groups = [...backdrop.querySelectorAll('.palette-group')];

  // a row matches on its own name plus its group's label, so "navigate" pulls
  // the whole group. Keycaps stay out of it: nobody searches for a shortcut.
  rows.forEach((row, i) => {
    row.id ||= `palette-opt-${i}`;
    const label = row.closest('.palette-group')?.querySelector('.palette-group-label');
    row.dataset.hay = `${row.querySelector('.palette-name')?.textContent || ''} ${label?.textContent || ''}`
      .toLowerCase().trim().replace(/\s+/g, ' ');
  });

  let shown = rows.slice();
  let activeIdx = 0;
  let lastFocus = null;
  let closeTimer = 0;

  // .closing counts as closed: a second Escape mid-exit must not restart the
  // exit, and a reopen mid-exit must not return early
  const isOpen = () => !backdrop.hidden && !backdrop.classList.contains('closing');

  // a row that opens its group scrolls the group in, or Home lands on a cut-off label
  const reveal = row => {
    const group = row.parentElement;
    const opensGroup = row === group.querySelector(VISIBLE_ROW);
    scrollIntoBox(list, (opensGroup ? group : row).offsetTop, row.offsetTop + row.offsetHeight);
  };

  const markActive = i => {
    if (!shown.length) { input.removeAttribute('aria-activedescendant'); return; }
    activeIdx = (i + shown.length) % shown.length;
    rows.forEach(r => { r.classList.remove('active'); r.setAttribute('aria-selected', 'false'); });
    const row = shown[activeIdx];
    row.classList.add('active');
    row.setAttribute('aria-selected', 'true');
    input.setAttribute('aria-activedescendant', row.id);
    reveal(row);
  };

  const filter = () => {
    const typed = input.value.trim();
    const words = queryWords(typed);
    shown = [];
    rows.forEach(row => {
      const hit = matchesQuery(row.dataset.hay, words);
      row.hidden = !hit;
      if (hit) shown.push(row);
    });
    groups.forEach(g => { g.hidden = !g.querySelector(VISIBLE_ROW); });
    const none = !shown.length;
    list.hidden = none;
    if (empty) empty.hidden = !none;
    input.setAttribute('aria-expanded', String(!none));
    if (none) {
      const msg = words.length > 1
        ? `No match for '${typed}'. Try fewer words.`
        : `No match for '${typed}'. Try another word.`;
      if (empty) empty.textContent = msg;
      // the visible row is content; this region is what announces it, and it
      // stays in the tree so the change lands on a live region
      if (announcer) announcer.textContent = msg;
      input.removeAttribute('aria-activedescendant');
    } else {
      if (announcer) announcer.textContent = '';
      markActive(0);
    }
  };

  const open = () => {
    if (isOpen()) return;
    clearTimeout(closeTimer);
    backdrop.classList.remove('closing');
    panel.classList.remove('closing');
    lastFocus = document.activeElement;
    backdrop.hidden = false;
    input.value = '';
    list.scrollTop = 0;   // a hidden box keeps the last open's scroll position
    filter();
    input.focus();
  };

  const close = () => {
    if (!isOpen()) return;
    clearTimeout(closeTimer);
    closeTimer = runExit(backdrop, panel, () => animMs(panel));
    // focus goes back where it came from, or to the control that names the palette
    const back = lastFocus && lastFocus.isConnected && lastFocus !== document.body ? lastFocus : trigger;
    back?.focus();
  };

  // the palette names the command and closes; what a command does belongs to
  // the page, keyed on data-command rather than on a label that gets reworded
  const run = row => {
    backdrop.dispatchEvent(new CustomEvent('run', {
      bubbles: true,
      detail: { command: row.dataset.command, name: row.querySelector('.palette-name')?.textContent },
    }));
    close();
  };

  panel.addEventListener('keydown', e => {
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); markActive(activeIdx + 1); break;
      case 'ArrowUp':   e.preventDefault(); markActive(activeIdx - 1); break;
      case 'Home':      e.preventDefault(); markActive(0); break;
      case 'End':       e.preventDefault(); markActive(shown.length - 1); break;
      case 'Enter':     if (shown.length) { e.preventDefault(); run(shown[activeIdx]); } break;
      case 'Escape':    e.preventDefault(); close(); break;
      // one focusable element under an aria-modal, so Tab steps the row
      // instead of walking focus into a document assistive tech cannot see
      case 'Tab':       e.preventDefault(); markActive(activeIdx + (e.shiftKey ? -1 : 1)); break;
    }
  });
  input.addEventListener('input', filter);

  // a press anywhere but the query field keeps focus on the query field
  panel.addEventListener('mousedown', e => { if (e.target !== input) e.preventDefault(); });
  backdrop.addEventListener('mousedown', e => { if (e.target === backdrop) e.preventDefault(); });
  backdrop.addEventListener('click', e => { if (e.target === backdrop) close(); });

  rows.forEach(row => {
    row.addEventListener('click', () => run(row));
    row.addEventListener('pointermove', () => {
      const i = shown.indexOf(row);
      if (i >= 0 && i !== activeIdx) markActive(i);
    });
  });

  trigger?.addEventListener('click', open);
  addEventListener('keydown', e => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); open(); }
  });
}

/* ============================= tooltip ============================ */

const tooltipControllers = [];

export function initTooltip() {
  document.querySelectorAll('.tooltip').forEach(tip => {
    const root = tip.parentElement;
    const trigger = root && root.querySelector(`[aria-describedby="${tip.id}"]`);
    if (!root || !trigger || !once(tip, 'uiTooltip')) return;

    let hovering = false, focused = false, hideTimer = 0, hoverTimer = 0;
    const isOpen = () => !tip.hidden;

    const place = () => {
      const r = root.getBoundingClientRect();
      const w = tip.offsetWidth, h = tip.offsetHeight;
      // no room above and room below: open toward the space that exists
      tip.classList.toggle('below', r.top < h + 12 && innerHeight - r.bottom > h + 12);
      let x = (r.width - w) / 2;   // centered on the trigger
      x = Math.max(8 - r.left, Math.min(x, innerWidth - 8 - w - r.left));
      tip.style.left = Math.round(x) + 'px';
    };

    const open = () => {
      clearTimeout(hoverTimer);
      clearTimeout(hideTimer);
      tip.classList.remove('closing');   // a reopen mid-exit restarts the entrance
      if (isOpen()) { place(); return; }
      tip.hidden = false;
      place();
    };
    const close = () => {
      if (!isOpen()) return;
      clearTimeout(hideTimer);
      hideTimer = runExit(tip, null, () => animMs(tip));
    };
    const maybeClose = () => { if (!hovering && !focused) close(); };

    trigger.addEventListener('pointerenter', e => {
      if (e.pointerType !== 'mouse') return;
      hovering = true;
      // hover intent: 300ms, so a scan across the toolbar does not flicker
      clearTimeout(hoverTimer);
      hoverTimer = setTimeout(open, 300);
    });
    trigger.addEventListener('pointerleave', e => {
      if (e.pointerType !== 'mouse') return;
      hovering = false;
      clearTimeout(hoverTimer);
      maybeClose();
    });
    // touch has no hover: the first contact shows it
    trigger.addEventListener('pointerdown', e => { if (e.pointerType !== 'mouse') open(); });
    trigger.addEventListener('focus', () => {
      // click focuses the trigger too, and a pointer focus must not count as the
      // show condition: :focus-visible separates keyboard focus from pointer focus
      focused = trigger.matches(':focus-visible');
      if (focused) open();
    });
    trigger.addEventListener('blur', () => { focused = false; maybeClose(); });
    trigger.addEventListener('keydown', e => { if (e.key === 'Escape') close(); });

    tooltipControllers.push({ root, tip, close, place, isOpen });
  });

  if (once(document.body, 'uiTooltipGlobal')) {
    closeOnOutside(tooltipControllers);
    // an open tooltip re-places on resize; it is anchored to its wrapper,
    // so scrolling carries it with the page
    addEventListener('resize', () => {
      tooltipControllers.forEach(c => { if (c.isOpen()) c.place(); });
    });
  }
}

/* ============================== modal ============================= */

/* A dialog opens from `[data-modal-open="<backdrop id>"]` and closes on Escape, on
   the backdrop's own click, and on any `[data-modal-close]` inside it.

   THE FOCUS PROTOCOL IS THE PART EVERY CONSUMER GOT WRONG, so it lives here rather
   than in prose. On open, every other child of <body> goes `inert`: that is the
   focus trap, with no key handling and no tabbable-list arithmetic, and it also
   takes the page behind out of the accessibility tree. Focus lands on the panel's
   first close control, and returns to the opener at close.

   A MENU THAT OPENS A MODAL STEALS FOCUS BACK, because the menu's own restore runs
   after the dialog has taken it, and Escape then does nothing. Opening on the next
   frame is what orders the two. */
// module scope, not per call: the listeners a first init() registered close over
// this map, so a second init() creating a fresh one would leave initModal.open
// writing to a map nothing reads
const modalState = new WeakMap();

export function initModal() {
  const state = modalState;

  const openers = document.querySelectorAll('[data-modal-open]');
  openers.forEach(btn => {
    if (!once(btn, 'uiModalOpen')) return;
    btn.addEventListener('click', () => {
      const backdrop = document.getElementById(btn.dataset.modalOpen);
      if (backdrop) requestAnimationFrame(() => openModal(backdrop, btn));
    });
  });

  // the palette is a modal backdrop wearing a second class, and its own controller
  // owns its open, close, focus and exit. Two controllers on one element left it
  // visible after a command ran, swallowing every click on the page behind it.
  const dialogs = () => [...document.querySelectorAll('.modal-backdrop:not(.palette-backdrop)')];

  dialogs().forEach(backdrop => {
    if (!once(backdrop, 'uiModal')) return;
    backdrop.addEventListener('click', e => {
      if (e.target === backdrop || e.target.closest('[data-modal-close]')) closeModal(backdrop);
    });
  });

  if (once(document.body, 'uiModalEsc')) {
    addEventListener('keydown', e => {
      if (e.key !== 'Escape') return;
      const open = dialogs().filter(b => !b.hidden && !b.classList.contains('closing')).pop();
      if (open) closeModal(open);
    });
  }

  /* The ambient layer is not page content, so the trap never reaches it: the ring
     and the grid paint, the toast stack still announces, and the drawn scrollbar
     keeps taking a drag. `inert` on that bar would leave the dialog's own scroller
     with no bar to grab, since the platform's is hidden by then. */
  function siblings(backdrop) {
    return [...document.body.children].filter(el =>
      el !== backdrop && el.id !== 'cursor' && el.id !== 'bg-dots' && el.id !== 'toast-stack'
      && !el.classList.contains('ui-scrollbar'));
  }

  function openModal(backdrop, opener) {
    const s = state.get(backdrop) || {};
    clearTimeout(s.timer);
    backdrop.classList.remove('closing');
    backdrop.querySelector('.modal')?.classList.remove('closing');
    s.opener = opener || document.activeElement;
    s.inert = siblings(backdrop).filter(el => !el.inert);
    s.inert.forEach(el => { el.inert = true; });
    state.set(backdrop, s);
    backdrop.hidden = false;
    const focusTarget = backdrop.querySelector('[data-modal-autofocus]')
      || backdrop.querySelector('[data-modal-close]')
      || backdrop.querySelector('.modal');
    focusTarget?.focus?.();
  }

  function closeModal(backdrop) {
    if (backdrop.hidden || backdrop.classList.contains('closing')) return;
    const s = state.get(backdrop) || {};
    const panel = backdrop.querySelector('.modal');
    clearTimeout(s.timer);
    s.timer = runExit(backdrop, panel, () => (panel ? animMs(panel) : 180));
    s.inert?.forEach(el => { el.inert = false; });
    s.inert = null;
    state.set(backdrop, s);
    // an opener inside a popover is hidden the moment its menu folds, and
    // focusing a hidden element does nothing, so the menu's own trigger is
    // the fallback. The .closing branch covers the window where the menu
    // has not hidden yet but will: focus lands on the row and then drops.
    if (s.opener?.isConnected) s.opener.focus();
    const menu = s.opener?.closest?.('.popover');
    if (menu && (menu.hidden || menu.classList.contains('closing'))) {
      document.querySelector(`[aria-controls="${menu.id}"]`)?.focus();
    }
  }

  initModal.open = openModal;
  initModal.close = closeModal;
}

/* ============================== toasts ============================ */

/* THE SPRITE IS THE ONE DRAWING OF EVERY GLYPH. These four were redrawn here
   and each one had drifted from the symbol it stands for: the success tick ran
   M5 8 l2 2 l4 -4 against the sprite's M5 8 L7 10 L11 6, the warning triangle
   sat a half unit lower, and both dots were 0.5-unit strokes where the sprite
   draws a .01 round cap. Nothing could see it, because a redrawn glyph passes
   every guard the sprite has. */
const toastIcons = {
  success: '<svg aria-hidden="true"><use href="#i-success"/></svg>',
  warning: '<svg aria-hidden="true"><use href="#i-warning"/></svg>',
  danger: '<svg aria-hidden="true"><use href="#i-error"/></svg>',
  info: '<svg aria-hidden="true"><use href="#i-info"/></svg>',
};
const toastIconColors = {
  success: 'var(--success)', warning: 'var(--warning)',
  danger: 'var(--danger)', info: 'var(--accent)',
};

/* `title` and `msg` are caller strings: a service name, a log line, a filename.
   They go in as TEXT. The markup around them is the only thing built from a
   template, so a value carrying angle brackets lands as characters, not markup. */
export function showToast(type, title, msg) {
  const stack = document.getElementById('toast-stack');
  if (!stack) return null;
  const t = document.createElement('div');
  t.className = 'toast';
  t.setAttribute('role', 'status');
  t.innerHTML = `
    <div class="toast-icon" style="color:${toastIconColors[type] || toastIconColors.info}">${toastIcons[type] || toastIcons.info}</div>
    <div class="toast-body">
      <div class="toast-title"></div>
      <div class="toast-msg"></div>
    </div>
    <button class="close" aria-label="Dismiss">
      <svg aria-hidden="true"><use href="#i-x"/></svg>
    </button>`;
  t.querySelector('.toast-title').textContent = title;
  t.querySelector('.toast-msg').textContent = msg;
  wireDismiss(t.querySelector('.close'));
  stack.appendChild(t);
  setTimeout(() => removeToast(t), 5000);
  return t;
}

export function removeToast(t) {
  if (!t || !t.parentNode) return;
  t.classList.add('removing');
  setTimeout(() => t.remove(), 110);   // --dur-out-near, the toast's own exit
}

/* `.close` is the one remove, for a toast and for the page-level banner alike. A toast
   this file builds is wired here at creation, because nothing re-runs init() after it:
   the sweep below only ever sees the controls already sitting in the markup. */
const wireDismiss = btn => {
  if (!once(btn, 'uiDismiss')) return;
  btn.addEventListener('click', () => {
    const toast = btn.closest('.toast');
    if (toast) return removeToast(toast);
    const banner = btn.closest('.banner');
    if (!banner) return;
    banner.classList.add('closing');
    setTimeout(() => banner.remove(), animMs(banner));
  });
};

export function initDismiss() {
  document.querySelectorAll('.close').forEach(wireDismiss);
}

/* ============================== theme ============================= */

/* The saved theme is restored in <head> before the stylesheet loads, so a light
   visitor never sees a dark flash; that snippet is in SKILL.md and cannot live
   here, because a module runs too late to beat the first paint. */
export function setTheme(t) {
  document.documentElement.dataset.theme = t;
  try { localStorage.setItem('osu-collect-theme', t); } catch { /* private mode */ }
  document.querySelectorAll('[data-theme-toggle]').forEach(b => {
    b.setAttribute('aria-pressed', String(t === 'light'));
    b.setAttribute('aria-label', t === 'light' ? 'Switch to dark theme' : 'Switch to light theme');
  });
}

/* Exported, because the button is not the only thing that swaps a theme: a palette
   command and a keyboard shortcut both want to call this, and inlining it in the
   click handler is what made a consumer write its own copy. */
export function toggleTheme() {
  const next = document.documentElement.dataset.theme === 'light' ? 'dark' : 'light';
  // the crossfade snapshots the old frame and fades the two together; a
  // browser without view transitions gets the same swap on the spot
  if (!document.startViewTransition) return setTheme(next);
  document.startViewTransition(() => setTheme(next));
}

export function initTheme() {
  setTheme(document.documentElement.dataset.theme === 'light' ? 'light' : 'dark');
  document.querySelectorAll('[data-theme-toggle]').forEach(btn => {
    if (!once(btn, 'uiTheme')) return;
    btn.addEventListener('click', toggleTheme);
  });
}

/* ====================== reveals and sticky headers ====================== */

/* --vp-w is the viewport width minus a classic scrollbar, which 100vw wrongly
   includes. Every full-bleed rule reads it (.band, .section-sticky.is-stuck), so
   it is written unconditionally: the old split, where only the reveal controller
   wrote it, shipped a horizontal scrollbar on any page with a sticky header and
   no reveals. In a sidebar shell, call this with the scrolling column instead. */
export function initViewportWidth(el = document.documentElement) {
  const set = () => el.style.setProperty('--vp-w', el.clientWidth + 'px');
  set();
  // the guard is per element, not global: a sidebar shell calls this a second time
  // for its scrolling column, and a global latch would leave that column's --vp-w
  // correct at boot and frozen on every resize after it
  if (once(el === document.documentElement ? document.body : el, 'uiVpw')) {
    addEventListener('resize', set);
  }
}

/* A 1px sentinel above each header toggles .is-stuck. Shipped in two places
   before this file existed, so a page pasting both blocks inserted two sentinels
   per header and toggled the class twice per scroll.

   A SENTINEL IS OUT OF VIEW IN BOTH DIRECTIONS, so `!isIntersecting` alone reads
   "the reader has not reached this section yet" as "this header is stuck". The
   stuck costume is a full-bleed page-toned band, so the reference sheet's one
   header painted that band at its natural position for the whole scroll and
   dropped it on arrival, the state backwards from top to bottom: measured, stuck
   with its top 14080px below the fold and unstuck with its top at 80. Which side
   the sentinel left on is the whole state, so the side is what the test reads. */
export function initSticky() {
  document.querySelectorAll('.section-sticky').forEach(h => {
    if (!once(h, 'uiSticky')) return;
    const sentinel = document.createElement('div');
    h.parentNode.insertBefore(sentinel, h);
    /* THE PIN OFFSET IS A BREAKPOINT, not a constant: the header pins under the
       navbar at 56px above 768 and under the mobile topbar at 52 below it, and
       both the rootMargin and the side test are built from it. Read once, a
       viewport crossing 768 left the class flipping 4px early, exactly the
       difference between the two bars. `initViewportWidth` in this same section
       already re-reads on resize for the same reason. */
    let io = null;
    const arm = () => {
      const top = parseInt(getComputedStyle(h).top) || 0;
      if (io) io.disconnect();
      io = new IntersectionObserver(
        ([e]) => h.classList.toggle('is-stuck',
          !e.isIntersecting && e.boundingClientRect.top < top + 1),
        { rootMargin: `-${top + 1}px 0px 0px 0px`, threshold: 1 }
      );
      io.observe(sentinel);
    };
    arm();
    addEventListener('resize', arm);
  });
}

/* Plays each target once, in batches of what arrives together: when it crosses
   the line 10% above the bottom edge, or when it is on screen and no scroller
   holding it can lift it any higher. Without the second clause a short element
   at a page's end, or one low on a page that cannot scroll, stays hidden for
   good: measured, a 40px chart as the sheet's last element sat at 859-899px of
   a 900px window at full scroll, under the 810px line. Returns the call that
   plays a batch from outside, for the reveal's net. */
function watchEntrance(targets, play) {
  const pending = new Set(targets);
  const onScreen = new Set();
  const settle = els => {
    const batch = els.filter(el => pending.delete(el));
    batch.forEach(el => {
      onScreen.delete(el);
      line.unobserve(el);
      seen.unobserve(el);
    });
    if (!pending.size) {
      removeEventListener('scroll', sweep, true);
      removeEventListener('resize', sweep);
    }
    if (batch.length) play(batch);
  };
  function sweep() { settle([...onScreen].filter(cannotRise)); }
  const line = new IntersectionObserver(
    entries => settle(entries.filter(e => e.isIntersecting).map(e => e.target)),
    { threshold: 0, rootMargin: '0px 0px -10% 0px' });
  const seen = new IntersectionObserver(entries => {
    entries.forEach(e => e.isIntersecting ? onScreen.add(e.target) : onScreen.delete(e.target));
    sweep();
  }, { threshold: 0 });
  addEventListener('scroll', sweep, { capture: true, passive: true });
  addEventListener('resize', sweep);
  targets.forEach(el => {
    line.observe(el);
    seen.observe(el);
  });
  return settle;
}

/* every scroller holding the element already sits at its end */
const cannotRise = el => {
  for (let n = el.parentElement; n && n !== document.body; n = n.parentElement) {
    const overflowY = getComputedStyle(n).overflowY;
    const scrolls = overflowY === 'auto' || overflowY === 'scroll';
    if (scrolls && n.scrollTop < n.scrollHeight - n.clientHeight - 1) return false;
  }
  const page = document.scrollingElement;
  return page.scrollTop >= page.scrollHeight - page.clientHeight - 1;
};

/* JS adds the hiding class at runtime, so no-JS keeps the content visible.
   Reduced motion skips the hide entirely rather than zeroing a duration. */
export function initReveal() {
  if (reducedMotion() || !('IntersectionObserver' in window)) return;

  const targets = [...document.querySelectorAll('[data-reveal]')].filter(el => once(el, 'uiReveal'));
  if (!targets.length) return;
  targets.forEach(el => el.classList.add('reveal'));   // hiding class added by JS, never authored

  /* The stagger belongs to what arrives TOGETHER, so it is applied at reveal time
     rather than at init. A document-order index makes a section that crosses the
     trigger on its own wait for a position the reader cannot see: measured on the
     sheet, section 5 first moved at 250ms and settled at 399, its neighbour 6
     moved at 60 and settled at 211, on no grouping reason. 40ms, capped at six. */
  const land = els => els.forEach((el, i) => {
    el.style.transitionDelay = Math.min(i, 5) * 40 + 'ms';
    el.classList.add('in');
  });
  const settle = watchEntrance(targets, land);

  /* The net covers an observer that never fired, so it reveals only what is ON
     SCREEN when it lands. Revealing every target instead settles the whole page
     before the reader reaches any of it: measured on the rendered sheet, 44 of
     46 sections were off screen at 1500ms, so no section could ever animate. */
  setTimeout(() => settle(targets.filter(el => {
    const r = el.getBoundingClientRect();
    return r.top < innerHeight && r.bottom > 0;
  })), 1500);
}

/* ============================= chart entrance ============================= */

/* JS adds the hiding class at runtime, so a page with no controller paints its
   marks instead of nothing. `.is-drawn` lands once, when the chart intersects
   the viewport, and the observer unobserves it: the entrance never replays on a
   second scroll past. */
/* A card whose whole surface is the control cannot be a `<button>`: its content
   is block-level, and a button may not hold a heading or a paragraph. So it is a
   div with `role="button"` and a tabindex, and a div gets no activation for
   free: Enter and Space are wired here, and Space's own default is refused
   because it scrolls the page. Reaching the card is the consumer's half of the
   contract (the two attributes); activating it is this system's. */
function initCardButtons() {
  document.querySelectorAll('.card-interactive[role="button"]').forEach(card => {
    if (!once(card, 'uiCardButton')) return;
    if (!card.hasAttribute('tabindex')) card.tabIndex = 0;
    card.addEventListener('keydown', e => {
      if (e.key !== 'Enter' && e.key !== ' ') return;
      e.preventDefault();
      card.click();
    });
  });
}

/* A legend built from buttons is a control, and `charts.md` had described this
   one for a round before anything wrote it: every key on every page was inert
   text beside a swatch. The toggle names its slot with the same
   `chart-series-N` class the mark carries, so the pair cannot drift, and it
   finds its chart by walking up to the first ancestor holding one, which lets a
   legend sit either side of the figure with nothing to wire. */
function initLegend() {
  document.querySelectorAll('.legend-toggle').forEach(btn => {
    const slot = [...btn.classList].find(c => /^chart-series-\d+$/.test(c));
    if (!slot || !once(btn, 'uiLegend')) return;
    let chart = null;
    for (let p = btn.parentElement; p && !chart; p = p.parentElement) chart = p.querySelector('.chart');
    if (!chart) return;
    btn.addEventListener('click', () => {
      const on = btn.getAttribute('aria-pressed') !== 'false';
      btn.setAttribute('aria-pressed', String(!on));
      chart.querySelectorAll(`.chart-series.${slot}`).forEach(s => s.classList.toggle('is-off', on));
    });
  });
}

/* A line's area and its end point are the line's own geometry, so a page writes
   the polyline and this derives the other two: the flat tint down to the plot's
   floor, under the line, and the round mark on its last point, over it. Both are
   read off the `points` attribute rather than kept, so a page that redraws a line
   rewrites that one attribute and the pair follows; an emptied or removed line
   takes its pair with it, or a tint would outlive the line it stood under. A write through
   the `SVGPointList` API fires no mutation, measured in Chrome, so it redraws
   nothing. A sparkline takes the same pair. A `<path>` line has no point list to
   read, and gets neither. */
let lineMarks = null;
function drawLineMarks(line) {
  const n = line.points?.numberOfItems;
  const box = line.ownerSVGElement?.viewBox.baseVal;
  const parent = line.parentNode;
  if (!parent) return;
  if (!n || !box?.height) {
    parent.querySelectorAll(':scope > .chart-area, :scope > .chart-point.chart-end').forEach(m => m.remove());
    return;
  }
  const pts = Array.from({ length: n }, (_, i) => line.points.getItem(i));
  const floor = box.y + box.height;
  const first = pts[0];
  const last = pts[n - 1];
  const area = ownChild(parent, 'chart-area', { tag: 'polygon', svg: true, first: true });
  area.setAttribute('points',
    [...pts.map(p => `${p.x},${p.y}`), `${last.x},${floor}`, `${first.x},${floor}`].join(' '));
  const end = ownChild(parent, 'chart-point chart-end', { tag: 'line', svg: true });
  end.setAttribute('x1', last.x);
  end.setAttribute('y1', last.y);
  end.setAttribute('x2', last.x);
  end.setAttribute('y2', last.y);
}
function initLineMarks() {
  document.querySelectorAll('.chart-series > .chart-line, .stat-spark svg > polyline').forEach(line => {
    if (!once(line, 'uiLineMarks')) return;
    lineMarks ||= new MutationObserver(records => records.forEach(r => {
      if (r.type === 'attributes') return drawLineMarks(r.target);
      if (![...r.removedNodes].some(n => n.matches?.('.chart-line, polyline'))) return;
      if (r.target.querySelector(':scope > .chart-line, :scope > polyline')) return;
      r.target.querySelectorAll(':scope > .chart-area, :scope > .chart-point.chart-end').forEach(m => m.remove());
    }));
    drawLineMarks(line);
    lineMarks.observe(line, { attributes: true, attributeFilter: ['points'] });
    lineMarks.observe(line.parentNode, { childList: true });
  });
}

export function initChart() {
  initLegend();
  initLineMarks();
  const charts = [...document.querySelectorAll('.chart')].filter(el => once(el, 'uiChart'));
  if (!charts.length) return;

  // the hiding class lands in this same synchronous task, before the first paint
  charts.forEach(el => el.classList.add('will-draw'));

  if (!('IntersectionObserver' in window)) {
    charts.forEach(el => el.classList.add('is-drawn'));
    return;
  }
  watchEntrance(charts, els => els.forEach(el => el.classList.add('is-drawn')));
}

/* ========================== popovers and menus ========================== */

/* Any `.popover` opened by a button that names it with `aria-controls` AND
   `aria-haspopup`. Covers the table's filter menu and the context menu, and it is
   what `.popover` needed to stop being an entrance with no exit: the exit runs the
   `.closing` costume for the popover's own animation duration.

   THE GLOBAL HANDLERS TOUCH ONLY THE PAIRS THIS CONTROLLER WIRED. `.popover` is
   also the shell the palette and the select panel are built from, so a handler
   written against every `.popover` on the page reaches into components that own
   their own open state: it hid the palette's panel behind its own backdrop, and
   the next open showed an empty dialog that swallowed every click on the page. */
const popoverPairs = [];

export function initPopovers() {
  const close = (menu, btn) => {
    if (menu.hidden || menu.classList.contains('closing')) return;
    btn?.setAttribute('aria-expanded', 'false');
    menu.querySelectorAll('.menu-item.focus').forEach(r => r.classList.remove('focus'));
    menu.uiPopoverExit = runExit(menu, null, () => animMs(menu));
  };

  document.querySelectorAll('[aria-controls][aria-haspopup]').forEach(btn => {
    const menu = document.getElementById(btn.getAttribute('aria-controls'));
    if (!menu || !menu.classList.contains('popover') || !once(btn, 'uiPopover')) return;
    popoverPairs.push({ btn, menu, close });

    const show = () => {
      clearTimeout(menu.uiPopoverExit);
      menu.classList.remove('closing');
      menu.hidden = false;
      btn.setAttribute('aria-expanded', 'true');
    };

    // a checkbox row has to survive several picks, so an input inside the
    // item keeps the menu open; a button row is an action, and an action
    // row that opens a modal leaves a menu open behind the dialog, whose
    // Escape handler then eats the dialog's
    menu.addEventListener('click', e => {
      const item = e.target.closest?.('.menu-item');
      if (!item || (isControl(e.target) && !e.target.closest?.('button.menu-item'))) return;
      close(menu, btn);
      btn.focus();
    });
    btn.addEventListener('click', () => {
      if (!menu.hidden) return close(menu, btn);
      show();
    });

    // the keyboard model and the row focus belong to the menu costume alone;
    // a bare popover keeps the base open / outside-click / Escape / exit
    if (!menu.classList.contains('popover-menu')) return;

    const rows = [...menu.querySelectorAll('.menu-item')];
    // the menu-button pattern: the button is the one Tab stop, rows are focusable
    // by script only, and a checkbox row's input is never a second target; the
    // box stays the state and the row's aria-checked mirrors it
    rows.forEach(row => {
      row.tabIndex = -1;
      const input = row.querySelector('input');
      if (input) {
        input.tabIndex = -1;
        // the menuitemcheckbox row is the one checkbox a screen reader hears;
        // the input stays the state and is never announced or Tab-stopped
        input.setAttribute('aria-hidden', 'true');
        const sync = () => row.setAttribute('aria-checked', String(input.checked));
        sync();
        input.addEventListener('change', sync);
      }
    });
    let focusIdx = 0;
    const isOpen = () => !menu.hidden && !menu.classList.contains('closing');
    const activeIndex = () => {
      const at = rows.findIndex(r => r.classList.contains('active'));
      return at >= 0 ? at : 0;
    };
    const focusRow = idx => {
      if (!rows.length) return;
      focusIdx = ((idx % rows.length) + rows.length) % rows.length;
      rows[focusIdx].focus();
    };
    // the .focus mark follows the real focus wherever it lands, and the row is
    // the only thing that ever holds focus: a pointer click focuses the checkbox
    // input, which is aria-hidden, so focus moves to its row instead
    menu.addEventListener('focusin', e => {
      const row = e.target.closest?.('.menu-item');
      if (!row) return;
      if (e.target.matches?.('input')) { row.focus(); return; }
      const pos = rows.indexOf(row);
      if (pos >= 0) {
        focusIdx = pos;
        rows.forEach(r => r.classList.toggle('focus', r === rows[pos]));
      }
    });
    // Enter/Space fire the row's own click, so a consumer's listener on it runs;
    // the menu's click listener above then closes it. A checkbox row toggles its
    // box and stays open.
    const activate = () => {
      const row = rows[focusIdx];
      if (!row) return;
      const input = row.querySelector('input');
      if (input) { input.click(); return; }
      row.click();
    };
    const open = (toLast = false) => {
      if (isOpen()) return;
      show();
      focusRow(toLast ? rows.length - 1 : activeIndex());
    };

    // focus is on the button: closed keys open, and a pointer-opened menu moves
    // focus into the rows on the first arrow
    btn.addEventListener('keydown', e => {
      switch (e.key) {
        case 'ArrowDown': e.preventDefault(); isOpen() ? focusRow(activeIndex()) : open(false); break;
        case 'ArrowUp':   e.preventDefault(); isOpen() ? focusRow(rows.length - 1) : open(true); break;
        case 'Home':      if (isOpen()) { e.preventDefault(); focusRow(0); } break;
        case 'End':       if (isOpen()) { e.preventDefault(); focusRow(rows.length - 1); } break;
        case 'Enter':
        case ' ':         e.preventDefault(); isOpen() ? close(menu, btn) : open(false); break;
        case 'Tab':       close(menu, btn); break;
      }
    });
    // focus is on a row: the arrows walk it, Enter/Space activate, Escape is the
    // global handler's, and Tab closes while the default moves focus on
    menu.addEventListener('keydown', e => {
      switch (e.key) {
        case 'ArrowDown': e.preventDefault(); focusRow(focusIdx + 1); break;
        case 'ArrowUp':   e.preventDefault(); focusRow(focusIdx - 1); break;
        case 'Home':      e.preventDefault(); focusRow(0); break;
        case 'End':       e.preventDefault(); focusRow(rows.length - 1); break;
        case 'Enter':
        case ' ':         e.preventDefault(); activate(); break;
        case 'Tab':       close(menu, btn); break;
      }
    });
  });
  if (!once(document.body, 'uiPopoverGlobal')) return;

  const openPairs = () => popoverPairs.filter(p => !p.menu.hidden);
  document.addEventListener('pointerdown', e => {
    openPairs().forEach(({ btn, menu }) => {
      if (menu.contains(e.target) || btn.contains(e.target)) return;
      close(menu, btn);
      // focus moves into the rows now, so an outside press must not leave it on
      // a row the exit is about to hide
      if (menu.contains(document.activeElement)) btn.focus();
    });
  });
  document.addEventListener('keydown', e => {
    if (e.key !== 'Escape') return;
    const open = openPairs();
    if (!open.length) return;
    e.stopPropagation();          // a menu's Escape belongs to the menu, not to a dialog behind it
    open.forEach(({ btn, menu }) => { close(menu, btn); btn.focus(); });
  });
}

/* ============================== tables ============================= */

/* The DOM half of sort, selection and paging. The data half belongs to the page,
   which renders rows and listens for these events:
     `table:sort`      {key, dir}   dir is 'asc' | 'desc' | 'none'
     `table:selection` {selected}   the data-name values currently checked
     `table:page`      {page}
   Every one of them bubbles from the `.table-wrap`. Nothing here needs to know
   what a row holds, which is why it can wire every table on every page. */
const ROW_BOX = 'input[type="checkbox"][data-name]';

export function initTables() {
  const ariaSort = { asc: 'ascending', desc: 'descending' };

  document.querySelectorAll('.table-wrap').forEach(wrap => {
    // outside the once() guard, so a table that re-renders its head is wrapped again.
    // A sortable cell's text lives in the button, not the cell, so both are offered.
    wrap.querySelectorAll('thead th, thead .th-sort').forEach(el => ownLabel(el, 'th-label'));
    if (!once(wrap, 'uiTable')) return;
    const state = { key: null, dir: 'none' };

    wrap.querySelectorAll('.th-sort').forEach(btn => {
      btn.addEventListener('click', () => {
        const key = btn.dataset.sort;
        const th = btn.closest('th');
        if (state.key === key) {
          state.dir = state.dir === 'none' ? 'asc' : state.dir === 'asc' ? 'desc' : 'none';
        } else { state.key = key; state.dir = 'asc'; }
        wrap.querySelectorAll('th[aria-sort]').forEach(h => {
          h.setAttribute('aria-sort', h === th && state.dir !== 'none' ? ariaSort[state.dir] : 'none');
        });
        wrap.dispatchEvent(new CustomEvent('table:sort', { bubbles: true, detail: { ...state } }));
      });
    });

    const selectAll = wrap.querySelector('input[type="checkbox"][data-select-all], thead input[type="checkbox"]');
    const boxes = () => [...wrap.querySelectorAll(`tbody ${ROW_BOX}`)];
    const selected = () => boxes().filter(b => b.checked).map(b => b.dataset.name);
    const syncAll = () => {
      if (!selectAll) return;
      const all = boxes();
      const n = all.filter(b => b.checked).length;
      selectAll.checked = all.length > 0 && n === all.length;
      selectAll.indeterminate = n > 0 && n < all.length;
    };
    const announce = () => wrap.dispatchEvent(new CustomEvent('table:selection',
      { bubbles: true, detail: { selected: selected() } }));

    selectAll?.addEventListener('change', () => {
      boxes().forEach(b => {
        b.checked = selectAll.checked;
        // the class toggles in place, so the selection tint transitions
        b.closest('tr')?.classList.toggle('row-selected', b.checked);
      });
      syncAll();
      announce();
    });
    wrap.addEventListener('change', e => {
      const b = e.target.closest?.(`tbody ${ROW_BOX}`);
      if (!b) return;
      b.closest('tr')?.classList.toggle('row-selected', b.checked);
      syncAll();
      announce();
    });
    // the row is the target, the same ruling the setting row runs on: a click
    // anywhere on a selectable row that missed a control drives its own box
    wrap.addEventListener('click', e => {
      if (isControl(e.target)) return;
      e.target.closest('tr')?.querySelector(ROW_BOX)?.click();
    });

    // a re-rendered body arrives with its own checked state: re-derive rather
    // than carry the last render's count
    new MutationObserver(syncAll).observe(wrap, { childList: true, subtree: true });
    syncAll();
  });

  /* A pager usually sits OUTSIDE the .table-wrap it drives, so an event dispatched
     from it bubbles past the wrap and never reaches a listener there. Name the
     table with `aria-controls` and the event lands on that table's wrap, next to
     `table:sort` and `table:selection`. With no `aria-controls` it fires on the
     pager itself, which is then where the page has to listen. */
  document.querySelectorAll('.pagination').forEach(pager => {
    if (!once(pager, 'uiPager')) return;
    pager.addEventListener('click', e => {
      const b = e.target.closest('button[data-page]');
      if (!b || b.disabled) return;
      const named = pager.getAttribute('aria-controls');
      const target = (named && document.getElementById(named)?.closest('.table-wrap'))
        || b.closest('.table-wrap') || pager;
      target.dispatchEvent(new CustomEvent('table:page',
        { bubbles: true, detail: { page: Number(b.dataset.page) } }));
    });
  });
}

/* ============================= code copy =========================== */

/* Copies the block's text and swaps to a check for a beat. A failed write leaves
   the button unchanged; there is no state to celebrate. */
export function initCodeCopy() {
  document.querySelectorAll('.code-copy').forEach(btn => {
    const code = btn.closest('.code-body')?.querySelector('code');
    if (!code || !once(btn, 'uiCopy')) return;
    const iconCopy = btn.querySelector('.icon-copy');
    const iconCheck = btn.querySelector('.icon-check');
    let timer = 0;
    btn.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(code.textContent);
      } catch (err) {
        console.warn('copy failed', err);
        return;
      }
      if (iconCopy) iconCopy.hidden = true;
      if (iconCheck) iconCheck.hidden = false;
      btn.setAttribute('aria-label', 'Copied');
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (iconCopy) iconCopy.hidden = false;
        if (iconCheck) iconCheck.hidden = true;
        btn.setAttribute('aria-label', 'Copy code');
      }, 1200);
    });
  });
}

/* ======================= typed placeholder ======================= */

/* The third sanctioned ambient effect. An overlay sitting on the field types one
   real example, holds it, deletes it, and moves to the next. Ported from the
   contact form it was built for; the timings are that build's, unchanged.

   A field declares its own examples and nothing else:
     <input class="input" data-typed='["you@example.com", "discord: cool_person"]'>

   Three things the obvious version gets wrong:
   - It renders REAL example values, so it is content rather than decoration, and
     it takes a text colour that clears 4.5:1 rather than a placeholder grey.
   - A value in the field hides the overlay, so the loop parks at the frame before
     the next example instead of running invisibly, and resumes on the next clear.
     Resuming moves forward, so the interrupted example does not replay.
   - Autofill never fires `input`, so a value already present at boot is checked. */
const TYPE_CHAR = 80, TYPE_ERASE = 50, TYPE_HOLD = 5000, TYPE_GAP = 400, TYPE_START = 600;

export function initTypedPlaceholder() {
  // ambient motion is off under reduced motion, and this one has no reduced form:
  // a typed line with the typing removed is just a placeholder the field already has
  if (reducedMotion()) return;

  document.querySelectorAll('input[data-typed], textarea[data-typed]').forEach(input => {
    if (!once(input, 'uiTyped')) return;
    let examples;
    try {
      examples = JSON.parse(input.dataset.typed);
    } catch {
      console.warn('data-typed is not JSON on', input);
      return;
    }
    if (!Array.isArray(examples) || !examples.length) return;

    const wrap = ownWrap(input, 'typed-wrap');
    // a marker this module owns, aria-hidden like the inks: the field's own
    // label names it, so the typed examples need no second voice
    const overlay = ownChild(wrap, 'typed-overlay');

    let idx = 0, timer = 0, paused = false;

    // one span per character, so each one fades on its own clock
    const build = text => {
      overlay.textContent = '';
      const frag = document.createDocumentFragment();
      for (const ch of text) {
        const span = document.createElement('span');
        span.className = 'typed-char';
        span.textContent = ch;
        frag.appendChild(span);
      }
      overlay.appendChild(frag);
      return [...overlay.children];
    };

    const type = (spans, i, done) => {
      if (i >= spans.length) { timer = setTimeout(done, TYPE_HOLD); return; }
      spans[i].style.opacity = '1';
      timer = setTimeout(() => type(spans, i + 1, done), TYPE_CHAR);
    };
    const erase = (spans, i, done) => {
      if (i < 0) { done(); return; }
      spans[i].style.opacity = '0';
      timer = setTimeout(() => erase(spans, i - 1, done), TYPE_ERASE);
    };
    const cycle = () => {
      const spans = build(examples[idx]);
      new Promise(done => type(spans, 0, done))
        .then(() => new Promise(done => erase(spans, spans.length - 1, done)))
        .then(() => {
          idx = (idx + 1) % examples.length;
          timer = setTimeout(cycle, TYPE_GAP);
        });
    };

    const pause = () => {
      paused = true;
      clearTimeout(timer);
      overlay.textContent = '';
      idx = (idx + 1) % examples.length;   // resume moves forward, never replays
    };
    const resume = () => { paused = false; timer = setTimeout(cycle, TYPE_GAP); };

    input.addEventListener('input', () => {
      const filled = !!input.value;
      overlay.hidden = filled;
      if (filled && !paused) pause();
      else if (!filled && paused) resume();
    });

    if (input.value) { overlay.hidden = true; pause(); }
    else timer = setTimeout(cycle, TYPE_START);
  });
}

/* ============================ scroll fade =========================== */

/* The cue that a scroller has more. The scrollers carry it themselves, so no
   consumer opts in: this one selector names every component the stylesheet makes
   scroll, and the controller writes the cue class onto each. The axis is derived
   from what the element actually scrolls, never from a class the author wrote.
   A consumer scroller opts in with `data-scroll-fade`; `no-scroll-fade` on any
   scroller is the one opt-out. Floating panels never carry it: the select menu
   and the palette scroll bare, her call 2026-09-23. Off at rest, so a no-JS
   page paints no cue. The two has-more classes are the CSS's own gate. */
const SCROLL_FADE_SELECTOR = 'pre, .table-wrap, .chart-scroll, .modal-body, .log-view, .pagination-pages, .sidebar-nav, .navbar-nav, [data-scroll-fade]';

/* One resize listener and one observer serve every scroller on the page. A
   consumer that swaps markup (an htmx partial on every tick) replaces a
   scroller each time, and a per-element listener plus observation would hold
   every replaced node in memory for the page's life. */
const fadeSyncs = new Map();
const fadeObserver = new ResizeObserver(entries => {
  for (const entry of entries) fadeSyncs.get(entry.target)?.();
});
let fadeResizeBound = false;

export function initScrollFade() {
  document.querySelectorAll(SCROLL_FADE_SELECTOR).forEach(el => {
    if (el.classList.contains('no-scroll-fade') || !once(el, 'uiFade')) return;
    el.classList.add('scroll-fade');
    const sync = () => {
      const y = el.scrollHeight > el.clientHeight + 1;
      el.classList.toggle('scroll-fade-y', y);
      const pos  = y ? el.scrollTop    : el.scrollLeft;
      const view = y ? el.clientHeight : el.clientWidth;
      const full = y ? el.scrollHeight : el.scrollWidth;
      el.classList.toggle('has-more-start', pos > 1);
      el.classList.toggle('has-more-end', pos + view < full - 1);
    };
    fadeSyncs.set(el, sync);
    el.addEventListener('scroll', sync, { passive: true });
    /* The box changes size with no scroll and no window resize: the rail collapse
       takes this nav's clientHeight 756 to 722, which left the cue saying nothing
       was below with 34px still there, until the next scroll. */
    fadeObserver.observe(el);
    sync();
  });
  if (!fadeResizeBound) {
    addEventListener('resize', () => {
      for (const [el, sync] of fadeSyncs) if (el.isConnected) sync();
    });
    fadeResizeBound = true;
  }
  for (const [el, sync] of fadeSyncs) {
    if (el.isConnected) continue;
    el.removeEventListener('scroll', sync);
    fadeObserver.unobserve(el);
    fadeSyncs.delete(el);
  }
}

/* ============================ scrollbar =========================== */

/* The page draws its own, because `cursor: none` cannot reach a native
   scrollbar's shadow tree and the platform pointer paints over the ring for as
   long as one is dragged. `components.css` hides the native bars; this puts a
   real element back, so the ring paints over it like any other surface.

   ONE bar per axis for the whole page, moved onto whichever scroller is active,
   which is what an overlay scrollbar already is. Per-scroller bars would need a
   positioned wrapper around every scrolling component in the system.

   It appears on any pointer move inside a scroller and on a scroll, and retires
   `SCROLLBAR_IDLE` after both stop, her call 2026-09-02. Two things hold it past
   that: a drag, and the pointer resting on the thumb itself, or the bar would
   retire out from under a pointer on its way to grab it. */
const SCROLLBAR_IDLE = 900;
const SCROLLBAR_MIN = 28;    // a thumb shorter than this stops reading as a handle
const SCROLLBAR_GAP = 2;     // the scroller's own edge to the thumb's outer edge
const SCROLLBAR_LANE = 8;    // and on to its inner edge: the 6px thumb plus that gap.
                             // The lane is where the bar is placed, hit-tested and held
const SCROLLBAR_HOLD = 10;   // slop around a 6px thumb, so the hold is reachable
const SCROLLBAR_Z = 999;     // the bar's own slot in the stacking ladder
const SCROLLBAR_CHAIN = 1;   // px of gap a cover chain still composes through:
                             // sub-pixel rounding in a stuck header's rect, never
                             // a real strip of visible page

export function initScrollbars() {
  if (!once(document.documentElement, 'uiScrollbars')) return;
  const root = document.scrollingElement || document.documentElement;
  const bars = {
    y: ownChild(document.body, 'ui-scrollbar ui-scrollbar-y'),
    x: ownChild(document.body, 'ui-scrollbar ui-scrollbar-x'),
  };
  // the stylesheet hides the platform's bars only once these two exist
  document.documentElement.classList.add('has-drawn-scrollbars');

  const view = (el, axis) => axis === 'y'
    ? (el === root ? innerHeight : el.clientHeight)
    : (el === root ? innerWidth : el.clientWidth);
  const full = (el, axis) => axis === 'y' ? el.scrollHeight : el.scrollWidth;
  const at = (el, axis) => axis === 'y' ? el.scrollTop : el.scrollLeft;

  const scrolls = (el, axis) => {
    /* `no-scrollbar` opts a scroller out of the drawn bar: it keeps scrolling,
       and the bar keeps riding the page or column it belongs to. A surface
       that reads as fixed chrome carries it, so it never grows a bar of its
       own. */
    if (el.closest('.no-scrollbar')) return false;
    if (full(el, axis) <= view(el, axis) + 1) return false;
    if (el === root) return true;
    const ov = getComputedStyle(el)[axis === 'y' ? 'overflowY' : 'overflowX'];
    return ov === 'auto' || ov === 'scroll';
  };
  /* The INNERMOST scroller ON THIS AXIS under a node, so a table inside a modal
     wins and a table that scrolls only x leaves the y bar with the page. Asking
     one question for both axes answered the wrong one: a wide table inside a
     tall page took BOTH bars, and the page's own y bar went with it. */
  const scrollerFrom = (node, axis) => {
    for (let el = node; el instanceof Element; el = el.parentElement) {
      if (scrolls(el, axis)) return el;
    }
    return root;
  };
  const boxOf = el => el === root
    ? { top: 0, left: 0, right: innerWidth, bottom: innerHeight }
    : el.getBoundingClientRect();

  /* A rounded panel takes its own corner out from under the thumb, so a track
     running the full length leaves the thumb's last pixels outside the paint.
     The corner is often not the scroller's own: a modal body scrolls inside the
     modal's radius and declares none itself, so the radius comes from every box
     that shares the edge the bar runs along. The thumb's outer edge sits
     SCROLLBAR_GAP in from that edge, and a corner of radius r is inset by more
     than the gap for the first r - sqrt(2*r*gap - gap^2) px along it: 6.79px on
     the 14px corner of --r-2 at a 2px gap. The track gives that up at each end.

     Cached per element, because it is layout that decides which ancestors share
     an edge and the walk would otherwise run on every frame of a scroll. */
  const inset = r => (r <= SCROLLBAR_GAP ? 0
    : r - Math.sqrt(2 * r * SCROLLBAR_GAP - SCROLLBAR_GAP * SCROLLBAR_GAP));
  let radii = new WeakMap();
  const corner = (el, axis) => {
    let cached = radii.get(el);
    if (!cached) radii.set(el, cached = {});
    if (cached[axis]) return cached[axis];
    const pads = [0, 0];
    if (el !== root) {
      const own = el.getBoundingClientRect();
      const px = v => (v.includes('px') ? parseFloat(v) || 0 : 0);
      const same = (a, b) => Math.abs(a - b) <= 1;
      for (let a = el; a instanceof Element; a = a.parentElement) {
        const r = a.getBoundingClientRect();
        const cs = getComputedStyle(a);
        if (axis === 'y' && same(r.right, own.right)) {
          if (same(r.top, own.top)) pads[0] = Math.max(pads[0], inset(px(cs.borderTopRightRadius)));
          if (same(r.bottom, own.bottom)) pads[1] = Math.max(pads[1], inset(px(cs.borderBottomRightRadius)));
        } else if (axis === 'x' && same(r.bottom, own.bottom)) {
          if (same(r.left, own.left)) pads[0] = Math.max(pads[0], inset(px(cs.borderBottomLeftRadius)));
          if (same(r.right, own.right)) pads[1] = Math.max(pads[1], inset(px(cs.borderBottomRightRadius)));
        }
      }
    }
    cached[axis] = pads;
    return pads;
  };

  /* Chrome pinned over the scrollport hides the track under it, and a thumb
     drawn there marks a position nobody can scroll to. Her call 2026-09-02, and
     the covers are DERIVED rather than named: every visible fixed or sticky box
     is a candidate, and one counts against this bar only where it overlaps the
     bar's own lane and sits below the bar in the stacking ladder.

     A cover counts only CONTIGUOUS with the run it joins: it may start no
     further than SCROLLBAR_CHAIN past the end already given up, which is what
     lets a header stuck flush under a navbar extend the run the navbar
     started. A navbar floating clear of the viewport edge as a card counts
     for nothing, because the strip of page above it is fully visible and the
     thumb's origin must not move with the viewport width; an unstuck sticky
     header sitting mid-page stays out of the sum the same way. The ambient
     layer drops out by taking no pointer, the scroller's own pinned ancestors
     by containment: the sidebar is fixed and spans its nav's whole
     scrollport. */
  /* The stylesheets name which selectors CAN pin, so the candidates come from one
     `querySelectorAll` rather than a computed read of every element: that read
     forces rendering inside any `content-visibility` subtree it crosses, and this
     system's collapsed rows all carry one. Selectors are proved usable once,
     because one bad one poisons the whole list. The list itself is static, since
     stylesheets do not change; the VERDICTS are not, so they are re-read every
     time. An earlier version cached them per interaction burst on the claim that
     an overlay can only mount between bursts. That claim was false: the move that
     carries a pointer to a control arms the timer, so the click that opens the
     overlay always lands inside a live burst, and the stale verdict shipped for up
     to a full idle period. The whole walk measures 0.37ms on the reference sheet
     (16 selectors, 18 matches, 9 kept), which buys nothing worth being wrong for.

     One cover this can never name: a pseudo-element. `collectPropSelectors` drops
     any selector holding `::`, so the navbar's fixed `body::before` dock strip is
     outside the derivation by construction. It is subsumed today, since the
     navbar band above it already covers that strip's whole height, and the next
     such cover may not be. */
  const pinned = collectPropSelectors(document.styleSheets, 'position', ['fixed', 'sticky'])
    .filter(sel => { try { bars.y.matches(sel); return true; } catch { return false; } })
    .join(', ');
  const coverList = () => (pinned ? [...document.querySelectorAll(pinned)] : []).flatMap(c => {
    if (c.classList.contains('ui-scrollbar')) return [];
    const cs = getComputedStyle(c);
    if (cs.position !== 'fixed' && cs.position !== 'sticky') return [];
    if (cs.pointerEvents === 'none') return [];
    const z = parseInt(cs.zIndex, 10);       // `auto` is NaN, which counts as 0
    if (z >= SCROLLBAR_Z) return [];         // paints over the bar, so it hides the bar
    return [{ el: c, z: Number.isNaN(z) ? 0 : z }];
  });

  /* The scroller's own paint slot. A z-index orders siblings inside ONE stacking
     context, so "is this candidate below the bar" answers the wrong question: it
     says whether a cover hides the BAR, never whether it hides the SCROLLER. The
     drawer's scrim sits at 299 and the drawer itself at 300, so the scrim was
     charged against the drawer's own nav, which paints above it, and that nav lost
     its bar entirely at exactly the moment it is on screen. */
  const floorOf = el => {
    for (let n = el; n instanceof Element; n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (cs.position === 'static') continue;
      const z = parseInt(cs.zIndex, 10);
      if (!Number.isNaN(z)) return z;
    }
    return 0;
  };

  const cover = (el, axis) => {
    const box = boxOf(el);
    const lane = axis === 'y' ? [box.right - SCROLLBAR_LANE, box.right - SCROLLBAR_GAP]
                              : [box.bottom - SCROLLBAR_LANE, box.bottom - SCROLLBAR_GAP];
    const spans = [];
    /* `checkVisibility` is guarded because this runs inside an rAF callback, and a
       TypeError there reaches the ring's `window.onerror` bail, which drops the
       hidden cursor for the life of the page: a missing method would cost the
       whole ambient layer, not the bar. Every other newer capability in this
       system is guarded the same way. `getClientRects().length` is the fallback,
       which answers the same question for a `display: none` subtree. */
    const shown = c => (typeof c.checkVisibility === 'function'
      ? c.checkVisibility() : c.getClientRects().length > 0);
    const floor = floorOf(el);
    for (const { el: c, z } of coverList()) {
      if (c === el || c.contains(el) || z < floor || !shown(c)) continue;
      const r = c.getBoundingClientRect();
      if (!r.width || !r.height) continue;
      const across = axis === 'y' ? r.left < lane[1] && r.right > lane[0]
                                  : r.top < lane[1] && r.bottom > lane[0];
      if (across) spans.push(axis === 'y' ? [r.top, r.bottom] : [r.left, r.right]);
    }
    const [lo, hi] = axis === 'y' ? [box.top, box.bottom] : [box.left, box.right];
    // each pass admits the covers the previous one brought within reach
    const run = (from, forward) => {
      for (let pass = 0; pass < spans.length; pass++) {
        let next = from;
        for (const [a, b] of spans) {
          if (forward) { if (a <= from + SCROLLBAR_CHAIN && b > next) next = b; }
          else if (b >= from - SCROLLBAR_CHAIN && a < next) next = a;
        }
        if (next === from) return from;
        from = next;
      }
      return from;
    };
    return [Math.max(0, run(lo, true) - lo), Math.max(0, hi - run(hi, false))];
  };

  /* One place derives the thumb, because the drag's scroll rate has to agree
     with it: a rate taken from a different track length drifts away from the
     pointer over the length of the drag. */
  const metrics = (el, axis) => {
    const v = view(el, axis);
    const [cornerA, cornerB] = corner(el, axis);
    /* A DRAG FREEZES ITS OWN TRACK. `drag.rate` is captured at the press, and the
       cover is live, so chrome that docks or sticks mid-drag re-derives `track`
       under a rate that cannot follow it and the thumb slips against the pointer.
       Measured on the playground, one drag: the grab offset ran 49.72 to -5.33 to
       109.17, a range of 114.5px on a 99.4px thumb, and the 48.0px of it owed to
       the navbar docking was predicted from `track - len` 800.55 to 744.55 before
       it was measured. Freezing is also the honest reading: the track a hand is
       holding does not change length under it. */
    const [coverA, coverB] = (drag && drag.axis === axis && drag.el === el)
      ? drag.pads : cover(el, axis);
    const padA = Math.max(cornerA, coverA), padB = Math.max(cornerB, coverB);
    const room = v - padA - padB;
    const track = Math.max(SCROLLBAR_MIN, room);
    const len = Math.min(track, Math.max(SCROLLBAR_MIN, v * v / full(el, axis)));
    return { padA, track, len, room, max: full(el, axis) - v };
  };

  /* ONE OWNER PER AXIS. A page scroll used to hand both bars to the document, so
     a wide table's x bar lost its scroller in the middle of a wheel: `place`
     returns before it repositions, and the bar then fades out over four frames
     wherever it last painted. Measured on the reference sheet, one 240px wheel
     with the pointer inside the table: the bar held top 545.1 while the table's
     bottom went 553.1 to 313.1, so it sat 240px low, outside the card, over the
     section under it. Per axis it keeps its owner and tracks the table instead. */
  const active = { x: root, y: root };
  let hideAt = 0;
  let drag = null;
  let frame = 0;
  let pointer = null;
  // the thumb's last painted state per scroller and axis, for place()'s re-anchor
  let anchors = new WeakMap();
  const anchor = (el, axis) => {
    let c = anchors.get(el);
    if (!c) anchors.set(el, c = {});
    return c[axis] || (c[axis] = {});
  };

  const place = axis => {
    const bar = bars[axis];
    const el = drag && drag.axis === axis ? drag.el : active[axis];
    if (!el.isConnected || !scrolls(el, axis)) { bar.classList.remove('is-on'); return; }
    const box = boxOf(el);
    const { padA, track, len, room, max } = metrics(el, axis);
    // pinned chrome can take the whole scrollport, and a track with no room for
    // a handle is a mark, not a bar
    if (room < SCROLLBAR_MIN) { bar.classList.remove('is-on'); return; }
    /* A cover that moves under a live scroll (a stuck header at its section's
       end) shrinks `padA` one-to-one with the scroll while the progress term
       follows only at (track - len) / max, so the thumb runs backward under a
       forward scroll. Re-anchoring the line at the last painted position keeps
       the thumb continuous through the change and still reaches the track's end
       exactly at full scroll. With static chrome the re-anchored line IS the
       standard one, so nothing else changes. */
    const s = at(el, axis);
    const a = anchor(el, axis);
    let run;
    if (max > 0 && track - len >= 1 && a.at < max) {
      run = a.run + (s - a.at) * (padA + track - len - a.run) / (max - a.at);
    } else {
      run = padA + (max > 0 ? (s / max) * (track - len) : 0);
    }
    a.at = s;
    a.run = run;
    if (axis === 'y') {
      bar.style.top = `${box.top + run}px`;
      bar.style.left = `${box.right - SCROLLBAR_LANE}px`;
      bar.style.height = `${len}px`;
    } else {
      bar.style.left = `${box.left + run}px`;
      bar.style.top = `${box.bottom - SCROLLBAR_LANE}px`;
      bar.style.width = `${len}px`;
    }
    // the thumb's own box, padded: a bar that retired under a pointer reaching
    // for it would take the grab with it
    const near = pointer && (axis === 'y'
      ? pointer.x >= box.right - SCROLLBAR_LANE - SCROLLBAR_HOLD && pointer.x <= box.right - SCROLLBAR_GAP + SCROLLBAR_HOLD
        && pointer.y >= box.top + run - SCROLLBAR_HOLD && pointer.y <= box.top + run + len + SCROLLBAR_HOLD
      : pointer.y >= box.bottom - SCROLLBAR_LANE - SCROLLBAR_HOLD && pointer.y <= box.bottom - SCROLLBAR_GAP + SCROLLBAR_HOLD
        && pointer.x >= box.left + run - SCROLLBAR_HOLD && pointer.x <= box.left + run + len + SCROLLBAR_HOLD);
    const on = drag ? drag.axis === axis : Boolean(near) || performance.now() < hideAt;
    const wasOn = bar.classList.contains('is-on');
    bar.classList.toggle('is-on', on);
    /* THE MOVE THAT ARMS A RETIRED BAR is hit-tested while the bar still
       computes `pointer-events: none`, so `e.target` named the page and the
       ring derived nothing it can hover; a press with no further move then
       read no hover and painted neither the pressed costume nor the release
       pulse. The bar is what a real move would hit now that it takes a
       pointer, so the pointermove path is re-run with the BAR as the target.
       The bubbled event reaches four handlers, all traced: this controller's
       own move handler, which leaves `active` alone for a bar target; the
       bar's drag handler, inert without a held drag; and cursor.js's grid and
       ring handlers, of which only the ring's state moves, to `hover`.
       Dispatching on window instead would re-derive the scroller off
       `document` and hand the bar to the page. */
    if (on && !wasOn && !drag && pointer
        && document.elementFromPoint(pointer.x, pointer.y) === bar) {
      bar.dispatchEvent(new PointerEvent('pointermove',
        { pointerType: 'mouse', clientX: pointer.x, clientY: pointer.y, bubbles: true }));
    }
  };

  /* The loop runs only while something is actually moving: a drag, or the tail of
     a scroll or a pointer move. A bar held on because the pointer rests on the
     thumb needs no frames, and `place` has already run for that state; the next
     pointermove is what changes it. An rAF that never stops is ambient motion
     with a battery cost, which this system bans everywhere else. */
  const tick = () => {
    frame = 0;
    place('y');
    place('x');
    if (drag || performance.now() < hideAt) sync();
  };
  const sync = () => { frame ||= requestAnimationFrame(tick); };
  // every route that shows the bar restarts its retirement from now
  const arm = () => { hideAt = performance.now() + SCROLLBAR_IDLE; sync(); };

  addEventListener('scroll', e => {
    const el = e.target === document || e.target === root ? root : e.target;
    if (!(el instanceof Element) && el !== root) return;
    // a bare scroller never claims the bar, nor arms it for the page behind it
    if (el.closest('.no-scrollbar')) return;
    // the event names the element that scrolled and never the axis, so it takes
    // the axes it can actually drive and leaves the other bar with its owner
    if (!drag) for (const axis of ['x', 'y']) if (scrolls(el, axis)) active[axis] = el;
    arm();
  }, { capture: true, passive: true });

  addEventListener('pointermove', e => {
    pointer = { x: e.clientX, y: e.clientY };
    /* THE BAR IS ITS OWN POINTER TARGET while it is on, so a move onto the thumb
       makes `e.target` the bar and `scrollerFrom` walks bar -> body -> the root,
       which scrolls. Measured on the reference sheet: the sidebar nav's thumb sat
       at left 243 and jumped to 1432 on the first move that reached it, and the
       press that then landed dragged the page. No inner scroller's bar could be
       grabbed, and the native one is already hidden. A move ON the bar says
       nothing about which scroller is under the pointer: the bar belongs to
       `active` by construction, so leave it alone. */
    const onBar = e.target instanceof Element && e.target.classList.contains('ui-scrollbar');
    if (!drag && !onBar) {
      active.x = scrollerFrom(e.target, 'x');
      active.y = scrollerFrom(e.target, 'y');
    }
    // the move is what arms the timer: a resting pointer retires the bar
    arm();
  }, { passive: true });

  /* `pointer` was write-only, and the Y bar's hold band runs to `innerWidth + 8`,
     so a pointer leaving the window to the right exits THROUGH it and its last
     move is inside it: the bar then stayed lit for the life of the page. Measured
     parked at x 1439, still on after 3s. The ring's own host does the same thing
     for the same reason. */
  document.documentElement.addEventListener('pointerleave', () => { pointer = null; sync(); });

  addEventListener('resize', () => { radii = new WeakMap(); anchors = new WeakMap(); sync(); });

  for (const axis of ['y', 'x']) {
    bars[axis].addEventListener('pointerdown', e => {
      const el = active[axis];
      if (!scrolls(el, axis)) return;
      if (e.pointerType === 'mouse' && e.button !== 0) return;   // middle and right are not a drag
      if (drag) return;                 // one bar at a time: a second press would
      e.preventDefault();               // orphan the first bar's capture and is-drag
      const m = metrics(el, axis);
      // a track with no room for a handle cannot be dragged: without this the
      // `Math.max(SCROLLBAR_MIN, room)` clamp makes `track - len` 0 and one pixel
      // of travel scrolls the whole document
      if (m.room < SCROLLBAR_MIN) return;
      bars[axis].setPointerCapture(e.pointerId);
      bars[axis].classList.add('is-drag');
      drag = {
        axis, el,
        pads: cover(el, axis),          // frozen for the drag, see `metrics`
        from: at(el, axis),
        origin: axis === 'y' ? e.clientY : e.clientX,
        // one pixel of thumb travel is this much scroll
        rate: m.max / Math.max(1, m.track - m.len),
      };
      sync();
    });
    bars[axis].addEventListener('pointermove', e => {
      if (!drag || drag.axis !== axis) return;
      const now = axis === 'y' ? e.clientY : e.clientX;
      const to = drag.from + (now - drag.origin) * drag.rate;
      if (axis === 'y') drag.el.scrollTop = to; else drag.el.scrollLeft = to;
      arm();
    });
    const end = () => {
      if (!drag || drag.axis !== axis) return;
      drag = null;
      bars[axis].classList.remove('is-drag');
      arm();
    };
    bars[axis].addEventListener('pointerup', end);
    bars[axis].addEventListener('pointercancel', end);
  }

  sync();
}

/* ============================= dropzone =========================== */

/* The native input stays the accessible control and the label keeps opening the
   picker, but the input no longer covers the zone: a stretched one paints the
   platform pointer straight through the ring's `cursor: none`, because its UA
   shadow tree is out of the page's reach. The zone takes the drop instead and
   hands the files over. Drag counting is by depth, since a dragleave fires for
   every child the pointer crosses. */
export function initDropzone() {
  document.querySelectorAll('.dropzone').forEach(zone => {
    const input = zone.querySelector('.dropzone-input');
    if (!input || !once(zone, 'uiDropzone')) return;
    let depth = 0;
    const clear = () => { depth = 0; zone.classList.remove('is-over'); };

    /* a disabled input takes no files from the keyboard, so the zone must not
       take them from the pointer either: refusing the dragenter and dragover
       defaults also keeps the browser from marking the dead zone a target */
    zone.addEventListener('dragenter', e => {
      if (input.disabled) return;
      e.preventDefault();
      if (++depth === 1) zone.classList.add('is-over');
    });
    zone.addEventListener('dragover', e => {
      if (input.disabled) return;
      e.preventDefault();
    });
    zone.addEventListener('dragleave', () => { if (--depth <= 0) clear(); });
    zone.addEventListener('drop', e => {
      if (input.disabled) { clear(); return; }   // a drag that began live can end disabled
      e.preventDefault();
      clear();
      if (!e.dataTransfer?.files.length) return;
      input.files = e.dataTransfer.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });
  });
}

/* ============================== stepper =========================== */

/* Both bounds are read off the value, so an authored `disabled` is overwritten on
   the first sync: the state belongs to the component. A NaN loses both comparisons,
   which is what carries an empty field and an undeclared bound: neither can be at a
   floor or a ceiling, so neither button goes dead. */
const syncParts = ({ input, minus, plus }) => {
  const n = input.valueAsNumber;
  minus.disabled = n <= parseFloat(input.min);
  plus.disabled = n >= parseFloat(input.max);
};

/* The parts each wired wrap's listeners actually hold, so the reset writes bounds
   onto the same buttons a press does: a fresh lookup diverges the moment a wrap
   grows a third `.input-step`, and the button the user clicks then stays live at
   its own ceiling while an inert one goes dead. Weak, so a wrap a page drops is
   collectable rather than held by this file, which is what the per-form listener
   this replaced got wrong. Membership is also the only honest answer to "did THIS
   controller wire it": `once`'s mark is an attribute, and an attribute survives the
   clone that a repeated form row is built from, listeners and all left behind. */
const wiredSteppers = new WeakMap();

/* The number stepper's two routes into one value: the buttons and the keyboard.
   `stepUp` and `stepDown` ARE the bounded move, so nothing here restates the grid
   snap, the clamp or the number formatting. They throw where the field declares no
   allowed value step, and a throw inside a listener reaches the cursor's `error`
   handler and takes the page's ring down with it, so a field that cannot step is
   left unwired rather than caught on every press. */
const STEPPER = '[data-stepper]';

export function initStepper() {
  /* A form reset restores its fields with no event on any of them, so the bounds
     would stay where the old values left them and a `+` could sit dead over a
     mid-range number for the life of the page. `reset` bubbles, and the restore
     lands after it, on the NEXT TASK rather than on a microtask: measured on one
     instrumented handler, a trusted press reads the old value at the first and
     second microtask tick and the restored one at a timeout and a frame, while a
     scripted `.click()` reads it restored at the first tick. So the activation a
     person performs is exactly the one a microtask is too early for, and a probe
     that drives the button from inside the page cannot see it.

     ONE page-level listener, never one per stepper on its own form. A listener on a
     node outside the once() mark outlives the markup it was wired for: measured, a
     wrap replaced inside a persisting form left the detached generation still being
     synced. A reset is rare and user-driven, so the sweep asks no question about
     which form owns which field, which is also what carries a field associated by
     `form=""` from outside the form's own subtree. What it does ask is whether this
     controller wired the wrap, so a stepper it refused cannot take a bound costume
     from a reset elsewhere on the page and still answer no press. */
  if (once(document.body, 'uiStepperReset')) {
    document.addEventListener('reset', () => setTimeout(() =>
      document.querySelectorAll(STEPPER).forEach(wrap => {
        const parts = wiredSteppers.get(wrap);
        if (parts) syncParts(parts);
      }), 0));
  }

  // the field and its two controls, from the wrap alone: a `.input-step` before the
  // field decrements and one after it increments
  document.querySelectorAll(STEPPER).forEach(wrap => {
    const kids = [...wrap.children];
    const input = kids.find(el => el.matches('input[type="number"]'));
    if (!input) return;
    const at = kids.indexOf(input);
    const minus = kids.slice(0, at).findLast(el => el.matches('.input-step'));
    const plus = kids.slice(at + 1).find(el => el.matches('.input-step'));
    if (!minus || !plus) return;
    if (input.step.trim().toLowerCase() === 'any' || !once(wrap, 'uiStepper')) return;
    const parts = { input, minus, plus };
    wiredSteppers.set(wrap, parts);
    const sync = () => syncParts(parts);

    // a programmatic value write fires neither event, so a consumer would hear a
    // press and a keystroke differently
    const notify = () => input.dispatchEvent(new Event('input', { bubbles: true }));
    const commit = () => {
      notify();
      input.dispatchEvent(new Event('change', { bubbles: true }));
    };

    const press = (btn, move) => btn.addEventListener('click', () => {
      if (input.disabled) return;   // the group's :has() rule refuses the pointer, never the keyboard
      const before = input.value;
      move();
      sync();
      if (input.value !== before) commit();
    });
    press(minus, () => input.stepDown());
    press(plus, () => input.stepUp());

    /* Typing is the route the platform leaves open: a number field holds 99 under
       max="12" and only reports itself invalid. `min` is the one floor the markup
       declares, so an empty or unparseable field falls back to it, and to nothing
       where no min is declared rather than to a zero this file would be inventing.

       This one only NOTIFIES: the commit it corrects is the change still dispatching
       around it, and that one carries the clamped value to every listener after this
       file's, because the write above lands before they run. A second dispatch here
       would run the whole listener list a second time nested, and the outer dispatch
       would then resume into the same consumer, so one keystroke would commit twice. */
    input.addEventListener('change', () => {
      const before = input.value;
      const n = input.valueAsNumber;
      if (Number.isNaN(n)) { if (input.min !== '') input.value = input.min; }
      else if (n < parseFloat(input.min)) input.value = input.min;
      else if (n > parseFloat(input.max)) input.value = input.max;
      sync();
      if (input.value !== before) notify();
    });

    sync();
  });
}

/* ============================== boot ============================== */

export function init() {
  initViewportWidth();
  initTabs();
  initNavbar();
  initSidebar();
  initDrawer();
  initToggles();
  initDisclosures();
  initSelect();
  initSlider();
  initPalette();
  initTooltip();
  initPopovers();
  initModal();
  initDismiss();
  initTheme();
  initSticky();
  initReveal();
  initChart();
  initCardButtons();
  initTables();
  initCodeCopy();
  initTypedPlaceholder();
  initScrollFade();
  initScrollbars();
  initDropzone();
  initStepper();
}

// a module script is deferred, so the document is parsed by the time this runs
init();
