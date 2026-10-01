use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::{Add, Div, Mul, Sub};

/// Fixed point with scale 1000: `Fx(1500)` is 1.5.
/// In data and serde it is written in whole units: `45` is `Fx(45000)`. `clamp` comes from `Ord`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fx(pub i64);

impl Fx {
    pub const SCALE: i64 = 1000;

    pub const fn from_int(v: i64) -> Fx {
        Fx(v * Self::SCALE)
    }

    pub const fn from_milli(v: i64) -> Fx {
        Fx(v)
    }
}

impl Add for Fx {
    type Output = Fx;
    fn add(self, o: Fx) -> Fx {
        Fx(self.0 + o.0)
    }
}

impl Sub for Fx {
    type Output = Fx;
    fn sub(self, o: Fx) -> Fx {
        Fx(self.0 - o.0)
    }
}

/// Rounds toward zero. The i128 intermediate avoids overflow before rescaling.
impl Mul for Fx {
    type Output = Fx;
    fn mul(self, o: Fx) -> Fx {
        Fx((self.0 as i128 * o.0 as i128 / Self::SCALE as i128) as i64)
    }
}

/// Rounds toward zero. Panics on division by zero.
impl Div for Fx {
    type Output = Fx;
    fn div(self, o: Fx) -> Fx {
        Fx((self.0 as i128 * Self::SCALE as i128 / o.0 as i128) as i64)
    }
}

/// A fractional value is refused instead of being silently truncated.
impl Serialize for Fx {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.0 % Self::SCALE != 0 {
            return Err(serde::ser::Error::custom(format!(
                "Fx {self} is not a whole number"
            )));
        }
        s.serialize_i64(self.0 / Self::SCALE)
    }
}

impl<'de> Deserialize<'de> for Fx {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Fx, D::Error> {
        let v = i64::deserialize(d)?;
        v.checked_mul(Self::SCALE)
            .map(Fx)
            .ok_or_else(|| serde::de::Error::custom(format!("Fx {v} out of range")))
    }
}

/// `1.5`, `-0.25`, `45`: trailing fractional zeros dropped.
impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let a = self.0.unsigned_abs();
        let (int, frac) = (a / 1000, a % 1000);
        if frac == 0 {
            write!(f, "{sign}{int}")
        } else {
            let frac = format!("{frac:03}");
            write!(f, "{sign}{int}.{}", frac.trim_end_matches('0'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_edges() {
        let (i, m) = (Fx::from_int, Fx::from_milli);
        assert_eq!(i(-3), m(-3000));
        assert_eq!(i(2) + m(-2500), m(-500));
        assert_eq!(m(-500) - i(1), m(-1500));
        assert_eq!(m(1500) * m(-1500), m(-2250));
        assert_eq!(m(-1) * m(1), m(0)); // -0.000001 truncates to zero
        assert_eq!(m(1) * m(999), m(0));
        assert_eq!(i(7) / i(2), m(3500));
        assert_eq!(i(1) / i(3), m(333)); // 0.3333... toward zero
        assert_eq!(i(-1) / i(3), m(-333)); // not -334
        assert_eq!(i(2) / i(-3), m(-666)); // not -667
        assert_eq!(i(-2) / i(-3), m(666));
        // No overflow in the intermediate product.
        assert_eq!(i(3_000_000_000) * i(2), i(6_000_000_000));
        assert_eq!(i(3_000_000_000) / i(3), i(1_000_000_000));
    }

    #[test]
    #[should_panic]
    fn div_by_zero_panics() {
        let _ = Fx::from_int(1) / Fx(0);
    }

    #[test]
    fn clamp() {
        let (lo, hi) = (Fx::from_int(0), Fx::from_int(100));
        assert_eq!(Fx::from_int(-5).clamp(lo, hi), lo);
        assert_eq!(Fx::from_int(150).clamp(lo, hi), hi);
        assert_eq!(Fx::from_milli(42_500).clamp(lo, hi), Fx::from_milli(42_500));
    }

    #[test]
    fn display() {
        let s = |v| Fx::from_milli(v).to_string();
        assert_eq!(s(45_000), "45");
        assert_eq!(s(1_500), "1.5");
        assert_eq!(s(-250), "-0.25");
        assert_eq!(s(-1_005), "-1.005");
        assert_eq!(s(0), "0");
    }

    #[test]
    fn serde_in_whole_units() {
        assert_eq!(ron::to_string(&Fx::from_int(-15)).unwrap(), "-15");
        assert_eq!(ron::from_str::<Fx>("45").unwrap(), Fx::from_milli(45_000));
        assert!(ron::to_string(&Fx::from_milli(1500)).is_err());
        assert!(ron::from_str::<Fx>("1.5").is_err());
        assert!(ron::from_str::<Fx>(&i64::MAX.to_string()).is_err());
    }
}
