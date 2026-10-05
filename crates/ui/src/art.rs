//! Stage 29: the art of the old map: the sprites of assets/sprites, one PNG each, the paper,
//! the sea and the strips that repeat, and data/sprites.ron.

use eframe::egui::{
    ColorImage, Context, TextureFilter, TextureHandle, TextureId, TextureOptions,
    TextureWrapMode, Vec2, vec2,
};
use serde::Deserialize;
use std::collections::BTreeMap;

macro_rules! png {
    ($($p:literal),* $(,)?) => {
        &[$(($p, include_bytes!(concat!("../../../assets/sprites/", $p, ".png")) as &[u8])),*]
    };
}

/// Every sprite the map draws, `<sheet>/<name>-<variant>`, one or two variants of each.
pub const SPRITES: &[(&str, &[u8])] = png![
    "buildings/fort-1", "buildings/fort-2", "buildings/castle-1", "buildings/castle-2",
    "buildings/market-1", "buildings/market-2", "buildings/abbey-1", "buildings/abbey-2",
    "buildings/cathedral-1", "buildings/cathedral-2", "buildings/milestone-1",
    "buildings/dikes-1", "buildings/scaffold-1",
    "settlements/cottage-1", "settlements/cottage-2", "settlements/hamlet-1",
    "settlements/hamlet-2", "settlements/town-1", "settlements/town-2",
    "settlements/town-tower-1", "settlements/town-tower-3", "settlements/capital-1",
    "settlements/capital-2", "settlements/crown-1", "settlements/banner-1",
    "nature/conifer-1", "nature/conifer-2", "nature/trees-1", "nature/trees-2",
    "nature/hills-1", "nature/hills-2", "nature/hill-1", "nature/mountains-1",
    "nature/mountains-2", "nature/mountain-1", "nature/field-1", "nature/field-2",
    "nature/reeds-1", "nature/reeds-2",
    "sea/ship-1", "sea/cog-1", "sea/boat-1",
    "extras/fort-building-1", "extras/church-building-1", "extras/camp-1", "extras/fire-1",
    "extras/revolt-1", "extras/graves-1", "extras/ruins-1",
    "decor/compass-1", "decor/cartouche-1", "decor/corner-1", "decor/ribbon-1",
    "decor/wind-1", "decor/fish-1",
];

/// Laid as tiles or along lines: their textures repeat.
pub const TILES: &[(&str, &[u8])] = png![
    "tiles/paper", "tiles/sea", "strips/border-band", "strips/border-band-mask",
    "strips/coast", "strips/river", "strips/road-cobble",
];

macro_rules! jpg {
    ($($p:literal),* $(,)?) => {
        &[$(($p, include_bytes!(concat!("../../../assets/sprites/events/", $p, ".jpg")) as &[u8])),*]
    };
}

/// The event pictures by theme (stage 29b, `Event.image`), decoded when first shown.
pub const PICTURES: &[(&str, &[u8])] = jpg![
    "abdication", "appanage", "arrest", "assassin", "battle", "bishop", "border", "brigands",
    "campaign", "cathedral", "city-fire", "coronation", "council", "defeat", "envoy", "famine",
    "festival", "flood", "heaven-wrath", "heresy", "intrigue", "mine", "mourning", "petition",
    "pilgrimage", "plague", "quarrel", "raid", "refugees", "refused", "revolt", "runaways",
    "sickbed", "siege", "talks", "trade", "tutor", "victory", "war-declared", "wedding",
    "claimants", "consecration", "drought", "empty-village", "guild", "old-king", "plague-end",
    "sick-heir", "tavern", "ultimatum", "wounded-king",
];

const STYLE: &str = include_str!("../../../data/sprites.ron");

/// data/sprites.ron.
#[derive(Deserialize)]
pub struct Style {
    pub settlements: Vec<(i32, String)>,
    pub capital: String,
    pub crown: String,
    pub in_capital: BTreeMap<String, String>,
    pub scaffold: String,
    pub underway: BTreeMap<String, String>,
    pub road: String,
    pub terrain: BTreeMap<String, Vec<String>>,
    pub marks: Vec<Mark>,
    pub banner: String,
    pub ships: Vec<String>,
}

#[derive(Deserialize)]
pub struct Mark {
    pub when: When,
    pub sprite: String,
}

