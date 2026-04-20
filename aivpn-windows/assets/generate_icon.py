#!/usr/bin/env python3
"""Generate AIVPN Windows .ico — exact replica of the Android adaptive icon.

Android drawable/ic_launcher.xml:
  background: #121218
  foreground: Material Security shield path at #6C5CE7
  pathData="M12,1L3,5v6c0,5.55 3.84,10.74 9,12
            c5.16,-1.26 9,-6.45 9,-12V5L12,1z
            M12,11.99h7c-0.53,4.12 -3.28,7.79 -7,8.94
            V12H5V6.3l7,-3.11V11.99z"

The path is a half-filled shield (right half filled, left half outline only).
"""

from PIL import Image, ImageDraw
import math, os


ACCENT = (0x6C, 0x5C, 0xE7)          # #6C5CE7
ACCENT_LIGHT = (0x8B, 0x7E, 0xF0)    # lighter half
BG_DARK = (0x12, 0x12, 0x18)         # #121218
STEPS = 32                            # curve interpolation resolution


def _cubic(p0, p1, p2, p3, n=STEPS):
    """Cubic Bézier → list of (x,y) points."""
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        x = u**3*p0[0] + 3*u**2*t*p1[0] + 3*u*t**2*p2[0] + t**3*p3[0]
        y = u**3*p0[1] + 3*u**2*t*p1[1] + 3*u*t**2*p2[1] + t**3*p3[1]
        pts.append((x, y))
    return pts


def _shield_outer():
    """Return the outer shield polygon in viewBox 0..24 coords.

    M12,1 L3,5 v6 c0,5.55 3.84,10.74 9,12
                   c5.16,-1.26 9,-6.45 9,-12 V5 L12,1 z
    """
    pts = [(12, 1), (3, 5)]
    # v6 → (3,11)
    pts.append((3, 11))
    # c0,5.55 3.84,10.74 9,12  (relative from 3,11)
    pts += _cubic((3, 11), (3, 16.55), (6.84, 21.74), (12, 23))
    # c5.16,-1.26 9,-6.45 9,-12  (relative from 12,23)
    pts += _cubic((12, 23), (17.16, 21.74), (21, 16.55), (21, 11))[1:]
    # V5
    pts.append((21, 5))
    # close back to (12,1) implicitly
    return pts


def _shield_inner():
    """Return the inner (left+bottom) half polygon.

    M12,11.99 h7 c-0.53,4.12 -3.28,7.79 -7,8.94
                  V12 H5 V6.3 l7,-3.11 V11.99 z
    """
    # Start at (12, 11.99)
    # h7 → (19, 11.99)
    # c-0.53,4.12 -3.28,7.79 -7,8.94  relative from (19,11.99)
    #   → control1 (18.47, 16.11), control2 (15.72, 19.78), end (12, 20.93)
    # V12 → (12, 12)
    # H5 → (5, 12)
    # V6.3 → (5, 6.3)
    # l7,-3.11 → (12, 3.19)
    # V11.99 → (12, 11.99)
    pts = [(12, 11.99), (19, 11.99)]
    pts += _cubic(
        (19, 11.99),
        (18.47, 16.11),
        (15.72, 19.78),
        (12, 20.93),
    )[1:]
    pts.append((12, 12))
    pts.append((5, 12))
    pts.append((5, 6.3))
    pts.append((12, 3.19))
    pts.append((12, 11.99))
    return pts


def draw_icon(size: int) -> Image.Image:
    """Render the Android-identical icon at *size* px."""
    # Work at 4× for anti-aliasing, then downscale
    ss = 4
    big = size * ss
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Rounded-rect background
    pad = big * 0.04
    draw.rounded_rectangle(
        [pad, pad, big - pad, big - pad],
        radius=big * 0.20,
        fill=BG_DARK,
    )

    margin = big * 0.16    # icon inset inside the rounded rect
    area = big - 2 * margin

    def sx(x):
        return margin + (x / 24.0) * area

    def sy(y):
        return margin + (y / 24.0) * area

    def scale(poly):
        return [(sx(x), sy(y)) for x, y in poly]

    # Draw outer shield — full accent colour
    draw.polygon(scale(_shield_outer()), fill=ACCENT)

    # Draw inner half — lighter shade to create the Material two-tone effect
    draw.polygon(scale(_shield_inner()), fill=ACCENT_LIGHT)

    # Downsample with high-quality Lanczos
    img = img.resize((size, size), Image.LANCZOS)
    return img


def main():
    sizes = [256, 128, 64, 48, 32, 16]
    images = [draw_icon(s) for s in sizes]

    out = os.path.join(os.path.dirname(__file__), "aivpn.ico")
    images[0].save(
        out,
        format="ICO",
        sizes=[(s, s) for s in sizes],
        append_images=images[1:],
    )
    print(f"Created {out}  ({os.path.getsize(out)} bytes)")

    png_out = os.path.join(os.path.dirname(__file__), "aivpn_preview.png")
    images[0].save(png_out)
    print(f"Preview: {png_out}")


if __name__ == "__main__":
    main()
