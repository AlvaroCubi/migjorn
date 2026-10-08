//! Typed values of cell parameters: `FILL`, `TRCL`, `U`, `MAT`, `RHO`, `LAT`
//! and `IMP`.
//!
//! These readers are strict where the rest of the projection is forgiving: a
//! value that cannot be read in full is an `Err` naming the token, never a
//! partial value. The same readers back `CellView::well_formed` and the parse
//! diagnostics, so a getter that returns `Err` always has a diagnostic to match.

use migjorn_syntax::{Card, SyntaxKind};
use std::ops::Range;

use crate::scan::{float_at, int_at, kind_at, parse_int, sig};

/// A transform written on a cell: `TRCL=…`, or the group after a `FILL`
/// universe.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformSpec {
    /// `TRCL=5`, `TRCL=(5)`, `FILL=u (5)`: a `TRn` card.
    Number(i64),
    /// `TRCL=(dx dy dz …)`: the values as written, in order. Their meaning
    /// (degrees or cosines, the optional `M` entry) is the caller's to read.
    Inline(Vec<f64>),
}

/// One element of a lattice fill array.
#[derive(Debug, Clone, PartialEq)]
pub struct FillEntry {
    /// The universe as written; the sign is kept.
    pub universe: i64,
    pub transform: Option<TransformSpec>,
}

/// A cell's `FILL` / `*FILL` value.
#[derive(Debug, Clone, PartialEq)]
pub enum FillSpec {
    /// `FILL=u`, optionally followed by a `(…)` transform.
    Single {
        universe: i64,
        transform: Option<TransformSpec>,
    },
    /// `FILL=i1:i2 j1:j2 k1:k2 u u (…) u nR …`, the lattice-array form.
    Array {
        /// Inclusive index ranges, i then j then k.
        ranges: [(i32, i32); 3],
        /// One entry per element, i fastest, `nR` already expanded.
        entries: Vec<FillEntry>,
    },
}

/// Number of values an inline transform may hold: the displacement, then a
/// rotation given in full (9), as two vectors (6), as one vector and one
/// component (5) or as one vector (3), then the `M` flag after a full rotation.
const INLINE_TRANSFORM_LENGTHS: [usize; 6] = [3, 6, 8, 9, 12, 13];

/// Largest lattice fill array read, in elements. Far past any real model; it
/// keeps a typo in an index range from allocating gigabytes.
const MAX_FILL_ARRAY: i64 = 1 << 26;

/// Why a value could not be read, and the tokens it is about.
#[derive(Debug, Clone)]
pub(crate) struct ValueError {
    pub tokens: Range<usize>,
    pub message: String,
}

type Read<T> = Result<T, ValueError>;

fn fail<T>(tokens: Range<usize>, message: String) -> Read<T> {
    Err(ValueError { tokens, message })
}

/// The significant (non-trivia) token indices in `range`.
pub(crate) fn sig_tokens(card: &Card, range: &Range<usize>) -> Vec<usize> {
    let end = range.end.min(card.tokens().len());
    let mut out = Vec::new();
    let mut i = range.start;
    while let Some(k) = sig(card, i) {
        if k >= end {
            break;
        }
        out.push(k);
        i = k + 1;
    }
    out
}

fn whole(toks: &[usize]) -> Range<usize> {
    match (toks.first(), toks.last()) {
        (Some(&a), Some(&b)) => a..b + 1,
        _ => 0..0,
    }
}

fn one(k: usize) -> Range<usize> {
    k..k + 1
}

/// Whether the number of values in an inline transform is one MCNP accepts.
pub(crate) fn inline_length_ok(n: usize) -> bool {
    INLINE_TRANSFORM_LENGTHS.contains(&n)
}

pub(crate) fn inline_length_message(n: usize) -> String {
    format!("a transform has {n} values; expected 3, 6, 8, 9, 12 or 13")
}

