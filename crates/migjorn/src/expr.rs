//! A cell's geometry expression as a tree.
//!
//! The grammar is checked once, by `cell::geometry_problems` (which also backs
//! `CellView::well_formed` and the parse diagnostics); the builder here only
//! runs on an expression that passed it, so a geometry that reads as a tree
//! never has a diagnostic and one that does not always has.

use migjorn_syntax::{Card, SyntaxKind};
use std::fmt;
use std::ops::Range;

use crate::cell::{self, SurfaceRef};
use crate::param::sig_tokens;
use crate::scan::{byte_span, kind_at};

/// A geometry expression, with MCNP's precedence applied: `#` binds tightest,
/// then intersection (juxtaposition), then union (`:`).
///
/// Parentheses group but leave no node of their own: `(1 2) 3` is
/// `And([And([1, 2]), 3])`, and `(1)` is just `1`. An `And` or `Or` always has
/// at least two operands.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A signed surface, or a macrobody facet: `-1`, `470.2`.
    Surface(SurfaceRef),
    /// `#n`: everything outside cell `n`.
    Complement(i64),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    /// `#( … )`: everything outside the region.
    Not(Box<Expr>),
}

/// Writes MCNP geometry text: operands of an intersection separated by a
/// space, unions by `:`, and parentheses only where precedence needs them.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Surface(s) => write!(f, "{s}"),
            Expr::Complement(n) => write!(f, "#{n}"),
            Expr::Not(e) => write!(f, "#({e})"),
            Expr::And(items) => {
                for (k, e) in items.iter().enumerate() {
                    if k > 0 {
                        f.write_str(" ")?;
                    }
                    match e {
                        Expr::Or(_) => write!(f, "({e})")?,
                        _ => write!(f, "{e}")?,
                    }
                }
                Ok(())
            }
            Expr::Or(items) => {
                for (k, e) in items.iter().enumerate() {
                    if k > 0 {
                        f.write_str(":")?;
                    }
                    write!(f, "{e}")?;
                }
                Ok(())
            }
        }
    }
}

/// Why a geometry expression could not be read into an [`Expr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeometryError {
    pub message: String,
    /// Byte range in the cell card's text ([`crate::CellView::text`]) of the
    /// token(s) at fault: the same span [`crate::CellView::diagnostics`]
    /// reports.
    pub span: Range<usize>,
    /// The source text at `span`, trimmed: `-1.5`, `:`, `#`.
    pub text: String,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GeometryError {}

/// Read the geometry tokens in `range` into a tree.
pub(crate) fn build(card: &Card, range: &Range<usize>) -> Result<Expr, GeometryError> {
    let error = |toks: Range<usize>, message: String| {
        let span = byte_span(card, toks);
        GeometryError {
            text: card.text()[span.clone()].trim().to_owned(),
            message,
            span,
        }
    };
    if let Some((toks, message)) = cell::geometry_problems(card, range).into_iter().next() {
        return Err(error(toks, message));
    }
    let toks = sig_tokens(card, range);
    if toks.is_empty() {
        return Err(error(range.clone(), "the cell has no geometry".to_owned()));
    }
    let mut p = Builder {
        card,
        toks: &toks,
        at: 0,
    };
    match p.union() {
        Some(e) if p.at == toks.len() => Ok(e),
        // `geometry_problems` accepted it, so this is a bug here, not in the
        // input; report it rather than panic.
        _ => Err(error(
            range.clone(),
            "geometry could not be read".to_owned(),
        )),
    }
}

struct Builder<'a> {
    card: &'a Card,
    toks: &'a [usize],
    at: usize,
}

impl Builder<'_> {
    fn peek(&self) -> Option<SyntaxKind> {
        kind_at(self.card, *self.toks.get(self.at)?)
    }

    fn eat(&mut self, kind: SyntaxKind) -> Option<()> {
        (self.peek()? == kind).then(|| self.at += 1)
    }

    /// `inter (':' inter)*`
    fn union(&mut self) -> Option<Expr> {
        let mut items = vec![self.inter()?];
        while self.eat(SyntaxKind::Colon).is_some() {
            items.push(self.inter()?);
        }
        Some(one_or(items, Expr::Or))
    }

    /// `unary+`, up to a `:`, a `)` or the end.
    fn inter(&mut self) -> Option<Expr> {
        let mut items = Vec::new();
        while matches!(
            self.peek(),
            Some(SyntaxKind::Number | SyntaxKind::Hash | SyntaxKind::LParen)
        ) {
            items.push(self.unary()?);
        }
        (!items.is_empty()).then(|| one_or(items, Expr::And))
    }

    /// A surface, `#n`, `#( … )` or `( … )`.
    fn unary(&mut self) -> Option<Expr> {
        let k = *self.toks.get(self.at)?;
        self.at += 1;
        let text = self.card.token_text(k);
        match kind_at(self.card, k)? {
            SyntaxKind::Number => SurfaceRef::parse(text).map(Expr::Surface),
            SyntaxKind::Hash => {
                if self.peek()? == SyntaxKind::Number {
                    let n = self.card.token_text(self.toks[self.at]).parse().ok()?;
                    self.at += 1;
                    Some(Expr::Complement(n))
                } else {
                    Some(Expr::Not(Box::new(self.group()?)))
                }
            }
            SyntaxKind::LParen => {
                self.at -= 1;
                self.group()
            }
            _ => None,
        }
    }

    /// `'(' union ')'`
    fn group(&mut self) -> Option<Expr> {
        self.eat(SyntaxKind::LParen)?;
        let e = self.union()?;
        self.eat(SyntaxKind::RParen)?;
        Some(e)
    }
}

