//! Typed view of cell cards, including the geometry boolean expression.
//!
//! The geometry is parsed into a [`GeomExpr`] tree with MCNP precedence
//! (`#` complement tightest, then intersection by juxtaposition, then union
//! `:`). Every leaf keeps the CST token it came from, so renumbering can
//! rewrite surface and cell references in place while the rest of the model
//! stays byte-for-byte intact.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::num::{parse_float, parse_int};

/// A reference to a surface from within a geometry expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceRef {
    /// CST token index of the (signed) surface number.
    pub(crate) token: u32,
    /// Surface number magnitude (always positive).
    pub id: i64,
    /// True if written with a negative sense (`-id`).
    pub negative: bool,
}

/// A reference to a cell (from a `#n` complement or a `LIKE n BUT` card).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    /// CST token index of the cell number.
    pub(crate) token: u32,
    /// Cell number.
    pub id: i64,
}

/// A parsed geometry boolean expression.
#[derive(Debug, Clone, PartialEq)]
pub enum GeomExpr {
    /// A signed surface (a sense of a surface).
    Surface(SurfaceRef),
    /// `#n` — the complement of cell `n`.
    CellComplement(CellRef),
    /// `#(...)` — the complement of a region.
    Complement(Box<GeomExpr>),
    /// Space-separated intersection of two or more regions.
    Intersection(Vec<GeomExpr>),
    /// `:`-separated union of two or more regions.
    Union(Vec<GeomExpr>),
    /// A malformed sub-expression (recorded rather than aborting the parse).
    Error,
}

impl GeomExpr {
    /// Visit every surface reference in the expression.
    pub fn for_each_surface_ref(&self, f: &mut impl FnMut(SurfaceRef)) {
        match self {
            GeomExpr::Surface(s) => f(*s),
            GeomExpr::CellComplement(_) | GeomExpr::Error => {}
            GeomExpr::Complement(e) => e.for_each_surface_ref(f),
            GeomExpr::Intersection(v) | GeomExpr::Union(v) => {
                for e in v {
                    e.for_each_surface_ref(f);
                }
            }
        }
    }

    /// Visit every cell reference (`#n` complements) in the expression.
    pub fn for_each_cell_ref(&self, f: &mut impl FnMut(CellRef)) {
        match self {
            GeomExpr::CellComplement(c) => f(*c),
            GeomExpr::Surface(_) | GeomExpr::Error => {}
            GeomExpr::Complement(e) => e.for_each_cell_ref(f),
            GeomExpr::Intersection(v) | GeomExpr::Union(v) => {
                for e in v {
                    e.for_each_cell_ref(f);
                }
            }
        }
    }

    /// True if this is the [`GeomExpr::Error`] placeholder (an empty/invalid
    /// region), used to detect an edit that would leave a cell with no geometry.
    pub fn is_empty(&self) -> bool {
        matches!(self, GeomExpr::Error)
    }

    /// Intersect the whole region with a signed surface (`id` is the magnitude;
    /// `negative` selects the negative sense). This is the usual meaning of
    /// "add a surface to a cell": the new sense is ANDed with everything.
    pub fn intersect_surface(&mut self, id: i64, negative: bool) {
        self.intersect_with(GeomExpr::Surface(SurfaceRef {
            token: NEW_TOKEN,
            id,
            negative,
        }));
    }

    /// Intersect the whole region with a `#n` cell complement.
    pub fn intersect_cell_complement(&mut self, id: i64) {
        self.intersect_with(GeomExpr::CellComplement(CellRef {
            token: NEW_TOKEN,
            id,
        }));
    }

    fn intersect_with(&mut self, other: GeomExpr) {
        match self {
            GeomExpr::Intersection(parts) => parts.push(other),
            _ => {
                let old = std::mem::replace(self, GeomExpr::Error);
                *self = GeomExpr::Intersection(vec![old, other]);
            }
        }
    }

