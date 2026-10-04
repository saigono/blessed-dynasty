#!/usr/bin/env python3
"""Cuts the generated sheets in assets/raw/ into sprites with a transparent background.

    python3 scripts/cut-sprites.py

assets/raw/<sheet>-<n>.jpg -> assets/sprites/<sheet>/<item>-<n>.png. A sheet is a grid of
items on a white background (prompts in assets/README.md). The background outside the
items is flooded from the border; the rest falls into connected pieces, and each piece
goes to the grid cell its centre is in, so a spire that reaches into the row above stays
with its church.

assets/raw/events/<theme>.jpg, the pictures of events, lose the plain paper around the
drawing and become 3:2 jpegs in sprites/events/.

assets/raw/tiles/ holds textures: sea-1 and paper-1 become seamless squares in
sprites/tiles/; the bands of strips-1 become strips in sprites/strips/ that repeat left to
right, to be laid along a border, a road or a river.
"""
import os
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageOps

ROOT = os.path.join(os.path.dirname(__file__), '..', 'assets')
SHEETS = {
    'buildings': (4, 3, ['fort', 'castle', 'market', 'abbey', 'cathedral', 'cathedral-dome',
                         'dikes', 'milestone', 'scaffold', 'watchtower', 'windmill', 'pier']),
    'settlements': (4, 3, ['cottage', 'cottages', 'hamlet', 'town', 'town-tower', 'capital',
                           'crown', 'shield', 'shield-round', 'banner', 'swords', 'scroll']),
    'nature': (4, 3, ['tree', 'trees', 'conifer', 'bush', 'mountain', 'mountains',
                      'mountain-snow', 'hill', 'hills', 'field', 'reeds', 'rocks']),
    'sea': (3, 2, ['ship', 'cog', 'boat', 'serpent', 'whale', 'waves']),
    'extras': (4, 3, ['fort-building', 'church-building', 'dikes-arc', 'bridge', 'fire', 'camp',
                      'ruins', 'revolt', 'boundary', 'graves', 'barn', 'siege']),
}
# A sheet whose cells are not a grid: the box of each item, in the pixels of the sheet.
BOXES = {
    'decor': {'compass': (18, 12, 295, 345), 'cartouche': (300, 12, 855, 238),
              'corner': (860, 12, 1137, 345), 'ribbon': (300, 244, 855, 345),
              'label': (18, 352, 295, 418),
              'wind': (18, 426, 378, 612), 'island': (384, 426, 770, 612),
              'serpent': (775, 426, 1137, 612), 'fish': (18, 618, 295, 850),
              'cloud': (300, 618, 575, 850), 'dividers': (580, 618, 855, 850),
              'medallion': (860, 618, 1137, 850)},
}
# Enclosed white is part of these (a shield's field, a banner's cloth, snow, foam); every
# other item loses its near-white pixels too: the holes of a scaffold, under a market's
# canopy, a fort's gate.
KEEP = {'shield', 'shield-round', 'banner', 'scroll', 'mountain-snow', 'whale', 'waves',
        'cartouche', 'ribbon', 'label', 'medallion', 'cloud', 'wind', 'fire', 'revolt'}
# Off the palette of the map (blue walls, green and blue roofs): not cut.
REJECT = {'capital-4', 'town-tower-2', 'hamlet-3', 'cottages-3', 'town-3'}
SIZE = 160  # the longest side of a sprite: twice the largest it is drawn on the map
MARK = (255, 0, 255)
STEP = 4  # components are found on a grid this coarse
# The bands of strips-1, top to bottom; None: drawn as a plain line instead.
# The dirt road's ruts run aslant and break where it repeats.
STRIPS = [None, None, 'border-band', None, 'road-cobble', 'river', 'coast']
TILE = 512
EVENT = (600, 400)


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


def cut(img, a, item, n, out):
    """Saves the part of img under the alpha a, trimmed and shrunk to SIZE."""
    a = a.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.GaussianBlur(0.6))
    box = a.point(lambda v: 255 if v > 24 else 0).getbbox()
    sprite = img.copy()
    sprite.putalpha(a)
    sprite = sprite.crop(box)
    sprite.thumbnail((SIZE, SIZE), Image.Resampling.LANCZOS)
    sprite.save(os.path.join(out, '%s-%s.png' % (item, n)), optimize=True)


def masks(img, item):
    """The background (reachable from the border) and the near-white of img, as masks."""
    work = background(img)
    w, h = img.size
    bg, near_white = Image.new('L', (w, h), 0), Image.new('L', (w, h), 0)
    pb, pw, pk, pi = bg.load(), near_white.load(), work.load(), img.load()
    for y in range(h):
        for x in range(w):
            if pk[x, y] == MARK:
                pb[x, y] = 255
            r, g, b = pi[x, y]
            if min(r, g, b) > 232 and max(r, g, b) - min(r, g, b) < 14:
                pw[x, y] = 255
    return bg, near_white


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
        if not f.endswith('.jpg'):
            continue
        sheet, n = f[:-4].rsplit('-', 1)
        img = Image.open(os.path.join(raw, f)).convert('RGB')
        out = os.path.join(ROOT, 'sprites', sheet)
        os.makedirs(out, exist_ok=True)
        if sheet in BOXES:
            for item, box in BOXES[sheet].items():
                part = img.crop(box)
                bg, near_white = masks(part, item)
                a = ImageChops.invert(bg)
                if item not in KEEP:
                    a = ImageChops.subtract(a, near_white)
                cut(part, a, item, n, out)
            print(f)
            continue
        cols, rows, items = SHEETS[sheet]
        w, h = img.size
        bg, near_white = masks(img, None)
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
            cut(img, a, item, n, out)
        print(f)


