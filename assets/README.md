# Спрайты карты

Пиктограммы для карты в духе Carta Marina (1539): здания, поселения, природа, море, геральдика. Сгенерированы Grok по промптам ниже, по 4 варианта каждого листа.

```
raw/<лист>-<n>.jpg         исходные листы, n = вариант 1–4
sprites/<лист>/<имя>-<n>.png  нарезка: прозрачный фон, длинная сторона 160 px
```

Нарезка: `python3 scripts/cut-sprites.py` (PIL). Скрипт заливает белый фон от края, режет лист на связные куски, отдаёт каждый кусок ячейке сетки, где лежит его центр, и убирает белые дыры, например между балками лесов. Имена и сетка листов, а также отбракованные варианты (`REJECT`) задаются в начале скрипта.

## Что есть

| Лист | Сетка | Спрайты |
|---|---|---|
| buildings | 4×3 | fort, castle, market, abbey, cathedral, cathedral-dome, dikes, milestone, scaffold, watchtower, windmill, pier |
| settlements | 4×3 | cottage, cottages, hamlet, town, town-tower, capital, crown, shield, shield-round, banner, swords, scroll |
| nature | 4×3 | tree, trees, conifer, bush, mountain, mountains, mountain-snow, hill, hills, field, reeds, rocks |
| sea | 3×2 | ship, cog, boat, serpent, whale, waves |
| extras | 4×3 | fort-building, church-building (стройка), dikes-arc, bridge, fire, camp, ruins, revolt, boundary, graves, barn, siege |
| decor | клетки неровные, `BOXES` | compass, cartouche, corner, ribbon, label, wind, island, serpent, fish, cloud, dividers, medallion |

Подложка (`raw/tiles/` → `sprites/tiles`, `sprites/strips`):

| Файл | Что | Как рисовать |
|---|---|---|
| tiles/sea.png | море, 512² | плиткой по всей воде, стыкуется со всех сторон |
| tiles/paper.png | пергамент, 256² | плиткой под всей картой, суша — он же с подкраской в цвет государства |
| strips/border-band.png | розовая акварельная кайма, с прозрачностью | лентой вдоль границы своего королевства, внутрь |
| strips/border-band-mask.png | та же кайма, белая, сила в альфе | лентой вдоль границы чужих государств, тинт цветом вершины |
| strips/road-cobble.png | мощёная дорога | лентой вдоль дороги |
| strips/river.png | река | лентой вдоль реки |
| strips/coast.png | берег со штриховкой, с прозрачностью | лентой вдоль берега, штриховка в море |

Полоса — горизонтальная лента высотой 64 px, повторяется слева направо. В egui она кладётся вдоль ломаной мешем: на каждый отрезок четырёхугольник, `u` растёт с длиной, `v` поперёк. Чернильные и точечные границы рисуются линиями `Painter`, без полос.

Постройки игры (`rules.ron`, `buildings`):

| Постройка | Спрайт |
|---|---|
| крепость | fort |
| дорога | milestone, сама дорога рисуется линией |
| рынок | market |
| обитель | abbey |
| валы | dikes |
| собор | cathedral или cathedral-dome |
| идёт стройка | scaffold поверх постройки |

Крепость в столице рисуется спрайтом castle. Поселения подбираются по населению провинции: cottage → hamlet → town → capital.

## Как ставить на карту

- Точка привязки внизу по центру: спрайт стоит на точке провинции.
- Высота на карте при обычном масштабе:

  | Спрайты | Высота, px |
  |---|---|
  | домик | 20 |
  | деревня | 22 |
  | город | 30 |
  | крепость | 40 |
  | столица, собор | 44–48 |
  | дерево | 20–24 |
  | гора | 34 |
  | корабль | 48 |

  При такой высоте всё читается (проверено на пергаменте `#ECE0C4`).
- Варианты 1–4 одного спрайта выбираются детерминированно по id провинции, чтобы на карте не было повторов.
- Щиты и знамя белые внутри: цвет и знак государства накладываются поверх.

## Что не так

- **Отбраковано из-за палитры** (синие стены, зелёные и синие крыши): capital-4, town-tower-2, hamlet-3, cottages-3, town-3.
- **Трава под домами** в settlements-1: прочие варианты без неё. Мелочь.
- **Щиты:** shield и shield-round почти одинаковые, круглого низа модель не нарисовала. Хватает одного.
- **Стены зданий** кремовые, на кремовом пергаменте контраст держит только контур. Если не хватит, можно затемнить заливку провинции под спрайтом или дать спрайту лёгкую тень.
- **extras:** взят один вариант из четырёх. В двух других модель нарисовала линии сетки и траву под предметами, в первом повторила руины и поставила башенный кран.
- **Грунтовая дорога** отброшена: колеи идут наискось и рвутся на стыке. Сухая суша (land-1) тоже не годится, на ней узор из камней; суша — пергамент с подкраской.
- **Пергамент:** у листа расписаны края и есть сгиб посередине. Взят чистый кусок, свет выровнен; повтор чуть виден вблизи.
- **Объём:** 163 спрайта занимают 5,3 МБ. В игру пойдут 1–2 варианта нужных спрайтов, это около 1,5 МБ в атласе. Wasm вырастет на столько же: лимит мягкий, только замер.

## Промпты

Общий блок стиля в начале каждого:

```
Sprite sheet for a strategy game map in the style of the 1539 Carta Marina by Olaus Magnus: hand-colored Renaissance woodcut, black ink outlines with fine hatching, flat watercolor fills, muted palette — brick-red roofs, cream walls, slate-blue and teal-green, ochre. Every object drawn in side profile (not isometric, not top-down), centered in its cell, standing on the bottom edge of the cell, same scale across the sheet. Plain flat white background, no shadows, no ground under objects, no text, no labels, no frame, no numbers. Evenly spaced grid with clear empty gaps between cells.
```

Дальше перечень ячеек по порядку, как в таблице «Что есть». Для нового листа: тот же блок, приписка `match the style of the attached sheet exactly` с одним из листов в приложении и строка в `SHEETS` скрипта.
