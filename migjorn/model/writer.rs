use crate::{Card, Model};
use std::io::{BufWriter, Write};
use std::path::Path;

impl Model {
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let mut file = BufWriter::new(std::fs::File::create(path.as_ref())?);

        // Write title
        file.write_all(self.title.as_ref())?;

        // Write cells section
        for card in &self.cells {
            card.write_into(&mut file)?;
        }

        // Write blank line between sections
        file.write_all(b"\n")?;

        // Write surfaces section
        for card in &self.surfaces {
            card.write_into(&mut file)?;
        }

        // Write blank line between sections
        file.write_all(b"\n")?;

        // Write data cards section
        for card in &self.data_cards {
            card.write_into(&mut file)?;
        }

        Ok(())
    }
}
