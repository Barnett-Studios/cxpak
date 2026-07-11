// cxpak SPA controller
// Sections: 1) Bootstrap  2) Router  3) Palette  4) Inspector  5) Theme  6) Keyboard+a11y+freshness

(function() {
  'use strict';

  // =============================================================================
  // 1) BOOTSTRAP
  // =============================================================================
  var CX = window.CX = window.CX || {};
  CX.state = {
    view: 'dashboard',
    focus: null, module: null, file: null, symbol: null, files: null,
    inspector: null,
    inspectorTrigger: null,
    prePaletteFocus: null,
    paletteOpen: false,
    helpOverlayOpen: false,
    localStorageAvailable: true,
    clipboardAvailable: (typeof navigator.clipboard !== 'undefined' && typeof navigator.clipboard.writeText === 'function'),
  };

  try { localStorage.getItem('cxpak-theme'); } catch (e) { CX.state.localStorageAvailable = false; }

  CX.data = {};
  // Data tags use per-renderer legacy names (cxpak-dashboard, cxpak-explorer, etc.)
  // so the shared view renderers from src/visual/render.rs work unchanged in SPA mode.
  var TAG_MAP = {
    dashboard: 'cxpak-dashboard',
    architecture: 'cxpak-explorer',
    risk: 'cxpak-heatmap',
    timeline: 'cxpak-timeline',
    flow: 'cxpak-flow',
    diff: 'cxpak-diff',
    meta: 'cxpak-meta',
    'search-index': 'cxpak-search-index',
  };
  Object.keys(TAG_MAP).forEach(function(name) {
    var tagId = TAG_MAP[name];
    var el = document.getElementById(tagId);
    if (!el) { throw new Error('missing data tag: ' + tagId); }
    try {
      CX.data[name] = JSON.parse(el.textContent);
    } catch (e) {
      console.error('failed to parse ' + tagId, e);
      CX.data[name] = null;
    }
  });

  // Router-param sanitization regex (matches spec § 1.3).
  var ROUTE_PARAM_RE = /^[A-Za-z0-9._/\-]{1,512}$/;
  function sanitizeRouteParam(v) {
    if (typeof v !== 'string' || !ROUTE_PARAM_RE.test(v)) return '';
    return v;
  }

  // Shared format helper — all score formatting routes through this.
  CX.format = {
    score: function(x) { return (typeof x === 'number') ? x.toFixed(1) : '--'; }
  };

  // =============================================================================
  // 2) ROUTER
  // =============================================================================
  var VIEWS = ['dashboard','explore','flow','timeline','diff'];
  var initialized = {};
  CX._initialized = initialized; // expose for toggleTheme re-render

  function parseHash() {
    var raw = window.location.hash.replace(/^#/, '') || 'dashboard';
    var qidx = raw.indexOf('?');
    var name = qidx >= 0 ? raw.slice(0, qidx) : raw;
    var params = {};
    if (qidx >= 0) {
      raw.slice(qidx + 1).split('&').forEach(function(pair) {
        var eq = pair.indexOf('=');
        if (eq > 0) {
          var k = decodeURIComponent(pair.slice(0, eq));
          var v = sanitizeRouteParam(decodeURIComponent(pair.slice(eq + 1)));
          params[k] = v;
        }
      });
    }
    // Legacy deep-links (#architecture / #risk, incl. palette file/module
    // targets) redirect to the merged Explore mode; the lens param survives.
    if (name === 'architecture') { name = 'explore'; if (!params.lens) params.lens = 'deps'; }
    else if (name === 'risk') { name = 'explore'; if (!params.lens) params.lens = 'risk'; }
    if (VIEWS.indexOf(name) < 0) name = 'dashboard';
    return { name: name, params: params };
  }

  function closeInspector() {
    CX.state.inspector = null;
    var el = document.getElementById('cxpak-inspector');
    if (el) {
      el.setAttribute('hidden', '');
      el.classList.remove('open');
    }
    // Return focus to the element that triggered the inspector, if still in DOM.
    var trigger = CX.state.inspectorTrigger;
    if (trigger && document.body.contains(trigger)) {
      try { trigger.focus(); } catch (e) { /* ignore */ }
    }
    var live = document.getElementById('cxpak-live');
    if (live) live.textContent = 'Inspector closed';
    CX.state.inspectorTrigger = null;
  }

  function interruptView(name) {
    if (window.d3) {
      window.d3.selectAll('#view-' + name + ' *').interrupt();
    }
  }

  function navigate() {
    var parsed = parseHash();
    var newView = parsed.name;
    CX.state.focus = parsed.params.focus || null;
    CX.state.module = parsed.params.module || null;
    CX.state.file = parsed.params.file || null;
    CX.state.symbol = parsed.params.symbol || null;
    CX.state.lens = parsed.params.lens || null;

    // Interrupt old, close inspector
    if (CX.state.view && CX.state.view !== newView) {
      interruptView(CX.state.view);
      closeInspector();
    }

    // Hide all, show target
    VIEWS.forEach(function(v) {
      var el = document.getElementById('view-' + v);
      if (!el) return;
      if (v === newView) el.removeAttribute('hidden');
      else el.setAttribute('hidden', '');
    });

    CX.state.view = newView;

    // Init if first visit
    if (!initialized[newView]) {
      var initFn = CX.init && CX.init[newView];
      if (typeof initFn === 'function') initFn();
      initialized[newView] = true;
    } else {
      var updateFn = CX.update && CX.update[newView];
      if (typeof updateFn === 'function') updateFn();
    }

    // Announce to screen readers
    var live = document.getElementById('cxpak-live');
    if (live) {
      var hint = {
        dashboard: 'Dashboard view with health, risks, and alerts',
        explore: 'Explore view with Dependencies and Risk lenses',
        flow: 'Flow diagram view',
        timeline: 'Timeline view',
        diff: 'Diff view',
      }[newView] || newView;
      live.textContent = 'Navigated to ' + hint;
    }

    // Update active nav tab + roving-tabindex state.  Per WAI-ARIA APG
    // tablist pattern: the active tab gets tabindex="0" and
    // aria-selected="true"; all others get tabindex="-1" and
    // aria-selected="false".  Combined with the ArrowKey handler below,
    // this makes Tab+Arrow-keys reach any view; previously only
    // dashboard was a tab stop and keyboard users needed Tab through
    // the entire page to reach Architecture/Risk/Flow/Timeline/Diff.
    document.querySelectorAll('.cxpak-nav-link').forEach(function(a) {
      var match = a.getAttribute('data-view') === newView;
      a.classList.toggle('active', match);
      a.setAttribute('tabindex', match ? '0' : '-1');
      a.setAttribute('aria-selected', match ? 'true' : 'false');
    });
  }

  CX.navigate = navigate;
  window.addEventListener('hashchange', function() {
    if (CX._suppressHashChange) return;
    navigate();
  });
  window.addEventListener('DOMContentLoaded', function() {
    // Initial focus on first nav tab.  navigate() below also sets the
    // tabindex/aria-selected pair correctly via the roving update.
    var firstNav = document.querySelector('.cxpak-nav-link[data-view="dashboard"]');
    if (firstNav) firstNav.setAttribute('tabindex', '0');
    navigate();

    // Roving-tabindex arrow-key handler.  ArrowLeft/ArrowRight cycle
    // through the nav tabs (with wraparound); Home/End jump to first/
    // last.  The tabindex update happens via navigate() once the new
    // view is selected so a single source of truth (the active view)
    // drives both visual state and focus order.
    var navEl = document.querySelector('.cxpak-nav[role="tablist"]');
    if (!navEl) return;
    navEl.addEventListener('keydown', function(ev) {
      if (ev.key !== 'ArrowLeft' && ev.key !== 'ArrowRight'
          && ev.key !== 'Home' && ev.key !== 'End') return;
      var tabs = Array.prototype.slice.call(
        document.querySelectorAll('.cxpak-nav-link[role="tab"]')
      );
      if (tabs.length === 0) return;
      var current = tabs.indexOf(document.activeElement);
      if (current < 0) current = 0;
      var next;
      if (ev.key === 'ArrowLeft') next = (current - 1 + tabs.length) % tabs.length;
      else if (ev.key === 'ArrowRight') next = (current + 1) % tabs.length;
      else if (ev.key === 'Home') next = 0;
      else next = tabs.length - 1;
      ev.preventDefault();
      var target = tabs[next];
      // Move focus AND navigate (auto-activation on focus is the
      // simpler ARIA tab pattern; works well for view switching).
      CX.pushHash(target.getAttribute('href'));
      navigate();
      target.focus();
    });
  });

  // Programmatic hash updates use pushState (no re-entrant hashchange).
  // The fallback path assigns location.hash, which fires `hashchange` synchronously;
  // suppress the synchronous re-navigate so callers can drive `navigate()` themselves.
  CX.pushHash = function(hash) {
    try {
      window.history.pushState(null, '', hash);
    } catch (e) {
      CX._suppressHashChange = true;
      try { window.location.hash = hash; } finally { CX._suppressHashChange = false; }
    }
  };

  // =============================================================================
  // 3) COMMAND PALETTE
  // =============================================================================
  function openPalette() {
    if (CX.state.paletteOpen) return;
    CX.state.prePaletteFocus = document.activeElement;
    CX.state.paletteOpen = true;
    var overlay = document.getElementById('cxpak-palette-overlay');
    overlay.removeAttribute('hidden');
    var input = document.getElementById('cxpak-palette-input');
    input.value = '';
    // ARIA combobox spec: aria-expanded must reflect popup visibility.
    // Hardcoded "true" lied while the palette was hidden — flip it here
    // and in closePalette() so screen readers announce the state honestly.
    input.setAttribute('aria-expanded', 'true');
    input.focus();
    renderPaletteResults('');
  }
  function closePalette() {
    if (!CX.state.paletteOpen) return;
    CX.state.paletteOpen = false;
    document.getElementById('cxpak-palette-overlay').setAttribute('hidden', '');
    var input = document.getElementById('cxpak-palette-input');
    if (input) input.setAttribute('aria-expanded', 'false');
    try { CX.state.prePaletteFocus && CX.state.prePaletteFocus.focus(); }
    catch (e) { document.querySelector('.cxpak-nav-link').focus(); }
  }

  function rankEntry(entry, q) {
    var lbl = entry.label.toLowerCase();
    var ql = q.toLowerCase();
    if (ql === '') return [3, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    if (lbl === ql) return [4, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    if (lbl.indexOf(ql) === 0) return [3, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    var idx = lbl.indexOf(ql);
    if (idx >= 0) return [2, -idx, kindRank(entry.kind), lbl.length, lbl, entry.context];
    // Subsequence
    var i = 0;
    for (var j = 0; j < lbl.length && i < ql.length; j++) if (lbl[j] === ql[i]) i++;
    if (i === ql.length) return [1, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    return null;
  }
  function kindRank(kind) {
    return kind === 'view' ? 3 : kind === 'module' ? 2 : kind === 'file' ? 1 : 0;
  }
  function cmpKey(a, b) {
    for (var i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return a[i] > b[i] ? -1 : 1;
    }
    return 0;
  }

  function renderPaletteResults(q) {
    var list = document.getElementById('cxpak-palette-results');
    list.textContent = '';
    var index = CX.data['search-index'] || [];
    var scored = [];
    for (var i = 0; i < index.length; i++) {
      var s = rankEntry(index[i], q);
      if (s) scored.push({ s: s, e: index[i] });
    }
    scored.sort(function(a, b) { return cmpKey(a.s, b.s); });
    // Empty query: show 6 views + top-10 files by PageRank (first views by sort, then files).
    if (q === '') {
      var views = scored.filter(function(x) { return x.e.kind === 'view'; });
      var files = scored.filter(function(x) { return x.e.kind === 'file'; }).slice(0, 10);
      scored = views.concat(files);
    }
    scored = scored.slice(0, 50);
    scored.forEach(function(x, idx) {
      var li = document.createElement('div');
      li.className = 'cxpak-palette-item' + (idx === 0 ? ' active' : '');
      li.setAttribute('role', 'option');
      li.id = 'cxpak-palette-item-' + idx;
      li.setAttribute('aria-selected', idx === 0 ? 'true' : 'false');
      var k = document.createElement('span');
      k.className = 'kind ' + x.e.kind;
      k.textContent = x.e.kind;
      var l = document.createElement('span');
      l.className = 'label';
      l.textContent = x.e.label;
      var d = document.createElement('span');
      d.className = 'detail';
      d.textContent = x.e.detail;
      li.appendChild(k); li.appendChild(l); li.appendChild(d);
      li.setAttribute('data-target', x.e.target);
      li.addEventListener('click', function() {
        CX.pushHash(x.e.target);
        navigate();
        closePalette();
      });
      list.appendChild(li);
    });
    // Set aria-activedescendant on the input to track the first active item.
    var paletteInput = document.getElementById('cxpak-palette-input');
    if (paletteInput) {
      var firstItem = document.getElementById('cxpak-palette-item-0');
      paletteInput.setAttribute('aria-activedescendant', firstItem ? 'cxpak-palette-item-0' : '');
    }
    if (scored.length === 0 && q !== '') {
      var empty = document.createElement('div');
      empty.className = 'cxpak-palette-empty';
      empty.textContent = 'No results for "';
      empty.appendChild(document.createTextNode(q));
      empty.appendChild(document.createTextNode('"'));
      list.appendChild(empty);
    }
    // Announce result count to screen readers via the live region.
    // Sighted users see the listbox grow; SR users get no cue without this.
    var live = document.getElementById('cxpak-live');
    if (live) {
      var n = scored.length;
      if (n === 0) {
        live.textContent = q ? ('No results for "' + q + '"') : 'No results';
      } else if (n === 1) {
        live.textContent = '1 result';
      } else {
        live.textContent = n + ' results';
      }
    }
  }
  CX.openPalette = openPalette;
  CX.closePalette = closePalette;

  // =============================================================================
  // 4) INSPECTOR PANEL
  // =============================================================================
  function openInspector(node, opts) {
    CX.state.inspector = node;
    CX.state.inspectorTrigger = document.activeElement;
    var el = document.getElementById('cxpak-inspector');
    if (!el) return;
    el.removeAttribute('hidden');
    el.classList.add('open');
    var title = el.querySelector('.cxpak-inspector-title');
    if (title) title.textContent = node.label || node.id || 'details';
    // Populate body via textContent only.
    var body = el.querySelector('.cxpak-inspector-body');
    if (body) {
      body.textContent = '';
      // If the caller provides context-specific fields, use those; otherwise
      // fall back to the three generic metadata rows.
      var rows = (opts && opts.fields)
        ? opts.fields
        : [
            ['PageRank', node.metadata && node.metadata.pagerank != null ? CX.format.score(node.metadata.pagerank * 100) : '--'],
            ['Risk score', node.metadata && node.metadata.risk_score != null ? CX.format.score(node.metadata.risk_score * 100) : '--'],
            ['Tokens', String(node.metadata && node.metadata.token_count || 0)],
          ];
      rows.forEach(function(r) {
        var row = document.createElement('div');
        row.className = 'cxpak-inspector-row';
        var lab = document.createElement('span'); lab.className = 'cxpak-inspector-label'; lab.textContent = r[0];
        var val = document.createElement('span'); val.className = 'cxpak-inspector-value'; val.textContent = String(r[1]);
        row.appendChild(lab); row.appendChild(val);
        body.appendChild(row);
      });
    }
    // Announce to screen readers via the dedicated live region.
    var live = document.getElementById('cxpak-live');
    if (live) {
      var label = node.label || node.id || 'details';
      var pr = node.metadata && node.metadata.pagerank != null
        ? ', PageRank ' + CX.format.score(node.metadata.pagerank * 100)
        : '';
      var rs = node.metadata && node.metadata.risk_score != null
        ? ', risk score ' + CX.format.score(node.metadata.risk_score * 100)
        : '';
      live.textContent = 'Inspector open: ' + label + pr + rs;
    }
  }
  CX.openInspector = openInspector;
  CX.closeInspector = closeInspector;

  // =============================================================================
  // 5) THEME TOGGLE
  // =============================================================================
  function readTheme() {
    if (!CX.state.localStorageAvailable) return null;
    try {
      var v = localStorage.getItem('cxpak-theme');
      return (v === 'dark' || v === 'light') ? v : null;
    } catch (e) { return null; }
  }
  function writeTheme(v) {
    if (!CX.state.localStorageAvailable) return;
    try { localStorage.setItem('cxpak-theme', v); } catch (e) { /* ignore */ }
  }
  function applyTheme(t) {
    document.documentElement.setAttribute('data-theme', t);
    var btn = document.querySelector('.cxpak-theme-toggle');
    if (btn) {
      btn.textContent = t === 'dark' ? '☀' : '☾';
      btn.setAttribute('aria-label', 'Switch to ' + (t === 'dark' ? 'light' : 'dark') + ' mode');
    }
  }
  var savedTheme = readTheme();
  var initialTheme = savedTheme || (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');
  applyTheme(initialTheme);
  CX.toggleTheme = function() {
    var curr = document.documentElement.getAttribute('data-theme') || 'dark';
    var next = curr === 'dark' ? 'light' : 'dark';
    applyTheme(next);
    writeTheme(next);
    // Re-render the active view so D3 elements with hardcoded hex fills
    // pick up the new theme's color palette.
    var current = CX.state.view;
    var section = document.getElementById('view-' + current);
    if (section) {
      section.textContent = ''; // clear DOM
      if (CX._initialized) CX._initialized[current] = false;
      if (typeof CX.init[current] === 'function') {
        CX.init[current]();
      }
    }
  };

  // Wire the theme-toggle button click. Runs at script load since the button
  // lives in the header which is rendered before this script executes.
  (function() {
    var btn = document.querySelector('.cxpak-theme-toggle');
    if (btn) btn.addEventListener('click', CX.toggleTheme);
  })();

  // Catch renderer-generated links to standalone view files (e.g. cxpak-architecture.html)
  // and redirect them through the SPA router. Defense-in-depth — Fix 1 handles the main
  // path (dashboard_js:navTo), this catches anything we missed.
  document.addEventListener('click', function(ev) {
    var a = ev.target;
    while (a && a.tagName !== 'A' && a !== document.body) a = a.parentElement;
    if (!a || a.tagName !== 'A') return;
    var href = a.getAttribute('href') || '';
    var m = href.match(/(?:^|\/)[^\/]*?-(dashboard|architecture|risk|flow|timeline|diff)\.html?(?:$|[#?])/);
    if (m) {
      ev.preventDefault();
      CX.pushHash('#' + m[1]);
      CX.navigate();
    }
  });

  // Delegated node-click handler: opens the inspector for any `g.cxpak-node`
  // click inside a view section. Coexists with view-specific click handlers
  // (architecture drill-down, etc.) — this listener runs at the document
  // level in the bubble phase after view handlers. Node datum is pulled from
  // D3's `__data__` property bound via .data().
  document.addEventListener('click', function(ev) {
    var t = ev.target;
    // Walk up to find a g.cxpak-node ancestor (clicks often land on child rect/text).
    while (t && t !== document.body) {
      if (t.tagName === 'g' && t.classList && t.classList.contains('cxpak-node')) {
        var datum = t.__data__;
        if (datum && typeof datum === 'object' && (datum.id || datum.label)) {
          CX.openInspector(datum);
        }
        return;
      }
      t = t.parentNode;
    }
  });

  // Delegated keyboard activation for node focus: Enter/Space opens the
  // inspector without a mouse (matches spec § 1.9 a11y requirement).
  document.addEventListener('keydown', function(ev) {
    if (ev.key !== 'Enter' && ev.key !== ' ') return;
    var t = document.activeElement;
    if (!t) return;
    while (t && t !== document.body) {
      if (t.tagName === 'g' && t.classList && t.classList.contains('cxpak-node')) {
        var datum = t.__data__;
        if (datum && typeof datum === 'object') {
          ev.preventDefault();
          CX.openInspector(datum);
        }
        return;
      }
      t = t.parentNode;
    }
  });

  // Wire the inspector close button (first one — the actual inspector aside).
  (function() {
    var btn = document.querySelector('#cxpak-inspector .cxpak-inspector-close');
    if (btn) btn.addEventListener('click', closeInspector);
  })();

  // Wire the help-overlay close button. Clicking on the backdrop also closes it.
  // Focus returns to whatever had focus before `?` opened the overlay.
  function closeHelp() {
    var ho = document.getElementById('cxpak-help-overlay');
    if (ho) { ho.setAttribute('hidden', ''); CX.state.helpOverlayOpen = false; }
    // Restore focus to the pre-help element (typically a nav link or
    // whichever control the user was interacting with). Without this the
    // keyboard user lands on body after Esc, losing their place.
    var back = CX.state.preHelpFocus;
    CX.state.preHelpFocus = null;
    if (back && typeof back.focus === 'function') {
      try { back.focus(); } catch (_) { /* element may have been removed */ }
    }
  }
  CX.closeHelp = closeHelp;
  (function() {
    var btn = document.querySelector('#cxpak-help-overlay .cxpak-inspector-close');
    if (btn) btn.addEventListener('click', closeHelp);
    var overlay = document.getElementById('cxpak-help-overlay');
    if (overlay) overlay.addEventListener('click', function(ev) { if (ev.target === overlay) closeHelp(); });
  })();

  // Palette: clicking the backdrop (not the palette body) closes it.
  (function() {
    var overlay = document.getElementById('cxpak-palette-overlay');
    if (!overlay) return;
    overlay.addEventListener('click', function(ev) {
      if (ev.target === overlay) closePalette();
    });
  })();

  // Focus trap for modal dialogs (palette + help overlay + inspector).
  // Tab/Shift-Tab cycles within the dialog when one is open.  Inspector is
  // role=dialog aria-modal=false: non-blocking but still focus-bounded so
  // keyboard users do not Tab out into the background SVG by surprise.
  function trapFocus(ev) {
    if (ev.key !== 'Tab') return;
    var modal = null;
    if (CX.state.paletteOpen) modal = document.getElementById('cxpak-palette-overlay');
    else if (CX.state.helpOverlayOpen) modal = document.getElementById('cxpak-help-overlay');
    else if (CX.state.inspector) modal = document.getElementById('cxpak-inspector');
    if (!modal || modal.hasAttribute('hidden')) return;
    var focusables = modal.querySelectorAll(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    );
    if (focusables.length === 0) return;
    var first = focusables[0];
    var last = focusables[focusables.length - 1];
    if (ev.shiftKey && document.activeElement === first) {
      ev.preventDefault();
      last.focus();
    } else if (!ev.shiftKey && document.activeElement === last) {
      ev.preventDefault();
      first.focus();
    }
  }
  document.addEventListener('keydown', trapFocus);

  // =============================================================================
  // 6) KEYBOARD + A11Y + FRESHNESS
  // =============================================================================
  document.addEventListener('keydown', function(ev) {
    var mod = ev.metaKey || ev.ctrlKey;
    if (mod && ev.key === 'k') { ev.preventDefault(); openPalette(); return; }
    if (ev.key === '/') {
      // Skip when palette is already open (avoid duplicate trigger and
      // not stealing the `/` keystroke from the palette input itself) and
      // when the user is typing into any other input/textarea/editable
      // surface — pressing `/` in a search field should produce a `/`,
      // not open a new palette.
      if (CX.state.paletteOpen) return;
      var t = ev.target;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      ev.preventDefault();
      openPalette();
      return;
    }
    if (ev.key === 'Escape') {
      if (CX.state.paletteOpen) { closePalette(); return; }
      if (CX.state.inspector) { closeInspector(); return; }
      if (CX.state.helpOverlayOpen) { closeHelp(); return; }
    }
    if (['1','2','3','4','5'].indexOf(ev.key) >= 0 && !CX.state.paletteOpen) {
      var v = VIEWS[parseInt(ev.key) - 1];
      if (v) { CX.pushHash('#' + v); navigate(); }
    }
    if (ev.key === 't' && !CX.state.paletteOpen) { CX.toggleTheme(); }
    if (ev.key === '?' && !CX.state.paletteOpen) {
      var ho = document.getElementById('cxpak-help-overlay');
      if (ho) {
        // Save the element that had focus before the overlay opened so
        // closeHelp() can restore it. Without this, keyboard users lose
        // their place after Esc.
        CX.state.preHelpFocus = document.activeElement;
        ho.removeAttribute('hidden');
        CX.state.helpOverlayOpen = true;
        var closeBtn = ho.querySelector('.cxpak-inspector-close');
        if (closeBtn) closeBtn.focus();
      }
    }
  });

  // Palette input handling
  document.addEventListener('DOMContentLoaded', function() {
    var input = document.getElementById('cxpak-palette-input');
    if (input) {
      input.addEventListener('input', function() { renderPaletteResults(input.value); });
      input.addEventListener('keydown', function(ev) {
        var items = document.querySelectorAll('.cxpak-palette-item');
        var active = document.querySelector('.cxpak-palette-item.active');
        var idx = Array.prototype.indexOf.call(items, active);
        if (ev.key === 'ArrowDown') {
          ev.preventDefault();
          if (active) { active.classList.remove('active'); active.setAttribute('aria-selected', 'false'); }
          idx = Math.min(idx + 1, items.length - 1);
          if (items[idx]) {
            items[idx].classList.add('active');
            items[idx].setAttribute('aria-selected', 'true');
            input.setAttribute('aria-activedescendant', items[idx].id || '');
          }
        } else if (ev.key === 'ArrowUp') {
          ev.preventDefault();
          if (active) { active.classList.remove('active'); active.setAttribute('aria-selected', 'false'); }
          idx = Math.max(idx - 1, 0);
          if (items[idx]) {
            items[idx].classList.add('active');
            items[idx].setAttribute('aria-selected', 'true');
            input.setAttribute('aria-activedescendant', items[idx].id || '');
          }
        } else if (ev.key === 'Enter') {
          ev.preventDefault();
          if (active) active.click();
        }
      });
    }
  });

  // Freshness badge — updates every 60s, pauses on hidden.
  var freshnessInterval = null;
  function updateFreshness() {
    var el = document.querySelector('.cxpak-freshness');
    if (!el) return;
    var meta = CX.data.meta;
    if (!meta || !meta.generated_at) return;
    var genMs = Date.parse(meta.generated_at);
    var ageHours = (Date.now() - genMs) / 3600000;
    el.className = 'cxpak-freshness';
    if (ageHours < 1) { el.textContent = 'just now'; el.classList.add('fresh'); }
    else if (ageHours < 24) { el.textContent = Math.floor(ageHours) + 'h ago'; el.classList.add('fresh'); }
    else if (ageHours < 72) { el.textContent = Math.floor(ageHours / 24) + 'd ago'; el.classList.add('stale'); }
    else {
      var days = Math.floor(ageHours / 24);
      el.textContent = '';
      el.appendChild(document.createTextNode(days + 'd ago · '));
      if (CX.state.clipboardAvailable) {
        var btn = document.createElement('button');
        btn.textContent = 'copy refresh command';
        btn.addEventListener('click', function() {
          // .catch is REQUIRED — clipboard.writeText rejects on insecure
          // contexts (file://, non-HTTPS), permissions denied, or focus
          // loss.  Without the catch the user gets stuck on "copy
          // refresh command" with no feedback.
          navigator.clipboard.writeText('cxpak visual')
            .then(function() {
              btn.textContent = 'Copied!';
              setTimeout(function() { updateFreshness(); }, 2000);
            })
            .catch(function() {
              btn.textContent = 'copy failed — run: cxpak visual';
              setTimeout(function() { updateFreshness(); }, 4000);
            });
        });
        el.appendChild(btn);
      } else {
        var code = document.createElement('code');
        code.textContent = 'cxpak visual';
        el.appendChild(code);
      }
      el.classList.add('old');
    }
    el.title = meta.generated_at;
  }
  function startFreshness() {
    updateFreshness();
    if (freshnessInterval) clearInterval(freshnessInterval);
    freshnessInterval = setInterval(updateFreshness, 60000);
  }
  function stopFreshness() {
    if (freshnessInterval) { clearInterval(freshnessInterval); freshnessInterval = null; }
  }
  document.addEventListener('visibilitychange', function() {
    if (document.hidden) stopFreshness();
    else startFreshness();
  });
  window.addEventListener('DOMContentLoaded', startFreshness);

})();
