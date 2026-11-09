"""Assemble manuscript Figures 1, 3 and 4 from the analysis outputs.

Every map panel is an SVG exported by Coxeter-MFV in the analysis folders
(zuniga/, steadycom/). Figure 1 joins the schematic fig1_schematic.svg to
fig1_interface.png, a screenshot of the web application in Firefox at a 1600 by
960 window and a device pixel ratio of 2: the syntrophic pair of Stolyar et al.
opened from the scenario list, solved with pFBA, in the Simulate tab with the
results panel open. This script only places panels, letters, legends and the
charts, then renders each figure with headless Firefox to a PNG 2400 px wide
(about 340 dpi at NAR's 17.8 cm page width). Figure 2 is composed by
yeast9/compose_solver_figure.py. Every manuscript figure is written to out/.

    python -I compose_figures.py
"""
import base64
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ANALYSIS = HERE.parent
ROOT = ANALYSIS.parent
sys.path.insert(0, str(ANALYSIS / "lib"))
from firefox_png import render  # noqa: E402

# Every image used in the manuscript lives in this one folder.
OUT = HERE / "out"
OUT.mkdir(parents=True, exist_ok=True)
WIDTH = 2400
# The page has 50 px side margins, so panels are this wide.
CONTENT = WIDTH - 100

INK = "#0b0b0b"
INK_2 = "#4b5563"
MUTED = "#8a8985"
GRID = "#e4e3df"
SE_GREEN = "#047857"   # S. elongatus, as its cell is drawn on the map
EC_ORANGE = "#b45309"  # E. coli, as its cell is drawn on the map
RATE_1 = "#2a78d6"     # categorical slot 1
RATE_2 = "#eb6834"     # categorical slot 2
FONT = "Arial, 'Liberation Sans', sans-serif"


def plain_text(svg, replacements=()):
    """Write the map's text as plain prose: commas for the middle-dot separators, and any other replacements.

    The renderer fits some text to a fixed length, so a text that changes drops
    that fit rather than being stretched over its old width.
    """
    def tidy(m):
        content = re.sub(r"\s*·\s*", ", ", m.group(2))
        for old, new in replacements:
            content = content.replace(old, new)
        if content == m.group(2):
            return m.group(0)
        attrs = re.sub(r'\s(textLength|lengthAdjust)="[^"]*"', "", m.group(1))
        return f"{attrs}{content}</text>"
    return re.sub(r"(<text\b[^>]*>)([^<]*)</text>", tidy, svg)


def inline(svg_path, view_box=None, css="", replacements=()):
    """An exported map, sized by its container instead of its own pixels, optionally cropped and restyled."""
    svg = plain_text(Path(svg_path).read_text(), replacements)
    if css:
        svg = re.sub(r'(<svg[^>]*>)', lambda m: m.group(1) + f"<style>{css}</style>", svg, count=1)
    svg = re.sub(r'(<svg[^>]*?)\swidth="[^"]*"', r"\1", svg, count=1)
    svg = re.sub(r'(<svg[^>]*?)\sheight="[^"]*"', r"\1", svg, count=1)
    if view_box:
        svg = re.sub(r'viewBox="[^"]*"', f'viewBox="{view_box}"', svg, count=1)
    return svg.replace("<svg", "<svg style='width:100%;height:auto;display:block'", 1)


def view_box(svg_path):
    box = re.search(r'viewBox="([\d.\-]+) ([\d.\-]+) ([\d.]+) ([\d.]+)"', Path(svg_path).read_text())
    return [float(box.group(i)) for i in range(1, 5)]


