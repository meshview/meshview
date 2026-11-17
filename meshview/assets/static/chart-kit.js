/* What the chart cards of the node and radio pages share: their periods, how
   they zoom, a legend whose item shown alone is a double click away, a legend
   that keeps inside its card, the pill toggles over them, and expanding one.
   Plain globals, as the pages' own scripts call them. */

// The charts show the latest 24 hours, 7 days or 30 days: no further, to
// spare the server.
const FINE_POINTER = window.matchMedia("(pointer: fine)").matches;

function chartDays(key) {
    return Number(document.querySelector(`.chart-period[data-chart="${key}"]`)?.value || 7);
}

function setChartDays(key, days) {
    const select = document.querySelector(`.chart-period[data-chart="${key}"]`);
    if (select) select.value = String(days);
}

function sinceUs(days) {
    return (Date.now() - days * 86400000) * 1000;
}

// Nothing in the period: a line in the middle of the card instead of the
// chart, and its tools off.
function showChartEmpty(card, empty) {
    card.querySelector(".chart-empty").hidden = !empty;
    card.querySelector(".chart-canvas").hidden = empty;
    card.querySelectorAll(".chart-tool").forEach(tool => { tool.disabled = empty; });
}

// With a mouse, dragging selects the span to zoom into and the wheel moves
// along time. On a touch screen a finger on the page has to scroll it, so the
// charts there do not zoom: expanded, they pinch and drag as usual. The box zoom
// only works with its toolbox shown: it is, out of sight.
function spanZoomOption(expanded = false) {
    if (!FINE_POINTER && !expanded) return {};
    return {
        toolbox: { show: true, left: -1000, top: -1000, showTitle: false,
            feature: { dataZoom: { yAxisIndex: "none" } } },
        dataZoom: [{ type: "inside", xAxisIndex: 0, filterMode: "none",
            ...(FINE_POINTER ? { zoomOnMouseWheel: false, moveOnMouseWheel: true, moveOnMouseMove: false } : {}) }]
    };
}

// The box zoom always on, and a double click to see it all again.
function enableSpanZoom(chart) {
    if (!FINE_POINTER) return;
    chart.dispatchAction({ type: "takeGlobalCursor", key: "dataZoomSelect", dataZoomSelectActive: true });
    chart.getZr().off("dblclick");
    chart.getZr().on("dblclick", () => chart.dispatchAction({ type: "dataZoom", start: 0, end: 100 }));
}

// Whether a click is the second on the same thing in a short while: a double
// click, or a double tap, told apart from two clicks that each toggle.
function secondClick(memory, key) {
    const now = Date.now();
    const second = memory.last?.key === key && now - memory.last.time < 400;
    memory.last = second ? null : { key, time: now };
    return second;
}

// Which of these names to show: only one, or, when it already is the only one,
// all of them again.
function soloSelection(names, name, selected) {
    const alone = names.every(other => other === name ? selected[other] !== false : selected[other] === false);
    return Object.fromEntries(names.map(other => [other, alone || other === name]));
}

// A legend whose item, clicked twice in a row, is shown alone, as in Plotly;
// and buttons at its end to show all of them or swap the shown and hidden.
function legendSelector() {
    return [{ type: "all", title: MeshI18n.t("All") }, { type: "inverse", title: MeshI18n.t("Invert") }];
}
function enableLegendSolo(chart) {
    const memory = {};
    chart.off("legendselectchanged");
    chart.on("legendselectchanged", event => {
        if (!secondClick(memory, event.name)) return;
        const names = Object.keys(event.selected);
        chart.setOption({ legend: { selected: soloSelection(names, event.name, event.selected) } });
    });
}

// A chart drawn into its card, with the option replacing what was there.
function drawChart(el, option, zoom = true) {
    const chart = echarts.getInstanceByDom(el) || initMeshChart(el);
    // Shown again after being hidden, the chart has to take its size anew.
    chart.resize();
    chart.setOption(option, { replaceMerge: ["series", "xAxis", "yAxis", "legend"] });
    fitLegend(chart);
    if (zoom) enableSpanZoom(chart);
    return chart;
}

/* Keep chart legends inside the card on narrow screens: the legend gets the
   card width and wraps, and the plot starts below however many rows it
   needs (ECharts does not move the grid for the legend by itself). */