/// The inside of a transform group (parentheses excluded), or a bare `TRCL=n`.
fn transform_group(card: &Card, toks: &[usize], span: Range<usize>) -> Read<TransformSpec> {
    match toks {
        [] => fail(span, "empty transform `()`".to_owned()),
        [k] => match int_at(card, *k).filter(|&n| n > 0) {
            Some(n) => Ok(TransformSpec::Number(n)),
            None => fail(
                one(*k),
                format!(
                    "transform number `{}` is not a positive integer",
                    card.token_text(*k)
                ),
            ),
        },
        _ => {
            let mut values = Vec::with_capacity(toks.len());
            for &k in toks {
                match float_at(card, k) {
                    Some(v) => values.push(v),
                    None => {
                        return fail(
                            one(k),
                            format!("`{}` in a transform is not a number", card.token_text(k)),
                        )
                    }
                }
            }
            if !inline_length_ok(values.len()) {
                return fail(span, inline_length_message(values.len()));
            }
            Ok(TransformSpec::Inline(values))
        }
    }
}

/// A `( … )` group starting at `toks[0]`, which must be the `(`. Returns the
/// transform and how many tokens the group used, parentheses included.
fn paren_group(card: &Card, toks: &[usize]) -> Read<(TransformSpec, usize)> {
    debug_assert_eq!(kind_at(card, toks[0]), Some(SyntaxKind::LParen));
    for (n, &k) in toks.iter().enumerate().skip(1) {
        match kind_at(card, k) {
            Some(SyntaxKind::RParen) => {
                let spec = transform_group(card, &toks[1..n], toks[0]..k + 1)?;
                return Ok((spec, n + 1));
            }
            Some(SyntaxKind::LParen) => {
                return fail(one(k), "nested `(` in a transform".to_owned());
            }
            _ => {}
        }
    }
    fail(whole(toks), "transform `(` is never closed".to_owned())
}

/// Read a `TRCL` value.
pub(crate) fn read_trcl(card: &Card, value: &Range<usize>) -> Read<TransformSpec> {
    let toks = sig_tokens(card, value);
    let Some(&first) = toks.first() else {
        return fail(value.clone(), "TRCL has no value".to_owned());
    };
    let (spec, used) = if kind_at(card, first) == Some(SyntaxKind::LParen) {
        paren_group(card, &toks)?
    } else if toks.len() == 1 {
        (transform_group(card, &toks, whole(&toks))?, 1)
    } else {
        return fail(
            whole(&toks),
            "an inline TRCL transform must be in parentheses".to_owned(),
        );
    };
    if let Some(&extra) = toks.get(used) {
        return fail(
            extra..whole(&toks).end,
            format!(
                "unexpected `{}` after the TRCL transform",
                card.token_text(extra)
            ),
        );
    }
    Ok(spec)
}

/// Read a `FILL` value, single or lattice array.
pub(crate) fn read_fill(card: &Card, value: &Range<usize>) -> Read<FillSpec> {
    let toks = sig_tokens(card, value);
    if toks.is_empty() {
        return fail(value.clone(), "FILL has no value".to_owned());
    }
    // A `:` outside any group means index ranges, i.e. the array form.
    let mut depth = 0i32;
    let mut array = false;
    for &k in &toks {
        match kind_at(card, k) {
            Some(SyntaxKind::LParen) => depth += 1,
            Some(SyntaxKind::RParen) => depth -= 1,
            Some(SyntaxKind::Colon) if depth == 0 => array = true,
            _ => {}
        }
    }
    if array {
        read_fill_array(card, &toks)
    } else {
        read_fill_single(card, &toks)
    }
}

fn read_fill_single(card: &Card, toks: &[usize]) -> Read<FillSpec> {
    let u = toks[0];
    let Some(universe) = int_at(card, u) else {
        return fail(
            one(u),
            format!("fill universe `{}` is not an integer", card.token_text(u)),
        );
    };
    let mut transform = None;
    let mut used = 1;
    if let Some(&k) = toks.get(1) {
        if kind_at(card, k) == Some(SyntaxKind::LParen) {
            let (spec, n) = paren_group(card, &toks[1..])?;
            transform = Some(spec);
            used += n;
        }
    }
    if let Some(&extra) = toks.get(used) {
        return fail(
            extra..whole(toks).end,
            format!("unexpected `{}` in FILL", card.token_text(extra)),
        );
    }
    Ok(FillSpec::Single {
        universe,
        transform,
    })
}

