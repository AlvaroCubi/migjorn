//! In-card value edits (milestone M2).
//!
//! Every method here mutates exactly one card by splicing its own `text` and
//! fixing that card's token spans — never re-lexing or re-parsing the file, and
//! never touching a second card. Most of these edits do not change any defined
//! id, so the model's id indices stay valid without maintenance; the
//! `set_*_id` methods are the exception — they do change a defined id, so they
//! additionally patch that one index entry, still without touching any other
//! card (in particular, they do not move references the way `renumber_*`
//! does — see each method's doc comment).
//!
//! Writes are addressed by stable `slot` (the anchor a live view resolves
//! through), so a view can hand its `slot` to the model and the edit lands on the
//! right card even after inserts/removes elsewhere.

use migjorn_syntax::{CardKind, Cst};

use crate::data;
use crate::model::Model;
use crate::renumber::{remap_name, remap_token};
use crate::scan::sig;
use crate::{cell, surface};

/// Why an edit could not be applied. Reads never fail (they project a best-effort
/// value); edits do, because the caller asked to change a field that isn't there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The slot resolves to a removed card, or a card of the wrong kind for this
    /// edit (e.g. `set_cell_material` on a surface).
    WrongKind,
    /// The addressed field or index does not exist on this card (density on a
    /// void cell, coefficient index past the end, entry past the last one).
    NoSuchField,
    /// There is no block of the relevant kind to add the new card into (e.g.
    /// `add_cell` on a model that has no cell block at all).
    NoBlock,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            EditError::WrongKind => "card is missing or is not the right kind for this edit",
            EditError::NoSuchField => "the addressed field does not exist on this card",
            EditError::NoBlock => "the model has no block of this kind to add the card into",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for EditError {}

/// Format a number the way MCNP reads it back: the shortest round-trippable
/// decimal. Integral values print without a fractional part (`-1.0` -> `-1`),
/// which is valid MCNP and matches how these fields are usually written.
fn fmt_num(v: f64) -> String {
    // Rust's default float formatting is already shortest-round-trip.
    format!("{v}")
}

/// Byte offset of a card's content end — before its `\r\n` / `\n` terminator.
fn content_end(text: &str) -> usize {
    let b = text.as_bytes();
    let mut e = b.len();
    if e > 0 && b[e - 1] == b'\n' {
        e -= 1;
        if e > 0 && b[e - 1] == b'\r' {
            e -= 1;
        }
    }
    e
}

/// Byte offset just past the last non-trivia token (so an append lands before any
/// trailing `$` comment).
fn last_significant_end(card: &migjorn_syntax::Card) -> Option<usize> {
    card.tokens()
        .iter()
        .rposition(|t| !t.is_trivia())
        .map(|i| card.tokens()[i].end() as usize)
}

/// Where to land an edit when a cell's geometry has no terms to anchor on
/// (it's empty): before whatever follows immediately — typically a keyword
/// parameter — with a trailing separator, or, if nothing follows at all,
/// after the last significant token with a leading one, like
/// `add_cell_param`. Shared by `insert_geometry_term` and `set_cell_geometry`.
fn empty_geometry_anchor(
    card: &migjorn_syntax::Card,
    l: &cell::CellLayout,
    text: &str,
) -> (usize, String) {
    match card.tokens().get(l.geometry.start) {
        Some(t) => (t.start as usize, format!("{text} ")),
        None => {
            let at = last_significant_end(card).unwrap_or_else(|| content_end(card.text()));
            (at, format!(" {text}"))
        }
    }
}

fn join_nums(values: &[f64]) -> String {
    let mut out = String::new();
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&fmt_num(*v));
    }
    out
}

impl Model {
    // --- cells --------------------------------------------------------------

    /// Rewrite this cell's own id. Local to this card only — unlike
    /// [`Model::renumber_cells`], it does **not** move any `#<id>`/
    /// `LIKE <id> BUT` reference elsewhere in the file. Safe to use on a cell
    /// nothing references yet — e.g. right after `add_cell(source.text)`, to
    /// clone a cell under a new id; used on an already-referenced cell, those
    /// references go dangling — [`Model::validate`] will report them. Use
    /// `renumber_cells` instead when references should move with the
    /// definition.
    pub fn set_cell_id(&mut self, slot: u32, new_id: i64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let id_tok = sig(card, 0).ok_or(EditError::NoSuchField)?;
        let old_id = cell::layout(card).id;
        let new_text =
            remap_token(card.token_text(id_tok), &|_| new_id).ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(id_tok, &new_text);
        if let Some(old) = old_id {
            if self.cell_index.get(&old) == Some(&slot) {
                self.cell_index.remove(&old);
            }
        }
        self.cell_index.entry(new_id).or_insert(slot);
        Ok(())
    }

    /// Set a cell's material number, crossing the void boundary as needed.
    ///
    /// Void (`0`) has no density field, a real material has one, so this is not
    /// always a token swap: going void -> real inserts a placeholder density, and
    /// real -> void drops the density field. The geometry and trailing parameters
    /// are untouched either way.
    pub fn set_cell_material(&mut self, slot: u32, material: i64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let mat_tok = l.material_tok.ok_or(EditError::NoSuchField)?;
        let mat_span = card.tokens()[mat_tok].range();
        let density_end = l.density_tok.map(|d| card.tokens()[d].range().end);

        let currently_void = l.material == Some(0);
        let becoming_void = material == 0;
        let target = self.cst.card_mut(slot).unwrap();

        match (currently_void, becoming_void) {
            // Plain swap: void stays void, or real stays real (density kept).
            (false, false) | (true, true) => {
                target.set_token_text(mat_tok, &material.to_string());
            }
            // void -> real: the material token grows into `<material> <density>`,
            // with a `0` placeholder the caller is expected to set next.
            (true, false) => {
                target.splice(mat_span, &format!("{material} 0"));
            }
            // real -> void: collapse `<material> <density>` down to just `0`.
            (false, true) => {
                let end = density_end.ok_or(EditError::NoSuchField)?;
                target.splice(mat_span.start..end, "0");
            }
        }
        Ok(())
    }

    /// Set a cell's density. Only meaningful on a non-void cell — a void cell has
    /// no density field, so this reports [`EditError::NoSuchField`] rather than
    /// inventing one (set the material first).
    pub fn set_cell_density(&mut self, slot: u32, density: f64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let dens_tok = cell::layout(card)
            .density_tok
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(dens_tok, &fmt_num(density));
        Ok(())
    }

