//! Typed projection of a cell card.

use migjorn_syntax::{Card, SyntaxKind};
use std::fmt;
use std::ops::Range;

use crate::param;
use crate::scan::{float_at, int_at, kind_at, next, prev, sig, text_at};

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
    /// The parsed reference of a `Surface` term. `None` for other kinds, and
    /// for a surface term that is not a valid reference (which makes the cell
    /// not well formed).
    pub surface: Option<SurfaceRef>,
    /// The cell of a `#n` complement. `None` for other kinds, for the `#` of a
    /// `#( … )` region, and for a `#` followed by something that is not a cell
    /// number (which makes the cell not well formed).
    pub cell: Option<i64>,
}

/// A signed surface reference in a geometry expression: `-3`, `+5`, `470.2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceRef {
    /// The surface number, always positive.
    pub id: i64,
    /// The macrobody facet after the `.`: `470.2` -> `Some(2)`.
    pub facet: Option<u8>,
    /// Written with a leading `-`: the negative sense.
    pub negative: bool,
}

impl SurfaceRef {
    /// Read one geometry number. `None` unless it is an optional sign, a
    /// positive integer, and optionally `.` and a single facet digit `1`–`9`.
    ///
    /// ```
    /// use migjorn::SurfaceRef;
    /// assert_eq!(
    ///     SurfaceRef::parse("-470.1"),
    ///     Some(SurfaceRef { id: 470, facet: Some(1), negative: true })
    /// );
    /// assert_eq!(SurfaceRef::parse("1e3"), None);
    /// ```
    pub fn parse(text: &str) -> Option<SurfaceRef> {
        let (negative, body) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let (id, facet) = match body.split_once('.') {
            Some((id, facet)) => (id, Some(facet)),
            None => (body, None),
        };
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let id = id.parse::<i64>().ok().filter(|&id| id > 0)?;
        let facet = match facet.map(str::as_bytes) {
            None => None,
            Some(&[d @ b'1'..=b'9']) => Some(d - b'0'),
            Some(_) => return None,
        };
        Some(SurfaceRef {
            id,
            facet,
            negative,
        })
    }

    /// The surface number with its sense: `-470.1` -> `-470`.
    pub fn signed_id(&self) -> i64 {
        if self.negative {
            -self.id
        } else {
            self.id
        }
    }
}

/// Writes the reference back as MCNP geometry text: `-470.1`, `5`.
impl fmt::Display for SurfaceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        write!(f, "{}", self.id)?;
        if let Some(facet) = self.facet {
            write!(f, ".{facet}")?;
        }
        Ok(())
    }
}

