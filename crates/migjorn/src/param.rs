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

/// Number of values MCNP 6.2 accepts on a transform, checked against MCNP
/// itself: the displacement, then optionally a rotation given as one vector
/// (3), two vectors (6) or in full (9), then the `M` flag. A `TRn` card may also
/// leave displacement entries off (0, 1 or 2 values; they default to 0); an
/// inline transform of one value is a `TRn` number instead.
const TRANSFORM_LENGTHS: [usize; 8] = [0, 1, 2, 3, 6, 9, 12, 13];

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

/// Whether a transform with `n` values is one MCNP accepts. `inline` is a
/// `( … )` group on a cell, which needs at least two values.
pub(crate) fn transform_length_ok(n: usize, inline: bool) -> bool {
    TRANSFORM_LENGTHS.contains(&n) && !(inline && n < 2)
}

pub(crate) fn transform_length_message(n: usize) -> String {
    format!(
        "a transform has {n} values; MCNP accepts 3, 6, 9, 12 or 13 (or fewer than 3 on a TR card)"
    )
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
            if !transform_length_ok(values.len(), true) {
                return fail(span, transform_length_message(values.len()));
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
        // MCNP reads `TRCL=5 0 0` as `TRCL=5` and ignores the rest or
        // fails, depending on the count; it is never an inline transform.
        return fail(
            whole(&toks),
            "TRCL takes one TR number, or an inline transform in parentheses".to_owned(),
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
        last: None,
    };
    let mut prev = Prev::Start;
    while let Some(&k) = toks.get(at) {
        let text = card.token_text(k);
        let kind = kind_at(card, k);
        // `nI` needs a universe written right after it.
        if let Prev::Interpolate { tok, from, n } = prev {
            let to = (kind == Some(SyntaxKind::Number))
                .then(|| parse_int(text))
                .flatten();
            let Some(to) = to else {
                return fail(
                    tok..k + 1,
                    format!(
                        "`{}` must be followed by a universe number",
                        card.token_text(tok)
                    ),
                );
            };
            // MCNP steps by a whole number, truncating `(to - from) / (n + 1)`,
            // so an uneven gap does not give the evenly spaced universes the
            // shortcut reads as. Only exact steps are expanded.
            if (to - from) % (n + 1) != 0 {
                return fail(
                    tok..k + 1,
                    format!(
                        "`{from} {} {to}` does not interpolate to whole universe numbers",
                        card.token_text(tok)
                    ),
                );
            }
            let step = (to - from) / (n + 1);
            for i in 1..=n {
                entries.push(from + step * i, 1);
            }
        }
        let shortcut = match kind {
            Some(SyntaxKind::Number) if parse_int(text).is_none() => Some(text),
            Some(SyntaxKind::Ident) => Some(text),
            _ => None,
        }
        .map(Shortcut::read);
        match (kind, shortcut) {
            (Some(SyntaxKind::LParen), _) => {
                let (spec, n) = paren_group(card, &toks[at..])?;
                let span = toks[at]..toks[at + n - 1] + 1;
                match prev {
                    Prev::Entry => {}
                    Prev::Start => {
                        return fail(
                            span,
                            "a fill array transform must follow a universe".to_owned(),
                        )
                    }
                    Prev::Group => {
                        return fail(span, "two transforms for one fill array entry".to_owned())
                    }
                    Prev::Interpolate { .. } => unreachable!("handled above"),
                }
                if let Some(last) = entries.list.last_mut() {
                    last.transform = Some(spec);
                }
                prev = Prev::Group;
                at += n;
                continue;
            }
            (Some(SyntaxKind::Number), None) => {
                let universe = parse_int(text).expect("checked above");
                entries.push(universe, 1);
                prev = Prev::Entry;
            }
            (_, Some(shortcut)) => {
                // MCNP rejects a shortcut right after a transform group.
                if matches!(prev, Prev::Group) {
                    return fail(one(k), format!("`{text}` cannot follow a transform group"));
                }
                let Some(last) = entries.last.filter(|_| prev != Prev::Start) else {
                    return fail(one(k), format!("`{text}` has no universe before it"));
                };
                match shortcut {
                    Shortcut::Repeat(n) if n > 0 => {
                        entries.push(last, n);
                        prev = Prev::Entry;
                    }
                    Shortcut::Multiply(m) => {
                        let Some(universe) = last.checked_mul(m) else {
                            return fail(one(k), format!("`{last} {text}` overflows"));
                        };
                        entries.push(universe, 1);
                        prev = Prev::Entry;
                    }
                    Shortcut::Interpolate(n) if n > 0 => {
                        prev = Prev::Interpolate {
                            tok: k,
                            from: last,
                            n: n as i64,
                        };
                    }
                    Shortcut::Other(message) => return fail(one(k), message),
                    Shortcut::Repeat(_) | Shortcut::Interpolate(_) => {
                        return fail(one(k), format!("`{text}` has a zero count"))
                    }
                }
            }
            _ => return fail(one(k), format!("unexpected `{text}` in a fill array")),
        }
        at += 1;
    }
    if let Prev::Interpolate { tok, .. } = prev {
        return fail(
            one(tok),
            format!(
                "`{}` must be followed by a universe number",
                card.token_text(tok)
            ),
        );
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

/// What the previous fill-array token was, which decides what may follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prev {
    Start,
    /// An entry: a universe, or one made by `nR` / `nM`.
    Entry,
    /// A `( … )` transform group.
    Group,
    /// `nI`, waiting for the universe after it.
    Interpolate {
        tok: usize,
        from: i64,
        n: i64,
    },
}

/// A data shortcut inside a fill array.
enum Shortcut {
    Repeat(usize),
    Multiply(i64),
    Interpolate(usize),
    /// Not usable in a fill array; the message says why.
    Other(String),
}

impl Shortcut {
    /// What MCNP 6.2 does with each, checked against MCNP itself:
    /// `nR` / `R` repeat the previous universe; `nI` / `I` interpolate whole
    /// numbers; `nM` multiplies by a whole number. `nJ`, `nILOG` and a bare
    /// `M` are fatal errors, and a fractional multiplier does not give the
    /// product, so those are not read.
    fn read(text: &str) -> Shortcut {
        let lower = text.to_ascii_lowercase();
        let split = lower
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(lower.len());
        let (count, letters) = lower.split_at(split);
        let n = || -> Option<usize> {
            if count.is_empty() {
                Some(1)
            } else if count.bytes().all(|b| b.is_ascii_digit()) {
                count.parse().ok()
            } else {
                None
            }
        };
        match letters {
            "r" => n().map_or_else(|| Shortcut::Other(bad_count(text)), Shortcut::Repeat),
            "i" => n().map_or_else(|| Shortcut::Other(bad_count(text)), Shortcut::Interpolate),
            "m" if count.is_empty() => {
                Shortcut::Other(format!("`{text}` needs a multiplier, as in `2M`"))
            }
            "m" => match parse_int(count) {
                Some(m) => Shortcut::Multiply(m),
                None => Shortcut::Other(format!(
                    "`{text}`: only whole-number multipliers are read in a fill array"
                )),
            },
            "j" | "ilog" => {
                Shortcut::Other(format!("shortcut `{text}` is not allowed in a fill array"))
            }
            _ => Shortcut::Other(format!("`{text}` is not a universe number")),
        }
    }
}

fn bad_count(text: &str) -> String {
    format!("`{text}` does not have a whole-number count")
}

/// Fill-array entries as they are read. Only the first `cap` are kept, so a
/// stray `1000000R` cannot allocate past the array's size; `count` still
/// counts every entry for the length check.
struct Entries {
    list: Vec<FillEntry>,
    count: i64,
    cap: i64,
    /// The last universe written, kept past the cap.
    last: Option<i64>,
}

impl Entries {
    /// `n` entries of `universe`, with no transform.
    fn push(&mut self, universe: i64, n: usize) {
        let room = (self.cap - self.list.len() as i64).max(0) as usize;
        let entry = FillEntry {
            universe,
            transform: None,
        };
        self.list.extend(std::iter::repeat_n(entry, n.min(room)));
        self.count = self.count.saturating_add(n as i64);
        self.last = Some(universe);
    }
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
        n @ 0..=2 => Ok(n as u8),
        n => fail(value.clone(), format!("LAT={n}; expected 0, 1 or 2")),
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

    /// The universes of a 4-element fill array, or the error.
    fn universes(entries: &str) -> Result<Vec<i64>, String> {
        let (spec, _) = fill(&format!("1 0 -1 lat=1 fill=0:3 0:0 0:0 {entries}"))?;
        let FillSpec::Array { entries, .. } = spec else {
            panic!("not an array")
        };
        Ok(entries.iter().map(|e| e.universe).collect())
    }

    /// Indices of the entries that carry a transform.
    fn transformed(entries: &str) -> Vec<usize> {
        let (spec, _) = fill(&format!("1 0 -1 lat=1 fill=0:3 0:0 0:0 {entries}")).unwrap();
        let FillSpec::Array { entries, .. } = spec else {
            panic!("not an array")
        };
        (0..entries.len())
            .filter(|&i| entries[i].transform.is_some())
            .collect()
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
        // MCNP accepts a two-value displacement
        assert!(fill("1 0 -1 fill=5 (1 2)").is_ok());
        assert!(fill("1 0 -1 fill=5 (3) 4").is_err());
        assert!(fill("1 0 -1 fill=5.5").is_err());
        assert!(fill("1 0 -1 fill=5 (1 2 3 4)").is_err());
        assert!(fill("1 0 -1 fill=5 (1 2 3").is_err());
    }

    // The fill-array cases below were run through MCNP 6.2: a 4-element
    // lattice, each element probed for the universe (and transform) it holds.

    #[test]
    fn fill_array_shortcuts_match_mcnp() {
        assert_eq!(universes("1 2 2R"), Ok(vec![1, 2, 2, 2]));
        assert_eq!(universes("1 R 2 3"), Ok(vec![1, 1, 2, 3]));
        assert_eq!(universes("1 1I 3 3"), Ok(vec![1, 2, 3, 3]));
        assert_eq!(universes("1 I 3 3"), Ok(vec![1, 2, 3, 3]));
        assert_eq!(universes("1 2I 4"), Ok(vec![1, 2, 3, 4]));
        assert_eq!(universes("4 1I 2 2"), Ok(vec![4, 3, 2, 2]));
        assert_eq!(universes("1 2M 3 3"), Ok(vec![1, 2, 3, 3]));
        assert_eq!(universes("1 2M 2M 3"), Ok(vec![1, 2, 4, 3]));
        assert_eq!(universes("1 2M R 3"), Ok(vec![1, 2, 2, 3]));
        assert_eq!(universes("1 R 1I 3"), Ok(vec![1, 1, 2, 3]));
    }

    #[test]
    fn fill_array_groups_attach_to_the_entry_before_them() {
        assert_eq!(transformed("1 (1) 1 (1) 1 2"), vec![0, 1]);
        assert_eq!(transformed("1 (1) 2 2R"), vec![0]);
        // after `nR`, only the last repeated entry
        assert_eq!(transformed("1 2R (1) 2"), vec![2]);
        // after `nM` and after the end of an interpolation
        assert_eq!(transformed("1 2M (1) 3 3"), vec![1]);
        assert_eq!(transformed("1 1I 3 (1) 3"), vec![2]);
        let (spec, _) = fill("1 0 -1 lat=1 fill=0:1 0:0 0:0 4 (4) -7 (0.5 0 0)").unwrap();
        let FillSpec::Array { entries, .. } = spec else {
            panic!()
        };
        assert_eq!(
            entries,
            vec![
                FillEntry {
                    universe: 4,
                    transform: Some(TransformSpec::Number(4))
                },
                FillEntry {
                    universe: -7,
                    transform: Some(TransformSpec::Inline(vec![0.5, 0.0, 0.0]))
                },
            ]
        );
    }

    #[test]
    fn fill_array_inputs_mcnp_rejects_are_errors() {
        let err = |s: &str| universes(s).unwrap_err();
        // fatal in MCNP
        assert!(err("1 1J 2 3").contains("`1J`"));
        assert!(err("1 1ILOG 3 3").contains("`1ILOG`"));
        assert!(err("2 M 3 3").contains("`M`"));
        assert!(err("1 (1) 2R 2").contains("cannot follow a transform"));
        assert!(err("1 (1) 2M 3 3").contains("cannot follow a transform"));
        assert!(err("1 (1) 1I 3 3").contains("cannot follow a transform"));
        assert!(err("1 1I (1) 3 3").contains("must be followed by a universe"));
        assert!(err("1 2 3").contains("3 entries for 4"));
        assert!(err("1 2 3 3 3").contains("5 entries for 4"));
        // accepted by MCNP, but not with the meaning the shortcut reads as
        assert!(err("1 1I 2 2").contains("whole universe numbers"));
        assert!(err("1 2I 3").contains("whole universe numbers"));
        assert!(err("4 0.5M 3 3").contains("whole-number multipliers"));
        // malformed
        assert!(err("2R 1 1 1").contains("no universe before"));
        assert!(err("1 2 3 1I").contains("must be followed by a universe"));
        assert!(err("(3) 1 1 1 1").contains("must follow"));
        assert!(err("1 (3) (4) 1 1 1").contains("two transforms"));
        let range = |s: &str| fill(s).unwrap_err();
        assert!(range("1 0 -1 lat=1 fill=1:0 0:0 0:0 1").contains("empty"));
        assert!(range("1 0 -1 lat=1 fill=0:1 0:0 1 2").contains("index range"));
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
        // value counts MCNP accepts inline: 2, 3, 6, 9, 12, 13
        let values = "5 0 0 1 0 0 0 1 0 0 0 1 1".split(' ').collect::<Vec<_>>();
        for n in 2..=13 {
            let ok = trcl(&format!("1 0 -1 trcl=({})", values[..n].join(" "))).is_ok();
            assert_eq!(ok, [2, 3, 6, 9, 12, 13].contains(&n), "{n} values");
        }
        assert!(trcl("1 0 -1 trcl=(1 2 2J)").unwrap_err().contains("`2J`"));
        // MCNP reads this as TR 5, not as a displacement
        assert!(trcl("1 0 -1 trcl=5 0 0")
            .unwrap_err()
            .contains("in parentheses"));
        assert!(trcl("1 0 -1 trcl=(0)").unwrap_err().contains("positive"));
    }

    #[test]
    fn particle_lists() {
        assert!(names_particle("n", "N"));
        assert!(names_particle("n,p", "p"));
        assert!(!names_particle("n,p", "e"));
    }
}
