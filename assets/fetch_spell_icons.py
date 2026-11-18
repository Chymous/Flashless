"""Downloads the summoner spell icons from Riot's Data Dragon CDN (used under Riot's fan-content policy)
and stores each as 64x64 raw RGBA in assets/spells/<SpellId>.rgba for embedding.
Usage: python fetch_spell_icons.py [version]"""
import io, json, os, sys, urllib.request
from PIL import Image

SPELLS = ["SummonerFlash", "SummonerDot", "SummonerBarrier", "SummonerExhaust", "SummonerHeal", "SummonerHaste",
          "SummonerBoost", "SummonerTeleport", "SummonerSmite", "SummonerMana", "SummonerSnowball"]
here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, "spells")
os.makedirs(out, exist_ok=True)

def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "flashless-build"}), timeout=30) as r:
        return r.read()

version = sys.argv[1] if len(sys.argv) > 1 else json.loads(get("https://ddragon.leagueoflegends.com/api/versions.json"))[0]
for sid in SPELLS:
    png = get(f"https://ddragon.leagueoflegends.com/cdn/{version}/img/spell/{sid}.png")
    img = Image.open(io.BytesIO(png)).convert("RGBA").resize((64, 64), Image.LANCZOS)
    open(os.path.join(out, sid + ".rgba"), "wb").write(img.tobytes())
    print(f"{sid}: {len(png)} bytes")
print("version", version)