    /// Set the value of an existing keyword parameter (`imp:n`, `vol`, `fill`,
    /// ...), matched case-insensitively by its qualified key. Returns `false` if
    /// the cell has no such parameter — adding one is a structural edit (M3), not
    /// this. Everything else on the card, including a trailing `$` comment, is
    /// preserved byte-for-byte.
    pub fn set_cell_param(&mut self, slot: u32, key: &str, value: &str) -> Result<bool, EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let params = cell::params(card, &l.params);
        let Some(p) = params
            .iter()
            .find(|p| p.qualified_key().eq_ignore_ascii_case(key))
        else {
            return Ok(false);
        };
        // An empty value slot means a bare keyword; giving it a value is really an
        // add, so leave it to M3 rather than guess an insertion point.
        if p.value_tokens.start >= p.value_tokens.end {
            return Ok(false);
        }
        let toks = card.tokens();
        let from = toks[p.value_tokens.start].start as usize;
        let to = toks[p.value_tokens.end - 1].end() as usize;
        self.cst.card_mut(slot).unwrap().splice(from..to, value);
        Ok(true)
    }

    /// Replace one term of a cell's geometry expression with new literal text,
    /// addressed by its position in the term list [`crate::CellView::geometry`]
    /// returns (0-based; parentheses and unions count as terms there too). The
    /// replacement need not be the same length — `"124"` for `"123"`, or
    /// `"#457"` for `"#123"` — but the term count is unchanged; growing or
    /// shrinking the geometry is [`Model::insert_geometry_term`].
    pub fn set_geometry_term(
        &mut self,
        slot: u32,
        position: usize,
        text: &str,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let spans = cell::walk_geometry_spans(card, &l.geometry);
        let (_, span) = spans.get(position).ok_or(EditError::NoSuchField)?;
        let toks = card.tokens();
        let from = toks[span.start].start as usize;
        let to = toks[span.end - 1].end() as usize;
        self.cst
            .card_mut(slot)
            .unwrap()
            .splice(from..to, text.trim());
        Ok(())
    }

    /// Insert a new term into a cell's geometry expression, addressed the same
    /// way as [`Model::set_geometry_term`]: `position` is where in the term list
    /// the new term lands, `position == terms.len()` appends after the last one
    /// — the same convention as `Vec::insert`. This is the one structural
    /// geometry primitive; anything else (wrapping in parentheses, adding a
    /// union or a cell complement) composes from repeated calls, e.g. hashing a
    /// cell's geometry with cell 123:
    ///
    /// ```ignore
    /// let n = model.cell(id).unwrap().geometry().len();
    /// model.insert_geometry_term(slot, 0, "(")?;
    /// model.insert_geometry_term(slot, n + 1, ")")?;
    /// model.insert_geometry_term(slot, n + 2, "#123")?;
    /// ```
    ///
    /// `n` is read once, before any insert, since each insert grows the term
    /// list by the one term it adds.
    pub fn insert_geometry_term(
        &mut self,
        slot: u32,
        position: usize,
        text: &str,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let spans = cell::walk_geometry_spans(card, &l.geometry);
        if position > spans.len() {
            return Err(EditError::NoSuchField);
        }
        let toks = card.tokens();
        let text = text.trim();
        let (at, insert) = if position < spans.len() {
            let start = toks[spans[position].1.start].start as usize;
            (start, format!("{text} "))
        } else if let Some((_, last_span)) = spans.last() {
            let end = toks[last_span.end - 1].end() as usize;
            (end, format!(" {text}"))
        } else {
            empty_geometry_anchor(card, &l, text)
        };
        self.cst.card_mut(slot).unwrap().splice(at..at, &insert);
        Ok(())
    }

    /// Replace a cell's entire geometry expression with new literal text — the
    /// bulk counterpart to `set_geometry_term`/`insert_geometry_term`, for
    /// rebuilding a geometry from scratch rather than editing term by term
    /// (e.g. uniting several cells' geometries into one). Material, density
    /// and trailing keyword parameters are untouched.
    pub fn set_cell_geometry(&mut self, slot: u32, text: &str) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let spans = cell::walk_geometry_spans(card, &l.geometry);
        let toks = card.tokens();
        let text = text.trim();
        let (from, to, insert) = match (spans.first(), spans.last()) {
            (Some((_, first)), Some((_, last))) => {
                let from = toks[first.start].start as usize;
                let to = toks[last.end - 1].end() as usize;
                (from, to, text.to_owned())
            }
            _ => {
                let (at, insert) = empty_geometry_anchor(card, &l, text);
                (at, at, insert)
            }
        };
        self.cst.card_mut(slot).unwrap().splice(from..to, &insert);
        Ok(())
    }

    // --- surfaces -----------------------------------------------------------

    /// Rewrite this surface's own id (a leading `+` white-boundary marker, if
    /// present, survives — it lives inside the id token itself). Local to this
    /// card only — unlike [`Model::renumber_surfaces`], it does **not** move
    /// any geometry reference to this surface elsewhere in the file. Safe on
    /// a surface nothing references yet; used on an already-referenced one,
    /// those references go dangling — [`Model::validate`] will report them.
    pub fn set_surface_id(&mut self, slot: u32, new_id: i64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let id_tok = l.id_tok.ok_or(EditError::NoSuchField)?;
        let new_text =
            remap_token(card.token_text(id_tok), &|_| new_id).ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(id_tok, &new_text);
        if let Some(old) = l.id {
            if self.surface_index.get(&old) == Some(&slot) {
                self.surface_index.remove(&old);
            }
        }
        self.surface_index.entry(new_id).or_insert(slot);
        Ok(())
    }

    /// Set (or clear) a surface's leading `*` reflective-boundary marker.
    /// Setting `true` also clears any `+` white-boundary marker on the id —
    /// the two are mutually exclusive in MCNP; setting `false` only removes
    /// the `*` and leaves the id (and any white marker) untouched.
    pub fn set_surface_reflective(&mut self, slot: u32, value: bool) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let id_tok = l.id_tok.ok_or(EditError::NoSuchField)?;
        let toks = card.tokens();
        let id_end = toks[id_tok].end() as usize;
        let id_text = card.token_text(id_tok).to_owned();
        let from = l
            .star_tok
            .map(|t| toks[t].start as usize)
            .unwrap_or(toks[id_tok].start as usize);

        let replacement = if value {
            let digits = id_text.strip_prefix('+').unwrap_or(&id_text);
            format!("*{digits}")
        } else {
            id_text
        };
        self.cst
            .card_mut(slot)
            .unwrap()
            .splice(from..id_end, &replacement);
        Ok(())
    }

    /// Set (or clear) a surface's leading `+` white-boundary marker (it lives
    /// inside the id token itself). Setting `true` also clears any `*`
    /// reflective marker — the two are mutually exclusive in MCNP; setting
    /// `false` only removes the `+` and leaves any `*` marker untouched.
    pub fn set_surface_white(&mut self, slot: u32, value: bool) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let id_tok = l.id_tok.ok_or(EditError::NoSuchField)?;
        let toks = card.tokens();
        let id_start = toks[id_tok].start as usize;
        let id_end = toks[id_tok].end() as usize;
        let id_text = card.token_text(id_tok).to_owned();
        let digits = id_text.strip_prefix('+').unwrap_or(&id_text).to_owned();

        let (from, replacement) = if value {
            let from = l
                .star_tok
                .map(|t| toks[t].start as usize)
                .unwrap_or(id_start);
            (from, format!("+{digits}"))
        } else {
            (id_start, digits)
        };
        self.cst
            .card_mut(slot)
            .unwrap()
            .splice(from..id_end, &replacement);
        Ok(())
    }

    /// Replace one coefficient of a surface, addressed by position.
    pub fn set_surface_coeff(
        &mut self,
        slot: u32,
        index: usize,
        value: f64,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let tok = *surface::coeff_tokens(card, &l)
            .get(index)
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(tok, &fmt_num(value));
        Ok(())
    }

    /// Replace a surface's entire coefficient list. When the count is unchanged
    /// this rewrites the tokens in place (each keeps its own spacing); when it
    /// changes, the whole coefficient span is respliced. A trailing inline
    /// comment after the coefficients is preserved.
    pub fn set_surface_coeffs(&mut self, slot: u32, values: &[f64]) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let toks = surface::coeff_tokens(card, &l);
        if toks.is_empty() {
            return Err(EditError::NoSuchField);
        }

        if toks.len() == values.len() {
            let strings: Vec<String> = values.iter().map(|v| fmt_num(*v)).collect();
            let edits: Vec<(usize, &str)> = toks
                .iter()
                .zip(&strings)
                .map(|(&t, s)| (t, s.as_str()))
                .collect();
            self.cst.card_mut(slot).unwrap().rewrite_tokens(&edits);
        } else {
            let from = card.tokens()[toks[0]].start as usize;
            let to = card.tokens()[*toks.last().unwrap()].end() as usize;
            self.cst
                .card_mut(slot)
                .unwrap()
                .splice(from..to, &join_nums(values));
        }
        Ok(())
    }

    /// Set (or clear) a surface's transform number. `Some(n)` sets it, inserting
    /// the field before the mnemonic if the surface had none; `None` removes it.
    /// A negative `n` marks a periodic surface and is written through verbatim.
    pub fn set_surface_transform(
        &mut self,
        slot: u32,
        transform: Option<i64>,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Surface {
            return Err(EditError::WrongKind);
        }
        let l = surface::layout(card);
        let toks = card.tokens();
        let mnemonic_start = l.mnemonic_tok.map(|m| toks[m].start as usize);
        let transform_start = l.transform_tok.map(|t| toks[t].start as usize);
        let target = self.cst.card_mut(slot).unwrap();

        match (transform, l.transform_tok) {
            (Some(n), Some(tok)) => target.set_token_text(tok, &n.to_string()),
            (Some(n), None) => {
                let at = mnemonic_start.ok_or(EditError::NoSuchField)?;
                target.splice(at..at, &format!("{n} "));
            }
            (None, Some(_)) => {
                // Drop the transform token and the whitespace up to the mnemonic.
                let start = transform_start.unwrap();
                let end = mnemonic_start.unwrap_or(start);
                target.splice(start..end, "");
            }
            (None, None) => {}
        }
        Ok(())
    }

    // --- materials ----------------------------------------------------------

    /// Rewrite this material's own id (`m1` -> `m501`; the alphabetic
    /// mnemonic's case is kept exactly as written). Local to this card only —
    /// unlike [`Model::renumber_materials`], it does **not** move any cell
    /// `material=` field or `MTn`/`MXn` card that references it. Safe on a
    /// material nothing references yet; used on an already-referenced one,
    /// those references go dangling — [`Model::validate`] will report them.
    pub fn set_material_id(&mut self, slot: u32, new_id: i64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let head = data::head(card).ok_or(EditError::WrongKind)?;
        let old_id = data::material_id(&head).ok_or(EditError::WrongKind)?;
        let new_text = remap_name(card.token_text(head.name_tok), &|_| new_id)
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(head.name_tok, &new_text);
        if self.material_index.get(&old_id) == Some(&slot) {
            self.material_index.remove(&old_id);
        }
        self.material_index.entry(new_id).or_insert(slot);
        Ok(())
    }

    /// Set the fraction of one material entry, addressed by position. A negative
    /// fraction is by weight; the sign is written exactly as given.
    pub fn set_material_fraction(
        &mut self,
        slot: u32,
        entry: usize,
        value: f64,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let head = data::head(card).ok_or(EditError::WrongKind)?;
        if data::material_id(&head).is_none() {
            return Err(EditError::WrongKind);
        }
        let (_, frac_tok) = *data::material_entry_tokens(card, &head)
            .get(entry)
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(frac_tok, &fmt_num(value));
        Ok(())
    }

    /// Set the ZAID of one material entry, addressed by position. The library
    /// suffix is part of the string the caller supplies (`"1001.31c"`).
    pub fn set_material_zaid(
        &mut self,
        slot: u32,
        entry: usize,
        zaid: &str,
    ) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let head = data::head(card).ok_or(EditError::WrongKind)?;
        if data::material_id(&head).is_none() {
            return Err(EditError::WrongKind);
        }
        let (zaid_tok, _) = *data::material_entry_tokens(card, &head)
            .get(entry)
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(zaid_tok, zaid);
        Ok(())
    }

    // --- transforms ---------------------------------------------------------

    /// Rewrite this transform's own id (`tr1` -> `tr501`; a leading `*` and
    /// the mnemonic's case are kept exactly as written). Local to this card
    /// only — unlike [`Model::renumber_transforms`], it does **not** move any
    /// surface `transform=` field that references it. Safe on a transform
    /// nothing references yet; used on an already-referenced one, those
    /// references go dangling — [`Model::validate`] will report them.
    pub fn set_transform_id(&mut self, slot: u32, new_id: i64) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let head = data::head(card).ok_or(EditError::WrongKind)?;
        let old_id = data::transform_id(&head).ok_or(EditError::WrongKind)?;
        let new_text = remap_name(card.token_text(head.name_tok), &|_| new_id)
            .ok_or(EditError::NoSuchField)?;
        self.cst
            .card_mut(slot)
            .unwrap()
            .set_token_text(head.name_tok, &new_text);
        if self.transform_index.get(&old_id) == Some(&slot) {
            self.transform_index.remove(&old_id);
        }
        self.transform_index.entry(new_id).or_insert(slot);
        Ok(())
    }

    /// Replace a transform's entire coefficient list. Same in-place vs. resplice
    /// strategy as [`Model::set_surface_coeffs`].
    pub fn set_transform_coeffs(&mut self, slot: u32, values: &[f64]) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let head = data::head(card).ok_or(EditError::WrongKind)?;
        if data::transform_id(&head).is_none() {
            return Err(EditError::WrongKind);
        }
        let Some((first, last)) = data::values_span(card, head.values_start) else {
            return Err(EditError::NoSuchField);
        };
        // Count the existing coefficients so an unchanged count edits in place.
        let existing = data::values(card, head.values_start).len();
        if existing == values.len() {
            let mut i = first;
            let mut edits: Vec<(usize, String)> = Vec::with_capacity(values.len());
            let mut vi = 0;
            let toks = card.tokens();
            while i <= last && vi < values.len() {
                if !toks[i].is_trivia() {
                    edits.push((i, fmt_num(values[vi])));
                    vi += 1;
                }
                i += 1;
            }
            let refs: Vec<(usize, &str)> = edits.iter().map(|(t, s)| (*t, s.as_str())).collect();
            self.cst.card_mut(slot).unwrap().rewrite_tokens(&refs);
        } else {
            let from = card.tokens()[first].start as usize;
            let to = card.tokens()[last].end() as usize;
            self.cst
                .card_mut(slot)
                .unwrap()
                .splice(from..to, &join_nums(values));
        }
        Ok(())
    }

    // --- title ---------------------------------------------------------------

    /// Where a synthesized title card belongs when none exists: position 0,
    /// or — if the model opens with a MESSAGE block — just after it. A
    /// MESSAGE block is zero-or-more `Message` cards (one per line unless
    /// continued, see segment.rs) followed by exactly one `Blank` card. A
    /// fixed-index guess ("Message ⇒ insert at 2") breaks on a multi-line
    /// message block.
    fn title_insert_pos(&self) -> usize {
        let mut i = 0;
        while self.cst.at(i).map(|c| c.kind()) == Some(CardKind::Message) {
            i += 1;
        }
        if i > 0 && self.cst.at(i).map(|c| c.kind()) == Some(CardKind::Blank) {
            i += 1;
        }
        i
    }

    /// Replace the model's title line, creating one if none exists. Title
    /// cards are lexically opaque (zero tokens — see lex.rs), so `splice`
    /// over the whole byte range is the only edit primitive that applies.
    pub fn set_title(&mut self, title: &str) {
        let text = self.terminate(title);
        let existing = self
            .cst
            .cards()
            .find(|c| c.kind() == CardKind::Title)
            .map(|c| c.slot());
        match existing {
            Some(slot) => {
                let len = self.cst.card(slot).unwrap().len_bytes();
                self.cst.card_mut(slot).unwrap().splice(0..len, &text);
            }
            None => {
                let at = self.title_insert_pos();
                self.cst.insert_at(at, Cst::new_card(CardKind::Title, text));
            }
        }
    }

    // --- in-card structural edits on a cell ---------------------------------

    /// Append a keyword parameter (`"imp:n=1"`, `"vol=3"`) to a cell, placing it
    /// after the last significant token and therefore before any trailing `$`
    /// comment. Whitespace and the comment are preserved.
    pub fn add_cell_param(&mut self, slot: u32, text: &str) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let pos = last_significant_end(card).unwrap_or_else(|| content_end(card.text()));
        let insert = format!(" {}", text.trim());
        self.cst.card_mut(slot).unwrap().splice(pos..pos, &insert);
        Ok(())
    }

    /// Remove a cell's keyword parameter by qualified key. Returns `false` if the
    /// cell has no such parameter. Drops the keyword, its value, and one leading
    /// separator space.
    pub fn remove_cell_param(&mut self, slot: u32, key: &str) -> Result<bool, EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        let l = cell::layout(card);
        let params = cell::params(card, &l.params);
        let Some(p) = params
            .iter()
            .find(|p| p.qualified_key().eq_ignore_ascii_case(key))
        else {
            return Ok(false);
        };
        let toks = card.tokens();
        let mut start = toks[p.key_token].start as usize;
        let end = if p.value_tokens.end > p.value_tokens.start {
            toks[p.value_tokens.end - 1].end() as usize
        } else {
            toks[p.key_token].end() as usize
        };
        if start > 0 && card.text().as_bytes()[start - 1] == b' ' {
            start -= 1;
        }
        self.cst.card_mut(slot).unwrap().splice(start..end, "");
        Ok(true)
    }

    /// Append an inline `$` comment to any card, before its line terminator. A
    /// `$` is prepended if the text does not already start with one.
    pub fn append_inline_comment(&mut self, slot: u32, text: &str) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        let t = text.trim();
        let body = if t.starts_with('$') {
            t.to_owned()
        } else {
            format!("$ {t}")
        };
        let pos = content_end(card.text());
        self.cst
            .card_mut(slot)
            .unwrap()
            .splice(pos..pos, &format!(" {body}"));
        Ok(())
    }

    /// Append an inline `$` comment to a cell specifically. See
    /// [`Model::append_inline_comment`] for the card-kind-agnostic version.
    pub fn append_cell_comment(&mut self, slot: u32, text: &str) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Cell {
            return Err(EditError::WrongKind);
        }
        self.append_inline_comment(slot, text)
    }

    // --- structural edits (add / remove whole cards) ------------------------

    /// Give a snippet the file's line terminator, normalising any the caller
    /// supplied. An added card is a proper line, and — crucially — an add of a
    /// snippet followed by its removal restores the source byte-for-byte, because
    /// the inserted card and its terminator vanish together.
    fn terminate(&self, text: &str) -> String {
        let body = text
            .strip_suffix('\n')
            .map(|s| s.strip_suffix('\r').unwrap_or(s))
            .unwrap_or(text);
        let mut out = String::with_capacity(body.len() + 2);
        out.push_str(body);
        out.push_str(self.cst.eol().as_str());
        out
    }

    /// Lex `text` as one card of `kind` and insert it at the end of that block.
    /// Pure `Vec<u32>` insert — no other card is touched and nothing is re-lexed.
    /// Lex `text` as one card of `kind` and insert it at raw file position `at`.
    fn insert_card(&mut self, kind: CardKind, at: usize, text: &str) -> u32 {
        let card = Cst::new_card(kind, self.terminate(text));
        self.cst.insert_at(at, card)
    }

    fn add_card(&mut self, kind: CardKind, text: &str) -> Result<u32, EditError> {
        let at = self
            .cst
            .end_of_block(kind)
            .or_else(|| self.empty_block_insert_pos(kind))
            .ok_or(EditError::NoBlock)?;
        Ok(self.insert_card(kind, at, text))
    }

    /// Position right after the title (and any leading `MESSAGE:` block) —
    /// where the cell block begins, empty or not.
    fn content_start_pos(&self) -> usize {
        let mut i = self.title_insert_pos();
        if self.cst.at(i).map(|c| c.kind()) == Some(CardKind::Title) {
            i += 1;
        }
        i
    }

    /// Where a new card of `kind` belongs when its block is currently empty —
    /// `end_of_block` can't find this, since it locates a block by its last
    /// existing card and an empty block has none.
    ///
    /// `remove_data_card`/`remove_cell`/`remove_surface` (like every removal
    /// in this module) only ever delete the one card asked for — they never
    /// touch the `Blank` cards that delimit the cell/surface/data blocks, so
    /// those delimiters are still there even once a block's last card is
    /// gone. This walks past exactly the number of delimiters that precede
    /// `kind` (cell has none before it, surface has one — cell|surface — data
    /// has two — cell|surface and surface|data), landing right in the gap
    /// where that now-empty block's content used to live.
    fn empty_block_insert_pos(&self, kind: CardKind) -> Option<usize> {
        let blanks_before = match kind {
            CardKind::Cell => 0,
            CardKind::Surface => 1,
            CardKind::Data => 2,
            _ => return None,
        };
        let mut pos = self.content_start_pos();
        let mut seen = 0;
        while seen < blanks_before {
            if self.cst.at(pos)?.kind() == CardKind::Blank {
                seen += 1;
            }
            pos += 1;
        }
        Some(pos)
    }

    /// Add a cell at the end of the cell block. Its id is indexed immediately, so
    /// the very next `cell(id)` finds it without any flush step.
    pub fn add_cell(&mut self, text: &str) -> Result<u32, EditError> {
        let slot = self.add_card(CardKind::Cell, text)?;
        if let Some(id) = self.cst.card(slot).map(cell::layout).and_then(|l| l.id) {
            self.cell_index.entry(id).or_insert(slot);
        }
        Ok(slot)
    }

    /// Add a cell immediately after an existing one (by id), instead of at the
    /// end of the cell block — e.g. so a clone lands next to the cell it came
    /// from rather than wherever the block currently ends. Its id is indexed
    /// immediately, same as `add_cell`. `EditError::NoSuchField` if `after` is
    /// not a currently-defined cell id.
    pub fn add_cell_after(&mut self, after: i64, text: &str) -> Result<u32, EditError> {
        let anchor_slot = self
            .cell_index
            .get(&after)
            .copied()
            .ok_or(EditError::NoSuchField)?;
        let at = self
            .cst
            .position_of(anchor_slot)
            .ok_or(EditError::NoSuchField)?;
        let slot = self.insert_card(CardKind::Cell, at + 1, text);
        if let Some(id) = self.cst.card(slot).map(cell::layout).and_then(|l| l.id) {
            self.cell_index.entry(id).or_insert(slot);
        }
        Ok(slot)
    }

    /// Add a surface at the end of the surface block.
    pub fn add_surface(&mut self, text: &str) -> Result<u32, EditError> {
        let slot = self.add_card(CardKind::Surface, text)?;
        if let Some(id) = self.cst.card(slot).map(surface::layout).and_then(|l| l.id) {
            self.surface_index.entry(id).or_insert(slot);
        }
        Ok(slot)
    }

    /// Add a material (`Mn`) card at the end of the data block.
    pub fn add_material(&mut self, text: &str) -> Result<u32, EditError> {
        let slot = self.add_card(CardKind::Data, text)?;
        if let Some(id) = self
            .cst
            .card(slot)
            .and_then(data::head)
            .and_then(|h| data::material_id(&h))
        {
            self.material_index.entry(id).or_insert(slot);
        }
        Ok(slot)
    }

    /// Add a transform (`TRn` / `*TRn`) card at the end of the data block.
    pub fn add_transform(&mut self, text: &str) -> Result<u32, EditError> {
        let slot = self.add_card(CardKind::Data, text)?;
        if let Some(id) = self
            .cst
            .card(slot)
            .and_then(data::head)
            .and_then(|h| data::transform_id(&h))
        {
            self.transform_index.entry(id).or_insert(slot);
        }
        Ok(slot)
    }

    // --- generic data cards ---------------------------------------------------

    /// Add a generic data card at the end of the data block. Cards with a
    /// dedicated typed constructor — `Mn` material, `TRn` transform — should
    /// normally go through `add_material`/`add_transform` instead, so their id
    /// is indexed immediately; this is for everything else (`sdef`, `mode`,
    /// `kcode`, `print`, `fmesh`, ...) that has no id to index at all.
    pub fn add_data_card(&mut self, text: &str) -> Result<u32, EditError> {
        self.add_card(CardKind::Data, text)
    }

    /// Replace a data card's entire text in place, addressed by slot (a
    /// `DataCard` handle's own slot — most data cards have no id to address
    /// by). Caveat: if `slot` happens to be a material or transform card and
    /// `text` changes its id, the model's id index is *not* updated to match
    /// — use a typed edit, or remove-and-re-add, for that.
    pub fn set_data_card_text(&mut self, slot: u32, text: &str) -> Result<(), EditError> {
        let card = self.cst.card(slot).ok_or(EditError::WrongKind)?;
        if card.kind() != CardKind::Data {
            return Err(EditError::WrongKind);
        }
        let len = card.len_bytes();
        let text = self.terminate(text);
        self.cst.card_mut(slot).unwrap().splice(0..len, &text);
        Ok(())
    }

    /// Remove a data card by slot. This is the slot-addressed counterpart to
    /// `remove_material`/`remove_transform` (which are id-addressed) — most
    /// data cards have no id, so a `DataCard`'s own slot is its only handle.
    /// If the slot happens to be an indexed material or transform, that id's
    /// index entry is cleaned up too (guarded against the duplicate-id
    /// "first wins" case, same as [`remove_by_id`]), so removing through the
    /// generic handle behaves identically to removing through the typed one
    /// — no dangling index entry left pointing at a tombstoned slot.
    pub fn remove_data_card(&mut self, slot: u32) -> bool {
        let Some(card) = self.cst.card(slot) else {
            return false;
        };
        if card.kind() != CardKind::Data {
            return false;
        }
        let head = data::head(card);
        if let Some(id) = head.as_ref().and_then(data::material_id) {
            if self.material_index.get(&id) == Some(&slot) {
                self.material_index.remove(&id);
            }
        }
        if let Some(id) = head.as_ref().and_then(data::transform_id) {
            if self.transform_index.get(&id) == Some(&slot) {
                self.transform_index.remove(&id);
            }
        }
        self.cst.remove_slot(slot)
    }

    /// Remove a cell by id. Returns `false` if no such cell is indexed. The slot
    /// is tombstoned, so any live handle to it fails cleanly rather than silently
    /// reading a different card.
    pub fn remove_cell(&mut self, id: i64) -> bool {
        remove_by_id(&mut self.cst, &mut self.cell_index, id)
    }

    pub fn remove_surface(&mut self, id: i64) -> bool {
        remove_by_id(&mut self.cst, &mut self.surface_index, id)
    }

    pub fn remove_material(&mut self, id: i64) -> bool {
        remove_by_id(&mut self.cst, &mut self.material_index, id)
    }

    pub fn remove_transform(&mut self, id: i64) -> bool {
        remove_by_id(&mut self.cst, &mut self.transform_index, id)
    }
}