def seamless(img):
    """img blended with itself shifted by half, weighted to the middle: it tiles both ways."""
    w, h = img.size
    shifted = ImageChops.offset(img, w // 2, h // 2)
    mask = Image.new('L', (w, h))
    pm = mask.load()
    for y in range(h):
        for x in range(w):
            pm[x, y] = int(255 * min(1, 2 * min(x, w - x, y, h - y) / min(w, h) * 2))
    return Image.composite(img, shifted, mask)


def seamless_x(img, overlap):
    """img with its right end faded into its left: it repeats left to right."""
    w, h = img.size
    out = img.crop((0, 0, w - overlap, h))
    tail = img.crop((w - overlap, 0, w, h))
    fade = Image.linear_gradient('L').rotate(90).resize((overlap, h))
    out.paste(Image.composite(out.crop((0, 0, overlap, h)), tail, fade), (0, 0))
    return out


def unpaper(img):
    """img painted on white, as colour over transparency: the white goes, the paint stays."""
    out = Image.new('RGBA', img.size)
    pi, po = img.load(), out.load()
    for y in range(img.height):
        for x in range(img.width):
            c = pi[x, y]
            a = 255 - min(c)
            po[x, y] = tuple(min(255, max(0, (v - (255 - a)) * 255 // a)) for v in c) + (a,) if a else (0, 0, 0, 0)
    return out


def tiles():
    raw = os.path.join(ROOT, 'raw', 'tiles')
    out = os.path.join(ROOT, 'sprites', 'tiles')
    os.makedirs(out, exist_ok=True)
    sea = Image.open(os.path.join(raw, 'sea-1.jpg')).convert('RGB')
    seamless(sea).resize((TILE, TILE), Image.Resampling.LANCZOS).save(os.path.join(out, 'sea.png'))
    # The paper is plain only inside its painted edges and off its folds through the middle;
    # its light is evened out so the repeats do not show as squares.
    paper = Image.open(os.path.join(raw, 'paper-1.jpg')).convert('RGB')
    w, h = paper.size
    paper = paper.crop((w * 55 // 100, h * 55 // 100, w * 76 // 100, h * 76 // 100))
    flat = ImageChops.subtract(paper, paper.filter(ImageFilter.GaussianBlur(30)), 1.0, 128)
    mean = paper.resize((1, 1), Image.Resampling.BOX).getpixel((0, 0))
    paper = ImageChops.add(flat, Image.new('RGB', paper.size, tuple(c - 128 for c in mean)))
    seamless(paper).resize((TILE // 2, TILE // 2), Image.Resampling.LANCZOS).save(
        os.path.join(out, 'paper.png'))
    img = Image.open(os.path.join(raw, 'strips-1.jpg')).convert('RGB')
    grey = img.convert('L')
    w, h = img.size
    px = grey.load()
    rows = [sum(px[x, y] for x in range(0, w, 4)) * 4 / w for y in range(h)] + [255]
    bands, top = [], None
    for y, v in enumerate(rows):
        if v < 246 and top is None:
            top = y
        elif v >= 246 and top is not None:
            if y - top > 8:
                bands.append((top, y))
            top = None
    out = os.path.join(ROOT, 'sprites', 'strips')
    os.makedirs(out, exist_ok=True)
    for name, (top, bottom) in zip(STRIPS, bands):
        if name:
            # Inside the ruled frame of the band.
            band = img.crop((24, top + 10, w - 24, bottom - 10))
            band = seamless_x(band, 120)
            band.thumbnail((4 * TILE, 64), Image.Resampling.LANCZOS)
            if name in ('border-band', 'coast'):
                # Laid over the land and the sea: only the paint, the paper shows through.
                band = unpaper(band)
            band.save(os.path.join(out, name + '.png'))
            if name == 'border-band':
                # White with the band's strength for alpha: tinted to each realm's colour.
                mask = Image.new('RGBA', band.size, (255, 255, 255, 0))
                mask.putalpha(band.getchannel('A'))
                mask.save(os.path.join(out, name + '-mask.png'))


def events():
    raw = os.path.join(ROOT, 'raw', 'events')
    out = os.path.join(ROOT, 'sprites', 'events')
    os.makedirs(out, exist_ok=True)
    for f in sorted(os.listdir(raw)):
        if not f.endswith('.jpg'):
            continue
        img = Image.open(os.path.join(raw, f)).convert('RGB')
        # The drawing is where the ink is: rows and columns with a few dark pixels.
        ink = img.convert('L').point(lambda v: 255 if v < 90 else 0)
        w, h = img.size
        cols = [ink.crop((x, 0, x + 1, h)).histogram()[255] for x in range(w)]
        rows = [ink.crop((0, y, w, y + 1)).histogram()[255] for y in range(h)]
        xs = [x for x, n in enumerate(cols) if n > h * 0.02]
        ys = [y for y, n in enumerate(rows) if n > w * 0.02]
        # In by a few pixels: past the ruled frame some pictures have.
        box = (xs[0] + 8, ys[0] + 8, xs[-1] - 8, ys[-1] - 8)
        img = ImageOps.fit(img.crop(box), EVENT, Image.Resampling.LANCZOS)
        img.save(os.path.join(out, f), quality=78, optimize=True)


main()
tiles()
events()