    /// Remove every surface leaf whose magnitude is `id` (either sense),
    /// simplifying the tree. Returns `true` if anything was removed. If the
    /// whole region collapses to nothing, `self` becomes [`GeomExpr::Error`]
    /// (callers should treat that as "would empty the geometry").
    pub fn remove_surface(&mut self, id: i64) -> bool {
        self.remove_matching(&|e| matches!(e, GeomExpr::Surface(s) if s.id == id))
    }

    /// Remove every `#n` cell complement referencing cell `id`. Returns `true`
    /// if anything was removed; may collapse to [`GeomExpr::Error`].
    pub fn remove_cell_complement(&mut self, id: i64) -> bool {
        self.remove_matching(&|e| matches!(e, GeomExpr::CellComplement(c) if c.id == id))
    }

    fn remove_matching(&mut self, pred: &impl Fn(&GeomExpr) -> bool) -> bool {
        let old = std::mem::replace(self, GeomExpr::Error);
        let (new, removed) = prune(old, pred);
        *self = new.unwrap_or(GeomExpr::Error);
        removed
    }
}

/// Token sentinel for a reference created programmatically (no source token).
/// Such refs live only in an owned/edited cell, which is re-emitted from its
/// ids rather than by rewriting tokens, so the value is never dereferenced.
const NEW_TOKEN: u32 = u32::MAX;

/// Remove every sub-expression matching `pred`, returning the pruned region
/// (`None` if it became empty) and whether anything was removed. Empty
/// intersections/unions collapse; a single survivor is unwrapped.
fn prune(e: GeomExpr, pred: &impl Fn(&GeomExpr) -> bool) -> (Option<GeomExpr>, bool) {
    if pred(&e) {
        return (None, true);
    }
    match e {
        GeomExpr::Surface(_) | GeomExpr::CellComplement(_) | GeomExpr::Error => (Some(e), false),
        GeomExpr::Complement(inner) => match prune(*inner, pred) {
            (Some(i), removed) => (Some(GeomExpr::Complement(Box::new(i))), removed),
            (None, removed) => (None, removed),
        },
        GeomExpr::Intersection(parts) => {
            let (survivors, removed) = prune_parts(parts, pred);
            (collapse(survivors, GeomExpr::Intersection), removed)
        }
        GeomExpr::Union(parts) => {
            let (survivors, removed) = prune_parts(parts, pred);
            (collapse(survivors, GeomExpr::Union), removed)
        }
    }
}

fn prune_parts(parts: Vec<GeomExpr>, pred: &impl Fn(&GeomExpr) -> bool) -> (Vec<GeomExpr>, bool) {
    let mut survivors = Vec::with_capacity(parts.len());
    let mut removed = false;
    for p in parts {
        let (kept, r) = prune(p, pred);
        removed |= r;
        if let Some(k) = kept {
            survivors.push(k);
        }
    }
    (survivors, removed)
}

fn collapse(mut parts: Vec<GeomExpr>, build: fn(Vec<GeomExpr>) -> GeomExpr) -> Option<GeomExpr> {
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(build(parts)),
    }
}

/// A promoted, owned cell: geometry is typed and editable, while the parameter
/// tail (`imp`, `u`, `fill`, inline `$` comments, ...) is kept verbatim so
/// editing the geometry never drops it. Emitted back to MCNP text by the
/// internal emitter when the cell is re-serialised.
#[derive(Debug, Clone)]
pub struct OwnedCell {
    /// Cell number.
    pub id: i64,
    /// Material number (0 = void).
    pub material: Option<i64>,
    /// Density (absent for void).
    pub density: Option<f64>,
    /// The editable geometry region.
    pub geometry: GeomExpr,
    /// The verbatim parameter tail after the geometry (may be empty).
    pub params_text: String,
}

impl OwnedCell {
    /// Surface reference magnitudes in the geometry, in order.
    pub fn surface_ids(&self) -> Vec<i64> {
        let mut out = Vec::new();
        self.geometry.for_each_surface_ref(&mut |s| out.push(s.id));
        out
    }

    /// Signed surface references in the geometry, in order.
    pub fn signed_surfaces(&self) -> Vec<i64> {
        let mut out = Vec::new();
        self.geometry
            .for_each_surface_ref(&mut |s| out.push(if s.negative { -s.id } else { s.id }));
        out
    }