def growth_chart(scenarios):
    """Figure 3A, growth of each organism in the three scenarios, direct-labelled."""
    w, h = CONTENT, 400
    left, right, top, bottom = 150, 40, 56, 118
    plot_h = h - top - bottom
    vmax = 0.10
    y = lambda v: top + plot_h * (1 - v / vmax)
    parts = [f'<svg viewBox="0 0 {w} {h}" style="width:100%;height:auto" font-family="{FONT}">']
    for tick in (0, 0.05, 0.10):
        parts.append(f'<line x1="{left}" y1="{y(tick):.1f}" x2="{w - right}" y2="{y(tick):.1f}" '
                     f'stroke="{GRID if tick else MUTED}" stroke-width="{1.5 if tick else 2}"/>')
        parts.append(f'<text x="{left - 16}" y="{y(tick) + 10:.1f}" font-size="28" text-anchor="end" fill="{INK_2}">{tick:.2f}</text>')
    parts.append(f'<text x="{left - 130}" y="{top - 26}" font-size="28" fill="{INK_2}">Growth rate (h⁻¹)</text>')
    groups = [("wt-monoculture", "WT monoculture"),
              ("mutant-monoculture", "ΔpheA monoculture"),
              ("mutant-coculture", "ΔpheA coculture")]
    group_w = (w - left - right) / 3
    bar_w = 150
    for i, (key, label) in enumerate(groups):
        cx = left + group_w * (i + 0.5)
        growth = scenarios[key]["growth"]
        for j, (sp, name, color) in enumerate([("Ec", "E. coli", EC_ORANGE), ("Se", "S. elongatus", SE_GREEN)]):
            x0 = cx - bar_w - 6 + j * (bar_w + 12)
            if sp not in growth:
                parts.append(f'<text x="{x0 + bar_w / 2}" y="{y(0) - 14}" font-size="27" text-anchor="middle" fill="{MUTED}">absent</text>')
                continue
            v = growth[sp]
            if v > 0:
                top_y = y(v)
                r = 4
                parts.append(f'<path d="M{x0},{y(0)} V{top_y + r} Q{x0},{top_y} {x0 + r},{top_y} '
                             f'H{x0 + bar_w - r} Q{x0 + bar_w},{top_y} {x0 + bar_w},{top_y + r} V{y(0)} Z" fill="{color}"/>')
            parts.append(f'<text x="{x0 + bar_w / 2}" y="{(y(v) if v > 0 else y(0)) - 12}" font-size="28" '
                         f'text-anchor="middle" fill="{INK}">{v:.4f}</text>')
            parts.append(f'<text x="{x0 + bar_w / 2}" y="{y(0) + 36}" font-size="27" font-style="italic" '
                         f'text-anchor="middle" fill="{INK_2}">{name}</text>')
        parts.append(f'<text x="{cx}" y="{y(0) + 82}" font-size="30" font-weight="700" text-anchor="middle" fill="{INK}">{label}</text>')
    parts.append("</svg>")
    return "".join(parts)


def map_legend(stops, map_scale, title):
    """The color and width scale of an exported map, at the size the panel draws it."""
    w, h = CONTENT, 110
    step = (w - 40) / len(stops)
    parts = [f'<svg viewBox="0 0 {w} {h}" style="width:100%;height:auto" font-family="{FONT}">',
             f'<text x="0" y="30" font-size="28" fill="{INK}">{title}</text>']
    for i, stop in enumerate(stops):
        x = i * step
        dash = ' stroke-dasharray="10 7"' if stop["value"] == 0 else ""
        stroke = max(stop["width"] * map_scale, 2)
        parts.append(f'<line x1="{x:.1f}" x2="{x + step - 50:.1f}" y1="62" y2="62" stroke="{stop["color"]}" '
                     f'stroke-width="{stroke:.1f}"{dash}/>')
        label = f'{stop["value"]:.2g}' if stop["value"] else "0"
        if i == len(stops) - 1:
            label = "≥ " + label
        parts.append(f'<text x="{x + (step - 50) / 2:.1f}" y="{h - 6}" font-size="27" text-anchor="middle" fill="{INK}">{label}</text>')
    parts.append("</svg>")
    return "".join(parts)


