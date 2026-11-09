"""Compose Figure 2 from the three Coxeter map panels and the FVA chart.

Panels A to C show the same part of the light-theme Yeast9 map, the
mitochondrion and the cytoplasm beside it, for FBA, pFBA and CycleFreeFlux.
Five reactions the text discusses are named on each panel with their flux in
that solution. Panel D draws ordinary FVA with the pFBA flux of each reaction.
The panels and the FVA chart stay with the analysis in results/solver-comparison.
The composed figure is the manuscript's Figure 2 and is written, like every
manuscript figure, to figures/out as Figure2.svg, Figure2.html and Figure2.png.

    python -I compose_solver_figure.py
"""

import json
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D

HERE = Path(__file__).resolve().parent
OUT = HERE / "results" / "solver-comparison"
# Every image used in the manuscript lives in this one folder.
FIGURES = HERE.parent / "figures" / "out"
sys.path.insert(0, str(HERE.parent / "lib"))
from firefox_png import render  # noqa: E402

FONT = "Arial, 'Liberation Sans', sans-serif"
INK = "#111111"
INK_2 = "#4b5563"
# 2400 px is NAR's 17.8 cm page width at about 340 dpi.
WIDTH = 2400
COLUMN = 1160
COLUMN_X = (20, 1220)
# The part of the map shown in panels A to C, in map units: the TCA cycle, the
# respiratory chain on the inner membrane and the cytoplasm to its right.
CROP = (3950, 850, 3000, 2000)
SCALE = COLUMN / CROP[2]
MAP_HEIGHT = round(CROP[3] * SCALE)

# Carbon metabolism, the reactions that change compartment between solutions,
# and representative amino acid synthesis steps.
SELECTED = [
    ("r_1714", "Glucose exchange"),
    ("r_0962", "PYK [c]"),
    ("r_2115", "ADH [c]"),
    ("r_2034", "Pyruvate transport [c,m]"),
    ("r_0302", "ACO step 1 [m]"),
    ("r_4262", "ACO direct [m]"),
    ("r_0658", "IDH NAD [m]"),
    ("r_2131", "IDH NADP [m]"),
    ("r_0659", "IDH NADP [c]"),
    ("r_0770", "External NADH DH [c,m]"),
    ("r_0773", "Internal NADH DH [m]"),
    ("r_0226", "ATP synthase [c,m]"),
    ("r_0714", "MDH [c]"),
    ("r_0123", "ACOX [p]"),
    ("r_0471", "GDH NADP [c]"),
    ("r_0476", "Gln synthetase [c]"),
    ("r_0207", "Argininosuccinate lyase [c]"),
    ("r_1088", "Val transaminase [m]"),
    ("r_0663", "Ile transaminase [c]"),
    ("r_0699", "Leu transaminase [c]"),
]

# The reactions named on panels A to C. Each label sits at (x, y) in map units,
# in free space near its reaction, and a leader line joins the two.
CALLOUTS = [
    ("r_0280", "ACO [m]", 5000, 1000),
    ("r_0658", "IDH NAD [m]", 5080, 1950),
    ("r_0770", "External NADH DH [c,m]", 5880, 960),
    ("r_0773", "Internal NADH DH [m]", 5100, 2420),
    ("r_0659", "IDH NADP [c]", 6380, 2600),
]
# Compartment names, at (x, y) in map units.
COMPARTMENTS = [("MITOCHONDRION", 4000, 2735), ("CYTOPLASM", 6560, 1420)]


def place_svg(source, x, y, width, height, view_box=None):
    svg = source[source.index("<svg"):]
    svg = re.sub(r'\swidth="[^"]*"', "", svg, count=1)
    svg = re.sub(r'\sheight="[^"]*"', "", svg, count=1)
    if view_box:
        svg = re.sub(r'viewBox="[^"]*"', f'viewBox="{view_box}"', svg, count=1)
    return svg.replace("<svg", f'<svg x="{x}" y="{y}" width="{width}" height="{height}"', 1)


def flux_text(value):
    """Two significant figures, an exact 0, and a minus sign rather than a hyphen."""
    if abs(value) < 1e-6:
        return "0"
    if abs(value) >= 100:
        text = f"{value:.0f}"
    elif abs(value) >= 1:
        text = f"{value:.2f}"
    else:
        text = f"{value:.2g}"
    return text.replace("-", "−")