    /// Cell references (`#n` complements) in the geometry, in order.
    pub fn cell_refs(&self) -> Vec<i64> {
        let mut out = Vec::new();
        self.geometry.for_each_cell_ref(&mut |c| out.push(c.id));
        out
    }
}

/// Promote the cell at `card_index` into an editable [`OwnedCell`], or `None`
/// if it has no editable geometry (a `LIKE n BUT` cell) or its geometry did not
/// parse cleanly. The parameter tail is captured verbatim from the source.
pub(crate) fn promote_cell(tree: &GreenTree, card_index: usize) -> Option<OwnedCell> {
    let cell = parse_cell(tree, card_index)?;
    let geometry = cell.geometry.clone()?;
    if !cell.well_formed {
        return None;
    }
    // Capture the parameter tail from the *effective* tree so any parameters
    // spliced in (or out) before promotion are reflected.
    let card = tree.cards()[card_index];
    let params_text = match tree.card_content_tokens(&card).last() {
        Some(last) => tree.params_effective_text(cell.params_start, last),
        None => String::new(),
    };
    Some(OwnedCell {
        id: cell.id,
        material: cell.material,
        density: cell.density,
        geometry,
        params_text,
    })
}

/// A parsed cell card.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Cell number.
    pub id: i64,
    /// Material number (0 = void). `None` for `LIKE n BUT` cells.
    pub material: Option<i64>,
    /// Token index of the material number, if present.
    pub(crate) material_token: Option<u32>,
    /// Density (positive = atom/b-cm, negative = mass g/cc). Absent for void.
    pub density: Option<f64>,
    /// Token index of the density value, if the cell has one (for edits).
    pub(crate) density_token: Option<u32>,
    /// For a `LIKE n BUT` card, the referenced base cell.
    pub like: Option<CellRef>,
    /// The geometry expression (absent for `LIKE n BUT`).
    pub geometry: Option<GeomExpr>,
    /// First meaningful CST token of the geometry region, if any. A stable
    /// anchor for splice edits (unaffected by later emit-only overlay edits).
    pub(crate) geom_first_token: Option<u32>,
    /// Last meaningful CST token of the geometry region, if any (may be a `)`
    /// rather than a surface leaf). The anchor a new intersection term is
    /// spliced in after.
    pub(crate) geom_last_token: Option<u32>,
    /// First token index of the parameter section (IMP, VOL, …), if any.
    pub(crate) params_start: Option<u32>,
    /// False if some part failed to parse.
    pub well_formed: bool,
}

impl Cell {
    /// Collect all surface references in the geometry.
    pub fn surface_refs(&self) -> Vec<SurfaceRef> {
        let mut out = Vec::new();
        if let Some(g) = &self.geometry {
            g.for_each_surface_ref(&mut |s| out.push(s));
        }
        out
    }

    /// Collect all cell references (geometry `#n` complements plus a `LIKE n`).
    pub fn cell_refs(&self) -> Vec<CellRef> {
        let mut out = Vec::new();
        if let Some(l) = self.like {
            out.push(l);
        }
        if let Some(g) = &self.geometry {
            g.for_each_cell_ref(&mut |c| out.push(c));
        }
        out
    }
}