def abundance_chart(points, members):
    """Figure 4A, each strain's abundance at both growth rates, with its SteadyCom FVA range."""
    w, h = CONTENT, 560
    left, right, top, bottom = 150, 40, 100, 90
    plot_h = h - top - bottom
    y = lambda v: top + plot_h * (1 - v)
    parts = [f'<svg viewBox="0 0 {w} {h}" style="width:100%;height:auto" font-family="{FONT}">']
    for tick in (0, 0.2, 0.4, 0.6, 0.8, 1.0):
        parts.append(f'<line x1="{left}" y1="{y(tick):.1f}" x2="{w - right}" y2="{y(tick):.1f}" '
                     f'stroke="{GRID if tick else MUTED}" stroke-width="{1.5 if tick else 2}"/>')
        parts.append(f'<text x="{left - 16}" y="{y(tick) + 10:.1f}" font-size="28" text-anchor="end" fill="{INK_2}">{tick:.1f}</text>')
    parts.append(f'<text x="{left - 130}" y="{top - 54}" font-size="28" fill="{INK_2}">Relative abundance</text>')
    group_w = (w - left - right) / len(members)
    series = [(points[0], RATE_1, f'μmax = {points[0]["mu"]:.5f} h⁻¹'),
              (points[1], RATE_2, f'0.5 μmax = {points[1]["mu"]:.5f} h⁻¹')]
    for i, member in enumerate(members):
        cx = left + group_w * (i + 0.5)
        for j, (point, color, _) in enumerate(series):
            x = cx - 80 + j * 160
            lo, hi = point["abundanceRanges"][member]
            parts.append(f'<rect x="{x - 15}" y="{y(hi):.1f}" width="30" height="{max(y(lo) - y(hi), 3):.1f}" '
                         f'rx="4" fill="{color}" fill-opacity="0.28" stroke="{color}" stroke-width="2"/>')
            value = point["abundances"][member]
            parts.append(f'<circle cx="{x}" cy="{y(value):.1f}" r="12" fill="{color}" stroke="#ffffff" stroke-width="3"/>')
            parts.append(f'<text x="{x + 26}" y="{y(value) + 10:.1f}" font-size="28" fill="{INK}">{value:.3f}</text>')
        parts.append(f'<text x="{cx}" y="{h - 34}" font-size="30" font-weight="700" text-anchor="middle" fill="{INK}">{member}</text>')
    # A dot and a bar swatch in each rate's color, then what the bars mean.
    lx = left + 380
    for point, color, label in series:
        parts.append(f'<circle cx="{lx}" cy="{top - 62}" r="11" fill="{color}"/>')
        parts.append(f'<rect x="{lx + 20}" y="{top - 78}" width="16" height="32" rx="3" fill="{color}" fill-opacity="0.28" '
                     f'stroke="{color}" stroke-width="2"/>')
        parts.append(f'<text x="{lx + 48}" y="{top - 52}" font-size="28" fill="{INK}">{label}</text>')
        lx += 520
    parts.append("</svg>")
    return "".join(parts)


AMINO_ACIDS = {"arg-L": "Arg", "lys-L": "Lys", "met-L": "Met", "phe-L": "Phe"}