/// What a mark shows: see data/sprites.ron.
#[derive(Deserialize, Debug, PartialEq)]
pub enum When {
    Flag(String),
    Unrest,
    WarBorder,
    WarTarget,
    Fewer(i32),
}

pub fn style() -> Style {
    ron::from_str(STYLE).expect("data/sprites.ron")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub id: TextureId,
    /// In pixels of the PNG.
    pub size: Vec2,
}

pub struct Art {
    /// By name: "fort" → its variants.
    sprites: BTreeMap<String, Vec<Sprite>>,
    pub tiles: BTreeMap<&'static str, TextureId>,
    pub style: Style,
    /// The event pictures shown so far, by theme.
    pub pictures: BTreeMap<String, Sprite>,
    /// The textures live while these do.
    handles: Vec<TextureHandle>,
}

impl Art {
    pub fn load(ctx: &Context) -> Art {
        let style = style();
        let mut handles = vec![];
        let mut load = |name: &str, img: ColorImage, wrap| {
            let options = TextureOptions {
                wrap_mode: wrap,
                mipmap_mode: Some(TextureFilter::Linear),
                ..TextureOptions::LINEAR
            };
            let size = vec2(img.size[0] as f32, img.size[1] as f32);
            let h = ctx.load_texture(name, img, options);
            let s = Sprite { id: h.id(), size };
            handles.push(h);
            s
        };
        let mut sprites: BTreeMap<String, Vec<Sprite>> = BTreeMap::new();
        for (path, bytes) in SPRITES {
            let img = decode(bytes);
            let file = path.rsplit('/').next().unwrap_or(path);
            let name = file.rsplit_once('-').map_or(file, |(n, _)| n);
            let s = load(path, img, TextureWrapMode::ClampToEdge);
            sprites.entry(name.to_string()).or_default().push(s);
        }
        let tiles = (TILES.iter())
            .map(|(path, bytes)| {
                let name = path.rsplit('/').next().unwrap_or(path);
                (name, load(path, decode(bytes), TextureWrapMode::Repeat).id)
            })
            .collect();
        Art {
            sprites,
            tiles,
            style,
            pictures: BTreeMap::new(),
            handles,
        }
    }

    /// The picture of theme `name`: decoded and uploaded the first time an event shows it,
    /// not at the start. None for a theme not in `PICTURES`.
    pub fn picture(&mut self, ctx: &Context, name: &str) -> Option<Sprite> {
        if let Some(s) = self.pictures.get(name) {
            return Some(*s);
        }
        let (_, bytes) = PICTURES.iter().find(|(n, _)| *n == name)?;
        let h = ctx.load_texture(name, jpeg(bytes), TextureOptions::LINEAR);
        let s = Sprite { id: h.id(), size: h.size_vec2() };
        self.handles.push(h);
        self.pictures.insert(name.to_string(), s);
        Some(s)
    }

    /// A variant of sprite `name` by `salt` (a province id): the same every time.
    pub fn sprite(&self, name: &str, salt: &str) -> Option<Sprite> {
        let all = self.sprites.get(name)?;
        Some(all[(hash(salt) % all.len() as u64) as usize])
    }

    #[cfg(test)]
    pub fn has(&self, name: &str) -> bool {
        self.sprites.contains_key(name)
    }
}

/// FNV-1a: the same on every platform.
pub fn hash(s: &str) -> u64 {
    (s.bytes()).fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

fn decode(bytes: &[u8]) -> ColorImage {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().expect("a png of assets/sprites");
    let mut buf = vec![0; r.output_buffer_size().expect("a png that fits memory")];
    let info = r.next_frame(&mut buf).expect("a png of assets/sprites");
    let size = [info.width as usize, info.height as usize];
    let buf = &buf[..info.buffer_size()];
    match info.color_type {
        png::ColorType::Rgba => ColorImage::from_rgba_unmultiplied(size, buf),
        _ => ColorImage::from_rgb(size, buf),
    }
}

fn jpeg(bytes: &[u8]) -> ColorImage {
    let cursor = zune_jpeg::zune_core::bytestream::ZCursor::new(bytes);
    let mut dec = zune_jpeg::JpegDecoder::new(cursor);
    let rgb = dec.decode().expect("a jpeg of assets/sprites/events");
    let info = dec.info().expect("decoded");
    ColorImage::from_rgb([info.width as usize, info.height as usize], &rgb)
}
