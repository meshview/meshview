/* A node's popup on the data pages' maps, laid out as the map page's node card:
   its short name on top, linked to its page (unless it is the page's own
   node), its long name under it, what it is here beside its id, and its facts,
   leaving out those it has none for, and figures, each its label over its
   value, in a row under them. Facts are text, escaped here, unless given as
   HTML already; a third item, true, marks one as data (a figure, a position, a
   version), set in mono by the technical style as the figures are. */
(() => {
    // A popup is drawn in the map, which never goes above the map's controls:
    // as it opens, the map moves it clear of them instead. Beside them on a
    // wide map; under the zoom on a narrow one, where a popup needs the width.
    if (window.L) {
        const adjustPan = L.Popup.prototype._adjustPan;
        L.Popup.include({
            _adjustPan(...args) {
                if (this._map) {
                    const narrow = this._map.getSize().x < 500;
                    this.options.autoPanPaddingTopLeft = narrow ? L.point(10, 90) : L.point(64, 64);
                    this.options.autoPanPaddingBottomRight = narrow ? L.point(10, 40) : L.point(64, 40);
                }
                return adjustPan.apply(this, args);
            }
        });
    }
    const hexId =id => Number.isInteger(Number(id)) ? ` (!${Number(id).toString(16).padStart(8, "0")})` : "";
    window.meshNodePopup = ({id, node, role, facts = [], figures = [], link = true, html = false}) => {
        const esc = window.escapeHtml;
        const unnamed = window.MeshI18n.t("Node {id}", {id});
        const short = node?.short_name || "";
        const long = node?.long_name && node.long_name !== short ? node.long_name : "";
        const head = short || long || unnamed;
        const top = link
            ? `<a class="node-popup-short" href="/node/${encodeURIComponent(id)}">${esc(head)}</a>`
            : `<span class="node-popup-short">${esc(head)}</span>`;
        const name = short && long ? `<div class="node-popup-name">${esc(long)}</div>` : "";
        const meta = [role, unnamed + hexId(id)].filter(Boolean).join(" · ");
        const rows = facts
            .filter(([, value]) => value != null && value !== "")
            .map(([label, value, data]) => `<dt>${esc(label)}</dt><dd${data ? ' class="is-data"' : ""}>${html ? value : esc(value)}</dd>`)
            .join("");
        const shown = figures.filter(([, value]) => value != null && value !== "")
            .map(([label, value]) => `<div><dt>${esc(label)}</dt><dd>${esc(value)}</dd></div>`)
            .join("");
        return `<div class="node-popup">${top}${name}<p class="node-popup-meta">${esc(meta)}</p>`
            + (rows ? `<dl class="node-popup-facts">${rows}</dl>` : "")
            + (shown ? `<dl class="node-popup-figures">${shown}</dl>` : "") + `</div>`;
    };
})();
