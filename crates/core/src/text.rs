//! Russian texts from templates: names in six cases, words agreeing with sex, variants.

use crate::data::Names;
use crate::rng::Rng;
use crate::state::Sex;

/// The cases of `{key.case}`, in the order of `Names.cases`.
pub const CASES: [&str; 6] = ["им", "род", "дат", "вин", "тв", "пр"];

/// A name a template may use: its key, the nominative and the sex of a person; a place
/// (None) is feminine when its nominative ends in -а or -я.
pub type Named<'a> = (&'a str, &'a str, Option<Sex>);

/// `s` with every `{key}`, `{key.род}` (a case of `CASES`) and `{key:взошёл|взошла}` (male,
/// female) of `named` filled in; other braces stay. A name without forms in `names` stays
/// in the nominative (`Names::declined`).
pub fn fill(s: &str, names: &Names, named: &[Named]) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out += &rest[..i];
        let Some(j) = rest[i..].find('}').map(|j| i + j) else {
            break;
        };
        let token = &rest[i + 1..j];
        match filled(token, names, named) {
            Some(t) => out += &t,
            None => out += &rest[i..=j],
        }
        rest = &rest[j + 1..];
    }
    out + rest
}

fn filled(token: &str, names: &Names, named: &[Named]) -> Option<String> {
    let end = token.find(['.', ':']).unwrap_or(token.len());
    let (key, how) = token.split_at(end);
    let (_, name, sex) = named.iter().find(|n| n.0 == key)?;
    let female = match sex {
        Some(s) => *s == Sex::Female,
        None => name.ends_with(['а', 'я']),
    };
    Some(match how.chars().next() {
        Some(':') => {
            let mut forms = how[1..].split('|');
            let male = forms.next().unwrap_or_default();
            forms.next().filter(|_| female).unwrap_or(male).to_string()
        }
        Some(_) => {
            let case = CASES.iter().position(|c| *c == &how[1..])?;
            names.declined(name, case)
        }
        None => name.to_string(),
    })
}

/// One of `variants` by `salt` and `n`, from an Rng of its own: the main stream stays as it
/// was. Empty without variants.
pub fn pick(variants: &[String], salt: u64, n: u64) -> &str {
    match variants.len() {
        0 => "",
        len => &variants[Rng::from_seed(salt ^ n).range(0, len as i64) as usize],
    }
}

/// A number of `s` and `n` for `pick` and the turns of texts, the same on every platform.
pub fn hash(s: &str, n: u32) -> u64 {
    let fnv = (s.bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    Rng::from_seed(fnv ^ n as u64).next_u64()
}

/// `n` and the word of `forms` (one, few, many) it takes: «1 год», «3 года», «11 лет».
pub fn plural(n: u32, (one, few, many): &(String, String, String)) -> String {
    let word = match (n % 10, n % 100) {
        (1, x) if x != 11 => one,
        (2..=4, x) if !(12..=14).contains(&x) => few,
        _ => many,
    };
    format!("{n} {word}")
}

/// `s` with its first letter in upper case.
pub fn capital(s: &str) -> String {
    let mut chars = s.chars();
    let first = chars.next().into_iter().flat_map(char::to_uppercase);
    first.chain(chars).collect()
}

/// The first sentence of `s`, its full stop kept.
pub fn first_sentence(s: &str) -> &str {
    let end = s.find(". ").map_or(s.len(), |i| i + 1);
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        let mut n = Names::default();
        for spec in [
            "Ульрих||а|у|а|ом|е",
            "Агнесс|а|ы|е|у|ой|е",
            "Строител|ь|я|ю|я|ем|е",
        ] {
            n.add_forms(spec).unwrap();
        }
        n
    }

    #[test]
    fn a_name_in_all_six_cases() {
        let n = names();
        let named = [("ruler", "Ульрих", Some(Sex::Male))];
        let all = "{ruler.им} {ruler.род} {ruler.дат} {ruler.вин} {ruler.тв} {ruler.пр} {ruler}";
        assert_eq!(
            fill(all, &n, &named),
            "Ульрих Ульриха Ульриху Ульриха Ульрихом Ульрихе Ульрих"
        );
        // A name with an epithet declines word by word.
        let named = [("prev", "Ульрих Строитель", Some(Sex::Male))];
        assert_eq!(
            fill("после {prev.род}", &n, &named),
            "после Ульриха Строителя"
        );
    }

    #[test]
    fn words_agree_with_the_sex() {
        let n = names();
        let t = "{ruler} {ruler:взошёл|взошла} на престол, {heir:он|она} в стороне";
        let king = [
            ("ruler", "Ульрих", Some(Sex::Male)),
            ("heir", "Агнесса", Some(Sex::Female)),
        ];
        let queen = [
            ("ruler", "Агнесса", Some(Sex::Female)),
            ("heir", "Ульрих", Some(Sex::Male)),
        ];
        assert_eq!(
            fill(t, &n, &king),
            "Ульрих взошёл на престол, она в стороне"
        );
        assert_eq!(
            fill(t, &n, &queen),
            "Агнесса взошла на престол, он в стороне"
        );
        // A place by its ending.
        let lands = |p| [("province", p, None)];
        let t = "{province:пал|пала}";
        assert_eq!(fill(t, &n, &lands("Пурпуляндия")), "пала");
        assert_eq!(fill(t, &n, &lands("Хольм")), "пал");
    }

    #[test]
    fn a_name_without_forms_stays_in_the_nominative_and_unknown_braces_stay() {
        let n = names();
        let named = [("heir", "Отто", Some(Sex::Male))];
        let t = "у {heir.род} по закону «{law}», {heir.зв}";
        assert_eq!(fill(t, &n, &named), "у Отто по закону «{law}», {heir.зв}");
        assert_eq!(n.declined("Отто", 1), "Отто");
    }

    #[test]
    fn plurals_and_variants() {
        let years = ("год".into(), "года".into(), "лет".into());
        let got: Vec<_> = [1, 3, 5, 11, 12, 21, 24, 111]
            .map(|n| plural(n, &years))
            .into();
        assert_eq!(
            got,
            [
                "1 год",
                "3 года",
                "5 лет",
                "11 лет",
                "12 лет",
                "21 год",
                "24 года",
                "111 лет"
            ]
        );
        let v: Vec<String> = vec!["а".into(), "б".into(), "в".into()];
        let picks: Vec<_> = (0..30).map(|n| pick(&v, 42, n)).collect();
        assert_eq!(picks, (0..30).map(|n| pick(&v, 42, n)).collect::<Vec<_>>());
        assert!(
            ["а", "б", "в"].iter().all(|x| picks.contains(x)),
            "{picks:?}"
        );
        assert_eq!(pick(&[], 1, 1), "");
    }
}