/// Parse the cell card at `card_index`, or `None` if it is not a cell card.
pub(crate) fn parse_cell(tree: &GreenTree, card_index: usize) -> Option<Cell> {
    let card = &tree.cards()[card_index];
    if card.kind != SyntaxKind::CELL_CARD {
        return None;
    }
    // Meaningful tokens, dropping `&` continuation markers.
    let toks: Vec<u32> = tree
        .card_content_tokens(card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP)
        .collect();
    let mut pos = 0usize;

    // Cell id.
    let id_token = *toks.get(pos)?;
    if tree.token_kind(id_token) != SyntaxKind::NUMBER {
        return None;
    }
    let id = parse_int(&tree.token_text(id_token))?;
    pos += 1;

    // `LIKE n BUT` form?
    if toks
        .get(pos)
        .map(|&i| {
            tree.token_kind(i) == SyntaxKind::IDENT
                && tree.token_text(i).eq_ignore_ascii_case("like")
        })
        .unwrap_or(false)
    {
        pos += 1;
        let ref_tok = *toks.get(pos)?;
        let ref_id = parse_int(&tree.token_text(ref_tok))?;
        pos += 1;
        // Optional BUT keyword.
        let mut well_formed = true;
        if toks
            .get(pos)
            .map(|&i| tree.token_text(i).eq_ignore_ascii_case("but"))
            .unwrap_or(false)
        {
            pos += 1;
        } else {
            well_formed = false;
        }
        return Some(Cell {
            card_index,
            id,
            material: None,
            material_token: None,
            density: None,
            density_token: None,
            like: Some(CellRef {
                token: ref_tok,
                id: ref_id,
            }),
            geometry: None,
            geom_first_token: None,
            geom_last_token: None,
            params_start: toks.get(pos).copied(),
            well_formed,
        });
    }

    // Material number.
    let mat_tok = *toks.get(pos)?;
    if tree.token_kind(mat_tok) != SyntaxKind::NUMBER {
        return None;
    }
    let material = parse_int(&tree.token_text(mat_tok))?;
    pos += 1;

    // Density, only when not void.
    let mut density = None;
    let mut density_token = None;
    if material != 0 {
        let d_tok = *toks.get(pos)?;
        density = parse_float(&tree.token_text(d_tok));
        density_token = Some(d_tok);
        pos += 1;
    }

    // Geometry runs until the first IDENT (start of the parameter section).
    let geom_end = toks[pos..]
        .iter()
        .position(|&i| tree.token_kind(i) == SyntaxKind::IDENT)
        .map(|off| pos + off)
        .unwrap_or(toks.len());
    let geom_tokens = &toks[pos..geom_end];
    let params_start = toks.get(geom_end).copied();
    let geom_first_token = geom_tokens.first().copied();
    let geom_last_token = geom_tokens.last().copied();

    let (geometry, geom_ok) = if geom_tokens.is_empty() {
        (None, false)
    } else {
        let mut p = GeomParser {
            tree,
            toks: geom_tokens,
            pos: 0,
            ok: true,
        };
        let e = p.parse_union();
        // Trailing unparsed geometry tokens => malformed.
        let ok = p.ok && p.pos == geom_tokens.len();
        (Some(e), ok)
    };

    Some(Cell {
        card_index,
        id,
        material: Some(material),
        material_token: Some(mat_tok),
        density,
        density_token,
        like: None,
        geometry,
        geom_first_token,
        geom_last_token,
        params_start,
        well_formed: geom_ok,
    })
}

/// Iterate all parseable cells in the model, in source order.
pub(crate) fn cells(tree: &GreenTree) -> impl Iterator<Item = Cell> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_cell(tree, i))
}

/// One `keyword=value…` entry from a cell's parameter section (`imp`, `vol`,
/// `u`, `fill`, `trcl`, …). The value tokens are the meaningful tokens up to the
/// next keyword; interpretation is left to callers (e.g. universe renumbering).
#[derive(Debug, Clone)]
pub struct CellParam {
    /// Uppercased keyword without any `:particle` designator (e.g. `"U"`).
    pub key: String,
    /// The value tokens (numbers, colons, parens, …) belonging to this keyword.
    pub(crate) value_tokens: Vec<u32>,
    /// First CST token of the whole entry (the `*` prefix or the keyword).
    pub(crate) start_token: u32,
    /// Last CST token of the whole entry (the last value token, or the keyword
    /// itself for a value-less flag). Together with `start_token` this is the
    /// span a splice deletes to remove the parameter.
    pub(crate) end_token: u32,
}