/// Walk a cell's geometry expression.
///
/// The distinction that matters for renumbering: a number directly after `#` is
/// a **cell** reference (complement of a cell), whereas numbers inside `#( ... )`
/// are **surface** references (complement of a region). Getting this wrong makes
/// a cell renumber silently corrupt surfaces, so it is resolved here once and
/// every reader shares it.
pub(crate) fn walk_geometry(card: &Card, range: &Range<usize>) -> Vec<GeometryTerm> {
    walk_geometry_spans(card, range)
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

/// Same walk as [`walk_geometry`], paired with each term's token index span
/// (half-open; `Hash` included for a complement). `GeometryTerm::token` names
/// only the token a reader cares about (the complement's digit, say); an edit
/// that inserts before or after a whole term needs its full extent, which is
/// what the span gives.
pub(crate) fn walk_geometry_spans(
    card: &Card,
    range: &Range<usize>,
) -> Vec<(GeometryTerm, Range<usize>)> {
    let mut out = Vec::new();
    let tokens = card.tokens();
    let mut i = range.start;
    while i < range.end.min(tokens.len()) {
        if tokens[i].is_trivia() {
            i += 1;
            continue;
        }
        let (kind, text, token, span) = match tokens[i].kind {
            SyntaxKind::Number => {
                let text = card.token_text(i);
                out.push((
                    GeometryTerm {
                        kind: GeometryTermKind::Surface,
                        text: text.to_owned(),
                        token: i,
                        surface: SurfaceRef::parse(text),
                        cell: None,
                    },
                    i..i + 1,
                ));
                i += 1;
                continue;
            }
            SyntaxKind::LParen => (GeometryTermKind::LParen, "(", i, i..i + 1),
            SyntaxKind::RParen => (GeometryTermKind::RParen, ")", i, i..i + 1),
            SyntaxKind::Colon => (GeometryTermKind::Union, ":", i, i..i + 1),
            SyntaxKind::Hash => {
                // `#n` complements cell n; `#(` complements a region of surfaces.
                match next(card, i).filter(|&j| kind_at(card, j) == Some(SyntaxKind::Number)) {
                    Some(j) => {
                        let number = card.token_text(j);
                        out.push((
                            GeometryTerm {
                                kind: GeometryTermKind::Complement,
                                text: format!("#{number}"),
                                token: j,
                                surface: None,
                                cell: complement_cell(number),
                            },
                            i..j + 1,
                        ));
                        i = j + 1;
                        continue;
                    }
                    None => (GeometryTermKind::Complement, "#", i, i..i + 1),
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };
        out.push((
            GeometryTerm {
                kind,
                text: text.to_owned(),
                token,
                surface: None,
                cell: None,
            },
            span,
        ));
        i += 1;
    }
    out
}

/// The cell number of a `#n` complement: a positive integer, no sign.
fn complement_cell(text: &str) -> Option<i64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse::<i64>().ok())
        .flatten()
        .filter(|&n| n > 0)
}

/// Problems with a geometry expression that the layout does not catch: a
/// token that is not a surface, cell or operator, a number that is not a valid
/// reference, and unbalanced parentheses. Each is a token range and a message.
pub(crate) fn geometry_problems(card: &Card, range: &Range<usize>) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let tokens = card.tokens();
    let end = range.end.min(tokens.len());
    let mut depth = 0i32;
    let mut i = range.start;
    while i < end {
        let tok = tokens[i];
        if tok.is_trivia() {
            i += 1;
            continue;
        }
        let text = card.token_text(i);
        match tok.kind {
            SyntaxKind::Number => {
                if SurfaceRef::parse(text).is_none() {
                    out.push((i..i + 1, format!("`{text}` is not a surface reference")));
                }
            }
            SyntaxKind::Hash => {
                if let Some(j) = next(card, i).filter(|&j| j < end) {
                    if kind_at(card, j) == Some(SyntaxKind::Number) {
                        let number = card.token_text(j);
                        if complement_cell(number).is_none() {
                            out.push((i..j + 1, format!("`#{number}` is not a cell complement")));
                        }
                        i = j + 1;
                        continue;
                    }
                }
            }
            SyntaxKind::LParen => depth += 1,
            SyntaxKind::RParen => {
                depth -= 1;
                if depth < 0 {
                    out.push((i..i + 1, "unmatched `)` in geometry".to_owned()));
                    depth = 0;
                }
            }
            SyntaxKind::Colon => {}
            _ => out.push((i..i + 1, format!("unexpected `{text}` in geometry"))),
        }
        i += 1;
    }
    if depth > 0 {
        out.push((
            range.start..end,
            format!("{depth} unclosed `(` in geometry"),
        ));
    }
    out
}

/// A cell's geometry expression exactly as written, trimmed. The read
/// counterpart to `Model::set_cell_geometry` — reading this from one or more
/// cells, combining the text, and writing it back to another is how a caller
/// rebuilds a geometry expression wholesale (e.g. uniting several cells' into
/// one), rather than composing many single-term edits.
pub(crate) fn geometry_text(card: &Card, range: &Range<usize>) -> String {
    slice_tokens(card, range.clone())
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

/// Where one parameter sits, as token indices; the allocation-free form of
/// [`CellParam`].
#[derive(Debug, Clone)]
pub(crate) struct ParamSpan {
    pub starred: bool,
    pub key_token: usize,
    pub particle_token: Option<usize>,
    pub value_tokens: Range<usize>,
}

impl ParamSpan {
    pub fn key<'a>(&self, card: &'a Card) -> &'a str {
        card.token_text(self.key_token)
    }

    pub fn particle<'a>(&self, card: &'a Card) -> Option<&'a str> {
        self.particle_token.map(|i| card.token_text(i))
    }
}

/// Parse the trailing keyword parameters of a cell card.
pub(crate) fn params(card: &Card, range: &Range<usize>) -> Vec<CellParam> {
    param_spans(card, range)
        .0
        .into_iter()
        .map(|p| CellParam {
            key: p.key(card).to_owned(),
            particle: p.particle(card).map(str::to_owned),
            starred: p.starred,
            value: slice_tokens(card, p.value_tokens.clone()),
            key_token: p.key_token,
            value_tokens: p.value_tokens,
        })
        .collect()
}

