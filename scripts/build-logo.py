"""Regenerate the driveby logo from clean vector geometry.

The original source-1024.png was a lossy raster flattened onto a white
background and then keyed to transparency, which baked a 3-6px white halo
into the opaque pixels all around the shape. The halo is invisible on white
but shows as a bright outline on any dark surface (tray, taskbar, dark
titlebar), and it makes recolouring impossible. This module replaces it with
the measured geometry, so the artwork is crisp at every size and the palette
is a one-line change.

Geometry below was least-squares fitted against the original raster; the
resulting outer silhouette matches it at 99.3% IoU.

Outputs (written next to src-tauri/icons/):
  logo.svg          editable master
  source-1024.png   raster master consumed by `npm run tauri icon`

Usage:  py scripts/build-logo.py
Requires Pillow. Rasterising is done here rather than from the SVG so the
project needs no SVG renderer; both outputs come from the same constants.
"""

import math
import os
from PIL import Image, ImageDraw, ImageMath

# ------------------------------------------------------------------ palette
BODY = (0xA8, 0xB0, 0xBA)   # drive body   (was #DC2626)
ARROW = (0x00, 0x00, 0x00)  # arrow        (was a transparent cut-out)
BAR = (0x01, 0x03, 0x12)    # front panel
PILL = (0xFE, 0x51, 0x1F)   # activity bar
LED_B = (0x02, 0xB3, 0xFF)
LED_G = (0x60, 0xD0, 0x4A)
LED_R = (0xFC, 0x51, 0x22)

# ----------------------------------------------------------------- geometry
SIZE = 1024.0
CX = 512.0

Y_TOP = 183.0      # top edge of the body
HW_TOP = 308.6     # half-width at the top edge
SLOPE = 0.3621     # half-width gained per pixel of descent
R_BODY = 55.0      # top corner radius
Y_BODY_B = 607.0   # bottom of the body (hidden behind the front panel)

A_HW = 47.0        # arrow shaft half-width
A_HEAD = 365.0     # y of the barbs
A_HHW = 148.0      # arrow head half-width
A_TIP = 540.0      # y of the point

B_X0, B_X1 = 46.0, 978.0
B_Y0, B_Y1 = 598.5, 843.5
B_RT, B_RB = 40.0, 50.0     # panel corner radii, top / bottom

P_X0, P_X1, P_Y0, P_Y1 = 139.0, 466.0, 701.0, 735.0
LED_RADIUS, LED_Y = 27.0, 717.0
LED_X = (698.5, 783.5, 868.5)

SS = 4             # supersampling factor used when rasterising

# ------------------------------------------------------------------ outlines


def arc(cx, cy, r, a0, a1, step=0.75):
    """Flatten a circular arc to points. Screen coords, so increasing angle
    runs clockwise."""
    n = max(2, int(abs(a1 - a0) / step) + 1)
    return [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
             cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n)))
            for i in range(n + 1)]


def body_corner(side):
    """Rounded top corner. side=-1 left, +1 right.

    The body widens downward, so the interior angle at the top corners is
    obtuse and the tangent length is R/tan(theta/2), shorter than R.
    Returns (centre, top tangent, flank tangent, start angle, end angle).
    """
    hyp = math.hypot(SLOPE, 1.0)
    theta = math.acos(-SLOPE / hyp)
    t = R_BODY / math.tan(theta / 2.0)
    sharp = (CX + side * HW_TOP, Y_TOP)
    tang_top = (sharp[0] - side * t, Y_TOP)
    centre = (tang_top[0], Y_TOP + R_BODY)
    tang_side = (sharp[0] + side * t * SLOPE / hyp, Y_TOP + t / hyp)

    def angle(p):
        return math.degrees(math.atan2(p[1] - centre[1], p[0] - centre[0])) % 360.0

    if side < 0:
        return centre, tang_top, tang_side, angle(tang_side), 270.0
    return centre, tang_top, tang_side, 270.0, angle(tang_side)


def edge_x(y, side):
    return CX + side * (HW_TOP + SLOPE * (y - Y_TOP))


def body_poly():
    cl, _, _, l0, l1 = body_corner(-1)
    cr, _, _, r0, r1 = body_corner(+1)
    return (arc(*cl, R_BODY, l0, l1) + arc(*cr, R_BODY, r0, r1)
            + [(edge_x(Y_BODY_B, +1), Y_BODY_B), (edge_x(Y_BODY_B, -1), Y_BODY_B)])


def arrow_poly():
    return [(CX - A_HW, Y_TOP), (CX + A_HW, Y_TOP), (CX + A_HW, A_HEAD),
            (CX + A_HHW, A_HEAD), (CX, A_TIP), (CX - A_HHW, A_HEAD),
            (CX - A_HW, A_HEAD)]


def bar_poly():
    return (arc(B_X1 - B_RT, B_Y0 + B_RT, B_RT, 270, 360)
            + arc(B_X1 - B_RB, B_Y1 - B_RB, B_RB, 0, 90)
            + arc(B_X0 + B_RB, B_Y1 - B_RB, B_RB, 90, 180)
            + arc(B_X0 + B_RT, B_Y0 + B_RT, B_RT, 180, 270))


