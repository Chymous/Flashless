"""App icon: the Flash spell art (assets/flash_source.png, Riot Games art used under Riot's fan-content
policy) on a rounded tile, with a League-style cooldown pie and a gold rim.
Writes app.ico (multi-size), icon_preview.png, icon_64.rgba and icon_128.rgba.
Usage: python make_icon.py <out_dir> [remaining_fraction]"""
import math, os, sys
from PIL import Image, ImageDraw, ImageFilter, ImageChops

here = os.path.dirname(os.path.abspath(__file__))
out = sys.argv[1] if len(sys.argv) > 1 else here
remaining = float(sys.argv[2]) if len(sys.argv) > 2 else 0.6   # share of the cooldown still to go
S = 1024
M = 40                      # tile margin
R = 170                     # tile corner radius

mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([M, M, S - M, S - M], radius=R, fill=255)

src = Image.open(os.path.join(here, "flash_source.png")).convert("RGBA")
inner = S - 2 * M
art = Image.new("RGBA", (S, S), (0, 0, 0, 0))
art.paste(src.resize((inner, inner), Image.LANCZOS), (M, M))
art.putalpha(ImageChops.multiply(art.getchannel("A"), mask))
cx = cy = S / 2

# cooldown pie: darken what is still cooling down, from 12 o'clock going clockwise
elapsed_deg = 360 * (1 - remaining)
pie = Image.new("L", (S, S), 0)
big = S * 1.5
ImageDraw.Draw(pie).pieslice([cx - big, cy - big, cx + big, cy + big], start=-90 + elapsed_deg, end=270, fill=255)
pie = ImageChops.multiply(pie, mask)
shade = Image.new("RGBA", (S, S), (6, 5, 4, 0))
shade.putalpha(pie.point(lambda v: int(v * 0.62)))
art = Image.alpha_composite(art, shade)

# the timer "hand" along the sweep edge
edge = Image.new("RGBA", (S, S), (0, 0, 0, 0))
a = math.radians(-90 + elapsed_deg)
ImageDraw.Draw(edge).line([(cx, cy), (cx + 720 * math.cos(a), cy + 720 * math.sin(a))], fill=(255, 236, 170, 235), width=12)
edge.putalpha(ImageChops.multiply(edge.getchannel("A"), mask))
art = Image.alpha_composite(art, edge.filter(ImageFilter.GaussianBlur(1.5)))

# gold rim
rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(rim).rounded_rectangle([M + 6, M + 6, S - M - 6, S - M - 6], radius=R - 6, outline=(212, 175, 55, 255), width=18)
art = Image.alpha_composite(art, rim)

os.makedirs(out, exist_ok=True)
art.save(os.path.join(out, "app.ico"), sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)])
art.resize((256, 256), Image.LANCZOS).save(os.path.join(out, "icon_preview.png"))
for n in (64, 128):
    open(os.path.join(out, f"icon_{n}.rgba"), "wb").write(art.resize((n, n), Image.LANCZOS).convert("RGBA").tobytes())
strip = Image.new("RGBA", (16 + 24 + 32 + 48 + 64 + 6 * 12, 76), (32, 32, 32, 255))
x = 12
for s in (16, 24, 32, 48, 64):
    strip.alpha_composite(art.resize((s, s), Image.LANCZOS), (x, 76 - 6 - s))
    x += s + 12
strip.save(os.path.join(out, "icon_sizes.png"))
print("icon written")
