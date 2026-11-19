/* The visitor's preferences, kept in this browser and applied before the page
   paints: the theme (light, dark, or the system's), the accent (blue, green,
   amber, tile, violet or graphite) and the maps' (the page's, light or dark), as data-theme, data-accent and
   data-map-theme on <html>. The language is a cookie,
   since the server renders the page in it; see the preferences dialog. */
(() => {
    const root = document.documentElement;
    const THEME_KEY = 'meshview-theme';
    const ACCENT_KEY = 'meshview-accent';
    const ACCENTS = ['blue', 'green', 'amber', 'tile', 'violet', 'graphite'];
    const MAP_KEY = 'meshview-map-theme';
    const system = matchMedia('(prefers-color-scheme: dark)');
    const read = key => { try { return localStorage.getItem(key); } catch { return null; } };
    const write = (key, value) => {
        try { value === null ? localStorage.removeItem(key) : localStorage.setItem(key, value); } catch {}
    };

    let theme = read(THEME_KEY);
    if (!['light', 'dark'].includes(theme)) theme = null;
    let accent = read(ACCENT_KEY);
    if (!ACCENTS.includes(accent)) accent = 'blue';
    let mapTheme = read(MAP_KEY);
    if (!['light', 'dark'].includes(mapTheme)) mapTheme = null;
    const effective = () => theme || (system.matches ? 'dark' : 'light');
    root.dataset.theme = effective();
    root.dataset.accent = accent;
    root.dataset.mapTheme = mapTheme || effective();
    // Nothing animates while the page loads: what it restores, a folded panel or
    // a theme, shows at once. Two frames after the document is ready, it may.
    root.classList.add('mesh-preload');
    document.addEventListener('DOMContentLoaded', () => requestAnimationFrame(() =>
        requestAnimationFrame(() => root.classList.remove('mesh-preload'))));

    // Every palette's colours seen on this page, so a chart drawn in any of them
    // takes the current one's: the accents' and their seconds' (as r,g,b), texts,
    // muted, borders.
    const ACCENT_RGB = new Set(['36,94,168', '139,188,255', '11,122,82', '76,195,138']);
    const SECOND_RGB = new Set(['232,152,40']);
    const TEXT = new Set(['#333','#334155','#243247','#26364a','#e7eef7','#e2ece5','#e2eaf6']);
    const MUTED = new Set(['#ccc','#64748b','#566579','#888','#9faea6','#8190a5','#a0aec2','#a9b6c9']);
    const BORDER = new Set(['#aaa','#c3cfdf','#34483c','#dce7df','#dce4ef','#34465f','#dbdee4']);
    const SURFACE = new Set(['#fff','#ffffff','#18231e','#182335']);
    const colors = () => {
        const style = getComputedStyle(root);
        return {
            surface:style.getPropertyValue('--mesh-surface').trim(),
            text:style.getPropertyValue('--mesh-text').trim(),
            muted:style.getPropertyValue('--mesh-muted').trim(),
            border:style.getPropertyValue('--mesh-border').trim(),
            font:style.getPropertyValue('--mesh-font').trim(),
            accent:style.getPropertyValue('--mesh-primary-rgb').trim(),
            second:style.getPropertyValue('--mesh-second-rgb').trim()
        };
    };
    // Remember the palette in use, before a chart draws in it.
    const seen = () => {
        const palette = colors();
        ACCENT_RGB.add(palette.accent.replace(/\s+/g, ''));
        SECOND_RGB.add(palette.second.replace(/\s+/g, ''));
        TEXT.add(palette.text.toLowerCase());
        MUTED.add(palette.muted.toLowerCase());
        BORDER.add(palette.border.toLowerCase());
        SURFACE.add(palette.surface.toLowerCase());
        return palette;
    };
    // The accent's colour, as rgba(): what a chart draws in the accent asks for,
    // and, whichever accent it was drawn in, takes the one chosen now.
    window.meshAccent = (alpha = 1) => `rgba(${seen().accent},${alpha})`;
    // A second series' colour beside the accent, as rgba(): one that stands apart
    // from it (orange, or a cool blue beside the warm accents), the same way.
    window.meshSecond = (alpha = 1) => `rgba(${seen().second},${alpha})`;
    // The alpha of a colour in one of these sets, else null.
    const alphaIn = (set, value) => {
        const match = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(value);
        return match && set.has(match.slice(1, 4).join(',')) ? (match[4] ?? '1') : null;
    };
    // Neutral chart colors follow the theme; packet/route/series colors stay intact.
    function chartOption(option) {
        const palette = seen();
        function visit(value, key) {
            if (Array.isArray(value)) return value.map(item => visit(item,key));
            if (value && Object.getPrototypeOf(value) === Object.prototype) {
                return Object.fromEntries(Object.entries(value).map(([k,v]) => [k,visit(v,k)]));
            }
            // A gradient, as an area under a line, recoloured stop by stop.
            if (value && Array.isArray(value.colorStops) && window.echarts) {
                return new window.echarts.graphic.LinearGradient(value.x, value.y, value.x2, value.y2,
                    value.colorStops.map(stop => ({...stop, color:visit(stop.color, 'color')})));
            }
            if (typeof value !== 'string') return value;
            const alpha = alphaIn(ACCENT_RGB, value);
            if (alpha !== null) return `rgba(${palette.accent},${alpha})`;
            const secondAlpha = alphaIn(SECOND_RGB, value);
            if (secondAlpha !== null) return `rgba(${palette.second},${secondAlpha})`;
            const hex = value.toLowerCase();
            if (key === 'backgroundColor' && SURFACE.has(hex)) return palette.surface;
            if (['borderColor','textBorderColor'].includes(key) && (hex === '#000' || SURFACE.has(hex))) return palette.surface;
            if (key === 'color' && TEXT.has(hex)) return palette.text;
            if (key === 'color' && MUTED.has(hex)) return palette.muted;
            if (key === 'color' && BORDER.has(hex)) return palette.border;
            return value;
        }
        const themed = visit(option,'');
        themed.textStyle = {...themed.textStyle,color:palette.text};
        // Keep ECharts 5's softer default series colors, which suit the site better.
        if (!themed.color) themed.color = ['#5470c6','#91cc75','#fac858','#ee6666','#73c0de','#3ba272','#fc8452','#9a60b4','#ea7ccc'];
        // ECharts 6 gives axis labels a fixed grey instead of the text color, and
        // its grid lines a light grey, bright on a dark card: the theme's border.
        const gridLine = color => !color || BORDER.has(String(color).toLowerCase()) ? palette.border : color;
        for (const name of ['xAxis','yAxis','radiusAxis','angleAxis']) {
            const axis = value => value && {...value,axisLabel:{color:palette.text,...value.axisLabel},
                splitLine:{...value.splitLine,lineStyle:{...value.splitLine?.lineStyle,
                    color:gridLine(value.splitLine?.lineStyle?.color)}}};
            if (Array.isArray(themed[name])) themed[name] = themed[name].map(axis);
            else if (themed[name]) themed[name] = axis(themed[name]);
        }
        // Tooltips are the house's cards, as the map's; confined, they stay on
        // screen on narrow phones. The shadow under a hovered bar is a faint wash
        // of the text colour, whatever the theme.
        // A plain text tooltip (richText) keeps ECharts' own.
        const tooltip = value => ({confine:true,
            // An axis tooltip is handed a list of points, an item's one point.
            ...(value?.renderMode === 'richText' ? {} : {formatter:params => Array.isArray(params)
                ? axisTip(params, value?.valueFormatter) : itemTip(params, value?.valueFormatter)}),
            ...value,
            backgroundColor:palette.surface,borderColor:palette.border,borderWidth:1,borderRadius:8,
            padding:[8,12],className:'mesh-chart-tip',
            // Themed again on a theme change, an option already has its shadow.
            extraCssText:(value?.extraCssText || '').includes('--mesh-shadow') ? value.extraCssText
                : 'box-shadow:var(--mesh-shadow);'+(value?.extraCssText || ''),
            textStyle:{fontFamily:palette.font,fontSize:12,lineHeight:18,...value?.textStyle,color:palette.text},
            ...(value?.axisPointer?.type === 'shadow'
                ? {axisPointer:{...value.axisPointer,shadowStyle:{color:'rgba(128,140,160,.12)'}}} : {})});
        // A chart's tooltip as it sets it, or, drawn without one, the house's; an
        // update of a part (its legend, its zoom) leaves the chart's own alone.
        if (Array.isArray(themed.tooltip)) themed.tooltip = themed.tooltip.map(tooltip);
        else if (themed.tooltip || themed.series) themed.tooltip = tooltip(themed.tooltip);
        return themed;
    }
    // A tooltip a page writes itself, in the same shape as ECharts' own: a
    // title, maybe with an aside beside it, rows of a marker, a label and its
    // value at the right, and a note.
    // Every piece is escaped here but the markers, ECharts' own HTML.
    const esc = value => String(value ?? '').replace(/[&<>"']/g, c =>
        ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'})[c]);
    window.meshTip = ({title, aside, rows = [], note} = {}) =>
        (title != null ? `<div class="mesh-tip-title"><span>${esc(title)}</span>`
            + (aside ? `<span class="mesh-tip-aside">${esc(aside)}</span>` : '') + '</div>' : '')
        + rows.map(([label, value, marker]) => `<div class="mesh-tip-row">${marker || ''}`
            + `<span class="mesh-tip-label">${esc(label)}</span><b>${esc(value)}</b></div>`).join('')
        + (note ? `<div class="mesh-tip-note">${esc(note)}</div>` : '');

    // ECharts' own tooltips in that shape too, numbers in the page's language.
    const number = value => {
        const v = Array.isArray(value) ? value[value.length - 1] : value;
        return typeof v === 'number' ? v.toLocaleString(window.MeshI18n?.language || undefined) : v;
    };
    // A chart's valueFormatter, if it has one, writes the values. A series
    // without a name has one ECharts makes up ("series0"): it shows none.
    const seriesName = p => /^\0?series\0?\d+$/.test(p.seriesName ?? '') ? '' : p.seriesName;
    // On a time axis, the moment in the page's language; a series with nothing
    // at that moment has no row.
    const moment = p => p?.axisType?.endsWith('time') && Number.isFinite(+p.axisValue)
        ? new Date(+p.axisValue).toLocaleString(window.MeshI18n?.language || undefined,
            {day:'numeric', month:'short', hour:'2-digit', minute:'2-digit'})
        : p?.axisValueLabel;
    const empty = value => value == null || (Array.isArray(value) && value[value.length - 1] == null);
    const axisTip = (params, format = number) => {
        const list = [].concat(params);
        return window.meshTip({title:moment(list[0]),
            rows:list.filter(p => !empty(p.value)).map(p => [seriesName(p), format(p.value), p.marker])});
    };
    const itemTip = (p, format = number) => window.meshTip(p.percent != null
        ? {rows:[[p.name, `${Math.round(p.percent)}% · ${format(p.value)}`, p.marker]]}
        : {title:seriesName(p) || undefined, rows:[[p.name, format(p.value), p.marker]]});

    const wrapped = new WeakSet();
    window.initMeshChart = (...args) => {
        const chart = window.echarts.init(...args);
        if (!wrapped.has(chart)) {
            const setOption = chart.setOption.bind(chart);
            chart.setOption = (option,...rest) => setOption(chartOption(option),...rest);
            // Resizing: only when its box did change size, which the window's
            // resize and the box's own observer both ask for, and at once,
            // without animating, as a window is dragged wider or narrower.
            const resize = chart.resize.bind(chart);
            let size = '';
            chart.resize = options => {
                const box = chart.getDom();
                const now = `${box.clientWidth}x${box.clientHeight}`;
                if (!options && now === size) return;
                size = now;
                resize({animation: {duration: 0}, ...options});
            };
            wrapped.add(chart);
        }
        return chart;
    };

    // Whatever was drawn with the old colors draws again with the new ones.
    let fading = 0;
    function apply() {
        // A fade into the new colours, for as long as it lasts.
        root.classList.add('mesh-theme-changing');
        clearTimeout(fading);
        fading = setTimeout(() => root.classList.remove('mesh-theme-changing'), 400);
        root.dataset.theme = effective();
        root.dataset.accent = accent;
        root.dataset.mapTheme = mapTheme || effective();
        if (window.echarts) {
            document.querySelectorAll('[_echarts_instance_]').forEach(el => {
                const chart = window.echarts.getInstanceByDom(el);
                // One still waiting for its data has nothing to recolour yet.
                if (chart?.getOption()) chart.setOption(chartOption(chart.getOption()));
            });
        }
        document.dispatchEvent(new CustomEvent('meshview:themechange', {detail:{theme:effective(), accent}}));
    }

    window.MeshPrefs = {
        /** 'light', 'dark', or 'auto' for the system's. */
        get theme() { return theme || 'auto'; },
        set theme(value) {
            theme = ['light', 'dark'].includes(value) ? value : null;
            write(THEME_KEY, theme);
            apply();
        },
        get accent() { return accent; },
        set accent(value) {
            accent = ACCENTS.includes(value) ? value : 'blue';
            write(ACCENT_KEY, accent === 'blue' ? null : accent);
            apply();
        },
        /** The maps': 'light', 'dark', or 'page' for the page's theme. */
        get mapTheme() { return mapTheme || 'page'; },
        set mapTheme(value) {
            mapTheme = ['light', 'dark'].includes(value) ? value : null;
            write(MAP_KEY, mapTheme);
            apply();
        },
    };

    system.addEventListener('change', () => { if (!theme) apply(); });
    // Another tab changed them.
    window.addEventListener('storage', event => {
        if (![THEME_KEY, ACCENT_KEY, MAP_KEY, null].includes(event.key)) return;
        theme = read(THEME_KEY);
        if (!['light', 'dark'].includes(theme)) theme = null;
        accent = read(ACCENT_KEY);
        if (!ACCENTS.includes(accent)) accent = 'blue';
        mapTheme = read(MAP_KEY);
        if (!['light', 'dark'].includes(mapTheme)) mapTheme = null;
        apply();
    });
})();
