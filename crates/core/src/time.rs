use serde::{Deserialize, Serialize};

/// How many ticks make one year. Durations in data are in years and become ticks on load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeUnit {
    pub ticks_per_year: u32,
}

/// Ticks elapsed since the start of the game.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Years(pub u32);

const SEASONS: [&str; 4] = ["весна", "лето", "осень", "зима"];
// The year starts in March, as in the old calendar.
const MONTHS: [&str; 12] = [
    "март",
    "апрель",
    "май",
    "июнь",
    "июль",
    "август",
    "сентябрь",
    "октябрь",
    "ноябрь",
    "декабрь",
    "январь",
    "февраль",
];

impl Tick {
    /// Whole years elapsed since tick 0.
    pub fn year(&self, unit: TimeUnit) -> u32 {
        self.0 / unit.ticks_per_year
    }

    /// `"весна 1187"` for 4 ticks per year, `"март 1187"` for 12, `"1187"` otherwise.
    /// January and February carry the next calendar year.
    pub fn date(&self, unit: TimeUnit, start_year: u32) -> String {
        let year = start_year + self.year(unit);
        let part = (self.0 % unit.ticks_per_year) as usize;
        match unit.ticks_per_year {
            4 => format!("{} {year}", SEASONS[part]),
            12 => format!("{} {}", MONTHS[part], year + (part >= 10) as u32),
            _ => year.to_string(),
        }
    }
}

impl Years {
    pub fn ticks(self, unit: TimeUnit) -> Tick {
        Tick(self.0 * unit.ticks_per_year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_years_roundtrip() {
        for tpy in [1, 4] {
            let unit = TimeUnit {
                ticks_per_year: tpy,
            };
            for y in [0, 1, 7, 250] {
                assert_eq!(Years(y).ticks(unit), Tick(y * tpy));
                assert_eq!(Years(y).ticks(unit).year(unit), y);
            }
        }
        // Partial years round down.
        let season = TimeUnit { ticks_per_year: 4 };
        assert_eq!(Tick(7).year(season), 1);
        assert_eq!(Tick(8).year(season), 2);
    }

    #[test]
    fn date() {
        let u = |ticks_per_year| TimeUnit { ticks_per_year };
        assert_eq!(Tick(0).date(u(1), 1187), "1187");
        assert_eq!(Tick(3).date(u(1), 1187), "1190");
        assert_eq!(Tick(0).date(u(4), 1187), "весна 1187");
        assert_eq!(Tick(3).date(u(4), 1187), "зима 1187");
        assert_eq!(Tick(5).date(u(4), 1187), "лето 1188");
        assert_eq!(Tick(0).date(u(12), 1187), "март 1187");
        assert_eq!(Tick(9).date(u(12), 1187), "декабрь 1187");
        assert_eq!(Tick(10).date(u(12), 1187), "январь 1188");
        assert_eq!(Tick(11).date(u(12), 1187), "февраль 1188");
        assert_eq!(Tick(12).date(u(12), 1187), "март 1188");
        assert_eq!(Tick(5).date(u(2), 1187), "1189");
    }
}