def transfer_chart(points, route_colors, w, h):
    """Figure 4C, the flux of each amino acid transfer at both growth rates.

    The values are the cross-feeding links Coxeter-MFV computes for each
    solution (producer, consumer and flux per gDW of community). Each row is
    named in the color of its route on the map of Figure 4B.
    """
    links = {}
    for k, point in enumerate(points):
        for link in point["crossFeeding"]:
            links.setdefault((link["base"], link["from"], link["to"]), [0.0, 0.0])[k] = link["flux"]
    rows = sorted(links, key=lambda key: (list(AMINO_ACIDS).index(key[0]), key[2]))
    left, right, top, bottom = 340, 110, 70, 110
    vmax = max(max(v) for v in links.values()) * 1.05
    plot_w = w - left - right
    x = lambda v: left + plot_w * v / vmax
    row_h = (h - top - bottom) / len(rows)
    bar_h = row_h * 0.34
    parts = [f'<svg viewBox="0 0 {w} {h}" style="width:100%;height:auto" font-family="{FONT}">']
    for tick in (0, 0.02, 0.04, 0.06, 0.08):
        if tick > vmax:
            continue
        parts.append(f'<line x1="{x(tick):.1f}" y1="{top}" x2="{x(tick):.1f}" y2="{h - bottom}" '
                     f'stroke="{GRID if tick else MUTED}" stroke-width="{1.5 if tick else 2}"/>')
        parts.append(f'<text x="{x(tick):.1f}" y="{h - bottom + 38}" font-size="28" text-anchor="middle" fill="{INK_2}">{tick:g}</text>')
    parts.append(f'<text x="{left + plot_w / 2}" y="{h - 18}" font-size="28" text-anchor="middle" fill="{INK_2}">'
                 f'Flux (mmol h⁻¹ per gDW of community)</text>')
    for i, key in enumerate(rows):
        base, source, target = key
        y0 = top + i * row_h + row_h / 2
        color = route_colors.get(base, INK)
        parts.append(f'<line x1="18" x2="70" y1="{y0:.1f}" y2="{y0:.1f}" stroke="{color}" stroke-width="10" stroke-linecap="round"/>')
        parts.append(f'<text x="84" y="{y0 + 10:.1f}" font-size="29" font-weight="700" fill="{INK}">{AMINO_ACIDS[base]}</text>')
        parts.append(f'<text x="{left - 14}" y="{y0 + 10:.1f}" font-size="28" text-anchor="end" fill="{INK}">{source} → {target}</text>')
        for k, rate_color in enumerate((RATE_1, RATE_2)):
            value = links[key][k]
            yb = y0 - bar_h - 2 if k == 0 else y0 + 2
            parts.append(f'<rect x="{left}" y="{yb:.1f}" width="{max(x(value) - left, 1.5):.1f}" height="{bar_h:.1f}" fill="{rate_color}"/>')
            parts.append(f'<text x="{x(value) + 8:.1f}" y="{yb + bar_h - 4:.1f}" font-size="25" fill="{INK}">{value:.3f}</text>')
    parts.append("</svg>")
    return "".join(parts)


def page(body):
    # The font list carries its own quotes, so it goes in a style block rather
    # than inside a quoted style attribute.
    return (f"<!doctype html><html><head><meta charset='utf-8'><style>body {{ margin: 0; background: #fff; "
            f"font-family: {FONT}; color: {INK}; }}</style></head><body>"
            f"<div style='width:{WIDTH}px;padding:30px 50px;box-sizing:border-box'>"
            f"{body}</div></body></html>")


def panel(letter, title, content, note=""):
    note_html = f"<div style='font-size:28px;color:{INK_2};margin:4px 0 10px 60px'>{note}</div>" if note else ""
    return (f"<div style='margin:14px 0 22px'><div style='display:flex;align-items:baseline;gap:22px'>"
            f"<span style='font-size:46px;font-weight:700'>{letter}</span>"
            f"<span style='font-size:34px;font-weight:700'>{title}</span></div>{note_html}"
            f"<div style='margin-top:10px'>{content}</div></div>")


figures = {}

# Figure 1, the schematic above the application itself.
screenshot = base64.b64encode((HERE / "fig1_interface.png").read_bytes()).decode()
figures["Figure1"] = page(
    inline(HERE / "fig1_schematic.svg")
    + panel("C", "The application with a solved community",
            f"<img src='data:image/png;base64,{screenshot}' style='width:100%;display:block;"
            f"border:2px solid #cbd5e1;box-sizing:border-box'>"))

