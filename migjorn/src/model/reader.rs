use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::parser_utils::{divide_cards_slices_into_sections, get_cards_original_bytes};
use crate::{CellCard, DataCard, Model, ModelError, ModelSection, SurfaceCard};

impl Model {
    /// Load a model from a file path. This is the main entry point for parsing.
    /// The file is read into memory as a single byte buffer, which is then sliced
    /// into cards and parsed in parallel. The original byte slices are retained
    /// for error reporting and lossless rewriting.
    ///
    /// Returns an error if the file cannot be read or if any card fails to parse.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Model, ModelError> {
        let bytes = fs::read(&path)?;
        let source: Arc<[u8]> = Arc::from(bytes);
        Self::parse_from_source(path.as_ref(), source)
    }

    /// Parse a model directly from in-memory text. Useful for language servers
    /// and other tools that have the document content in memory without a file.
    ///
    /// Returns an error if any card fails to parse.
    pub fn from_text<P: AsRef<Path>>(path: P, text: &str) -> Result<Model, ModelError> {
        let source: Arc<[u8]> = Arc::from(text.as_bytes());
        Self::parse_from_source(path.as_ref(), source)
    }

    fn parse_from_source<P: AsRef<Path>>(path: P, source: Arc<[u8]>) -> Result<Model, ModelError> {
        // Get card slices as OriginalBytes with references to the source buffer
        let cards = get_cards_original_bytes(&source);

        // Divide cards into sections and parse in parallel
        let sections = divide_cards_slices_into_sections(cards);

        // The title card is the first line of the file
        let title = sections.title;

        // Parse cells in parallel
        let cells: Vec<_> = sections
            .cells
            .into_par_iter()
            .map(|card| {
                CellCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Cells,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Parse surfaces in parallel
        let surfaces: Vec<_> = sections
            .surfaces
            .into_par_iter()
            .map(|card| {
                SurfaceCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Surfaces,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Parse data cards in parallel
        let data_cards: Vec<_> = sections
            .data_cards
            .into_iter()
            .map(|card| {
                DataCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Data,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Model {
            path: path.as_ref().to_owned(),
            title,
            cells,
            surfaces,
            data_cards,
            source,
        })
    }

    /// Same as `from_file` but parses sequentially without using Rayon.
    /// Useful for reading small files as it avoids the overhead of parallelism.
    #[doc(hidden)]
    pub fn from_file_sequential<P: AsRef<Path>>(path: P) -> Result<Model, ModelError> {
        let bytes = fs::read(&path)?;
        let source: Arc<[u8]> = Arc::from(bytes);
        Self::parse_from_source_sequential(path.as_ref(), source)
    }

    #[doc(hidden)]
    fn parse_from_source_sequential<P: AsRef<Path>>(
        path: P,
        source: Arc<[u8]>,
    ) -> Result<Model, ModelError> {
        let cards = get_cards_original_bytes(&source);
        let sections = divide_cards_slices_into_sections(cards);

        let title = sections.title;

        let cells: Vec<_> = sections
            .cells
            .into_iter()
            .map(|card| {
                CellCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Cells,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let surfaces: Vec<_> = sections
            .surfaces
            .into_iter()
            .map(|card| {
                SurfaceCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Surfaces,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let data_cards: Vec<_> = sections
            .data_cards
            .into_iter()
            .map(|card| {
                DataCard::try_from(card).map_err(|e| ModelError::Parse {
                    section: ModelSection::Data,
                    error: e,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Model {
            path: path.as_ref().to_owned(),
            title,
            cells,
            surfaces,
            data_cards,
            source,
        })
    }
}