/// The parameters of a cell card, and the indices of tokens in the parameter
/// range that belong to no parameter (a stray number or operator).
pub(crate) fn param_spans(card: &Card, range: &Range<usize>) -> (Vec<ParamSpan>, Vec<usize>) {
    let tokens = card.tokens();
    let end = range.end.min(tokens.len());
    let mut out = Vec::new();
    let mut strays = Vec::new();
    let mut cursor = range.start;

    while let Some(mut i) = sig(card, cursor) {
        if i >= end {
            break;
        }
        let starred = kind_at(card, i) == Some(SyntaxKind::Star);
        if starred {
            match next(card, i) {
                Some(j) if j < end => i = j,
                _ => {
                    strays.push(i);
                    break;
                }
            }
        }
        if kind_at(card, i) != Some(SyntaxKind::Ident) {
            // Not a keyword — malformed. Skip it rather than mis-parsing the rest.
            strays.push(i);
            cursor = i + 1;
            continue;
        }
        let key_token = i;
        let is_fill = card.token_text(i).eq_ignore_ascii_case("fill");
        let mut cur = next(card, i);

        let mut particle_token = None;
        if cur.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Colon)) {
            let after_colon = next(card, cur.unwrap());
            if after_colon.is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Ident)) {
                particle_token = after_colon;
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
        let mut in_fill_array = false;
        let mut j = value_start;
        while j < end {
            let Some(k) = sig(card, j) else { break };
            if k >= end {
                break;
            }
            match kind_at(card, k) {
                Some(SyntaxKind::LParen) => depth += 1,
                Some(SyntaxKind::RParen) => depth -= 1,
                // Lattice-array shortcuts written as a bare letter (`J`, `R`, ...)
                // lex as identifiers but belong to the `fill` array.
                Some(SyntaxKind::Ident) if in_fill_array && is_array_shortcut(card, k) => {}
                Some(SyntaxKind::Ident) | Some(SyntaxKind::Star) if depth <= 0 => break,
                Some(SyntaxKind::Colon) if is_fill => in_fill_array = true,
                _ => {}
            }
            value_end = k + 1;
            j = k + 1;
        }

        out.push(ParamSpan {
            starred,
            key_token,
            particle_token,
            value_tokens: value_start..value_end,
        });
        cursor = value_end.max(key_token + 1);
    }

    (out, strays)
}