fn read_fill_array(card: &Card, toks: &[usize]) -> Read<FillSpec> {
    let mut ranges = [(0i32, 0i32); 3];
    let mut at = 0;
    for range in &mut ranges {
        let (Some(&a), Some(&colon), Some(&b)) = (toks.get(at), toks.get(at + 1), toks.get(at + 2))
        else {
            return fail(
                whole(toks),
                "a fill array needs three `a:b` index ranges".to_owned(),
            );
        };
        let bound = |k: usize| int_at(card, k).and_then(|v| i32::try_from(v).ok());
        let (Some(lo), true, Some(hi)) = (
            bound(a),
            kind_at(card, colon) == Some(SyntaxKind::Colon),
            bound(b),
        ) else {
            return fail(
                a..b + 1,
                format!(
                    "`{}` is not a fill index range `a:b`",
                    card.text()[crate::scan::byte_span(card, a..b + 1)].trim()
                ),
            );
        };
        if hi < lo {
            return fail(a..b + 1, format!("fill index range {lo}:{hi} is empty"));
        }
        *range = (lo, hi);
        at += 3;
    }
    let want = ranges
        .iter()
        .map(|&(lo, hi)| i64::from(hi) - i64::from(lo) + 1)
        .product::<i64>();
    if want > MAX_FILL_ARRAY {
        return fail(
            whole(&toks[..at]),
            format!("fill array of {want} elements is too large to read"),
        );
    }

    let mut entries = Entries {
        list: Vec::new(),
        count: 0,
        cap: want,
    };
    // Whether the last entry was written directly (so a group may follow it).
    let mut group_allowed = false;
    while let Some(&k) = toks.get(at) {
        let text = card.token_text(k);
        match kind_at(card, k) {
            Some(SyntaxKind::LParen) => {
                let (spec, n) = paren_group(card, &toks[at..])?;
                if !group_allowed {
                    return fail(
                        toks[at]..toks[at + n - 1] + 1,
                        "a fill array transform must follow a universe".to_owned(),
                    );
                }
                if let Some(last) = entries.list.last_mut() {
                    last.transform = Some(spec);
                }
                group_allowed = false;
                at += n;
                continue;
            }
            Some(SyntaxKind::Number) => {
                if let Some(universe) = parse_int(text) {
                    entries.push(
                        FillEntry {
                            universe,
                            transform: None,
                        },
                        1,
                    );
                    group_allowed = true;
                } else if let Some(n) = repeat_count(text) {
                    entries.repeat(card, k, n)?;
                    group_allowed = false;
                } else if is_shortcut(text) {
                    return fail(one(k), unsupported_shortcut(text));
                } else {
                    return fail(one(k), format!("fill universe `{text}` is not an integer"));
                }
            }
            Some(SyntaxKind::Ident) if text.eq_ignore_ascii_case("r") => {
                entries.repeat(card, k, 1)?;
                group_allowed = false;
            }
            Some(SyntaxKind::Ident) if is_shortcut(text) => {
                return fail(one(k), unsupported_shortcut(text));
            }
            _ => return fail(one(k), format!("unexpected `{text}` in a fill array")),
        }
        at += 1;
    }

    if entries.count != want {
        return fail(
            whole(toks),
            format!(
                "fill array has {} entries for {want} elements",
                entries.count
            ),
        );
    }
    Ok(FillSpec::Array {
        ranges,
        entries: entries.list,
    })
}

/// Fill-array entries as they are read. Only the first `cap` are kept, so a
/// stray `1000000R` cannot allocate past the array's size; `count` still
/// counts every entry for the length check.
struct Entries {
    list: Vec<FillEntry>,
    count: i64,
    cap: i64,
}

impl Entries {
    fn push(&mut self, entry: FillEntry, n: usize) {
        let room = (self.cap - self.list.len() as i64).max(0) as usize;
        self.list.extend(std::iter::repeat_n(entry, n.min(room)));
        self.count = self.count.saturating_add(n as i64);
    }

    /// `nR`: repeat the previous entry, its transform included, `n` times.
    fn repeat(&mut self, card: &Card, k: usize, n: usize) -> Read<()> {
        if self.count == 0 {
            return fail(one(k), format!("`{}` repeats nothing", card.token_text(k)));
        }
        if n == 0 {
            return fail(
                one(k),
                format!("`{}` repeats zero times", card.token_text(k)),
            );
        }
        // Past the cap the kept entries no longer matter: the array is too
        // long and is reported as such.
        let last = self.list.last().cloned().unwrap_or(FillEntry {
            universe: 0,
            transform: None,
        });
        self.push(last, n);
        Ok(())
    }
}

