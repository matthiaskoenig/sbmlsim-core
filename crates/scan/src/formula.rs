//! A formula of a scan: the math of PEtab over the selections of the model, compiled with
//! `sbmlsim_petab::math`, so it is evaluated with the operations of the engine.
//!
//! The concentration of a species, `[S]`, and a name with a dot, the parameter `pk.cmax` of a
//! PK observable, are no identifiers of the math of PEtab: they are escaped into identifiers
//! before the formula is parsed and read back from them, as sbmlsim's
//! `simulator.formula._escaped` does.

use sbmlsim_petab::math;
use sbmlsim_sbml::math::Node;

/// The prefix of an escaped concentration, which no id of SBML or PEtab starts with.
const CONCENTRATION: &str = "__sbmlsim_concentration__";

/// The prefix of an escaped name with a dot.
const DOTTED: &str = "__sbmlsim_dotted__";

/// What stands for the dot of an escaped name with a dot.
const DOT: &str = "__sbmlsim_dot__";

/// The symbol an escaped identifier of the parsed formula stands for, the identifier itself
/// where it was not escaped.
pub(crate) fn unescaped(name: &str) -> String {
    if let Some(species) = name.strip_prefix(CONCENTRATION) {
        return format!("[{species}]");
    }
    if let Some(dotted) = name.strip_prefix(DOTTED) {
        return dotted.replacen(DOT, ".", 1);
    }
    name.to_string()
}

/// A compiled formula.
#[derive(Debug, Clone)]
pub(crate) struct CompiledFormula {
    formula: math::Formula,
    /// The parsed formula, its symbols escaped (see [`unescaped`]).
    node: Node,
    /// The symbols as the formula writes them (`[S]` a concentration), in the order of the
    /// values of [`CompiledFormula::evaluate`].
    symbols: Vec<String>,
    /// Whether the formula reads the time.
    reads_time: bool,
}

impl CompiledFormula {
    /// Compiles a formula.
    ///
    /// # Errors
    ///
    /// A formula which is no math of PEtab, with the reason.
    pub fn new(text: &str) -> Result<CompiledFormula, String> {
        let escaped = escape(text);
        let node = math::parse(&escaped).map_err(|err| err.message)?;
        let formula = math::Formula::new(&node)?;
        let symbols = formula.symbols().iter().map(|s| unescaped(s)).collect();
        Ok(CompiledFormula {
            formula,
            symbols,
            reads_time: math::reads_time(&node),
            node,
        })
    }

    /// The symbols, without the time.
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Whether the formula reads the time.
    pub fn reads_time(&self) -> bool {
        self.reads_time
    }

    /// The parsed formula, whose symbols are escaped (see [`unescaped`]).
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// The value at the values of the symbols and at a time.
    pub fn evaluate(&self, values: &[f64], time: f64) -> f64 {
        self.formula.evaluate(values, time)
    }
}

/// The formula with every `[S]` and every name with a dot written as an identifier.
fn escape(text: &str) -> String {
    escape_concentrations(&escape_dotted(text))
}

/// Whether a character continues an identifier or a dotted name.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The formula with every name with a dot (`pk.cmax`), which is neither the end of a longer
/// name nor followed by a further dot, written as an identifier, sbmlsim's `_DOTTED`.
fn escape_dotted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts = (c.is_ascii_alphabetic() || c == '_')
            && (i == 0 || !(word(chars[i - 1]) || chars[i - 1] == '.'));
        if starts {
            let mut j = i;
            while j < chars.len() && word(chars[j]) {
                j += 1;
            }
            let dotted = j + 1 < chars.len()
                && chars[j] == '.'
                && (chars[j + 1].is_ascii_alphabetic() || chars[j + 1] == '_');
            if dotted {
                let mut k = j + 1;
                while k < chars.len() && word(chars[k]) {
                    k += 1;
                }
                if k >= chars.len() || !(word(chars[k]) || chars[k] == '.') {
                    let head: String = chars[i..j].iter().collect();
                    let tail: String = chars[j + 1..k].iter().collect();
                    out.push_str(&format!("{DOTTED}{head}{DOT}{tail}"));
                    i = k;
                    continue;
                }
            }
            out.extend(&chars[i..j]);
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The formula with every `[S]` written as an identifier.
fn escape_concentrations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close)
                if close > 0
                    && after[..close]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                out.push_str(CONCENTRATION);
                out.push_str(&after[..close]);
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::CompiledFormula;

    #[test]
    fn a_formula_reads_concentrations_and_the_time() {
        let f = CompiledFormula::new("2 * [e__A] + k * time").unwrap();
        assert_eq!(f.symbols(), ["[e__A]", "k"]);
        assert!(f.reads_time());
        assert_eq!(f.evaluate(&[1.5, 2.0], 3.0), 9.0);
        let f = CompiledFormula::new("2 * Vmax_bA").unwrap();
        assert_eq!(f.symbols(), ["Vmax_bA"]);
        assert!(!f.reads_time());
        assert!(CompiledFormula::new("2 *").is_err());
        let f = CompiledFormula::new("pk.cmax / [C] + 1.5").unwrap();
        assert_eq!(f.symbols(), ["pk.cmax", "[C]"]);
        assert_eq!(f.evaluate(&[3.0, 2.0], 0.0), 3.0);
    }
}
