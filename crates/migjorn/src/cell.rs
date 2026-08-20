//! Typed projection of a cell card.

use migjorn_syntax::{Card, SyntaxKind};
use std::ops::Range;

use crate::scan::{float_at, int_at, kind_at, next, sig, text_at};

/// Where each field of a cell card sits, as token indices.
///
/// Recomputed on demand from the card's own tokens — nothing is cached, so a
/// read after an edit can never observe stale structure.
#[derive(Debug, Clone)]
pub(crate) struct CellLayout {
    pub id: Option<i64>,
    /// `LIKE n BUT ...` base cell. When present there is no material, density or
    /// geometry — the only reference is `n`.
    pub like: Option<i64>,
    pub like_tok: Option<usize>,
    pub material: Option<i64>,
    pub material_tok: Option<usize>,
    pub density: Option<f64>,
    pub density_tok: Option<usize>,
    /// Token index range of the geometry expression.
    pub geometry: Range<usize>,
    /// Token index range of the trailing keyword parameters.
    pub params: Range<usize>,
    pub well_formed: bool,
}

/// Read a cell card's structure.
pub(crate) fn layout(card: &Card) -> CellLayout {
    let end = card.tokens().len();
    let mut out = CellLayout {
        id: None,
        like: None,
        like_tok: None,
        material: None,
        material_tok: None,
        density: None,
        density_tok: None,
        geometry: end..end,
        params: end..end,
        well_formed: false,
    };

    let Some(id_tok) = sig(card, 0) else {
        return out;
    };
    out.id = int_at(card, id_tok).filter(|&id| id > 0);

    let Some(i) = next(card, id_tok) else {
        return out;
    };

    // `LIKE n BUT params...`
    if text_at(card, i).is_some_and(|t| t.eq_ignore_ascii_case("like")) {
        let Some(base_tok) = next(card, i) else {
            return out;
        };
        out.like = int_at(card, base_tok);
        out.like_tok = Some(base_tok);
        let after = next(card, base_tok);
        let but =
            after.filter(|&j| text_at(card, j).is_some_and(|t| t.eq_ignore_ascii_case("but")));
        out.params = but.and_then(|j| next(card, j)).unwrap_or(end)..end;
        out.well_formed = out.id.is_some() && out.like.is_some() && but.is_some();
        return out;
    }

    out.material = int_at(card, i);
    out.material_tok = Some(i);

    // A density field is present exactly when the material is not void.
    let mut cursor = next(card, i);
    if out.material.is_some_and(|m| m != 0) {
        if let Some(d) = cursor {
            out.density = float_at(card, d);
            out.density_tok = Some(d);
            cursor = next(card, d);
        }
    }

    // Geometry runs until the first keyword. A geometry expression contains only
    // numbers and the operators `(`, `)`, `:`, `#`, so the first `Ident` (or the
    // `*` of a `*fill=` / `*trcl=`) is where the parameters begin.
    let geometry_start = cursor.unwrap_or(end);
    let mut j = geometry_start;
    let params_start = loop {
        let Some(k) = sig(card, j) else { break end };
        match kind_at(card, k) {
            Some(SyntaxKind::Ident) | Some(SyntaxKind::Star) => break k,
            _ => j = k + 1,
        }
    };

    out.geometry = geometry_start..params_start;
    out.params = params_start..end;
    out.well_formed = out.id.is_some()
        && out.material.is_some()
        && (out.material == Some(0) || out.density.is_some())
        && has_surface(card, &out.geometry);
    out
}

fn has_surface(card: &Card, range: &Range<usize>) -> bool {
    (range.start..range.end.min(card.tokens().len()))
        .any(|i| kind_at(card, i) == Some(SyntaxKind::Number))
}

/// One element of a cell's geometry expression, in file order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryTermKind {
    Surface,
    Complement,
    LParen,
    RParen,
    Union,
}

/// A geometry term plus the token index it came from, so an edit can address it.
#[derive(Debug, Clone)]
pub struct GeometryTerm {
    pub kind: GeometryTermKind,
    pub text: String,
    /// Token index of the term's number (or of the operator itself).
    pub token: usize,
}