/// Problems with a cell's parameters: values `FILL`, `TRCL`, `U`, `MAT`,
/// `RHO`, `LAT` and `IMP` that cannot be read, an `IMP` particle given twice,
/// and tokens that belong to no parameter. Each is a token range and a message.
///
/// Other parameters given twice are not reported: MCNP uses the first `U`,
/// `FILL` or `TRCL` without complaint, and so do the getters. A repeated
/// `IMP` particle is a fatal error in MCNP.
pub(crate) fn param_problems(card: &Card, range: &Range<usize>) -> Vec<(Range<usize>, String)> {
    let (spans, strays) = param_spans(card, range);
    let mut out: Vec<(Range<usize>, String)> = strays
        .into_iter()
        .map(|k| {
            (
                k..k + 1,
                format!("`{}` is not part of any parameter", card.token_text(k)),
            )
        })
        .collect();
    // Particles with an `IMP` already, to report one given twice.
    let mut seen: Vec<&str> = Vec::new();
    for p in &spans {
        let key = p.key(card);
        let v = &p.value_tokens;
        let is = |name: &str| key.eq_ignore_ascii_case(name);
        let res = if is("fill") {
            param::read_fill(card, v).map(drop)
        } else if is("trcl") {
            param::read_trcl(card, v).map(drop)
        } else if is("u") {
            param::read_int(card, "U", v).map(drop)
        } else if is("mat") {
            param::read_material(card, v).map(drop)
        } else if is("rho") {
            param::read_float(card, "RHO", v).map(drop)
        } else if is("lat") {
            param::read_lattice(card, v).map(drop)
        } else if is("imp") {
            param::read_importance(card, v).map(drop)
        } else {
            continue;
        };
        if let Err(e) = res {
            out.push((e.tokens, e.message));
        }
        if !is("imp") {
            continue;
        }
        let whole = p.key_token..v.end.max(p.key_token + 1);
        let Some(list) = p.particle(card) else {
            out.push((whole, "IMP needs a particle, as in `IMP:N`".to_owned()));
            continue;
        };
        for q in list.split(',').map(str::trim) {
            if seen.iter().any(|s| s.eq_ignore_ascii_case(q)) {
                out.push((
                    whole.clone(),
                    format!("IMP:{} is given more than once", q.to_ascii_uppercase()),
                ));
            } else {
                seen.push(q);
            }
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fill {
    pub universe: i64,
    pub starred: bool,
    /// The parenthesised transform that may follow the universe, verbatim.
    pub transform: Option<String>,
}

impl Fill {
    /// A fill of `universe`, not starred, with no transform.
    pub fn new(universe: i64) -> Fill {
        Fill {
            universe,
            starred: false,
            transform: None,
        }
    }

    /// Attach a transform, given verbatim and parenthesised — `"(30)"` for a
    /// named transform, `"(0 0 5 90 0 90)"` for an inline one. This is exactly
    /// the form [`Fill::transform`] returns, so a value read off one cell can
    /// be passed straight to another's fill with no reformatting; wrapping it
    /// in parentheses again (`"((30))"`) is the mistake this exists to make
    /// unnecessary.
    pub fn with_transform(mut self, parenthesised: impl Into<String>) -> Fill {
        self.transform = Some(parenthesised.into());
        self
    }

    pub fn starred(mut self, yes: bool) -> Fill {
        self.starred = yes;
        self
    }
}

/// Renders as the parameter text `Model::set_fill` writes onto a cell:
/// `*fill=2 (30)` or `fill=2`.
impl fmt::Display for Fill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.starred {
            f.write_str("*")?;
        }
        write!(f, "fill={}", self.universe)?;
        if let Some(t) = &self.transform {
            write!(f, " {t}")?;
        }
        Ok(())
    }
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

/// A bare-letter data shortcut (`J` jump, `R` repeat, `I` interpolate, `M`
/// multiply, `ILOG`) as it can appear inside a `fill` array.
fn is_array_shortcut(card: &Card, i: usize) -> bool {
    let t = card.token_text(i);
    ["j", "r", "i", "m", "ilog"]
        .iter()
        .any(|s| t.eq_ignore_ascii_case(s))
}

/// Token indices of the ids a `fill` / `trcl` parameter names, split by family.
#[derive(Debug, Default)]
pub(crate) struct ParamRefs {
    /// Universe numbers (single fill, or every plain entry of a lattice array).
    pub universes: Vec<usize>,
    /// Transform numbers: `fill=u (n)`, `trcl=n`, `trcl=(n)`, and the `(n)`
    /// groups inside a fill array. Inline transforms `(dx dy dz ...)` hold
    /// several numbers and name no transform, so they are not listed.
    pub transforms: Vec<usize>,
}

/// Locate the universe and transform number tokens of a `fill` or `trcl`
/// parameter. Other keys return nothing.
pub(crate) fn param_refs(card: &Card, p: &CellParam) -> ParamRefs {
    let mut out = ParamRefs::default();
    let end = p.value_tokens.end.min(card.tokens().len());
    let is_fill = p.key.eq_ignore_ascii_case("fill");
    if !is_fill && !p.key.eq_ignore_ascii_case("trcl") {
        return out;
    }
    let is_plain_int = |k: usize| int_at(card, k).is_some();
    let mut k = p.value_tokens.start;
    // Whether the next plain number is a universe (fill) or transform (trcl).
    let mut in_parens = false;
    let mut group: Vec<usize> = Vec::new();
    while let Some(i) = sig(card, k) {
        if i >= end {
            break;
        }
        k = i + 1;
        match kind_at(card, i) {
            Some(SyntaxKind::LParen) => {
                in_parens = true;
                group.clear();
            }
            Some(SyntaxKind::RParen) => {
                if in_parens && group.len() == 1 {
                    out.transforms.push(group[0]);
                }
                in_parens = false;
            }
            Some(SyntaxKind::Number) if in_parens => group.push(i),
            Some(SyntaxKind::Number) => {
                // Range bounds (`-1:1`) sit next to a colon and are not ids.
                let next_is_colon =
                    next(card, i).is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Colon));
                let prev_is_colon =
                    prev(card, i).is_some_and(|j| kind_at(card, j) == Some(SyntaxKind::Colon));
                if next_is_colon || prev_is_colon || !is_plain_int(i) {
                    continue;
                }
                if is_fill {
                    out.universes.push(i);
                } else {
                    out.transforms.push(i);
                }
            }
            _ => {}
        }
    }
    out
}
