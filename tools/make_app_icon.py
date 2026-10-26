"""Hashi のアプリアイコン(rust/crates/hashi-desktop-tauri/icons/)を作る。

大きな画像を縮小すると細い線がつぶれるので、大きさごとに描き分ける。
四角形の辺は画素の境目にそろえ、曲線だけを滑らかにする。
形は「橋桁(水平の太線) + その下の太いアーチ」だけにし、16px でも見分けられるようにする。
色に頼らずに読めるよう、濃い地に白で描く(グレースケールでも明暗差が残る)。

使い方: python tools/make_app_icon.py [出力フォルダ]
Pillow が必要(開発時だけ。実行時の依存には入れない)。
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw

TILE = (15, 118, 110, 255)  # 従来のアイコンと同じティール
INK = (255, 255, 255, 255)
SCALE = 8  # 描画の細かさ(この倍率で描いてから縮小する)
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 256)
DEFAULT_OUT = Path(__file__).resolve().parent.parent / "rust/crates/hashi-desktop-tauri/icons"


def geometry(size: int) -> dict[str, int]:
    """大きさごとの寸法(画素単位の整数)。"""
    margin = 0 if size < 32 else round(size / 32)
    stroke = max(2, round(size * 0.115))
    deck_top = round(size * 0.31)
    # アーチの頂上を橋桁へ食い込ませ、橋桁とアーチを一つの形にする(接するだけだと小さいとき隙間に見える)
    overlap = max(1, stroke // 2)
    outer = round(size * 0.33)
    left = margin + max(2, round(size * 0.12))
    return {
        "margin": margin,
        "radius": max(3, round(size * 0.22)),
        "stroke": stroke,
        "deck_top": deck_top,
        "deck_bottom": deck_top + stroke,
        "outer": outer,
        "base": deck_top + overlap + outer,  # アーチの足元
        "left": left,
        "right": size - left,
    }


def draw(size: int) -> Image.Image:
    g = geometry(size)
    big = size * SCALE
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def px(v: float) -> int:
        return round(v * SCALE)

    m = g["margin"]
    d.rounded_rectangle(
        (px(m), px(m), px(size - m) - 1, px(size - m) - 1), radius=px(g["radius"]), fill=TILE
    )

    left, right = g["left"], g["right"]
    cx = size / 2
    outer = g["outer"]
    inner = outer - g["stroke"]
    # アーチ: 外側の半円から内側の半円をくり抜き、足元より下を切る
    arch = Image.new("L", (big, big), 0)
    a = ImageDraw.Draw(arch)
    a.ellipse(
        (px(cx - outer), px(g["base"] - outer), px(cx + outer) - 1, px(g["base"] + outer) - 1), fill=255
    )
    a.ellipse(
        (px(cx - inner), px(g["base"] - inner), px(cx + inner) - 1, px(g["base"] + inner) - 1), fill=0
    )
    a.rectangle((0, px(g["base"]), big, big), fill=0)
    img.paste(INK, (0, 0), arch)

    # 橋桁(アーチより幅広く、両岸へ渡す)
    d.rectangle((px(left), px(g["deck_top"]), px(right) - 1, px(g["deck_bottom"]) - 1), fill=INK)

    return img.resize((size, size), Image.Resampling.BOX)


def main() -> None:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUT
    out.mkdir(parents=True, exist_ok=True)
    images = {size: draw(size) for size in (*ICO_SIZES, 128, 1024)}
    largest = images[256]
    largest.save(
        out / "icon.ico",
        format="ICO",
        sizes=[(s, s) for s in ICO_SIZES],
        append_images=[images[s] for s in ICO_SIZES if s != 256],
    )
    images[32].save(out / "32x32.png")
    images[128].save(out / "128x128.png")
    images[1024].save(out / "icon.png")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