/// Remove the card an id resolves to and drop the index entry, but only the entry
/// that actually points at the removed slot — a duplicate id defined elsewhere
/// keeps its (separate) mapping.
fn remove_by_id(cst: &mut Cst, index: &mut crate::model::IdIndex, id: i64) -> bool {
    let Some(&slot) = index.get(&id) else {
        return false;
    };
    let removed = cst.remove_slot(slot);
    if removed {
        index.remove(&id);
    }
    removed
}

#[cfg(test)]
mod tests {
    use crate::Model;

    fn slot_of_cell(m: &Model, id: i64) -> u32 {
        m.cell(id).unwrap().slot()
    }

    #[test]
    fn set_material_crosses_void_boundary_both_ways() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 2);

        // void -> real: a density field appears, geometry and params intact.
        m.set_cell_material(s, 5).unwrap();
        assert_eq!(m.cell(2).unwrap().material(), Some(5));
        assert!(!m.cell(2).unwrap().is_void());
        assert!(m.to_source().contains("2 5 0 1 imp:n=0"));

        // real -> void: the density field is dropped again.
        m.set_cell_material(s, 0).unwrap();
        assert!(m.cell(2).unwrap().is_void());
        assert_eq!(m.cell(2).unwrap().density(), None);
        assert!(m.to_source().contains("2 0 1 imp:n=0"));
    }

    #[test]
    fn set_material_real_to_real_keeps_density() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_material(s, 7).unwrap();
        assert_eq!(m.cell(1).unwrap().material(), Some(7));
        assert_eq!(m.cell(1).unwrap().density(), Some(-1.0));
        assert!(m.to_source().contains("1 7 -1.0 -1 imp:n=1"));
    }

    #[test]
    fn set_density_replaces_only_the_density_field() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_density(s, -10.5).unwrap();
        assert_eq!(m.cell(1).unwrap().density(), Some(-10.5));
        assert!(m.to_source().contains("1 1 -10.5 -1 imp:n=1"));
        // void cell has no density to set
        let void = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let mut void = void;
        let vs = slot_of_cell(&void, 1);
        assert!(void.set_cell_density(vs, 1.0).is_err());
    }

    #[test]
    fn set_param_preserves_the_rest_of_the_card() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1 vol=3 $ keep\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        assert_eq!(m.set_cell_param(s, "imp:n", "2"), Ok(true));
        let out = m.to_source();
        assert!(out.contains("imp:n=2"), "{out}");
        assert!(out.contains("vol=3"), "{out}");
        assert!(out.contains("$ keep"), "{out}");
        // an absent parameter is reported, not invented
        assert_eq!(m.set_cell_param(s, "tmp", "300"), Ok(false));
    }

    #[test]
    fn read_after_write_sees_the_write() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_material(s, 3).unwrap();
        // No flush/materialize: the very next read reflects the edit.
        assert_eq!(m.cell(1).unwrap().material(), Some(3));
    }

    #[test]
    fn set_surface_coeff_and_coeffs() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 RPP -1 1 -2 2 -3 3\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_coeff(s, 0, -5.0).unwrap();
        assert_eq!(m.surface(1).unwrap().coeffs()[0], -5.0);
        assert!(m.to_source().contains("RPP -5 1 -2 2 -3 3"));

        // same-count replacement keeps the layout
        m.set_surface_coeffs(s, &[-1.0, 1.0, -2.0, 2.0, -3.0, 3.0])
            .unwrap();
        assert!(m.to_source().contains("RPP -1 1 -2 2 -3 3"));

        // different count reshapes the coefficient list
        m.set_surface_coeffs(s, &[9.0]).unwrap();
        assert_eq!(m.surface(1).unwrap().coeffs(), vec![9.0]);
    }

    #[test]
    fn set_surface_transform_set_insert_remove() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        // insert
        m.set_surface_transform(s, Some(4)).unwrap();
        assert_eq!(m.surface(1).unwrap().transform(), Some(4));
        assert!(m.to_source().contains("1 4 SO 5"));
        // replace
        m.set_surface_transform(s, Some(-4)).unwrap();
        assert_eq!(m.surface(1).unwrap().transform(), Some(-4));
        // remove
        m.set_surface_transform(s, None).unwrap();
        assert_eq!(m.surface(1).unwrap().transform(), None);
        assert!(m.to_source().contains("1 SO 5"));
    }

    #[test]
    fn set_surface_reflective_inserts_and_removes_the_star() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_reflective(s, true).unwrap();
        assert!(m.surface(1).unwrap().reflective());
        assert!(m.to_source().contains("*1 SO 5"));
        m.set_surface_reflective(s, false).unwrap();
        assert!(!m.surface(1).unwrap().reflective());
        assert!(m.to_source().contains("\n1 SO 5"));
    }

    #[test]
    fn set_surface_reflective_clears_a_white_marker() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n+1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_reflective(s, true).unwrap();
        assert!(m.surface(1).unwrap().reflective());
        assert!(!m.surface(1).unwrap().white());
        assert!(m.to_source().contains("*1 SO 5"));
    }

    #[test]
    fn set_surface_white_inserts_and_removes_the_prefix() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_white(s, true).unwrap();
        assert!(m.surface(1).unwrap().white());
        assert!(m.to_source().contains("+1 SO 5"));
        m.set_surface_white(s, false).unwrap();
        assert!(!m.surface(1).unwrap().white());
        assert!(m.to_source().contains("\n1 SO 5"));
    }

    #[test]
    fn set_surface_white_clears_a_reflective_marker() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n*1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_white(s, true).unwrap();
        assert!(m.surface(1).unwrap().white());
        assert!(!m.surface(1).unwrap().reflective());
        assert!(m.to_source().contains("+1 SO 5"));
    }

    #[test]
    fn set_material_entry_fields() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001.31c 1 8016 2\n");
        let s = m.material(1).unwrap().slot();
        m.set_material_fraction(s, 1, -0.5).unwrap();
        m.set_material_zaid(s, 0, "1002.31c").unwrap();
        let entries = m.material(1).unwrap().entries();
        assert_eq!(entries[0].0, "1002.31c");
        assert_eq!(entries[1].1, -0.5);
        assert!(m.to_source().contains("m1 1002.31c 1 8016 -0.5"));
    }

    #[test]
    fn set_transform_coeffs_replaces_the_list() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n");
        let s = m.transform(1).unwrap().slot();
        m.set_transform_coeffs(s, &[1.0, 2.0, 3.0]).unwrap();
        assert_eq!(m.transform(1).unwrap().coeffs(), vec![1.0, 2.0, 3.0]);
        assert!(m.to_source().contains("tr1 1 2 3"));
    }

    #[test]
    fn wrong_kind_is_rejected() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let surf = m.surface(1).unwrap().slot();
        assert_eq!(
            m.set_cell_material(surf, 2),
            Err(super::EditError::WrongKind)
        );
    }

    // --- id setters ----------------------------------------------------------

    #[test]
    fn set_cell_id_rewrites_the_token_and_the_index() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_id(s, 501).unwrap();
        assert!(m.cell(1).is_none());
        assert_eq!(m.cell(501).unwrap().slot(), s);
        assert!(m.to_source().contains("501 1 -1.0 -1 imp:n=1"));
    }

    #[test]
    fn set_cell_id_does_not_clobber_a_first_wins_duplicate() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let original = slot_of_cell(&m, 1);
        let dup = m.add_cell("1 0 -1 imp:n=1").unwrap();
        assert_eq!(m.cell(1).unwrap().slot(), original); // first wins
        m.set_cell_id(dup, 501).unwrap();
        // the original's index entry for id 1 must be untouched
        assert_eq!(m.cell(1).unwrap().slot(), original);
        assert_eq!(m.cell(501).unwrap().slot(), dup);
    }

    #[test]
    fn set_cell_id_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let surf = m.surface(1).unwrap().slot();
        assert_eq!(m.set_cell_id(surf, 501), Err(super::EditError::WrongKind));
    }

    #[test]
    fn set_cell_id_supports_the_clone_workflow() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1 vol=3\n\n1 SO 5\n\nm1 1001 1\n");
        let source_text = m.cell(1).unwrap().text().to_owned();
        let clone_slot = m.add_cell(&source_text).unwrap();
        m.set_cell_id(clone_slot, 501).unwrap();

        // the original is untouched
        assert!(m.cell(1).is_some());
        assert!(m.to_source().contains("1 1 -1.0 -1 imp:n=1 vol=3"));
        // the clone is independently addressable and editable
        assert_eq!(m.cell(501).unwrap().material(), Some(1));
        assert_eq!(m.cell(501).unwrap().density(), Some(-1.0));
        m.set_cell_geometry(clone_slot, "-2").unwrap();
        assert_eq!(m.cell(501).unwrap().signed_surfaces(), vec![-2]);
        assert_eq!(m.cell(1).unwrap().signed_surfaces(), vec![-1]); // original unaffected
    }

    #[test]
    fn set_cell_id_leaves_existing_references_dangling() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n2 0 1 #1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_id(s, 501).unwrap();
        // the complement still reads the old id — this is not a rename
        assert_eq!(m.cell(2).unwrap().cell_refs(), vec![1]);
        assert!(m.cell(1).is_none());
        assert!(!m.validate().is_empty());
    }

    #[test]
    fn set_surface_id_rewrites_the_token_and_the_index() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_id(s, 501).unwrap();
        assert!(m.surface(1).is_none());
        assert_eq!(m.surface(501).unwrap().slot(), s);
        assert!(m.to_source().contains("501 SO 5"));
    }

    #[test]
    fn set_surface_id_preserves_a_leading_white_prefix() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n+1 SO 5\n\nm1 1001 1\n");
        let s = m.surface(1).unwrap().slot();
        m.set_surface_id(s, 501).unwrap();
        assert!(m.surface(501).unwrap().white());
        assert!(m.to_source().contains("+501 SO 5"));
    }

    #[test]
    fn set_surface_id_does_not_clobber_a_first_wins_duplicate() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let original = m.surface(1).unwrap().slot();
        let dup = m.add_surface("1 PX 3").unwrap();
        m.set_surface_id(dup, 501).unwrap();
        assert_eq!(m.surface(1).unwrap().slot(), original);
        assert_eq!(m.surface(501).unwrap().slot(), dup);
    }

    #[test]
    fn set_surface_id_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let s = slot_of_cell(&m, 1);
        assert_eq!(m.set_surface_id(s, 501), Err(super::EditError::WrongKind));
    }

    #[test]
    fn set_material_id_rewrites_the_name_token_and_the_index() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nM1 1001 1\n");
        let s = m.material(1).unwrap().slot();
        m.set_material_id(s, 501).unwrap();
        assert!(m.material(1).is_none());
        assert_eq!(m.material(501).unwrap().slot(), s);
        // the mnemonic's original case is preserved
        assert!(m.to_source().contains("M501 1001 1"));
    }

    #[test]
    fn set_material_id_does_not_clobber_a_first_wins_duplicate() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let original = m.material(1).unwrap().slot();
        let dup = m.add_material("m1 8016 1").unwrap();
        m.set_material_id(dup, 501).unwrap();
        assert_eq!(m.material(1).unwrap().slot(), original);
        assert_eq!(m.material(501).unwrap().slot(), dup);
    }

    #[test]
    fn set_material_id_leaves_a_referencing_cell_dangling() {
        let mut m = Model::parse("t\n1 1 -1.0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = m.material(1).unwrap().slot();
        m.set_material_id(s, 501).unwrap();
        // cell 1's material= field still reads the old id — this is not a rename
        assert_eq!(m.cell(1).unwrap().material(), Some(1));
        assert!(!m.validate().is_empty());
    }

    #[test]
    fn set_material_id_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let s = slot_of_cell(&m, 1);
        assert_eq!(m.set_material_id(s, 501), Err(super::EditError::WrongKind));
    }

    #[test]
    fn set_transform_id_rewrites_the_name_token_and_the_index() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n*tr1 0 0 5\n");
        let s = m.transform(1).unwrap().slot();
        m.set_transform_id(s, 501).unwrap();
        assert!(m.transform(1).is_none());
        assert_eq!(m.transform(501).unwrap().slot(), s);
        // the leading `*` and the mnemonic's case are preserved
        assert!(m.to_source().contains("*tr501 0 0 5"));
    }

    #[test]
    fn set_transform_id_does_not_clobber_a_first_wins_duplicate() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\ntr1 0 0 5\n");
        let original = m.transform(1).unwrap().slot();
        let dup = m.add_transform("tr1 0 0 9").unwrap();
        m.set_transform_id(dup, 501).unwrap();
        assert_eq!(m.transform(1).unwrap().slot(), original);
        assert_eq!(m.transform(501).unwrap().slot(), dup);
    }

    #[test]
    fn set_transform_id_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let s = slot_of_cell(&m, 1);
        assert_eq!(m.set_transform_id(s, 501), Err(super::EditError::WrongKind));
    }

    // --- M3: structural edits ----------------------------------------------

    const SRC: &str = "t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n";

    #[test]
    fn add_cell_appends_and_is_visible() {
        let mut m = Model::parse(SRC);
        let before = m.num_cells();
        m.add_cell("3 1 -1.0 -1 imp:n=1").unwrap();
        assert_eq!(m.num_cells(), before + 1);
        assert_eq!(m.cell(3).unwrap().id(), Some(3));
        assert!(m.to_source().contains("3 1 -1.0 -1 imp:n=1"));
        // it landed inside the cell block, before the blank delimiter
        assert!(m
            .to_source()
            .contains("2 0 1 imp:n=0\n3 1 -1.0 -1 imp:n=1\n\n1 SO 5"));
    }

    #[test]
    fn add_cell_after_lands_right_after_the_anchor() {
        let mut m = Model::parse(SRC);
        m.add_cell_after(1, "3 0 -2 imp:n=1").unwrap();
        // between cell 1 and cell 2 in source order, not at the block's end
        assert!(
            m.to_source()
                .contains("1 1 -1.0 -1 imp:n=1\n3 0 -2 imp:n=1\n2 0 1 imp:n=0"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn add_cell_after_is_indexed_immediately() {
        let mut m = Model::parse(SRC);
        m.add_cell_after(1, "3 0 -2 imp:n=1").unwrap();
        assert_eq!(m.cell(3).unwrap().id(), Some(3));
        assert_eq!(m.num_cells(), 3);
    }

    #[test]
    fn add_cell_after_missing_anchor_is_rejected() {
        let mut m = Model::parse(SRC);
        assert_eq!(
            m.add_cell_after(999, "3 0 -2 imp:n=1"),
            Err(super::EditError::NoSuchField)
        );
    }

    #[test]
    fn add_cell_after_supports_placing_a_clone_next_to_its_source() {
        let mut m = Model::parse(
            "t\n1 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0\n3 0 1 imp:n=0\n\n1 SO 5\n\nm1 1001 1\n",
        );
        let source_text = m.cell(1).unwrap().text().to_owned();
        let clone_slot = m.add_cell_after(1, &source_text).unwrap();
        m.set_cell_id(clone_slot, 501).unwrap();
        // the clone sits between cell 1 and cell 2, not after cell 3
        assert!(
            m.to_source()
                .contains("1 1 -1.0 -1 imp:n=1\n501 1 -1.0 -1 imp:n=1\n2 0 1 imp:n=0"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn remove_cell_by_id() {
        let mut m = Model::parse(SRC);
        assert!(m.remove_cell(2));
        assert!(m.cell(2).is_none());
        assert_eq!(m.num_cells(), 1);
        // an absent id is a no-op, reported as false
        assert!(!m.remove_cell(999));
    }

    #[test]
    fn add_then_remove_is_byte_identity() {
        let mut m = Model::parse(SRC);
        let src = m.to_source();
        m.add_cell("42 0 -1 imp:n=1").unwrap();
        assert!(m.remove_cell(42));
        assert_eq!(m.to_source(), src);
    }

    #[test]
    fn interleaved_edits_and_reads_stay_consistent() {
        let mut m = Model::parse(SRC);
        m.add_cell("3 0 -1 imp:n=1").unwrap();
        assert!(m.cell(3).unwrap().is_void());
        // read-modify-write across the model, then read again — no explicit flush
        let s = m.cell(3).unwrap().slot();
        m.set_cell_material(s, 1).unwrap();
        assert_eq!(m.cell(3).unwrap().material(), Some(1));
        assert!(m.remove_cell(3));
        assert!(m.cell(3).is_none());
    }

    #[test]
    fn removed_handle_fails_cleanly() {
        let mut m = Model::parse(SRC);
        let view = m.cell(2).unwrap();
        let slot = view.slot();
        assert!(m.remove_cell(2));
        // the tombstoned slot resolves to nothing, rather than a different card
        assert!(m.cst().card(slot).is_none());
    }

    #[test]
    fn add_surface_and_material_index_immediately() {
        let mut m = Model::parse(SRC);
        m.add_surface("2 PX 3").unwrap();
        assert_eq!(m.surface(2).unwrap().kind(), Some("PX"));
        assert!(m.to_source().contains("1 SO 5\n2 PX 3\n\nm1 1001 1"));

        m.add_material("m2 8016 1").unwrap();
        assert_eq!(m.material(2).unwrap().id(), Some(2));
        assert!(m.to_source().contains("m2 8016 1"));
    }

    #[test]
    fn remove_surface_material_transform() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n2 PX 3\n\nm1 1001 1\ntr1 0 0 5\n");
        assert!(m.remove_surface(2));
        assert!(m.surface(2).is_none());
        assert!(m.remove_material(1));
        assert!(m.material(1).is_none());
        assert!(m.remove_transform(1));
        assert!(m.transform(1).is_none());
        assert!(!m.remove_transform(1)); // already gone
    }

    #[test]
    fn add_param_lands_before_a_trailing_comment() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1 $ keep\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.add_cell_param(s, "vol=3").unwrap();
        assert!(
            m.to_source().contains("1 0 -1 imp:n=1 vol=3 $ keep"),
            "{}",
            m.to_source()
        );
        assert_eq!(m.cell(1).unwrap().param("vol").unwrap().value, "3");
    }

    #[test]
    fn add_param_to_a_void_cell_without_params() {
        let mut m = Model::parse("t\n2 0 1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 2);
        m.add_cell_param(s, "imp:n=1").unwrap();
        assert!(m.to_source().contains("2 0 1 imp:n=1"));
    }

    #[test]
    fn remove_param_drops_key_and_value() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1 vol=3 $ keep\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        assert_eq!(m.remove_cell_param(s, "vol"), Ok(true));
        let out = m.to_source();
        assert!(out.contains("1 0 -1 imp:n=1 $ keep"), "{out}");
        assert!(!out.contains("vol=3"), "{out}");
        assert_eq!(m.remove_cell_param(s, "nope"), Ok(false));
    }

    #[test]
    fn append_comment_goes_before_the_newline() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.append_cell_comment(s, "added shim").unwrap();
        assert!(
            m.to_source().contains("1 0 -1 imp:n=1 $ added shim\n"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn append_inline_comment_works_on_any_card_kind() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let surf = m.surface(1).unwrap().slot();
        m.append_inline_comment(surf, "added shim").unwrap();
        assert!(
            m.to_source().contains("1 SO 5 $ added shim\n"),
            "{}",
            m.to_source()
        );

        let mat = m.material(1).unwrap().slot();
        m.append_inline_comment(mat, "$ already prefixed").unwrap();
        assert!(
            m.to_source().contains("m1 1001 1 $ already prefixed\n"),
            "{}",
            m.to_source()
        );
    }

    #[test]
    fn edit_error_has_a_readable_display() {
        assert_eq!(
            super::EditError::WrongKind.to_string(),
            "card is missing or is not the right kind for this edit"
        );
        assert_eq!(
            super::EditError::NoSuchField.to_string(),
            "the addressed field does not exist on this card"
        );
        assert_eq!(
            super::EditError::NoBlock.to_string(),
            "the model has no block of this kind to add the card into"
        );
        let e: Box<dyn std::error::Error> = Box::new(super::EditError::NoBlock);
        assert!(e.to_string().contains("no block"));
    }

    #[test]
    fn add_normalises_a_trailing_newline() {
        let mut m = Model::parse(SRC);
        m.add_cell("7 0 -1 imp:n=1\n").unwrap();
        // exactly one terminator, no blank line introduced
        assert!(m.to_source().contains("7 0 -1 imp:n=1\n\n1 SO 5"));
        assert!(m.remove_cell(7));
        assert_eq!(m.to_source(), SRC);
    }

    // --- title ---------------------------------------------------------------

    #[test]
    fn title_reads_the_positional_title_card() {
        assert_eq!(Model::parse(SRC).title(), Some("t"));
    }

    #[test]
    fn title_is_none_when_absent() {
        assert_eq!(Model::parse("").title(), None);
    }

    #[test]
    fn title_after_leading_message_block() {
        let m =
            Model::parse("MESSAGE: outp=o\n\nMy Title\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        assert_eq!(m.title(), Some("My Title"));
    }

    #[test]
    fn set_title_replaces_existing() {
        let mut m = Model::parse(SRC);
        m.set_title("New Title");
        assert_eq!(m.title(), Some("New Title"));
        assert!(m.to_source().starts_with("New Title\n"));
    }

    #[test]
    fn set_title_creates_at_front_when_absent() {
        let mut m = Model::parse("");
        m.set_title("Fresh");
        assert_eq!(m.title(), Some("Fresh"));
        assert_eq!(m.to_source(), "Fresh\n");
    }

    #[test]
    fn set_title_inserts_after_leading_message_and_blank() {
        let mut m = Model::parse("MESSAGE: outp=o\n\n");
        m.set_title("New Title");
        assert_eq!(m.to_source(), "MESSAGE: outp=o\n\nNew Title\n");
    }

    // Regression for the naive "first card is Message -> fixed index 2" design
    // this rejects: an unmarked, multi-line MESSAGE block segments into one
    // Message card per line (segment.rs), not one card for the whole block.
    #[test]
    fn set_title_inserts_after_a_multiline_leading_message_block() {
        let mut m = Model::parse("MESSAGE: outp=o\nrun continues\n\n");
        m.set_title("New Title");
        assert_eq!(
            m.to_source(),
            "MESSAGE: outp=o\nrun continues\n\nNew Title\n"
        );
    }

    // --- add_transform ---------------------------------------------------------

    #[test]
    fn add_transform_is_indexed_immediately() {
        let mut m = Model::parse(SRC);
        m.add_transform("tr2 0 0 5").unwrap();
        assert_eq!(m.transform(2).unwrap().coeffs(), vec![0.0, 0.0, 5.0]);
        assert!(m.to_source().contains("tr2 0 0 5"));
    }

    #[test]
    fn add_transform_then_remove_is_byte_identity() {
        let mut m = Model::parse(SRC);
        let src = m.to_source();
        m.add_transform("tr9 0 0 1").unwrap();
        assert!(m.remove_transform(9));
        assert_eq!(m.to_source(), src);
    }

    // --- generic data cards ------------------------------------------------

    #[test]
    fn data_cards_is_a_superset_including_sdef() {
        let m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\nsdef pos=0 0 0\n");
        let names: Vec<Option<&str>> = m.data_cards().map(|d| d.name()).collect();
        assert_eq!(names, vec![Some("m1"), Some("sdef")]);
        let sdef = m.data_cards().find(|d| d.name() == Some("sdef")).unwrap();
        assert_eq!(sdef.particle(), None);
        assert!(!sdef.starred());
        assert!(sdef.text().starts_with("sdef pos=0 0 0"));
    }

    #[test]
    fn add_data_card_appends_a_generic_card() {
        let mut m = Model::parse(SRC);
        let slot = m.add_data_card("sdef pos=0 0 0").unwrap();
        assert_eq!(m.data_card_at(slot).unwrap().name(), Some("sdef"));
        assert!(m.to_source().contains("sdef pos=0 0 0"));
    }

    #[test]
    fn add_data_card_after_removing_every_data_card() {
        let mut m = Model::parse(SRC);
        let slots: Vec<u32> = m.data_cards().map(|c| c.slot()).collect();
        for slot in slots {
            assert!(m.remove_data_card(slot));
        }
        let slot = m.add_data_card("sdef pos=0 0 0").unwrap();
        assert_eq!(m.data_card_at(slot).unwrap().name(), Some("sdef"));
        assert!(m.to_source().ends_with("1 SO 5\n\nsdef pos=0 0 0\n"));
    }

    #[test]
    fn add_cell_after_removing_every_cell() {
        let mut m = Model::parse(SRC);
        assert!(m.remove_cell(1));
        assert!(m.remove_cell(2));
        let slot = m.add_cell("3 0 -1 imp:n=1").unwrap();
        assert!(m.cell_at(slot).is_some());
        assert!(m.to_source().starts_with("t\n3 0 -1 imp:n=1\n\n1 SO 5"));
    }

    #[test]
    fn add_surface_after_removing_every_surface() {
        let mut m = Model::parse(SRC);
        assert!(m.remove_surface(1));
        let slot = m.add_surface("2 PX 3").unwrap();
        assert!(m.surface_at(slot).is_some());
        assert!(m
            .to_source()
            .contains("2 0 1 imp:n=0\n\n2 PX 3\n\nm1 1001 1"));
    }

    #[test]
    fn add_data_card_then_remove_is_byte_identity() {
        let mut m = Model::parse(SRC);
        let src = m.to_source();
        let slot = m.add_data_card("mode n").unwrap();
        assert!(m.remove_data_card(slot));
        assert_eq!(m.to_source(), src);
    }

    #[test]
    fn set_data_card_text_replaces_the_whole_card() {
        let mut m = Model::parse(SRC);
        let slot = m.add_data_card("sdef pos=0 0 0").unwrap();
        m.set_data_card_text(slot, "sdef pos=1 1 1").unwrap();
        assert_eq!(m.data_card_at(slot).unwrap().text(), "sdef pos=1 1 1\n");
        assert!(m.to_source().contains("sdef pos=1 1 1"));
        assert!(!m.to_source().contains("sdef pos=0 0 0"));
    }

    #[test]
    fn set_data_card_text_wrong_kind_errors() {
        let mut m = Model::parse(SRC);
        let cell_slot = m.cell(1).unwrap().slot();
        assert_eq!(
            m.set_data_card_text(cell_slot, "sdef pos=0 0 0"),
            Err(super::EditError::WrongKind)
        );
    }

    #[test]
    fn remove_data_card_by_slot() {
        let mut m = Model::parse(SRC);
        let slot = m.add_data_card("sdef pos=0 0 0").unwrap();
        assert!(m.remove_data_card(slot));
        assert!(m.data_card_at(slot).is_none());
        assert!(!m.remove_data_card(slot)); // already gone
    }

    #[test]
    fn remove_data_card_cleans_up_material_index() {
        let mut m = Model::parse(SRC);
        let slot = m.add_material("m2 8016 1").unwrap();
        assert!(m.material(2).is_some());
        assert!(m.remove_data_card(slot));
        assert!(m.material(2).is_none());
    }

    // --- geometry edits ------------------------------------------------------

    #[test]
    fn set_geometry_term_replaces_a_surface() {
        let mut m = Model::parse("t\n1 0 -1 2 -3 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_geometry_term(s, 1, "-4").unwrap();
        assert_eq!(m.cell(1).unwrap().signed_surfaces(), vec![-1, -4, -3]);
        assert!(m.to_source().contains("1 0 -1 -4 -3 imp:n=1"));
    }

    #[test]
    fn set_geometry_term_replaces_a_whole_complement() {
        let mut m = Model::parse("t\n1 0 -1 #2\n2 0 1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_geometry_term(s, 1, "#3").unwrap();
        assert_eq!(m.cell(1).unwrap().cell_refs(), vec![3]);
        assert!(m.to_source().contains("1 0 -1 #3"));
    }

    #[test]
    fn set_geometry_term_rejects_out_of_range_position() {
        let mut m = Model::parse(SRC);
        let s = slot_of_cell(&m, 1);
        assert_eq!(
            m.set_geometry_term(s, 99, "5"),
            Err(super::EditError::NoSuchField)
        );
    }

    #[test]
    fn set_geometry_term_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let surf = m.surface(1).unwrap().slot();
        assert_eq!(
            m.set_geometry_term(surf, 0, "1"),
            Err(super::EditError::WrongKind)
        );
    }

    #[test]
    fn insert_geometry_term_wraps_and_hashes_a_cell() {
        let mut m = Model::parse(
            "t\n1 0 1 2 3 imp:n=1\n2 0 -1 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n",
        );
        let s = slot_of_cell(&m, 1);
        let n = m.cell(1).unwrap().geometry().len();
        m.insert_geometry_term(s, 0, "(").unwrap();
        m.insert_geometry_term(s, n + 1, ")").unwrap();
        m.insert_geometry_term(s, n + 2, "#2").unwrap();
        let out = m.to_source();
        assert!(out.contains("1 0 ( 1 2 3 ) #2 imp:n=1"), "{out}");
        assert_eq!(m.cell(1).unwrap().cell_refs(), vec![2]);
    }

    #[test]
    fn insert_geometry_term_appends_a_union_when_position_equals_len() {
        let mut m = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        let n = m.cell(1).unwrap().geometry().len();
        m.insert_geometry_term(s, n, ": 2").unwrap();
        assert_eq!(m.cell(1).unwrap().signed_surfaces(), vec![-1, 2]);
        assert!(m.to_source().contains("1 0 -1 : 2 imp:n=1"));
    }

    #[test]
    fn insert_geometry_term_into_an_empty_geometry() {
        let mut m = Model::parse("t\n2 0 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 2);
        assert_eq!(m.cell(2).unwrap().geometry().len(), 0);
        m.insert_geometry_term(s, 0, "-1").unwrap();
        assert_eq!(m.cell(2).unwrap().signed_surfaces(), vec![-1]);
        assert!(m.to_source().contains("2 0 -1 imp:n=1"));
    }

    #[test]
    fn insert_geometry_term_rejects_position_past_the_end() {
        let mut m = Model::parse(SRC);
        let s = slot_of_cell(&m, 1);
        let n = m.cell(1).unwrap().geometry().len();
        assert_eq!(
            m.insert_geometry_term(s, n + 1, "2"),
            Err(super::EditError::NoSuchField)
        );
    }

    #[test]
    fn insert_geometry_term_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let surf = m.surface(1).unwrap().slot();
        assert_eq!(
            m.insert_geometry_term(surf, 0, "("),
            Err(super::EditError::WrongKind)
        );
    }

    #[test]
    fn geometry_text_reads_the_exact_source() {
        let m = Model::parse("t\n1 0 -1  2   #3\n2 0 1 imp:n=1\n\n1 SO 5\n2 SO 6\n\nm1 1001 1\n");
        assert_eq!(m.cell(1).unwrap().geometry_text(), "-1  2   #3");
    }

    #[test]
    fn geometry_text_is_empty_for_an_empty_geometry() {
        let m = Model::parse("t\n2 0 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        assert_eq!(m.cell(2).unwrap().geometry_text(), "");
    }

    #[test]
    fn set_cell_geometry_replaces_the_whole_expression() {
        let mut m =
            Model::parse("t\n1 0 -1 imp:n=1 vol=3\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 1);
        m.set_cell_geometry(s, "(1 2) : 3").unwrap();
        assert_eq!(m.cell(1).unwrap().geometry_text(), "(1 2) : 3");
        assert_eq!(m.cell(1).unwrap().signed_surfaces(), vec![1, 2, 3]);
        let out = m.to_source();
        assert!(out.contains("1 0 (1 2) : 3 imp:n=1 vol=3"), "{out}");
    }

    #[test]
    fn set_cell_geometry_unites_two_cells() {
        let mut m = Model::parse(
            "t\n1 0 -1 -2 imp:n=1\n2 0 -3 imp:n=1\n3 0 1 imp:n=1\n\n1 SO 5\n2 SO 6\n3 SO 7\n\nm1 1001 1\n",
        );
        let a = m.cell(1).unwrap().geometry_text();
        let b = m.cell(2).unwrap().geometry_text();
        let s = slot_of_cell(&m, 3);
        m.set_cell_geometry(s, &format!("({a}) : ({b})")).unwrap();
        assert_eq!(m.cell(3).unwrap().geometry_text(), "(-1 -2) : (-3)");
        assert_eq!(m.cell(3).unwrap().signed_surfaces(), vec![-1, -2, -3]);
    }

    #[test]
    fn set_cell_geometry_fills_in_an_empty_geometry() {
        let mut m = Model::parse("t\n2 0 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let s = slot_of_cell(&m, 2);
        m.set_cell_geometry(s, "-1").unwrap();
        assert_eq!(m.cell(2).unwrap().signed_surfaces(), vec![-1]);
        assert!(m.to_source().contains("2 0 -1 imp:n=1"));
    }

    #[test]
    fn set_cell_geometry_wrong_kind_is_rejected() {
        let mut m = Model::parse(SRC);
        let surf = m.surface(1).unwrap().slot();
        assert_eq!(
            m.set_cell_geometry(surf, "-1"),
            Err(super::EditError::WrongKind)
        );
    }

    #[test]
    fn remove_data_card_respects_duplicate_id_first_wins() {
        let mut m = Model::parse(SRC);
        let first = m.material(1).unwrap().slot();
        let dup = m.add_material("m1 8016 1").unwrap();
        assert_ne!(first, dup);
        // the index still points at the first-wins slot
        assert_eq!(m.material(1).unwrap().slot(), first);
        assert!(m.remove_data_card(dup));
        // removing the unindexed duplicate must not touch the live entry
        assert_eq!(m.material(1).unwrap().slot(), first);
    }
}