fn one_or(mut items: Vec<Expr>, many: fn(Vec<Expr>) -> Expr) -> Expr {
    if items.len() == 1 {
        items.pop().expect("one item")
    } else {
        many(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Model;

    fn expr(geometry: &str) -> Result<Expr, GeometryError> {
        let m = Model::parse(&format!("t\n1 0 {geometry} imp:n=1\n\n1 SO 5\n\n"));
        let got = m.cells().next().unwrap().geometry_expr();
        // The tree and the diagnostics agree on whether it reads.
        assert_eq!(
            got.is_ok(),
            m.cells().next().unwrap().well_formed(),
            "{geometry}: {got:?}"
        );
        got
    }

    fn s(signed: i64) -> Expr {
        Expr::Surface(SurfaceRef {
            id: signed.abs(),
            facet: None,
            negative: signed < 0,
        })
    }

    #[test]
    fn precedence_complement_then_intersection_then_union() {
        assert_eq!(expr("-1").unwrap(), s(-1));
        assert_eq!(
            expr("-1 2 : 3").unwrap(),
            Expr::Or(vec![Expr::And(vec![s(-1), s(2)]), s(3)])
        );
        assert_eq!(
            expr("-1 (2 : 3)").unwrap(),
            Expr::And(vec![s(-1), Expr::Or(vec![s(2), s(3)])])
        );
        assert_eq!(
            expr("#2 -1:3").unwrap(),
            Expr::Or(vec![Expr::And(vec![Expr::Complement(2), s(-1)]), s(3)])
        );
        assert_eq!(
            expr("-1 #(2:-3)").unwrap(),
            Expr::And(vec![
                s(-1),
                Expr::Not(Box::new(Expr::Or(vec![s(2), s(-3)])))
            ])
        );
        // no space needed around operators
        assert_eq!(
            expr("(-1:2)(3)").unwrap(),
            Expr::And(vec![Expr::Or(vec![s(-1), s(2)]), s(3)])
        );
        assert_eq!(expr("((((-1))))").unwrap(), s(-1));
    }

    #[test]
    fn facets_and_signs() {
        assert_eq!(
            expr("-470.2 +3").unwrap(),
            Expr::And(vec![
                Expr::Surface(SurfaceRef {
                    id: 470,
                    facet: Some(2),
                    negative: true
                }),
                s(3)
            ])
        );
    }

    #[test]
    fn malformed_expressions_point_at_the_token() {
        let err = |g: &str| expr(g).unwrap_err();
        assert_eq!(err("-1 :").text, ":");
        assert!(err("-1 :").message.contains("nothing after"));
        assert!(err(": -1").message.contains("nothing before"));
        assert!(err("-1 :: 2").message.contains("nothing before"));
        assert!(err("(-1 :) 2").message.contains("nothing after"));
        assert!(err("-1 ()").message.contains("empty"));
        assert!(err("-1 #()").message.contains("empty"));
        assert!(err("(-1").message.contains("unclosed"));
        assert!(err("-1)").message.contains("unmatched"));
        assert_eq!(err("-1.0 2").text, "-1.0");
        assert_eq!(err("-1 #-2").text, "#-2");
        assert!(err("-1 # : 2").message.contains("`#` must be followed"));
        let deep = format!("{}-1{}", "(".repeat(300), ")".repeat(300));
        assert!(err(&deep).message.contains("nested"));
    }

    #[test]
    fn span_is_in_the_card_text() {
        let m = Model::parse("t\n1 0 -1 : imp:n=1\n\n1 SO 5\n\n");
        let c = m.cells().next().unwrap();
        let e = c.geometry_expr().unwrap_err();
        assert_eq!(&c.text()[e.span.clone()], ":");
        assert!(c
            .diagnostics()
            .iter()
            .any(|d| d.span == e.span && d.message.contains(&e.message)));
    }

    #[test]
    fn display_round_trips_through_the_parser() {
        for g in [
            "-1 2 : 3",
            "-1 (2:3) #4",
            "#(1 -2 : 3) -470.1",
            "(1:2) (3:4)",
            "1:(2 #(3:4))",
        ] {
            let e = expr(g).unwrap();
            assert_eq!(expr(&e.to_string()).unwrap(), e, "{g} -> {e}");
        }
        assert_eq!(expr("-1 (2:3)").unwrap().to_string(), "-1 (2:3)");
    }
}
