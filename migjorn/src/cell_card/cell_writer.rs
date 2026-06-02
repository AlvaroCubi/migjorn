use crate::parser_utils::write_spanned;

use super::{CellCard, GeoElementSpanned};

impl CellCard {
    /// Reconstruct the card bytes with current field values, preserving original formatting
    pub fn to_bytes(&self) -> Vec<u8> {
        let source = self.original_bytes.as_ref();
        let mut result = Vec::with_capacity(source.len());
        let mut last_pos = 0;

        // Copy bytes before cell ID, then write current cell ID
        write_spanned(source, &mut result, &mut last_pos, &self.cell_id);

        // Copy bytes before material ID, then write current material ID
        write_spanned(source, &mut result, &mut last_pos, &self.material_id);

        // Handle density if present
        if let Some(density) = &self.density {
            write_spanned(source, &mut result, &mut last_pos, density);
        }

        // Write geometry elements
        let mut prev_geo: Option<&GeoElementSpanned> = None;
        for geo_elem in &self.geometry {
            geo_elem.write(source, &mut result, &mut last_pos, prev_geo);
            prev_geo = Some(geo_elem);
        }

        // Write parameters
        for param in &self.params {
            param.write(source, &mut result, &mut last_pos);
        }

        // Copy remaining bytes
        result.extend_from_slice(&source[last_pos..]);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cell_card::parser_cell_parameters::{CellParam, ParamType},
        parser_utils::{OriginalBytes, Span},
    };

    fn parse(s: &str) -> CellCard {
        CellCard::parse(OriginalBytes::from(s)).unwrap()
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

    #[test]
    fn test_roundtrip_with_params() {
        round_trip("3 2 0.08 -30  (-10  #11  :  -20) IMP:N=1");
    }

    #[test]
    fn test_modify_parameters() {
        let mut cell = parse("3 2 0.08 -30 IMP:N=1");
        cell.params[0].param_type = ParamType::ImpN(2.5);
        cell.params.push(CellParam {
            param_type: ParamType::Vol(100.2),
            span: Span(0, 0),
        });
        assert_eq!(
            std::str::from_utf8(&cell.to_bytes()).unwrap(),
            "3 2 0.08 -30 IMP:N=2.5 VOL=1.00200E+02"
        );
    }
}