/// Walk a cell's geometry expression.
///
/// The distinction that matters for renumbering: a number directly after `#` is
/// a **cell** reference (complement of a cell), whereas numbers inside `#( ... )`
/// are **surface** references (complement of a region). Getting this wrong makes
/// a cell renumber silently corrupt surfaces, so it is resolved here once and
/// every reader shares it.
pub(crate) fn walk_geometry(card: &Card, range: &Range<usize>) -> Vec<GeometryTerm> {
    let mut out = Vec::new();
    let tokens = card.tokens();
    let mut i = range.start;
    while i < range.end.min(tokens.len()) {
        if tokens[i].is_trivia() {
            i += 1;
            continue;
        }
        let (kind, text, token) = match tokens[i].kind {
            SyntaxKind::Number => (GeometryTermKind::Surface, card.token_text(i).to_owned(), i),
            SyntaxKind::LParen => (GeometryTermKind::LParen, "(".to_owned(), i),
            SyntaxKind::RParen => (GeometryTermKind::RParen, ")".to_owned(), i),
            SyntaxKind::Colon => (GeometryTermKind::Union, ":".to_owned(), i),
            SyntaxKind::Hash => {
                // `#n` complements cell n; `#(` complements a region of surfaces.
                match next(card, i).filter(|&j| kind_at(card, j) == Some(SyntaxKind::Number)) {
                    Some(j) => {
                        let text = format!("#{}", card.token_text(j));
                        out.push(GeometryTerm {
                            kind: GeometryTermKind::Complement,
                            text,
                            token: j,
                        });
                        i = j + 1;
                        continue;
                    }
                    None => (GeometryTermKind::Complement, "#".to_owned(), i),
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };
        out.push(GeometryTerm { kind, text, token });
        i += 1;
    }
    out
}

/// One `keyword[:particle][=]value` entry on a cell card.
#[derive(Debug, Clone)]
pub struct CellParam {
    pub key: String,
    pub particle: Option<String>,
    pub starred: bool,
    pub value: String,
    /// Token index of the keyword, so an edit can find it again.
    pub key_token: usize,
    /// Token index range of the value.
    pub value_tokens: Range<usize>,
}

impl CellParam {
    /// The key as written in `param(key)` lookups: `imp:n`, `vol`, `fill`.
    pub fn qualified_key(&self) -> String {
        match &self.particle {
            Some(p) => format!("{}:{}", self.key, p),
            None => self.key.clone(),
        }
    }
}

/// Parse the trailing keyword parameters of a cell card.
pub(crate) fn params(card: &Card, range: &Range<usize>) -> Vec<CellParam> {
    let tokens = card.tokens();
    let end = range.end.min(tokens.len());
    let mut out = Vec::new();
    let mut cursor = range.start;

    while let Some(mut i) = sig(card, cursor) {
        if i >= end {
            break;
        }
        let starred = kind_at(card, i) == Some(SyntaxKind::Star);
        if starred {
            match next(card, i) {
                Some(j) if j < end => i = j,
                _ => break,
            }
        }
        if kind_at(card, i) != Some(SyntaxKind::Ident) {
            // Not a keyword — malformed. Skip it rather than mis-parsing the rest.
            cursor = i + 1;
            continue;
        }
        let key_token = i;
        let key = card.token_text(i).to_owned();
        let mut cur = next(card, i);

        let mut particle = None;
        if cur.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Colon)) {
            let after_colon = next(card, cur.unwrap());
            if after_colon.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Ident)) {
                particle = Some(card.token_text(after_colon.unwrap()).to_owned());
                cur = next(card, after_colon.unwrap());
            } else {
                cur = after_colon;
            }
        }
        // The `=` is optional in MCNP; `lat 1` and `lat=1` mean the same thing.
        if cur.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Eq)) {
            cur = next(card, cur.unwrap());
        }

        // The value runs to the next top-level keyword. Parenthesised groups
        // (`fill=7 (0 0 5 90 90 0)`) are consumed whole.
        let value_start = cur.unwrap_or(end);
        let mut value_end = value_start;
        let mut depth = 0i32;
        let mut j = value_start;
        while j < end {
            let Some(k) = sig(card, j) else { break };
            if k >= end {
                break;
            }
            match kind_at(card, k) {
                Some(SyntaxKind::LParen) => depth += 1,
                Some(SyntaxKind::RParen) => depth -= 1,
                Some(SyntaxKind::Ident) | Some(SyntaxKind::Star) if depth <= 0 => break,
                _ => {}
            }
            value_end = k + 1;
            j = k + 1;
        }

        let value = slice_tokens(card, value_start..value_end);
        out.push(CellParam {
            key,
            particle,
            starred,
            value,
            key_token,
            value_tokens: value_start..value_end,
        });
        cursor = value_end.max(key_token + 1);
    }

    out
}

/// Exact source text spanning a token range, trivia included, trimmed.
fn slice_tokens(card: &Card, range: Range<usize>) -> String {
    let tokens = card.tokens();
    let end = range.end.min(tokens.len());
    if range.start >= end {
        return String::new();
    }
    let from = tokens[range.start].start as usize;
    let to = tokens[end - 1].end() as usize;
    card.text()[from..to].trim().to_owned()
}

/// A cell's `fill=` entry.
#[derive(Debug, Clone)]
pub struct Fill {
    pub universe: i64,
    pub starred: bool,
    /// The parenthesised transform that may follow the universe, verbatim.
    pub transform: Option<String>,
}

/// Read a `fill` / `*fill` parameter.
///
/// Returns `None` for the lattice-array form (`fill=0:2 0:1 0:0 5 6 ...`), which
/// names many universes rather than one.
pub(crate) fn fill(card: &Card, p: &CellParam) -> Option<Fill> {
    let tokens = card.tokens();
    let end = p.value_tokens.end.min(tokens.len());
    let mut i = sig(card, p.value_tokens.start)?;
    if i >= end {
        return None;
    }
    // A `:` at the top of the value means an index range, i.e. the array form.
    for k in p.value_tokens.start..end {
        if kind_at(card, k) == Some(SyntaxKind::Colon) {
            return None;
        }
    }

    let universe = int_at(card, i)?;
    i = match next(card, i) {
        Some(j) if j < end => j,
        _ => {
            return Some(Fill {
                universe,
                starred: p.starred,
                transform: None,
            })
        }
    };

    let transform = if kind_at(card, i) == Some(SyntaxKind::LParen) {
        Some(slice_tokens(card, i..end))
    } else {
        None
    };
    Some(Fill {
        universe,
        starred: p.starred,
        transform,
    })
}
