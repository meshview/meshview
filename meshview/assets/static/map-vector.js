/* The maps' base, as the preferences say: OpenStreetMap's PNG tiles (or the
   site's own tile server), or vector maps from OpenFreeMap's styles, sharp at
   any zoom. Vector maps are drawn by MapLibre GL JS as a layer of Leaflet's,
   through maplibre-gl-leaflet, so all that is drawn over them stays Leaflet's.
   MapLibre is loaded the first time a vector map is wanted. Where it can't
   draw (it wants WebGL 2) or fails to load, the map has the PNG tiles.

   window.meshTileLayer() gives a page's base layer; window.meshMapName() its
   name in the layer picker. Changing the map's theme or colours restyles a
   vector map in place. */
(() => {
    const root = document.documentElement;
    const VENDOR = '/static/vendor/';
    const MAPLIBRE = VENDOR + 'maplibre-gl-6.18.0/dist/';
    const PLUGIN = VENDOR + 'maplibre-gl-leaflet-0.1.4/leaflet-maplibre-gl.js';
    const OPENFREEMAP = 'https://tiles.openfreemap.org/styles/';
    const ATTRIBUTION = '<a href="https://openfreemap.org" target="_blank" rel="noopener">OpenFreeMap</a> '
        + '&copy; <a href="https://www.openmaptiles.org/" target="_blank" rel="noopener">OpenMapTiles</a> '
        + 'Data from <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener">OpenStreetMap</a>';

    const tiles = window.meshMapTiles;
    const png = (options = {}) => L.tileLayer(tiles.url, {
        maxZoom: tiles.max_zoom,
        attribution: tiles.attribution,
        ...options
    });

    // Whether this browser can draw them: MapLibre wants WebGL 2. Tried once,
    // on a canvas of its own, let go at once.
    let able;
    function supported() {
        if (able === undefined) {
            try {
                const gl = document.createElement('canvas').getContext('webgl2');
                able = !!gl;
                if (gl) gl.getExtension('WEBGL_lose_context')?.loseContext();
            } catch (_) {
                able = false;
            }
        }
        return able;
    }

    // Vector maps, if wanted and possible: never with the site's own tiles,
    // which usually means a network without Internet access.
    const vector = () => !tiles.custom && root.dataset.mapType === 'vector' && supported();

    // MapLibre, an ES module, made global for the plugin, then the plugin.
    let loading = null;
    function load() {
        loading ??= import(MAPLIBRE + 'maplibre-gl.mjs').then(library => {
            window.maplibregl = library;
            const css = document.createElement('link');
            css.rel = 'stylesheet';
            css.href = MAPLIBRE + 'maplibre-gl.css';
            document.head.append(css);
            return new Promise((resolve, reject) => {
                const script = document.createElement('script');
                script.src = PLUGIN;
                script.onload = resolve;
                script.onerror = reject;
                document.head.append(script);
            });
        });
        return loading;
    }

    // The style, as the preferences say: detailed, Liberty, or Fiord on a
    // dark map; vivid, Bright, made dark on a dark map, as it has no dark of
    // its own; plain, Positron, or Dark. As a key: its address, and "#dark"
    // when its colours are to be made dark.
    function styleKey() {
        const night = root.dataset.mapTheme === 'dark';
        const colors = root.dataset.mapColors;
        if (colors === 'vivid') return OPENFREEMAP + (night ? 'bright#dark' : 'bright');
        if (colors === 'gray') return OPENFREEMAP + (night ? 'dark' : 'positron');
        return OPENFREEMAP + (night ? 'fiord' : 'liberty');
    }

    // A colour made dark as the PNG tiles are, by the very filter they go
    // through: invert(1) hue-rotate(180deg) brightness(.85) saturate(.75).
    // Any colour CSS reads; null for anything else.
    const painter = document.createElement('canvas').getContext('2d');
    function darkened(css) {
        if (typeof css !== 'string' || !/^(#|rgb|hsl)/.test(css)) return null;
        painter.fillStyle = '#010203';
        painter.fillStyle = css;
        const read = painter.fillStyle;
        if (read === '#010203' && css.toLowerCase() !== '#010203') return null;
        let r, g, b, a = 1;
        if (read.startsWith('#')) {
            [r, g, b] = [1, 3, 5].map(i => parseInt(read.slice(i, i + 2), 16));
        } else {
            const parts = read.match(/[\d.]+/g).map(Number);
            [r, g, b] = parts;
            if (parts.length > 3) a = parts[3];
        }
        // Inverted, and its hue turned half round.
        [r, g, b] = [255 - r, 255 - g, 255 - b];
        [r, g, b] = [
            -0.574 * r + 1.43 * g + 0.144 * b,
            0.426 * r + 0.43 * g + 0.144 * b,
            0.426 * r + 1.43 * g - 0.856 * b
        ].map(v => Math.min(255, Math.max(0, v)));
        // Dimmed, and less saturated.
        [r, g, b] = [r, g, b].map(v => v * 0.85);
        const s = 0.75;
        [r, g, b] = [
            (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
            (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
            (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b
        ].map(v => Math.round(Math.min(255, Math.max(0, v))));
        return `rgba(${r}, ${g}, ${b}, ${a})`;
    }

    // Every colour of a paint's value made dark, within expressions too.
    const darkenAll = value => Array.isArray(value) ? value.map(darkenAll) : darkened(value) ?? value;

    // OpenFreeMap's road shields ask whether a road's reference is short, of
    // roads that may have none, and MapLibre warns of each: one with none is
    // taken as long, which leaves it out as well, warning of nothing.
    function quiet(expression) {
        if (!Array.isArray(expression)) return expression;
        if (expression.length === 2 && expression[0] === 'get' && expression[1] === 'ref_length') {
            return ['coalesce', expression, 99];
        }
        return expression.map(quiet);
    }

    function tidy(_before, style) {
        for (const layer of style.layers || []) {
            if (layer.filter) layer.filter = quiet(layer.filter);
        }
        return style;
    }

    function tidyDark(before, style) {
        tidy(before, style);
        for (const layer of style.layers || []) {
            for (const [name, value] of Object.entries(layer.paint || {})) {
                if (name.endsWith('color')) layer.paint[name] = darkenAll(value);
            }
        }
        return style;
    }

    function styled(glMap, key) {
        const [url, how] = key.split('#');
        glMap.setStyle(url, {diff: false, transformStyle: how === 'dark' ? tidyDark : tidy});
    }

    // An image a style asks for that its sprite lacks: given as nothing,
    // which is what is drawn of it anyway, rather than warned of.
    const NOTHING = {width: 1, height: 1, data: new Uint8Array(4)};

    // The base layers on this page, to change when the preferences do.
    const bases = new Set();

    // A page's base layer: it holds the PNG tiles, or the vector map once
    // MapLibre is loaded (the PNG tiles if it can't be), and changes from one
    // to the other, or restyles, as the preferences change.
    const BaseLayer = L.LayerGroup.extend({
        // The tiles' zoom limit from the start, as Leaflet's tile layers have:
        // a map fitted to a single place while MapLibre loads would otherwise
        // zoom without end.
        beforeAdd(map) {
            map._addZoomLimit(this);
        },
        onAdd(map) {
            L.LayerGroup.prototype.onAdd.call(this, map);
            bases.add(this);
            this._fill();
        },
        onRemove(map) {
            bases.delete(this);
            this._clear();
            L.LayerGroup.prototype.onRemove.call(this, map);
            map._removeZoomLimit(this);
        },
        _clear() {
            this._turn = (this._turn || 0) + 1;
            if (this._inner) this.removeLayer(this._inner);
            this._inner = this._kind = this._key = null;
        },
        _png() {
            this._inner = png();
            this.addLayer(this._inner);
        },
        _fill() {
            const turn = this._turn = (this._turn || 0) + 1;
            this._kind = vector() ? 'vector' : 'png';
            if (this._kind === 'png') return this._png();
            // Only once the map has a view: the vector map starts from it.
            const map = this._map;
            load().then(() => new Promise(resolve => map.whenReady(resolve))).then(() => {
                if (turn !== this._turn) return;
                const gl = L.maplibreGL({
                    style: {version: 8, sources: {}, layers: []},
                    maxZoom: tiles.max_zoom,
                    attributionControl: {customAttribution: ATTRIBUTION}
                });
                this._inner = gl;
                this.addLayer(gl);
                const glMap = gl.getMaplibreMap();
                glMap.setMissingStyleImageResolver(id => {
                    if (!glMap.hasImage(id)) glMap.addImage(id, NOTHING);
                });
                this._key = styleKey();
                styled(glMap, this._key);
            }).catch(error => {
                console.warn('Vector maps unavailable, PNG tiles instead:', error);
                if (turn === this._turn) this._png();
            });
        },
        refresh() {
            if ((vector() ? 'vector' : 'png') !== this._kind) {
                this._clear();
                this._fill();
                return;
            }
            const glMap = this._inner?.getMaplibreMap?.();
            const key = styleKey();
            if (glMap && key !== this._key) {
                this._key = key;
                styled(glMap, key);
            }
        }
    });

    document.addEventListener('meshview:themechange', () => bases.forEach(layer => layer.refresh()));

    window.meshTileLayer = () => new BaseLayer([], {maxZoom: tiles.max_zoom});
    window.meshMapName = () => vector() ? 'OpenFreeMap' : tiles.name;
    // Whether the vector maps can be offered here at all.
    window.meshVectorMaps = () => !tiles.custom && supported();
})();
