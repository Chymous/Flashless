"""Downloads the champion square portraits from Riot's Data Dragon CDN (used under Riot's fan-content policy)
and packs them into assets/champions.bin for embedding, like the spell icons. Build time only: the app never
downloads images.

Format: repeated records of [u8 id length][id bytes (ASCII)][SIZE*SIZE*4 bytes RGBA].
Usage: python fetch_champion_portraits.py [--dry-run] [version]   (default version: the embedded snapshot's)"""
import io, json, os, sys, urllib.request
from PIL import Image

SIZE = 40   # drawn at 24-28 px; 40 keeps it sharp at 150% scaling
CROP = 6    # Data Dragon squares (120 px) have a thin dark frame
here = os.path.dirname(os.path.abspath(__file__))
args = [a for a in sys.argv[1:] if a != "--dry-run"]
dry = "--dry-run" in sys.argv

def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "flashless-build"}), timeout=30) as r:
        return r.read()

snapshot = json.load(open(os.path.join(here, "snapshot.json"), encoding="utf-8-sig"))
version = args[0] if args else snapshot["Version"]
ids = sorted(c["Id"] for c in snapshot["Champions"])
print(f"{len(ids)} champions, Data Dragon {version} -> assets/champions.bin ({SIZE}x{SIZE})")
if dry:
    sys.exit(0)
out = bytearray()
for cid in ids:
    png = get(f"https://ddragon.leagueoflegends.com/cdn/{version}/img/champion/{cid}.png")
    img = Image.open(io.BytesIO(png)).convert("RGBA")
    w, h = img.size
    img = img.crop((CROP, CROP, w - CROP, h - CROP)).resize((SIZE, SIZE), Image.LANCZOS)
    b = cid.encode("ascii")
    out += bytes([len(b)]) + b + img.tobytes()
open(os.path.join(here, "champions.bin"), "wb").write(out)
print(f"wrote {len(out)} bytes")
