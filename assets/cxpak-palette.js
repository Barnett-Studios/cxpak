/* cxpak palette system (ADR-0191): one design language, many palettes.
   Palette is client-side runtime state applied as CSS custom properties on
   :root — the emitted HTML bytes are identical regardless of default or
   selection, so the golden fixture is unaffected (ADR-0198 determinism note).
   btop-schema token sets (bg/surf/ink/ink2/ink3/hair/accent/lo/mid/hi) are
   mapped onto the SPA's real CSS variables. Tokyo Night is the default. */
(function () {
  var root = document.documentElement;

  function P(key, label, group, mode, bg, surf, ink, ink2, ink3, hair, accent, lo, mid, hi, pair) {
    return { key: key, label: label, group: group, mode: mode, bg: bg, surf: surf, ink: ink, ink2: ink2, ink3: ink3, hair: hair, accent: accent, lo: lo, mid: mid, hi: hi, pair: pair };
  }

  var PALS = [
    // cxpak moods
    P("cyanotype-dark", "Cyanotype (dark)", "cxpak", "dark", "#0B1420", "#101E2E", "#E6EEF6", "#9BB0C4", "#657C90", "#2A3A4A", "#5FA8D3", "#2FBF96", "#F2B138", "#F0653B", "cyanotype-light"),
    P("cyanotype-light", "Cyanotype (light)", "cxpak", "light", "#F6F5F0", "#FFFFFF", "#182430", "#54626E", "#8B96A0", "#CFC8B7", "#14497A", "#009E73", "#E69F00", "#D55E00", "cyanotype-dark"),
    P("phosphor", "Phosphor", "cxpak", "dark", "#05080A", "#0A1210", "#D9E4DC", "#7E9488", "#4D5F56", "#1E2A24", "#FFB000", "#56B4E9", "#E6A100", "#F0653B"),
    P("fieldbook", "Field-book", "cxpak", "light", "#FAF6EC", "#FFFDF8", "#241E16", "#6B5F4E", "#9C8F78", "#E6DDCA", "#1F5C3F", "#1F7A54", "#B8860B", "#A5341B"),
    // popular dev palettes
    P("tokyo-night", "Tokyo Night", "popular", "dark", "#1A1B26", "#24283B", "#C0CAF5", "#9AA5CE", "#565F89", "#2F3549", "#7AA2F7", "#9ECE6A", "#E0AF68", "#F7768E"),
    P("catppuccin", "Catppuccin Macchiato", "popular", "dark", "#24273A", "#363A4F", "#CAD3F5", "#A5ADCB", "#6E738D", "#494D64", "#8AADF4", "#A6DA95", "#EED49F", "#ED8796", "catppuccin-latte"),
    P("catppuccin-latte", "Catppuccin Latte", "popular", "light", "#EFF1F5", "#FFFFFF", "#4C4F69", "#5C5F77", "#8C8FA1", "#CCD0DA", "#1E66F5", "#40A02B", "#DF8E1D", "#D20F39", "catppuccin"),
    P("everforest-dark", "Everforest (dark)", "popular", "dark", "#2D353B", "#343F44", "#D3C6AA", "#A6B0A0", "#859289", "#3D484D", "#7FBBB3", "#A7C080", "#DBBC7F", "#E67E80", "everforest-light"),
    P("everforest-light", "Everforest (light)", "popular", "light", "#FDF6E3", "#FFFFFF", "#5C6A72", "#829181", "#A6B0A0", "#E6E2CC", "#3A94C5", "#8DA101", "#DFA000", "#F85552", "everforest-dark"),
    P("gruvbox-dark", "Gruvbox (dark)", "popular", "dark", "#282828", "#32302F", "#EBDBB2", "#BDAE93", "#928374", "#3C3836", "#83A598", "#B8BB26", "#FABD2F", "#FB4934", "gruvbox-light"),
    P("gruvbox-light", "Gruvbox (light)", "popular", "light", "#FBF1C7", "#F9F5D7", "#3C3836", "#665C54", "#928374", "#EBDBB2", "#076678", "#79740E", "#B57614", "#9D0006", "gruvbox-dark"),
    P("nord", "Nord", "popular", "dark", "#2E3440", "#3B4252", "#E5E9F0", "#D8DEE9", "#4C566A", "#434C5E", "#88C0D0", "#A3BE8C", "#EBCB8B", "#BF616A"),
    P("dracula", "Dracula", "popular", "dark", "#282A36", "#343746", "#F8F8F2", "#C8CADB", "#6272A4", "#44475A", "#BD93F9", "#50FA7B", "#F1FA8C", "#FF5555"),
    P("monokai", "Monokai", "popular", "dark", "#272822", "#31322C", "#F8F8F2", "#C9CABF", "#75715E", "#3E3D32", "#66D9EF", "#A6E22E", "#E6DB74", "#F92672"),
    P("one-dark", "One Dark", "popular", "dark", "#282C34", "#31353F", "#ABB2BF", "#8B92A0", "#5C6370", "#3E4451", "#61AFEF", "#98C379", "#E5C07B", "#E06C75"),
    P("rose-pine", "Rosé Pine", "popular", "dark", "#191724", "#1F1D2E", "#E0DEF4", "#908CAA", "#6E6A86", "#26233A", "#C4A7E7", "#9CCFD8", "#F6C177", "#EB6F92", "rose-pine-dawn"),
    P("rose-pine-dawn", "Rosé Pine Dawn", "popular", "light", "#FAF4ED", "#FFFAF3", "#575279", "#797593", "#9893A5", "#DFDAD9", "#907AA9", "#56949F", "#EA9D34", "#B4637A", "rose-pine"),
    P("solarized-dark", "Solarized (dark)", "popular", "dark", "#002B36", "#073642", "#93A1A1", "#839496", "#586E75", "#0E4B59", "#268BD2", "#859900", "#B58900", "#DC322F", "solarized-light"),
    P("solarized-light", "Solarized (light)", "popular", "light", "#FDF6E3", "#EEE8D5", "#586E75", "#657B83", "#93A1A1", "#E0DCCB", "#268BD2", "#859900", "#B58900", "#DC322F", "solarized-dark")
  ];

  var BY = {};
  PALS.forEach(function (p) { BY[p.key] = p; });

  // Shift a #rrggbb toward lighter/darker by `amt`; returns an rgb() string.
  function lift(hex, amt) {
    var m = hex.replace('#', '');
    if (m.length !== 6) return hex;
    var r = parseInt(m.slice(0, 2), 16), g = parseInt(m.slice(2, 4), 16), b = parseInt(m.slice(4, 6), 16);
    r = Math.max(0, Math.min(255, r + amt));
    g = Math.max(0, Math.min(255, g + amt));
    b = Math.max(0, Math.min(255, b + amt));
    return 'rgb(' + r + ',' + g + ',' + b + ')';
  }

  // Map the btop token set onto the SPA's real CSS custom properties.
  function applyPalette(p) {
    if (!p) return;
    var s = root.style, dark = (p.mode === 'dark');
    s.setProperty('--bg-primary', p.bg);
    s.setProperty('--bg-secondary', p.surf);
    s.setProperty('--bg-card', p.surf);
    s.setProperty('--bg-card-hover', lift(p.surf, dark ? 14 : -10));
    s.setProperty('--text-primary', p.ink);
    s.setProperty('--text-secondary', p.ink2);
    s.setProperty('--text-dim', p.ink3);
    s.setProperty('--border', p.hair);
    s.setProperty('--border-light', lift(p.hair, dark ? 20 : -20));
    s.setProperty('--accent-blue', p.accent);
    s.setProperty('--accent-green', p.lo);
    s.setProperty('--accent-yellow', p.mid);
    s.setProperty('--accent-orange', lift(p.mid, dark ? -12 : 12));
    s.setProperty('--accent-red', p.hi);
    s.setProperty('--node-default', lift(p.surf, dark ? 20 : -14));
    s.setProperty('--edge-default', p.hair);
    // Keep data-theme in sync so theme-gated rules (severity dots, etc.) match.
    root.setAttribute('data-theme', dark ? 'dark' : 'light');
    var sel = document.getElementById('cxpak-palette-select');
    if (sel) sel.value = p.key;
    // Blueprint swatch strip: a live preview of the active palette's semantic
    // ramp (accent + risk lo/mid/hi + secondary ink). Decorative, aria-hidden.
    var sw = document.getElementById('cxpak-palette-swatches');
    if (sw) {
      sw.textContent = '';
      [p.accent, p.lo, p.mid, p.hi, p.ink2].forEach(function (c) {
        var d = document.createElement('span');
        d.className = 'cxpak-palette-swatch';
        d.style.background = c;
        sw.appendChild(d);
      });
    }
    try { localStorage.setItem('cxpak-palette', p.key); } catch (e) { /* ignore */ }
  }
  window.CX = window.CX || {};
  window.CX.applyPalette = applyPalette;

  // Populate the grouped picker.
  var sel = document.getElementById('cxpak-palette-select');
  if (sel) {
    [['cxpak', 'cxpak moods'], ['popular', 'popular schemes']].forEach(function (g) {
      var og = document.createElement('optgroup');
      og.label = g[1];
      PALS.filter(function (p) { return p.group === g[0]; }).forEach(function (p) {
        var o = document.createElement('option');
        o.value = p.key;
        o.textContent = p.label;
        og.appendChild(o);
      });
      sel.appendChild(og);
    });
    sel.addEventListener('change', function () { applyPalette(BY[sel.value]); });
  }

  // Apply the saved palette, or Tokyo Night by default.
  var saved = null;
  try { saved = localStorage.getItem('cxpak-palette'); } catch (e) { saved = null; }
  applyPalette(BY[(saved && BY[saved]) ? saved : 'tokyo-night']);
})();
