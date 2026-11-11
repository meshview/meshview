// meshview-rs's own (local/templates/marker-stroke.js), which scripts/import-web.py
// includes in the map: a node's dot gets the border that stands out more from its
// fill, white on dark colours and near-black on light ones (WCAG contrast). That
// shows it both on the light map and on the inverted one of night mode, without
// redrawing anything when the theme changes.
function markerStroke(fill){
    const hex = /^#([0-9a-f]{6})$/i.exec(String(fill || ""));
    if(!hex) return "#ffffff";
    const luminance = rgb => {
        const linear = i => {
            const c = parseInt(rgb.slice(i, i + 2), 16) / 255;
            return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
        };
        return 0.2126 * linear(0) + 0.7152 * linear(2) + 0.0722 * linear(4);
    };
    const fillLuminance = luminance(hex[1]);
    const dark = "#1b2430";
    const againstWhite = 1.05 / (fillLuminance + 0.05);
    const againstDark = (fillLuminance + 0.05) / (luminance(dark.slice(1)) + 0.05);
    return againstWhite >= againstDark ? "#ffffff" : dark;
}