/// `nR` (or `nr`) as a repeat count `n`.
fn repeat_count(text: &str) -> Option<usize> {
    let n = text.strip_suffix(['r', 'R'])?;
    n.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| n.parse().ok())
        .flatten()
}

/// Whether `text` is a data shortcut other than a repeat: `nJ`, `nI`, `nM`,
/// `nILOG`, with or without the count.
fn is_shortcut(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let letters = lower.trim_start_matches(|c: char| c.is_ascii_digit());
    matches!(letters, "j" | "i" | "m" | "ilog")
}

fn unsupported_shortcut(text: &str) -> String {
    format!("shortcut `{text}` is not supported in a fill array (only `nR` is)")
}

/// The single number a scalar parameter holds. Runs once per parameter at
/// parse, so it does not collect the tokens unless the value is wrong.
fn scalar(card: &Card, key: &str, value: &Range<usize>) -> Read<usize> {
    let end = value.end.min(card.tokens().len());
    let first = sig(card, value.start).filter(|&k| k < end);
    let second = first.and_then(|k| sig(card, k + 1)).filter(|&k| k < end);
    match (first, second) {
        (Some(k), None) => Ok(k),
        (None, _) => fail(value.clone(), format!("{key} has no value")),
        (Some(_), Some(_)) => {
            let toks = whole(&sig_tokens(card, value));
            fail(
                toks.clone(),
                format!(
                    "{key} takes one value, found `{}`",
                    card.text()[crate::scan::byte_span(card, toks)].trim()
                ),
            )
        }
    }
}

pub(crate) fn read_int(card: &Card, key: &str, value: &Range<usize>) -> Read<i64> {
    let k = scalar(card, key, value)?;
    int_at(card, k).map_or_else(
        || {
            fail(
                one(k),
                format!("{key} value `{}` is not an integer", card.token_text(k)),
            )
        },
        Ok,
    )
}

pub(crate) fn read_float(card: &Card, key: &str, value: &Range<usize>) -> Read<f64> {
    let k = scalar(card, key, value)?;
    float_at(card, k).map_or_else(
        || {
            fail(
                one(k),
                format!("{key} value `{}` is not a number", card.token_text(k)),
            )
        },
        Ok,
    )
}

pub(crate) fn read_material(card: &Card, value: &Range<usize>) -> Read<i64> {
    let m = read_int(card, "MAT", value)?;
    if m < 0 {
        return fail(value.clone(), format!("MAT={m} is negative"));
    }
    Ok(m)
}

pub(crate) fn read_lattice(card: &Card, value: &Range<usize>) -> Read<u8> {
    match read_int(card, "LAT", value)? {
        1 => Ok(1),
        2 => Ok(2),
        n => fail(value.clone(), format!("LAT={n}; expected 1 or 2")),
    }
}

pub(crate) fn read_importance(card: &Card, value: &Range<usize>) -> Read<f64> {
    let v = read_float(card, "IMP", value)?;
    if v < 0.0 {
        return fail(value.clone(), format!("IMP value {v} is negative"));
    }
    Ok(v)
}