def callouts(rows, positions, method, x0, y0):
    """The labels of one map panel, in figure pixels."""
    def to_figure(mx, my):
        return x0 + (mx - CROP[0]) * SCALE, y0 + (my - CROP[1]) * SCALE

    parts = []
    for rid, name, lx, ly in CALLOUTS:
        rx, ry = to_figure(*positions[rid])
        bx, by = to_figure(lx, ly)
        value = flux_text(rows[rid][method])
        box_w = 15.2 * max(len(name), len(value)) + 20
        box_h = 72
        # The leader leaves the box from its edge nearest the reaction.
        ex = min(max(rx, bx), bx + box_w)
        ey = by if ry < by else by + box_h
        parts.append(f'<line x1="{ex:.1f}" y1="{ey:.1f}" x2="{rx:.1f}" y2="{ry:.1f}" stroke="{INK}" stroke-width="2"/>')
        parts.append(f'<circle cx="{rx:.1f}" cy="{ry:.1f}" r="5" fill="{INK}"/>')
        parts.append(f'<rect x="{bx:.1f}" y="{by:.1f}" width="{box_w:.1f}" height="{box_h}" rx="6" '
                     f'fill="#ffffff" fill-opacity="0.94" stroke="{INK}" stroke-width="1.5"/>')
        parts.append(f'<text x="{bx + 10:.1f}" y="{by + 29:.1f}" font-family="{FONT}" font-size="27" '
                     f'fill="{INK}">{name}<tspan x="{bx + 10:.1f}" dy="32" font-weight="700">{value}</tspan></text>')
    for name, mx, my in COMPARTMENTS:
        cx, cy = to_figure(mx, my)
        anchor = "start" if mx < CROP[0] + CROP[2] / 2 else "middle"
        parts.append(f'<text x="{cx:.1f}" y="{cy:.1f}" text-anchor="{anchor}" font-family="{FONT}" font-size="26" '
                     f'font-weight="700" letter-spacing="1" fill="{INK_2}">{name}</text>')
    return parts


def fva_chart(by_id, path):
    plt.rcParams.update({"font.family": "Arial", "font.size": 19, "svg.fonttype": "none"})
    fig, ax = plt.subplots(figsize=(COLUMN / 100, 9.1))
    fig.subplots_adjust(left=0.36, right=0.97, top=0.93, bottom=0.10)
    for i, (rid, _) in enumerate(SELECTED):
        row = by_id[rid]
        ax.plot([row["fva"]["min"], row["fva"]["max"]], [i, i],
                color="#969da7", linewidth=4, marker="o", markersize=5)
        ax.plot(row["pfba"], i, marker="D", color="#c2410c", markersize=7,
                markeredgecolor="white", markeredgewidth=0.8, linestyle="none")
    ax.set_xscale("symlog", linthresh=0.05, linscale=0.8)
    ax.set_xlim(-1600, 1600)
    ax.set_xticks([-1000, -10, -1, 0, 1, 10, 1000])
    ax.set_xticklabels(["−1000", "−10", "−1", "0", "1", "10", "1000"])
    ax.set_yticks(range(len(SELECTED)))
    ax.set_yticklabels([label for _, label in SELECTED], fontsize=19)
    ax.set_ylim(len(SELECTED) - 0.4, -0.7)
    ax.axvline(0, color="#64748b", linewidth=0.8, zorder=0)
    ax.grid(axis="x", color="#e5e7eb", linewidth=0.8)
    ax.set_axisbelow(True)
    ax.set_xlabel("Flux (mmol gDW⁻¹ h⁻¹)", labelpad=10, fontsize=20)
    for side in ["top", "right", "left"]:
        ax.spines[side].set_visible(False)
    ax.tick_params(axis="y", length=0, pad=8)
    handles = [
        Line2D([0], [0], color="#969da7", marker="o", linewidth=4, markersize=5, label="FVA range"),
        Line2D([0], [0], color="#c2410c", marker="D", linestyle="none", markersize=7, label="pFBA"),
    ]
    fig.legend(handles=handles, loc="upper center", bbox_to_anchor=(0.66, 1.0), ncol=2, frameon=False)
    fig.savefig(path, facecolor="white")
    fig.savefig(path.with_suffix(".png"), dpi=300, facecolor="white")
    plt.close(fig)
    chart = path.read_text(encoding="utf-8")
    chart = re.sub(r"[ \t]+\n", "\n", chart)
    path.write_text(chart, encoding="utf-8")
    return chart