# Figure 3, the Zuñiga rescue. The map's own title repeats the caption, so the
# panel starts below it, and the transfer is named with its flux.
zuniga = json.loads((ANALYSIS / "zuniga" / "results" / "coxeter_zuniga_summary.json").read_text())
zuniga_map = ANALYSIS / "zuniga" / "results" / "zuniga_map_coculture_solved.svg"
zx, zy, zw, zh = view_box(zuniga_map)
crop_top = 110
transfer = zuniga["scenarios"]["mutant-coculture"]["pheTakenUpByEc"]
annotation = (f"<div style='position:absolute;left:{(952 - zx) / zw * 100:.2f}%;top:{(1124 - crop_top) / (zh - crop_top) * 100:.2f}%;"
              f"font-size:28px;font-weight:700;color:{INK}'>{transfer:.4f} mmol gDW⁻¹ h⁻¹</div>")
figures["Figure3"] = page(
    panel("A", "Growth in the three scenarios", growth_chart(zuniga["scenarios"]))
    + panel("B", "ΔpheA coculture on the two-cell map",
            "<div style='position:relative'>"
            + inline(zuniga_map, f"{zx} {crop_top} {zw} {zh - crop_top}",
                     # Cofactor names are too small to read in print and the
                     # cofactors themselves stay drawn.
                     ".emc-label-currency { display: none; }",
                     [("Shared medium (original smp)", "Shared medium")])
            + annotation + "</div>"
            + map_legend(zuniga["scale"]["legend"], CONTENT / zw, "Absolute flux (mmol gDW⁻¹ h⁻¹)")))

# Figure 4, the four auxotrophs at two community growth rates.
steady = json.loads((ANALYSIS / "steadycom" / "results" / "coxeter_steadycom_summary.json").read_text())
members = list(steady["points"][0]["abundances"])
four_cell = ANALYSIS / "steadycom" / "results" / "auxotrophs_map_mumax.svg"
fx, fy, fw, fh = view_box(four_cell)
# The map's own title and note sit above the shared medium and its cells.
map_top = 170
map_width = 1300
chart_width = CONTENT - map_width - 40
map_height = round(map_width * (fh - map_top) / fw)
# Each cell is named over its top edge, in its color on the map. The name
# stands this far (map units) above the edge, so that it clears the medium
# nodes of the cell's top exchanges, and the cross-feeding lines between the
# rows run above the names of the lower cells.
cell_name_top = 300
four_cell_document = json.loads((ROOT / "models" / "published" / "steadycom-ecoli-auxotrophs" / "map.json").read_text())
cells = four_cell_document["map"]["cells"]
cell_labels = ""
for cell in cells:
    name, needs, supplies = [part.strip() for part in cell["name"].split("·")]
    left = (cell["rect"]["x"] - fx) / fw * 100
    top = (cell["rect"]["y"] - map_top - cell_name_top) / (fh - map_top) * 100
    cell_labels += (f"<div style='position:absolute;left:{left:.2f}%;top:{top:.2f}%;font-size:27px;white-space:nowrap'>"
                    f"<b style='color:{cell['color']}'>{name}</b>&nbsp; {needs.replace(' + ', ' and ')}, {supplies}</div>")
figures["Figure4"] = page(
    panel("A", "Abundance of each strain", abundance_chart(steady["points"], members))
    + "<div style='display:flex;gap:40px;align-items:flex-start'>"
    + f"<div style='flex:0 0 {map_width}px'>" + panel("B", f"Four-cell map at μmax",
            "<div style='position:relative'>"
            # At this size every label of the map is a speck, so the map keeps
            # its drawing and the figure names the cells itself.
            + inline(four_cell, f"{fx} {map_top} {fw} {fh - map_top}",
                     ".emc-frame-label, .emc-label, .emc-map-text { display: none; }")
            + cell_labels + "</div>") + "</div>"
    + f"<div style='flex:0 0 {chart_width}px'>" + panel("C", "Amino acid transfers",
            transfer_chart(steady["points"], steady["routeColors"], chart_width, map_height)) + "</div>"
    + "</div>")

for name, html in figures.items():
    (OUT / f"{name}.html").write_text(html)
    print(name, render(OUT / f"{name}.html", OUT / f"{name}.png", WIDTH))