/// True if `toks[j]` starts a new cell parameter keyword: an `IDENT` followed by
/// `=`, or by `:particle =`.
fn is_keyword_start(tree: &GreenTree, toks: &[u32], j: usize) -> bool {
    if tree.token_kind(toks[j]) != SyntaxKind::IDENT {
        return false;
    }
    let mut k = j + 1;
    if k < toks.len() && tree.token_kind(toks[k]) == SyntaxKind::COLON {
        k += 1;
        if k < toks.len() && tree.token_kind(toks[k]) == SyntaxKind::IDENT {
            k += 1;
        }
    }
    k < toks.len() && tree.token_kind(toks[k]) == SyntaxKind::EQ
}

/// Parse the parameter section (after the geometry) of the cell at `card_index`
/// into `keyword=value…` entries. Empty for non-cells, `LIKE n BUT` cells, or
/// cells without parameters. Keywords may carry a `*` prefix (`*fill`, `*trcl`)
/// and a `:particle` designator (`imp:n`); both are handled.
pub(crate) fn cell_params(tree: &GreenTree, card_index: usize) -> Vec<CellParam> {
    let Some(cell) = parse_cell(tree, card_index) else {
        return Vec::new();
    };
    let Some(params_start) = cell.params_start else {
        return Vec::new();
    };
    let card = tree.cards()[card_index];
    let toks: Vec<u32> = tree
        .card_content_tokens(&card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP)
        .collect();
    let Some(mut j) = toks.iter().position(|&t| t == params_start) else {
        return Vec::new();
    };
    let n = toks.len();
    let mut params = Vec::new();
    while j < n {
        let start_token = toks[j];
        // A keyword may be prefixed with `*` (e.g. `*fill`, `*trcl`).
        if tree.token_kind(toks[j]) == SyntaxKind::STAR {
            j += 1;
        }
        if j >= n || tree.token_kind(toks[j]) != SyntaxKind::IDENT {
            break;
        }
        let key = tree.token_text(toks[j]).to_ascii_uppercase();
        j += 1;
        // Optional `:particle`.
        if j < n && tree.token_kind(toks[j]) == SyntaxKind::COLON {
            j += 1;
            if j < n && tree.token_kind(toks[j]) == SyntaxKind::IDENT {
                j += 1;
            }
        }
        if j < n && tree.token_kind(toks[j]) == SyntaxKind::EQ {
            j += 1;
        }
        // Values run until the next keyword (optionally `*`-prefixed).
        let vstart = j;
        while j < n {
            let at_next = match tree.token_kind(toks[j]) {
                SyntaxKind::STAR => j + 1 < n && is_keyword_start(tree, &toks, j + 1),
                SyntaxKind::IDENT => is_keyword_start(tree, &toks, j),
                _ => false,
            };
            if at_next {
                break;
            }
            j += 1;
        }
        params.push(CellParam {
            key,
            value_tokens: toks[vstart..j].to_vec(),
            start_token,
            end_token: toks[j - 1],
        });
    }
    params
}

/// Minimal read of a cell's material field: `(material_token, material)`, or
/// `None` for a non-cell card or a `LIKE n BUT` cell (which has no material
/// field). Allocation-light; used by material renumbering.
pub(crate) fn cell_material(tree: &GreenTree, card_index: usize) -> Option<(u32, i64)> {
    let card = tree.cards()[card_index];
    if card.kind != SyntaxKind::CELL_CARD {
        return None;
    }
    let mut it = tree
        .card_content_tokens(&card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP);
    let id_tok = it.next()?;
    if tree.token_kind(id_tok) != SyntaxKind::NUMBER {
        return None;
    }
    let mat_tok = it.next()?;
    // A `LIKE n BUT` cell has an IDENT here, not a material number.
    if tree.token_kind(mat_tok) != SyntaxKind::NUMBER {
        return None;
    }
    Some((mat_tok, parse_int(&tree.token_text(mat_tok))?))
}

/// Minimal cell-header read for indexing: `(id_token, id)`. Allocation-free.
pub(crate) fn cell_id(tree: &GreenTree, card_index: usize) -> Option<(u32, i64)> {
    let card = tree.cards()[card_index];
    if card.kind != SyntaxKind::CELL_CARD {
        return None;
    }
    let tok = tree.card_content_tokens(&card).next()?;
    if tree.token_kind(tok) != SyntaxKind::NUMBER {
        return None;
    }
    Some((tok, parse_int(&tree.token_text(tok))?))
}

