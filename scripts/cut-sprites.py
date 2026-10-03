#!/usr/bin/env python3
"""Cuts the generated sheets in assets/raw/ into sprites with a transparent background.

    python3 scripts/cut-sprites.py

assets/raw/<sheet>-<n>.jpg -> assets/sprites/<sheet>/<item>-<n>.png. A sheet is a grid of
items on a white background (prompts in assets/README.md). The background outside the
items is flooded from the border; the rest falls into connected pieces, and each piece
goes to the grid cell its centre is in, so a spire that reaches into the row above stays
with its church.
"""
import os
from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = os.path.join(os.path.dirname(__file__), '..', 'assets')
SHEETS = {
    'buildings': (4, 3, ['fort', 'castle', 'market', 'abbey', 'cathedral', 'cathedral-dome',
                         'dikes', 'milestone', 'scaffold', 'watchtower', 'windmill', 'pier']),
    'settlements': (4, 3, ['cottage', 'cottages', 'hamlet', 'town', 'town-tower', 'capital',
                           'crown', 'shield', 'shield-round', 'banner', 'swords', 'scroll']),
    'nature': (4, 3, ['tree', 'trees', 'conifer', 'bush', 'mountain', 'mountains',
                      'mountain-snow', 'hill', 'hills', 'field', 'reeds', 'rocks']),
    'sea': (3, 2, ['ship', 'cog', 'boat', 'serpent', 'whale', 'waves']),
}
# Enclosed white is part of these (a shield's field, a banner's cloth, snow, foam); every
# other item loses its near-white pixels too: the holes of a scaffold, under a market's
# canopy, a fort's gate.
KEEP = {'shield', 'shield-round', 'banner', 'scroll', 'mountain-snow', 'whale', 'waves'}
# Off the palette of the map (blue walls, green and blue roofs): not cut.
REJECT = {'capital-4', 'town-tower-2', 'hamlet-3', 'cottages-3', 'town-3'}
SIZE = 160  # the longest side of a sprite: twice the largest it is drawn on the map
MARK = (255, 0, 255)
STEP = 4  # components are found on a grid this coarse


def background(img):
    """The sheet with the pixels reachable from the border through near-white set to MARK."""
    work = img.copy()
    w, h = work.size
    seeds = [(x, y) for x in range(0, w, 8) for y in (0, h - 1)]
    seeds += [(x, y) for y in range(0, h, 8) for x in (0, w - 1)]
    for p in seeds:
        if work.getpixel(p) != MARK:
            ImageDraw.floodfill(work, p, MARK, thresh=40)
    return work


def components(solid):
    """8-connected components of the coarse grid: {label: [cells]}."""
    label, comps = {}, {}
    for start in solid:
        if start in label:
            continue
        n = len(comps)
        stack, cells = [start], []
        label[start] = n
        while stack:
            x, y = stack.pop()
            cells.append((x, y))
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    q = (x + dx, y + dy)
                    if q in solid and q not in label:
                        label[q] = n
                        stack.append(q)
        comps[n] = cells
    return comps


def main():
    raw = os.path.join(ROOT, 'raw')
    for f in sorted(os.listdir(raw)):
        sheet, n = f[:-4].rsplit('-', 1)
        cols, rows, items = SHEETS[sheet]
        img = Image.open(os.path.join(raw, f)).convert('RGB')
        w, h = img.size
        work = background(img)
        bg, near_white = Image.new('L', (w, h), 0), Image.new('L', (w, h), 0)
        pb, pw, pk, pi = bg.load(), near_white.load(), work.load(), img.load()
        for y in range(h):
            for x in range(w):
                if pk[x, y] == MARK:
                    pb[x, y] = 255
                r, g, b = pi[x, y]
                if min(r, g, b) > 232 and max(r, g, b) - min(r, g, b) < 14:
                    pw[x, y] = 255
        # A coarse cell is solid when anything of an item is in it.
        small = bg.resize((w // STEP, h // STEP), Image.Resampling.BOX).load()
        solid = {(x, y) for x in range(w // STEP) for y in range(h // STEP) if small[x, y] < 250}
        owner = {}
        for cells in components(solid).values():
            if len(cells) < 6:
                continue  # specks
            cx = sum(c[0] for c in cells) * STEP / len(cells)
            cy = sum(c[1] for c in cells) * STEP / len(cells)
            k = min(int(cy * rows / h), rows - 1) * cols + min(int(cx * cols / w), cols - 1)
            owner.setdefault(k, []).extend(cells)
        out = os.path.join(ROOT, 'sprites', sheet)
        os.makedirs(out, exist_ok=True)
        for k, item in enumerate(items):
            if '%s-%s' % (item, n) in REJECT:
                continue
            if k not in owner:
                print('empty', f, item)
                continue
            mine = Image.new('L', (w // STEP, h // STEP), 0)
            pm = mine.load()
            for c in owner[k]:
                pm[c] = 255
            mine = mine.resize((w, h), Image.Resampling.NEAREST).filter(ImageFilter.MaxFilter(5))
            a = ImageChops.subtract(mine, bg)
            if item not in KEEP:
                a = ImageChops.subtract(a, near_white)
            # Shave the jpeg halo by a pixel and soften the edge.
            a = a.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.GaussianBlur(0.6))
            box = a.point(lambda v: 255 if v > 24 else 0).getbbox()
            sprite = img.copy()
            sprite.putalpha(a)
            sprite = sprite.crop(box)
            sprite.thumbnail((SIZE, SIZE), Image.Resampling.LANCZOS)
            sprite.save(os.path.join(out, '%s-%s.png' % (item, n)), optimize=True)
        print(f)


main()