def pill_poly():
    r = (P_Y1 - P_Y0) / 2.0
    return arc(P_X1 - r, P_Y0 + r, r, 270, 450) + arc(P_X0 + r, P_Y0 + r, r, 90, 270)


# ------------------------------------------------------------- rasterising


def _mask(draw_on, size):
    m = Image.new("L", (int(SIZE * SS), int(SIZE * SS)), 0)
    draw_on(ImageDraw.Draw(m))
    # BOX is an exact area average: correct coverage, and unlike LANCZOS it has
    # no negative lobes, so a binary mask downsamples without ringing.
    return m.resize((size, size), Image.BOX)


def mask_poly(poly, size):
    return _mask(lambda d: d.polygon([(x * SS, y * SS) for x, y in poly], fill=255), size)


def mask_circle(cx, cy, r, size):
    box = [(cx - r) * SS, (cy - r) * SS, (cx + r) * SS, (cy + r) * SS]
    return _mask(lambda d: d.ellipse(box, fill=255), size)


def render(size):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    layers = [(body_poly(), BODY), (arrow_poly(), ARROW),
              (bar_poly(), BAR), (pill_poly(), PILL)]
    for poly, colour in layers:
        img.paste(Image.new("RGBA", (size, size), colour + (255,)), (0, 0),
                  mask_poly(poly, size))
    for x, colour in zip(LED_X, (LED_B, LED_G, LED_R)):
        img.paste(Image.new("RGBA", (size, size), colour + (255,)), (0, 0),
                  mask_circle(x, LED_Y, LED_RADIUS, size))

    # paste() blends the alpha channel too, which leaves edge pixels
    # premultiplied. PNG expects straight alpha, so divide it back out or the
    # antialiased rim renders as a dark fringe.
    r, g, b, a = img.split()
    ev = getattr(ImageMath, "unsafe_eval", None) or ImageMath.eval

    def straighten(c):
        return ev("convert(min((c*255 + a/2) / max(a, 1), 255), 'L')", c=c, a=a)

    return Image.merge("RGBA", (straighten(r), straighten(g), straighten(b), a))


# -------------------------------------------------------------------- svg


def svg():
    def f(v):
        return f"{v:.2f}".rstrip("0").rstrip(".")

    def hx(c):
        return "#%02X%02X%02X" % c

    _, ttl, tsl, *_ = body_corner(-1)
    _, ttr, tsr, *_ = body_corner(+1)
    body = (f"M {f(ttl[0])} {f(ttl[1])} L {f(ttr[0])} {f(ttr[1])} "
            f"A {f(R_BODY)} {f(R_BODY)} 0 0 1 {f(tsr[0])} {f(tsr[1])} "
            f"L {f(edge_x(Y_BODY_B, 1))} {f(Y_BODY_B)} "
            f"L {f(edge_x(Y_BODY_B, -1))} {f(Y_BODY_B)} "
            f"L {f(tsl[0])} {f(tsl[1])} "
            f"A {f(R_BODY)} {f(R_BODY)} 0 0 1 {f(ttl[0])} {f(ttl[1])} Z")
    arrow = "M " + " L ".join(f"{f(x)} {f(y)}" for x, y in arrow_poly()) + " Z"
    bar = (f"M {f(B_X0 + B_RT)} {f(B_Y0)} L {f(B_X1 - B_RT)} {f(B_Y0)} "
           f"A {f(B_RT)} {f(B_RT)} 0 0 1 {f(B_X1)} {f(B_Y0 + B_RT)} "
           f"L {f(B_X1)} {f(B_Y1 - B_RB)} "
           f"A {f(B_RB)} {f(B_RB)} 0 0 1 {f(B_X1 - B_RB)} {f(B_Y1)} "
           f"L {f(B_X0 + B_RB)} {f(B_Y1)} "
           f"A {f(B_RB)} {f(B_RB)} 0 0 1 {f(B_X0)} {f(B_Y1 - B_RB)} "
           f"L {f(B_X0)} {f(B_Y0 + B_RT)} "
           f"A {f(B_RT)} {f(B_RT)} 0 0 1 {f(B_X0 + B_RT)} {f(B_Y0)} Z")
    leds = "\n".join(
        f'  <circle cx="{f(x)}" cy="{f(LED_Y)}" r="{f(LED_RADIUS)}" fill="{hx(c)}"/>'
        for x, c in zip(LED_X, (LED_B, LED_G, LED_R)))
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <title>driveby</title>
  <path d="{body}" fill="{hx(BODY)}"/>
  <path d="{arrow}" fill="{hx(ARROW)}"/>
  <path d="{bar}" fill="{hx(BAR)}"/>
  <rect x="{f(P_X0)}" y="{f(P_Y0)}" width="{f(P_X1 - P_X0)}" height="{f(P_Y1 - P_Y0)}" rx="{f((P_Y1 - P_Y0) / 2)}" fill="{hx(PILL)}"/>
{leds}
</svg>
'''


if __name__ == "__main__":
    icons = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                         os.pardir, "src-tauri", "icons")
    icons = os.path.normpath(icons)
    with open(os.path.join(icons, "logo.svg"), "w", encoding="utf-8") as fh:
        fh.write(svg())
    render(1024).save(os.path.join(icons, "source-1024.png"))
    print("wrote logo.svg and source-1024.png to", icons)
    print("next: npm run tauri icon src-tauri/icons/source-1024.png")