/// What a scanned reference is. Reported by [`scan_cell_refs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// The cell's own id.
    CellId,
    /// A signed surface reference; the reported value is the magnitude.
    SurfaceRef { negative: bool },
    /// A cell reference (`#n` complement or `LIKE n` base).
    CellRef,
}

/// Allocation-light scan of a cell card's references, for bulk edits like
/// renumbering. Invokes `visit(kind, token, value)` for the cell id, every
/// surface reference (value = magnitude, sense in `kind`), and every cell
/// reference. Builds no geometry tree.
///
/// It applies the same structural rules as [`parse_cell`]: a `NUMBER` in the
/// geometry region is a surface reference unless it immediately follows `#`
/// (a cell complement); `#(` opens a region complement whose interior numbers
/// are surface references.
pub(crate) fn scan_cell_refs(
    tree: &GreenTree,
    card_index: usize,
    mut visit: impl FnMut(RefKind, u32, i64),
) {
    let card = tree.cards()[card_index];
    if card.kind != SyntaxKind::CELL_CARD {
        return;
    }
    let toks: Vec<u32> = tree
        .card_content_tokens(&card)
        .filter(|&i| tree.token_kind(i) != SyntaxKind::AMP)
        .collect();

    // Cell id.
    let Some(&id_tok) = toks.first() else { return };
    if tree.token_kind(id_tok) != SyntaxKind::NUMBER {
        return;
    }
    let Some(id) = parse_int(&tree.token_text(id_tok)) else {
        return;
    };
    visit(RefKind::CellId, id_tok, id);

    // LIKE n BUT: the only reference is the base cell; no geometry.
    if toks.get(1).is_some_and(|&i| {
        tree.token_kind(i) == SyntaxKind::IDENT && tree.token_text(i).eq_ignore_ascii_case("like")
    }) {
        if let Some(&rt) = toks.get(2) {
            if let Some(v) = parse_int(&tree.token_text(rt)) {
                visit(RefKind::CellRef, rt, v);
            }
        }
        return;
    }

    // Skip material (+ density when not void).
    let mut pos = 1;
    let material = match toks.get(pos) {
        Some(&i) if tree.token_kind(i) == SyntaxKind::NUMBER => parse_int(&tree.token_text(i)),
        _ => return,
    };
    pos += 1;
    if material != Some(0) {
        pos += 1; // density
    }

    // Geometry region, up to the first IDENT (start of parameters).
    let mut prev_hash = false;
    for &t in &toks[pos..] {
        match tree.token_kind(t) {
            SyntaxKind::IDENT => break,
            SyntaxKind::HASH => {
                prev_hash = true;
            }
            SyntaxKind::NUMBER => {
                if let Some(v) = parse_int(&tree.token_text(t)) {
                    if prev_hash {
                        visit(RefKind::CellRef, t, v);
                    } else {
                        visit(RefKind::SurfaceRef { negative: v < 0 }, t, v.abs());
                    }
                }
                prev_hash = false;
            }
            _ => prev_hash = false, // COLON, parens, etc.
        }
    }
}

/// Recursive-descent parser for a geometry token slice.
struct GeomParser<'a> {
    tree: &'a GreenTree,
    toks: &'a [u32],
    pos: usize,
    ok: bool,
}

