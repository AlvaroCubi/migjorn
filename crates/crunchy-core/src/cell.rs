//! Typed view of cell cards, including the geometry boolean expression.
//!
//! The geometry is parsed into a [`GeomExpr`] tree with MCNP precedence
//! (`#` complement tightest, then intersection by juxtaposition, then union
//! `:`). Every leaf keeps the CST token it came from, so renumbering can
//! rewrite surface and cell references in place while the rest of the deck
//! stays byte-for-byte intact.

use crunchy_syntax::{GreenTree, SyntaxKind};

use crate::num::{parse_float, parse_int};

/// A reference to a surface from within a geometry expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceRef {
    /// CST token index of the (signed) surface number.
    pub token: u32,
    /// Surface number magnitude (always positive).
    pub id: i64,
    /// True if written with a negative sense (`-id`).
    pub negative: bool,
}

/// A reference to a cell (from a `#n` complement or a `LIKE n BUT` card).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    /// CST token index of the cell number.
    pub token: u32,
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
}

/// A parsed cell card.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Index into `tree.cards()`.
    pub card_index: usize,
    /// Cell number.
    pub id: i64,
    /// Token index of the id (for renumbering).
    pub id_token: u32,
    /// Material number (0 = void). `None` for `LIKE n BUT` cells.
    pub material: Option<i64>,
    /// Token index of the material number, if present.
    pub material_token: Option<u32>,
    /// Density (positive = atom/b-cm, negative = mass g/cc). Absent for void.
    pub density: Option<f64>,
    /// For a `LIKE n BUT` card, the referenced base cell.
    pub like: Option<CellRef>,
    /// The geometry expression (absent for `LIKE n BUT`).
    pub geometry: Option<GeomExpr>,
    /// First token index of the parameter section (IMP, VOL, …), if any.
    pub params_start: Option<u32>,
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
pub fn parse_cell(tree: &GreenTree, card_index: usize) -> Option<Cell> {
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
        .map(|&i| tree.token_kind(i) == SyntaxKind::IDENT && tree.token_text(i).eq_ignore_ascii_case("like"))
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
            id_token,
            material: None,
            material_token: None,
            density: None,
            like: Some(CellRef {
                token: ref_tok,
                id: ref_id,
            }),
            geometry: None,
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
    if material != 0 {
        let d_tok = *toks.get(pos)?;
        density = parse_float(&tree.token_text(d_tok));
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
        id_token,
        material: Some(material),
        material_token: Some(mat_tok),
        density,
        like: None,
        geometry,
        params_start,
        well_formed: geom_ok,
    })
}

/// Iterate all parseable cells in the deck, in source order.
pub fn cells(tree: &GreenTree) -> impl Iterator<Item = Cell> + '_ {
    (0..tree.cards().len()).filter_map(move |i| parse_cell(tree, i))
}

/// Minimal cell-header read for indexing: `(id_token, id)`. Allocation-free.
pub fn cell_id(tree: &GreenTree, card_index: usize) -> Option<(u32, i64)> {
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
pub enum RefKind {
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
pub fn scan_cell_refs(tree: &GreenTree, card_index: usize, mut visit: impl FnMut(RefKind, u32, i64)) {
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
    if toks
        .get(1)
        .is_some_and(|&i| tree.token_kind(i) == SyntaxKind::IDENT && tree.token_text(i).eq_ignore_ascii_case("like"))
    {
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
        self.toks.get(self.pos).map(|&i| (self.tree.token_kind(i), i))
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
        while let Some((SyntaxKind::NUMBER | SyntaxKind::HASH | SyntaxKind::L_PAREN, _)) = self.peek()
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

    fn deck(cell_line: &str) -> GreenTree {
        let src = format!("title\n{cell_line}\n\n1 PX 0\n\nm1 1001 1\n");
        parse(src).tree
    }

    fn first_cell(t: &GreenTree) -> Cell {
        cells(t).next().unwrap()
    }

    #[test]
    fn void_and_material_cells() {
        let t = deck("1 1 -1.0 -1 imp:n=1");
        let c = first_cell(&t);
        assert_eq!(c.id, 1);
        assert_eq!(c.material, Some(1));
        assert_eq!(c.density, Some(-1.0));
        assert!(c.well_formed);

        let t = deck("2 0 1 imp:n=0");
        let c = first_cell(&t);
        assert_eq!(c.material, Some(0));
        assert_eq!(c.density, None);
    }

    #[test]
    fn surface_refs_with_sense() {
        let t = deck("1 0 -1 2 -3");
        let refs = first_cell(&t).surface_refs();
        assert_eq!(
            refs,
            vec![
                SurfaceRef { token: refs[0].token, id: 1, negative: true },
                SurfaceRef { token: refs[1].token, id: 2, negative: false },
                SurfaceRef { token: refs[2].token, id: 3, negative: true },
            ]
        );
    }

    #[test]
    fn union_and_intersection() {
        let t = deck("1 0 1 -2 : 3");
        let g = first_cell(&t).geometry.unwrap();
        match g {
            GeomExpr::Union(parts) => assert_eq!(parts.len(), 2),
            other => panic!("expected union, got {other:?}"),
        }
    }

    #[test]
    fn cell_complement_and_region_complement() {
        let t = deck("5 0 #3 #(1 -2) imp:n=1");
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
        let t = deck("1 0 (1 : (2 3)) -4");
        let c = first_cell(&t);
        assert!(c.well_formed, "nested parens must parse");
        let ids: Vec<_> = c.surface_refs().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn scan_matches_tree_walk() {
        // The fast scan must report the same refs as the GeomExpr tree walk.
        let t = deck("5 1 -2.0 (-1 : -2) 3 #4 #(5 -6) imp:n=1");
        let c = first_cell(&t);
        let tree_surface: Vec<_> = c.surface_refs().iter().map(|r| (r.id, r.negative)).collect();
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
        let t = deck("9 LIKE 3 BUT mat=2 rho=-1.0");
        let c = first_cell(&t);
        assert_eq!(c.id, 9);
        assert_eq!(c.like.unwrap().id, 3);
        assert!(c.geometry.is_none());
        assert_eq!(c.cell_refs()[0].id, 3);
    }
}
