use std::fmt::Display;

use crate::Card;
use crate::parser_utils::{
    CardError, OriginalBytes, ParseError, Span, Spanned, approx_eq_f64, parse_spanned_f64,
    parse_spanned_u32, skip_whitespace,
};
use parser_cell_parameters::parse_parameters;
use parser_geometry::parse_geometry;

pub(crate) use parser_geometry::GeoElementSpanned;

pub use parser_cell_parameters::{CellParam, FillData, ParamType};
pub use parser_geometry::GeoElement;

mod cell_writer;
mod parser_cell_parameters;
mod parser_geometry;

#[derive(Debug, Clone)]
pub struct CellCard {
    pub(crate) original_bytes: OriginalBytes,
    pub(crate) cell_id: Spanned<u32>,
    pub(crate) material_id: Spanned<u32>,
    pub(crate) density: Option<Spanned<f64>>,
    pub(crate) geometry: Vec<GeoElementSpanned>,
    pub(crate) params: Vec<CellParam>,
}

impl PartialEq for CellCard {
    fn eq(&self, other: &Self) -> bool {
        self.cell_id.value == other.cell_id.value
            && self.material_id.value == other.material_id.value
            && match (
                self.density.as_ref().map(|d| d.value),
                other.density.as_ref().map(|d| d.value),
            ) {
                (Some(a), Some(b)) => approx_eq_f64(a, b),
                (None, None) => true,
                _ => false,
            }
            && self
                .geometry
                .iter()
                .map(|geo| geo.value())
                .eq(other.geometry.iter().map(|geo| geo.value()))
            && self
                .params
                .iter()
                .map(|param| &param.param_type)
                .eq(other.params.iter().map(|param| &param.param_type))
    }
}

impl CellCard {
    pub fn cell_id(&self) -> u32 {
        self.cell_id.value
    }

    pub fn set_cell_id(&mut self, new_id: u32) {
        self.cell_id.value = new_id;
    }

    pub fn material_id(&self) -> u32 {
        self.material_id.value
    }

    pub fn set_material_id(&mut self, new_id: u32) {
        self.material_id.value = new_id;
    }

    pub fn density(&self) -> Option<f64> {
        self.density.as_ref().map(|d| d.value)
    }

    pub fn set_density(&mut self, new_density: f64) {
        if let Some(density) = &mut self.density {
            density.value = new_density;
        } else {
            // If there was no density before, we need to create a new Spanned with a zero-length span
            // at the end of the material ID so it is written after it without consuming source bytes.
            self.density = Some(Spanned::new(
                new_density,
                Span::empty_at(self.material_id.span.1),
            ));
        }
    }

    /// Iterator over geometry elements (zero allocation, preferred for large models)
    pub fn geometry(&self) -> impl Iterator<Item = GeoElement> + '_ {
        self.geometry.iter().map(|e| e.value())
    }

    /// Remove a geometry element by index
    pub fn remove_geometry_element(&mut self, index: usize) {
        // Spans will be adjusted to hide the removed element
        if index >= self.geometry.len() {
            panic!("Index out of bounds");
        }
        if index == 0 && self.geometry.len() == 1 {
            panic!("Would remove the only geometry element");
        }
        let span_to_hide = self.geometry[index].span();
        if index == 0 {
            self.geometry[1].span_mut().0 = span_to_hide.0;
        } else {
            self.geometry[index - 1].span_mut().1 = span_to_hide.1;
        }
        self.geometry.remove(index);
    }

    pub fn insert_geometry_element(&mut self, index: usize, geo_element: GeoElement) {
        assert!(index <= self.geometry.len(), "index out of bounds");

        // Compute a zero-length span at the right boundary so write_spanned
        // treats the element as virtually inserted. The complement/space logic
        // is handled by GeoElement::write using the prev parameter.
        let position = if index < self.geometry.len() {
            self.geometry[index].span().0
        } else {
            self.geometry.last().map_or(0, |e| e.span().1)
        };
        let span = Span::empty_at(position);
        let element = match geo_element {
            GeoElement::Surface(s) => GeoElementSpanned::Surface(Spanned::new(s, span)),
            GeoElement::Cell(c) => GeoElementSpanned::Cell(Spanned::new(c, span)),
            GeoElement::Operator(o) => GeoElementSpanned::Operator(Spanned::new(o, span)),
        };

        self.geometry.insert(index, element);
    }

    /// Get cell parameters as a slice
    pub fn params(&self) -> &[CellParam] {
        &self.params
    }

    /// Get mutable cell parameters as a slice
    pub fn params_mut(&mut self) -> &mut [CellParam] {
        &mut self.params
    }

    /// Get the universe of this cell if it has one (U= parameter)
    pub fn get_universe(&self) -> Option<u32> {
        self.params.iter().find_map(|p| match p.param_type {
            ParamType::U(u) => Some(u),
            _ => None,
        })
    }

    /// Get the fill of this cell if it has one (FILL= parameter)
    pub fn get_fill(&self) -> Option<FillData> {
        self.params.iter().find_map(|p| match &p.param_type {
            ParamType::Fill(f) => Some(f.clone()),
            _ => None,
        })
    }

    /// Remove a parameter by index
    pub fn remove_param(&mut self, index: usize) {
        // Spans will be adjusted to hide the removed parameter and its preceding whitespace
        assert!(index < self.params.len(), "index out of bounds");
        let span_to_hide = self.params[index].span;
        if index == 0 {
            if let Some(next) = self.params.get_mut(1) {
                next.span.0 = span_to_hide.0;
            }
        } else {
            self.params[index - 1].span.1 = span_to_hide.1;
        }
        self.params.remove(index);
    }

    /// Insert a parameter at `index`
    pub fn insert_param(&mut self, index: usize, mut param: CellParam) {
        assert!(index <= self.params.len(), "index out of bounds");
        let position = if index < self.params.len() {
            self.params[index].span.0
        } else {
            self.params.last().map_or(0, |p| p.span.1)
        };
        param.span = Span::empty_at(position);
        self.params.insert(index, param);
    }

    /// Set this cell as void (material ID 0 and no density).
    pub fn make_void(&mut self) {
        self.material_id.value = 0;
        // The span of the material will now end where the density span ended
        // so it is not written again
        self.material_id.span.1 = self
            .density
            .as_ref()
            .map_or(self.material_id.span.1, |d| d.span.1);
        self.density = None;
    }
}