/// Whether a `:particle` designator list (`n`, `n,p`) names `particle`.
pub(crate) fn names_particle(list: &str, particle: &str) -> bool {
    list.split(',')
        .any(|p| p.trim().eq_ignore_ascii_case(particle.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Model;

    fn fill(src: &str) -> Result<(FillSpec, bool), String> {
        let m = Model::parse(&format!("t\n{src}\n\n1 SO 5\n\nm1 1001 1\n"));
        let got = m.cells().next().unwrap().fill_spec().expect("has a fill");
        got
    }

    fn trcl(src: &str) -> Result<(TransformSpec, bool), String> {
        let m = Model::parse(&format!("t\n{src}\n\n1 SO 5\n\n"));
        let got = m.cells().next().unwrap().trcl().expect("has a trcl");
        got
    }

    fn e(universe: i64) -> FillEntry {
        FillEntry {
            universe,
            transform: None,
        }
    }

    #[test]
    fn single_fill_with_and_without_transforms() {
        assert_eq!(
            fill("1 0 -1 fill=5").unwrap(),
            (
                FillSpec::Single {
                    universe: 5,
                    transform: None
                },
                false
            )
        );
        assert_eq!(
            fill("1 0 -1 fill=5 (3)").unwrap().0,
            FillSpec::Single {
                universe: 5,
                transform: Some(TransformSpec::Number(3))
            }
        );
        assert_eq!(
            fill("1 0 -1 *fill=5(1 2 3)").unwrap(),
            (
                FillSpec::Single {
                    universe: 5,
                    transform: Some(TransformSpec::Inline(vec![1.0, 2.0, 3.0]))
                },
                true
            )
        );
        assert!(fill("1 0 -1 fill=5 (3) 4").is_err());
        assert!(fill("1 0 -1 fill=5.5").is_err());
        assert!(fill("1 0 -1 fill=5 (1 2)").is_err());
        assert!(fill("1 0 -1 fill=5 (1 2 3").is_err());
    }

    #[test]
    fn fill_array_expands_repeats_and_attaches_groups() {
        let (spec, starred) = fill("1 0 -1 lat=1 fill=0:1 -1:0 0:0 1 (4) 2r -7 (0.5 0 0)").unwrap();
        assert!(!starred);
        let t4 = Some(TransformSpec::Number(4));
        assert_eq!(
            spec,
            FillSpec::Array {
                ranges: [(0, 1), (-1, 0), (0, 0)],
                entries: vec![
                    FillEntry {
                        universe: 1,
                        transform: t4.clone()
                    },
                    FillEntry {
                        universe: 1,
                        transform: t4.clone()
                    },
                    FillEntry {
                        universe: 1,
                        transform: t4
                    },
                    FillEntry {
                        universe: -7,
                        transform: Some(TransformSpec::Inline(vec![0.5, 0.0, 0.0]))
                    },
                ],
            }
        );
        // a bare `R` repeats once
        let (spec, _) = fill("1 0 -1 lat=1 fill=0:1 0:0 0:0 2 R").unwrap();
        let FillSpec::Array { entries, .. } = spec else {
            panic!()
        };
        assert_eq!(entries, vec![e(2), e(2)]);
    }

    #[test]
    fn fill_array_errors_are_reported_not_guessed() {
        let err = |s: &str| fill(s).unwrap_err();
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 0:0 1").contains("1 entries for 2"));
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 0:0 1 2 3").contains("3 entries for 2"));
        assert!(err("1 0 -1 lat=1 fill=1:0 0:0 0:0 1").contains("empty"));
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 1 2").contains("index range"));
        assert!(err("1 0 -1 lat=1 fill=0:2 0:0 0:0 1 1J 2").contains("`1J`"));
        assert!(err("1 0 -1 lat=1 fill=0:2 0:0 0:0 1 1I 3").contains("`1I`"));
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 0:0 2R 1").contains("repeats nothing"));
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 0:0 (3) 1 1").contains("must follow"));
        assert!(err("1 0 -1 lat=1 fill=0:1 0:0 0:0 1 (3) (4) 1").contains("must follow"));
    }

    #[test]
    fn trcl_forms() {
        assert_eq!(
            trcl("1 0 -1 trcl=111").unwrap(),
            (TransformSpec::Number(111), false)
        );
        assert_eq!(trcl("1 0 -1 trcl=(5)").unwrap().0, TransformSpec::Number(5));
        assert_eq!(
            trcl("1 0 -1 trcl=(284.0 1 0)").unwrap().0,
            TransformSpec::Inline(vec![284.0, 1.0, 0.0])
        );
        let (spec, starred) =
            trcl("1 0 -1 *trcl=(314.0 0 0  30 60 90  120 30 90  90 90 0)").unwrap();
        assert!(starred);
        assert_eq!(
            spec,
            TransformSpec::Inline(vec![
                314.0, 0.0, 0.0, 30.0, 60.0, 90.0, 120.0, 30.0, 90.0, 90.0, 90.0, 0.0
            ])
        );
        assert!(trcl("1 0 -1 trcl=(1 2 3 4)")
            .unwrap_err()
            .contains("4 values"));
        assert!(trcl("1 0 -1 trcl=(1 2 2J)").unwrap_err().contains("`2J`"));
        assert!(trcl("1 0 -1 trcl=1 2 3")
            .unwrap_err()
            .contains("parentheses"));
        assert!(trcl("1 0 -1 trcl=(0)").unwrap_err().contains("positive"));
    }

    #[test]
    fn particle_lists() {
        assert!(names_particle("n", "N"));
        assert!(names_particle("n,p", "p"));
        assert!(!names_particle("n,p", "e"));
    }
}