impl GeomParser<'_> {
    fn peek(&self) -> Option<(SyntaxKind, u32)> {
        self.toks
            .get(self.pos)
            .map(|&i| (self.tree.token_kind(i), i))
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    /// union := intersection (':' intersection)*
    fn parse_union(&mut self) -> GeomExpr {
        let mut parts = vec![self.parse_intersection()];
        while let Some((SyntaxKind::COLON, _)) = self.peek() {
            self.bump();
            parts.push(self.parse_intersection());
        }
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            GeomExpr::Union(parts)
        }
    }

    /// intersection := factor (factor)*   (juxtaposition)
    fn parse_intersection(&mut self) -> GeomExpr {
        let mut parts = vec![self.parse_factor()];
        while let Some((SyntaxKind::NUMBER | SyntaxKind::HASH | SyntaxKind::L_PAREN, _)) =
            self.peek()
        {
            parts.push(self.parse_factor());
        }
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            GeomExpr::Intersection(parts)
        }
    }

    /// factor := '#(' union ')' | '#' NUMBER | '(' union ')' | signed-surface
    fn parse_factor(&mut self) -> GeomExpr {
        match self.peek() {
            Some((SyntaxKind::HASH, _)) => {
                self.bump();
                match self.peek() {
                    Some((SyntaxKind::L_PAREN, _)) => {
                        self.bump();
                        let e = self.parse_union();
                        self.expect_rparen();
                        GeomExpr::Complement(Box::new(e))
                    }
                    Some((SyntaxKind::NUMBER, tok)) => {
                        self.bump();
                        match parse_int(&self.tree.token_text(tok)) {
                            Some(id) => GeomExpr::CellComplement(CellRef { token: tok, id }),
                            None => {
                                self.ok = false;
                                GeomExpr::Error
                            }
                        }
                    }
                    _ => {
                        self.ok = false;
                        GeomExpr::Error
                    }
                }
            }
            Some((SyntaxKind::L_PAREN, _)) => {
                self.bump();
                let e = self.parse_union();
                self.expect_rparen();
                e
            }
            Some((SyntaxKind::NUMBER, tok)) => {
                self.bump();
                match parse_int(&self.tree.token_text(tok)) {
                    Some(v) => GeomExpr::Surface(SurfaceRef {
                        token: tok,
                        id: v.abs(),
                        negative: v < 0,
                    }),
                    None => {
                        self.ok = false;
                        GeomExpr::Error
                    }
                }
            }
            _ => {
                self.ok = false;
                GeomExpr::Error
            }
        }
    }

    fn expect_rparen(&mut self) {
        if let Some((SyntaxKind::R_PAREN, _)) = self.peek() {
            self.bump();
        } else {
            self.ok = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crunchy_syntax::parse;

    fn model(cell_line: &str) -> GreenTree {
        let src = format!("title\n{cell_line}\n\n1 PX 0\n\nm1 1001 1\n");
        parse(src).tree
    }

    fn first_cell(t: &GreenTree) -> Cell {
        cells(t).next().unwrap()
    }

    #[test]
    fn void_and_material_cells() {
        let t = model("1 1 -1.0 -1 imp:n=1");
        let c = first_cell(&t);
        assert_eq!(c.id, 1);
        assert_eq!(c.material, Some(1));
        assert_eq!(c.density, Some(-1.0));
        assert!(c.well_formed);

        let t = model("2 0 1 imp:n=0");
        let c = first_cell(&t);
        assert_eq!(c.material, Some(0));
        assert_eq!(c.density, None);
    }

    #[test]
    fn surface_refs_with_sense() {
        let t = model("1 0 -1 2 -3");
        let refs = first_cell(&t).surface_refs();
        assert_eq!(
            refs,
            vec![
                SurfaceRef {
                    token: refs[0].token,
                    id: 1,
                    negative: true
                },
                SurfaceRef {
                    token: refs[1].token,
                    id: 2,
                    negative: false
                },
                SurfaceRef {
                    token: refs[2].token,
                    id: 3,
                    negative: true
                },
            ]
        );
    }

    #[test]
    fn union_and_intersection() {
        let t = model("1 0 1 -2 : 3");
        let g = first_cell(&t).geometry.unwrap();
        match g {
            GeomExpr::Union(parts) => assert_eq!(parts.len(), 2),
            other => panic!("expected union, got {other:?}"),
        }
    }

    #[test]
    fn cell_complement_and_region_complement() {
        let t = model("5 0 #3 #(1 -2) imp:n=1");
        let c = first_cell(&t);
        let cell_refs = c.cell_refs();
        assert_eq!(cell_refs.len(), 1);
        assert_eq!(cell_refs[0].id, 3);
        // The region complement contributes two surface refs (1 and 2).
        let s: Vec<_> = c.surface_refs().iter().map(|r| r.id).collect();
        assert_eq!(s, vec![1, 2]);
    }

    #[test]
    fn nested_parentheses() {
        let t = model("1 0 (1 : (2 3)) -4");
        let c = first_cell(&t);
        assert!(c.well_formed, "nested parens must parse");
        let ids: Vec<_> = c.surface_refs().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn scan_matches_tree_walk() {
        // The fast scan must report the same refs as the GeomExpr tree walk.
        let t = model("5 1 -2.0 (-1 : -2) 3 #4 #(5 -6) imp:n=1");
        let c = first_cell(&t);
        let tree_surface: Vec<_> = c
            .surface_refs()
            .iter()
            .map(|r| (r.id, r.negative))
            .collect();
        let tree_cell: Vec<_> = c.cell_refs().iter().map(|r| r.id).collect();

        let mut scan_surface = Vec::new();
        let mut scan_cell = Vec::new();
        let mut ids = Vec::new();
        scan_cell_refs(&t, c.card_index, |kind, _tok, val| match kind {
            RefKind::CellId => ids.push(val),
            RefKind::SurfaceRef { negative } => scan_surface.push((val, negative)),
            RefKind::CellRef => scan_cell.push(val),
        });
        assert_eq!(ids, vec![5]);
        assert_eq!(scan_surface, tree_surface);
        assert_eq!(scan_cell, tree_cell);
    }

    #[test]
    fn like_but_form() {
        let t = model("9 LIKE 3 BUT mat=2 rho=-1.0");
        let c = first_cell(&t);
        assert_eq!(c.id, 9);
        assert_eq!(c.like.unwrap().id, 3);
        assert!(c.geometry.is_none());
        assert_eq!(c.cell_refs()[0].id, 3);
    }

    fn owned(cell_line: &str) -> OwnedCell {
        let t = model(cell_line);
        let ci = first_cell(&t).card_index;
        promote_cell(&t, ci).expect("promotable")
    }

    #[test]
    fn promote_captures_params_verbatim() {
        let oc = owned("1 1 -1.0 -1 2 imp:n=1 vol=3");
        assert_eq!(oc.id, 1);
        assert_eq!(oc.material, Some(1));
        assert_eq!(oc.density, Some(-1.0));
        assert_eq!(oc.signed_surfaces(), vec![-1, 2]);
        assert_eq!(oc.params_text, "imp:n=1 vol=3");
    }

    #[test]
    fn promote_refuses_like_and_malformed() {
        let t = model("9 LIKE 3 BUT imp:n=1");
        let ci = first_cell(&t).card_index;
        assert!(promote_cell(&t, ci).is_none());
    }

    #[test]
    fn intersect_surface_ands_with_region() {
        let mut oc = owned("1 0 -1 : 2");
        oc.geometry.intersect_surface(3, true); // add -3 to the whole region
                                                // The union is now intersected with -3: (-1 : 2) -3.
        assert_eq!(oc.signed_surfaces(), vec![-1, 2, -3]);
    }

    #[test]
    fn remove_surface_simplifies() {
        let mut oc = owned("1 0 -1 2 -3");
        assert!(oc.geometry.remove_surface(2));
        assert_eq!(oc.signed_surfaces(), vec![-1, -3]);
        // Removing a non-present surface reports false and changes nothing.
        assert!(!oc.geometry.remove_surface(99));
        assert_eq!(oc.signed_surfaces(), vec![-1, -3]);
    }

    #[test]
    fn remove_last_surface_empties_geometry() {
        let mut oc = owned("1 0 -1");
        assert!(oc.geometry.remove_surface(1));
        assert!(oc.geometry.is_empty());
    }

    #[test]
    fn add_and_remove_cell_complement() {
        let mut oc = owned("1 0 -1 2");
        oc.geometry.intersect_cell_complement(7);
        assert_eq!(oc.cell_refs(), vec![7]);
        assert!(oc.geometry.remove_cell_complement(7));
        assert_eq!(oc.cell_refs(), Vec::<i64>::new());
    }
}