const legendCanvas = document.createElement("canvas").getContext("2d");
function fitLegend(chart) {
    const option = chart.getOption();
    const legend = option.legend?.[0];
    if (!legend) return;
    const width = chart.getWidth() - 24;
    const labels = legend.data?.length
        ? legend.data.map(item => item?.name ?? item)
        : (option.series || []).map(series => series.name);
    // A hidden legend (direct receptions, toggled from its table) takes no room.
    let rows = legend.show === false ? 0 : 1;
    if (rows && legend.type !== "scroll") {
        legendCanvas.font = "12px Inter, sans-serif";
        let x = 0;
        for (const label of labels) {
            const item = legendCanvas.measureText(String(label)).width + 40;
            if (x && x + item > width) { rows++; x = 0; }
            x += item;
        }
    }
    const yAxes = [].concat(option.yAxis || []);
    const axisName = yAxes.some(axis => axis.name);
    chart.setOption({
        legend: { left: "center", top: 4, width },
        // Axis names start (left axis) or end (right axis) at their axis so
        // they are not cut by the card edge.
        yAxis: yAxes.map((axis, i) => ({ nameTextStyle: { align: i ? "right" : "left" } })),
        // A bottom subtitle (neighbors chart) needs room under the x labels.
        grid: { left: 12, right: 12, bottom: option.title?.[0]?.subtext ? 30 : 8, containLabel: true,
            top: 8 + rows * 22 + (axisName ? 30 : 12) }
    });
}

// Put each pill toggle's thumb under its pressed option; animated only once
// placed, so that it does not slide in from the left when its card first shows.
function placeToggleThumbs(root = document) {
    root.querySelectorAll(".pill-toggle").forEach(group => {
        const pressed = group.querySelector('[aria-pressed="true"]');
        if (!pressed || !pressed.offsetWidth) return;
        // offsetLeft counts from the group's padding edge, where the thumb starts.
        group.style.setProperty("--thumb-x", `${pressed.offsetLeft}px`);
        group.style.setProperty("--thumb-w", `${pressed.offsetWidth}px`);
        if (!group.classList.contains("is-ready")) requestAnimationFrame(() => group.classList.add("is-ready"));
    });
}

// Presses one option of a pill toggle, and slides its thumb there.
function pressToggle(button) {
    button.closest(".pill-toggle").querySelectorAll("button").forEach(other =>
        other.setAttribute("aria-pressed", String(other === button)));
    placeToggleThumbs(button.closest(".pill-toggle").parentElement);
}

/* Expanding a chart: a copy of it in the modal of chart-modal.html. Its
   canvas says what it does there: data-zoom="false" for one that does not
   zoom (bars), data-solo="true" for a legend whose items show alone. */
function expandChart(type) {
    const srcEl = document.getElementById(`chart_${type}`);
    const sourceChart = srcEl && echarts.getInstanceByDom(srcEl);
    if (!sourceChart) return;

    // A tooltip left open on the page would float above the modal.
    document.querySelectorAll("[_echarts_instance_]").forEach(el =>
        echarts.getInstanceByDom(el)?.dispatchAction({ type: "hideTip" }));
    const modal = document.getElementById("chartModal");
    const modalChart = initMeshChart(document.getElementById("modalChart"));
    modal.style.display = "flex";
    modalChart.setOption(sourceChart.getOption());
    modalChart.resize();
    fitLegend(modalChart);
    // Every chart over time zooms, as on the page with a mouse, or by pinching
    // on a touch screen, where only here it does.
    if (srcEl.dataset.zoom !== "false") {
        if (!FINE_POINTER) modalChart.setOption(spanZoomOption(true));
        enableSpanZoom(modalChart);
    }
    if (srcEl.dataset.solo === "true") enableLegendSolo(modalChart);
}

function closeModal() {
    document.getElementById("chartModal").style.display = "none";
}

// A CSV file of these rows, downloaded.
function downloadCsv(rows, filename) {
    const blob = new Blob([csvFromRows(rows)], { type: "text/csv" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = filename;
    link.click();
}

// The periods' labels, said short on a phone so that the card titles beside
// them keep room to break into two lines rather than run under them; each
// period, changed, loads its chart again through the loader given for it.
function initChartPeriods(loaders, fallback) {
    const narrow = window.matchMedia("(max-width: 700px)");
    const label = () => document.querySelectorAll(".chart-period option").forEach(option => {
        if (option.dataset.short) option.textContent = narrow.matches ? option.dataset.short : option.dataset.long;
    });
    narrow.addEventListener("change", label);
    label();
    document.querySelectorAll(".chart-period").forEach(select =>
        select.addEventListener("change", () => {
            const key = select.dataset.chart;
            const load = loaders[key] || (() => fallback(key));
            load().catch(error => console.error(error));
        }));
}

document.addEventListener("DOMContentLoaded", () => {
    // One handler for every chart on the page.
    window.addEventListener("resize", () => {
        document.querySelectorAll(".chart-canvas").forEach(el => {
            const chart = echarts.getInstanceByDom(el);
            if (chart && !el.hidden) { chart.resize(); fitLegend(chart); }
        });
        placeToggleThumbs();
    });
    document.fonts?.ready.then(() => placeToggleThumbs());
    // The modal chart follows its box, e.g. when a phone is rotated.
    const modalEl = document.getElementById("modalChart");
    if (modalEl) new ResizeObserver(() => {
        const chart = echarts.getInstanceByDom(modalEl);
        if (chart && document.getElementById("chartModal").style.display === "flex") {
            chart.resize();
            fitLegend(chart);
        }
    }).observe(modalEl);
});
