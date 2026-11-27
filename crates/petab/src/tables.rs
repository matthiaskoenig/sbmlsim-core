//! The tables of a problem, files of tab separated values with a header.

/// A table: its header and its rows of cells, every row as long as the header.
#[derive(Debug, Clone, Default)]
pub(crate) struct Table {
    /// The name of the file, for an error.
    pub file: String,
    /// The names of the columns.
    pub header: Vec<String>,
    /// The cells of every row, in the order of the header.
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Reads the text of a table.
    ///
    /// A cell is the text between two tabs with its whitespace trimmed, and a cell in double
    /// quotes is the text between them. A line without a cell is no row, and a row shorter than the header
    /// is filled with empty cells, which a writer of the tables leaves away at the end of a row.
    pub fn read(file: &str, text: &str) -> Result<Table, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.trim().is_empty());
        let header: Vec<String> = lines
            .next()
            .ok_or_else(|| format!("the table '{file}' has no header"))?
            .split('\t')
            .map(cell)
            .collect();
        let mut rows = Vec::new();
        for (number, line) in lines.enumerate() {
            let mut row: Vec<String> = line.split('\t').map(cell).collect();
            if row.len() > header.len() {
                if row[header.len()..].iter().any(|cell| !cell.is_empty()) {
                    return Err(format!(
                        "the row {} of the table '{file}' has {} cells and the header {}",
                        number + 1,
                        row.len(),
                        header.len()
                    ));
                }
                row.truncate(header.len());
            }
            row.resize(header.len(), String::new());
            rows.push(row);
        }
        Ok(Table {
            file: file.to_string(),
            header,
            rows,
        })
    }

    /// The position of a column, `None` for one the table does not have.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|column| column == name)
    }

    /// The position of a column the table has to have.
    pub fn required(&self, name: &str) -> Result<usize, String> {
        self.column(name)
            .ok_or_else(|| format!("the table '{}' has no column '{name}'", self.file))
    }

    /// Appends the rows of another table of the same kind, whose columns may come in another
    /// order and may be others: a column of one of them only is empty in the rows of the
    /// other.
    pub fn extend(&mut self, other: Table) {
        if self.header.is_empty() && self.rows.is_empty() {
            *self = other;
            return;
        }
        for name in &other.header {
            if self.column(name).is_none() {
                self.header.push(name.clone());
                for row in &mut self.rows {
                    row.push(String::new());
                }
            }
        }
        let positions: Vec<Option<usize>> =
            self.header.iter().map(|name| other.column(name)).collect();
        for row in other.rows {
            self.rows.push(
                positions
                    .iter()
                    .map(|position| position.map(|k| row[k].clone()).unwrap_or_default())
                    .collect(),
            );
        }
        self.file = format!("{}, {}", self.file, other.file);
    }
}

fn cell(text: &str) -> String {
    let text = text.trim();
    match text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        Some(inner) => inner.replace("\"\"", "\""),
        None => text.to_string(),
    }
}

/// A number of a table: a decimal, `inf`, `-inf` and `+inf`, ignoring the case, and `nan`.
pub(crate) fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    match text.to_ascii_lowercase().as_str() {
        "inf" | "+inf" | "infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        "nan" => Some(f64::NAN),
        _ => text.parse::<f64>().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_read_with_its_empty_cells() {
        let table = Table::read("t.tsv", "a\tb\tc\r\n1\t\t3\n\n4\t\"x\"\n").unwrap();
        assert_eq!(table.header, ["a", "b", "c"]);
        assert_eq!(table.rows, [vec!["1", "", "3"], vec!["4", "x", ""]]);
        assert_eq!(number("-inf"), Some(f64::NEG_INFINITY));
        assert_eq!(number("1e-3"), Some(1e-3));
    }

    #[test]
    fn tables_of_other_columns_are_joined() {
        let mut a = Table::read("a", "x\ty\n1\t2\n").unwrap();
        a.extend(Table::read("b", "y\tz\n3\t4\n").unwrap());
        assert_eq!(a.header, ["x", "y", "z"]);
        assert_eq!(a.rows, [vec!["1", "2", ""], vec!["", "3", "4"]]);
    }
}