impl Card for CellCard {
    fn original_bytes(&self) -> &[u8] {
        self.original_bytes.as_ref()
    }

    fn updated_bytes(&self) -> Vec<u8> {
        self.to_bytes()
    }
}

impl TryFrom<OriginalBytes> for CellCard {
    type Error = CardError;
    fn try_from(bytes: OriginalBytes) -> Result<Self, Self::Error> {
        CellCard::parse(bytes)
    }
}

impl TryFrom<&str> for CellCard {
    type Error = CardError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        CellCard::try_from(OriginalBytes::from(s))
    }
}

impl Display for CellCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CellCard
    card_id: {},
    material_id: {},
    density: {:?},
    geometry: {:?},
    params: {:?}",
            self.cell_id.value,
            self.material_id.value,
            self.density.as_ref().map(|d| d.value),
            self.geometry().collect::<Vec<_>>(),
            self.params,
        )
    }
}

impl CellCard {
    /// Parse a complete cell card
    ///
    /// Cell card format:
    /// <cell_id> <material> [density] <geometry> [parameters]
    ///
    /// Examples:
    /// - `1 1 -2.7 -10`  (cell 1, material 1, density -2.7g/cm³, surface -10)
    /// - `2 0 10 -20 #1` (cell 2, void, surfaces 10, -20, not cell 1)
    /// - `3 2 0.08 -30 IMP:N=1 VOL=100`
    fn parse(original_bytes: OriginalBytes) -> Result<CellCard, CardError> {
        let bytes = original_bytes.as_ref();
        let mut pos = 0;

        // Skip initial whitespace and comments
        skip_whitespace(bytes, &mut pos);

        if pos >= bytes.len() {
            return Err(CardError::new(ParseError::EmptyCard, original_bytes, pos));
        }

        // Parse cell ID and capture span
        let cell_id = parse_spanned_u32(bytes, &mut pos)
            .map_err(|_| CardError::new(ParseError::InvalidCellId, original_bytes.clone(), pos))?;

        // Parse material and capture span
        let material_id = parse_spanned_u32(bytes, &mut pos).map_err(|_| {
            CardError::new(ParseError::InvalidMaterial, original_bytes.clone(), pos)
        })?;

        // Parse density if material != 0 and capture span
        let density = if material_id.value != 0 {
            Some(parse_spanned_f64(bytes, &mut pos).map_err(|_| {
                CardError::new(ParseError::InvalidDensity, original_bytes.clone(), pos)
            })?)
        } else {
            None
        };

        // Parse geometry
        let geometry = parse_geometry(bytes, &mut pos)
            .map_err(|_| CardError::new(ParseError::InvalidGeo, original_bytes.clone(), pos))?;

        // Parse parameters
        let params = parse_parameters(bytes, &mut pos).map_err(|_| {
            CardError::new(ParseError::UnknownParameter, original_bytes.clone(), pos)
        })?;

        Ok(CellCard {
            original_bytes,
            cell_id,
            material_id,
            density,
            geometry,
            params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::parser_geometry::GeoOperator;
    use super::*;

    fn parse(s: &str) -> CellCard {
        CellCard::try_from(s).unwrap()
    }

    fn round_trip(input: &str) {
        let cell = parse(input);
        let output = cell.to_bytes();
        assert_eq!(
            std::str::from_utf8(&output).unwrap(),
            input,
            "round-trip failed"
        );
    }

    // --- round-trip tests (output must equal input when nothing is modified) ---

    #[test]
    fn test_roundtrip_single_surface() {
        round_trip("1 1 -2.7 -10");
    }

    #[test]
    fn test_roundtrip_multiple_surfaces_with_union() {
        round_trip("1 1 -2.7 -10 20 : -30");
    }

    #[test]
    fn test_roundtrip_complement_operator() {
        // #N references a cell, not a surface
        round_trip("2 0 10 -20 #1");
    }

    #[test]
    fn test_roundtrip_preserves_internal_whitespace_and_parens() {
        round_trip("100  2  -7.8   (-10  11  :  -20)");
    }

    #[test]
    fn test_roundtrip_with_params() {
        round_trip("3 2 0.08 -30 IMP:N=1");
    }

    #[test]
    fn test_roundtrip_with_unknown_params() {
        round_trip("3 2 0.08 -30 IMP:N=1 UNKNOWN=foo IMP:N,P=2 VOL=5.00000E+00");
    }

    #[test]
    fn test_void_cell() {
        let mut cell = parse("1 1 -2.7 -10 (20 :  -30) #40 IMP:N=1");
        cell.make_void();
        assert_eq!(cell.updated_text(), "1 0 -10 (20 :  -30) #40 IMP:N=1");
    }

    #[test]
    fn test_remove_geometry_element() {
        let mut cell = parse("1 1 -2.7 -10 20 : -30 IMP:N=1");
        cell.remove_geometry_element(1);
        assert_eq!("1 1 -2.7 -10 : -30 IMP:N=1", cell.updated_text());
        cell.remove_geometry_element(2);
        assert_eq!("1 1 -2.7 -10 : IMP:N=1", cell.updated_text());
    }

    #[test]
    fn test_insert_geometry_element() {
        let mut cell = parse("1 1 -2.7 (20 :  -30) IMP:N=1");
        cell.cell_id.value = 99999;
        cell.material_id.value = 80;
        if let Some(density) = &mut cell.density {
            density.value = -5.1;
        }
        cell.insert_geometry_element(0, GeoElement::Surface(-10));
        cell.insert_geometry_element(
            cell.geometry.len(),
            GeoElement::Operator(GeoOperator::Complement),
        );
        cell.insert_geometry_element(cell.geometry.len(), GeoElement::Cell(40));
        assert_eq!(
            "99999 80 -5.1 -10 (20 :  -30) #40 IMP:N=1",
            cell.updated_text()
        );
    }

    #[test]
    fn test_remove_and_insert_geometry_elements() {
        let mut cell = parse("1 1 -2.7 -10 (20 :  -30) #40 IMP:N=1");
        cell.remove_geometry_element(1);
        cell.remove_geometry_element(1);
        cell.remove_geometry_element(1);
        cell.remove_geometry_element(1);
        cell.remove_geometry_element(1);

        cell.insert_geometry_element(
            cell.geometry.len(),
            GeoElement::Operator(GeoOperator::Complement),
        );
        cell.insert_geometry_element(cell.geometry.len(), GeoElement::Cell(22));
        assert_eq!("1 1 -2.7 -10 #40 #22 IMP:N=1", cell.updated_text());
    }

    #[test]
    fn test_remove_param() {
        let mut cell = parse("1 1 -2.7 -10 IMP:N=1 VOL=5.0 U=3");
        cell.remove_param(1); // remove VOL
        assert_eq!("1 1 -2.7 -10 IMP:N=1 U=3", cell.updated_text());
        cell.remove_param(0); // remove IMP:N, U=3 absorbs whitespace
        assert_eq!("1 1 -2.7 -10 U=3", cell.updated_text());
    }

    #[test]
    fn test_insert_param() {
        use super::parser_cell_parameters::{CellParam, ParamType};
        let mut cell = parse("1 1 -2.7 -10 IMP:N=1");
        cell.insert_param(
            cell.params.len(),
            CellParam {
                param_type: ParamType::Vol(5.0),
                span: Span(0, 0),
            },
        );
        assert_eq!("1 1 -2.7 -10 IMP:N=1 VOL=5.00000E+00", cell.updated_text());
    }

    #[test]
    fn test_insert_other_param() {
        use super::parser_cell_parameters::{CellParam, ParamType};
        let mut cell = parse("1 1 -2.7 -10 IMP:N=1");
        cell.insert_param(
            cell.params.len(),
            CellParam {
                param_type: ParamType::Other("UNKNOWN=foo".to_string()),
                span: Span(0, 0),
            },
        );
        assert_eq!("1 1 -2.7 -10 IMP:N=1 UNKNOWN=foo", cell.updated_text());
    }

    #[test]
    fn test_geometry_iterator_api() {
        let cell = parse("1 1 -2.7 -10 (20 :  -30) #40 IMP:N=1");

        // Test iterator (no allocation)
        let mut iter = cell.geometry();
        assert_eq!(iter.next(), Some(GeoElement::Surface(-10)));
        assert_eq!(
            iter.next(),
            Some(GeoElement::Operator(GeoOperator::LeftParen))
        );

        // Test collect (allocates Vec)
        let geo_vec = cell.geometry().collect::<Vec<_>>();
        assert_eq!(geo_vec.len(), 8);
    }
}