def main():
    result = json.loads((OUT / "comparison.json").read_text(encoding="utf-8"))
    by_id = {row["id"]: row for row in result["rows"]}
    drawing = json.loads((OUT / "figure_map.json").read_text(encoding="utf-8"))["map"]["reactions"]
    if isinstance(drawing, dict):
        drawing = list(drawing.values())
    positions = {}
    for reaction in drawing:
        positions[reaction["id"].split("@")[-1]] = (reaction["x"], reaction["y"])
    chart = fva_chart(by_id, OUT / "intervals_selected.svg")

    header = 50
    rows_y = (70, 935)
    height = 1880
    view_box = " ".join(str(v) for v in CROP)
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
    ]
    maps = [("A", "FBA", "fba", COLUMN_X[0], rows_y[0]), ("B", "pFBA", "pfba", COLUMN_X[1], rows_y[0]),
            ("C", "CycleFreeFlux", "cyclefree", COLUMN_X[0], rows_y[1])]
    for letter, title, method, x, y in maps:
        parts.append(f'<text x="{x}" y="{y - 20}" font-family="{FONT}" font-size="44" font-weight="700" '
                     f'fill="{INK}">{letter}<tspan dx="18" font-size="34">{title}</tspan></text>')
        source = (OUT / f"map_{method}.svg").read_text(encoding="utf-8")
        parts.append(place_svg(source, x, y, COLUMN, MAP_HEIGHT, view_box))
        parts.append(f'<rect x="{x}" y="{y}" width="{COLUMN}" height="{MAP_HEIGHT}" fill="none" '
                     f'stroke="#9ca3af" stroke-width="1.5"/>')
        parts.extend(callouts(by_id, positions, method, x, y))
    parts.append(f'<text x="{COLUMN_X[1]}" y="{rows_y[1] - 20}" font-family="{FONT}" font-size="44" font-weight="700" '
                 f'fill="{INK}">D<tspan dx="18" font-size="34">FVA</tspan></text>')
    parts.append(place_svg(chart, COLUMN_X[1], rows_y[1] - 10, COLUMN, 910))

    # The shared color and width scale of panels A to C, under panel C. Widths
    # are the map's own, in map units, shown at the panels' scale.
    legend_y = rows_y[1] + MAP_HEIGHT + 70
    parts.append(f'<text x="{COLUMN_X[0]}" y="{legend_y - 26}" font-family="{FONT}" font-size="27" fill="{INK}">'
                 f'Absolute flux in panels A to C (mmol gDW⁻¹ h⁻¹)</text>')
    stops = result["sharedScale"]["legend"]
    step = COLUMN / len(stops)
    for i, stop in enumerate(stops):
        x = COLUMN_X[0] + i * step + 10
        dash = ' stroke-dasharray="10 7"' if i == 0 else ""
        stroke = max(stop["width"] * SCALE, 2)
        parts.append(f'<line x1="{x:.1f}" x2="{x + step - 40:.1f}" y1="{legend_y + 10}" y2="{legend_y + 10}" '
                     f'stroke="{stop["color"]}" stroke-width="{stroke:.1f}"{dash}/>')
        label = f'{stop["value"]:g}'
        if i == len(stops) - 1:
            label = "≥ " + label
        parts.append(f'<text x="{x + (step - 40) / 2:.1f}" y="{legend_y + 52}" text-anchor="middle" '
                     f'font-family="{FONT}" font-size="27" fill="{INK}">{label}</text>')
    parts.append("</svg>")
    svg = "\n".join(parts)
    (FIGURES / "Figure2.svg").write_text(svg, encoding="utf-8")
    page = (f'<!doctype html><html><head><meta charset="utf-8"><style>body {{margin:0;background:#fff}}</style>'
            f'</head><body>{svg}</body></html>')
    (FIGURES / "Figure2.html").write_text(page, encoding="utf-8")
    print("Figure2", render(FIGURES / "Figure2.html", FIGURES / "Figure2.png", WIDTH))


if __name__ == "__main__":
    main()
