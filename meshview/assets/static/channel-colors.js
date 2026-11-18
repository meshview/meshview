/* Channel colours, the same on every page that shows them (the maps, the
   chat): the one a visitor chose on the map page, kept in this browser; else
   the one the site configures for the channel; else one of a palette's, by the
   name. */
(() => {
    const PALETTE = ["#18aa18","#976049","#a5508c","#1898dc","#3a8846","#dc1884",
                     "#b8813d","#b64f35","#408b18","#4c719e","#be379c","#18aa52"];
    // Where the map page keeps the colours a visitor chose. Only plain #rrggbb
    // colours are taken from storage, so nothing else reaches the styles.
    const CHOSEN_KEY = "mapChannelColors";
    const HEX_COLOR = /^#[0-9a-f]{6}$/i;
    const configured = new Map();

    const unknown = () => window.MeshI18n ? MeshI18n.t("Unknown") : "Unknown";
    const normalize = name => String(name || unknown()).toLowerCase().replace(/[\s_-]+/g, "");
    function hashToUnit(text) {
        let h = 2166136261;
        for (let i = 0; i < text.length; i++) {
            h ^= text.charCodeAt(i);
            h = Math.imul(h, 16777619);
        }
        return (h >>> 0) / 0xffffffff;
    }
    function chosen() {
        try {
            const saved = JSON.parse(localStorage.getItem(CHOSEN_KEY) || "{}");
            return saved && typeof saved === "object" ? saved : {};
        } catch { return {}; }
    }

    // The site's configuration, asked for here if the page has not yet.
    window._siteConfigPromise ||= fetch("/api/v1/config")
        .then(res => res.json())
        .then(cfg => (window._siteConfig = cfg))
        .catch(() => ({}));
    const ready = window._siteConfigPromise.then(cfg => {
        for (const [channel, color] of Object.entries(cfg?.site?.map_channel_colors || {})) {
            configured.set(normalize(channel), color);
        }
    }).catch(() => {});

    window.MeshChannels = {
        palette: PALETTE,
        /** Resolves once the site's colours are known. */
        ready,
        /** A channel's colour leaving aside a visitor's choice. */
        defaultColor(channel) {
            const name = String(channel || unknown());
            return configured.get(normalize(name)) ||
                PALETTE[Math.floor(hashToUnit(name) * PALETTE.length) % PALETTE.length];
        },
        /** A channel's colour, as the maps draw it. */
        color(channel) {
            const color = chosen()[String(channel || unknown())];
            return typeof color === "string" && HEX_COLOR.test(color)
                ? color.toLowerCase() : this.defaultColor(channel);
        },
    };
})();
