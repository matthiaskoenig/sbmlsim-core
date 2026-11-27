//! Model references resolve only within the explicit source set supplied by a host.
use super::*;
use sbmlsim_document::{
    Extension,
    locations::{normalize_location, resolve_source},
    packages::{attr, children},
};

pub(super) struct Resolver<'a> {
    models: Vec<HashMap<&'a str, &'a Model>>,
    resolved: Vec<HashMap<&'a str, Option<(usize, &'a Model)>>>,
    budget: usize,
}
impl<'a> Resolver<'a> {
    pub fn new(sources: &'a [(String, Document)], budget: usize) -> Self {
        let models = sources
            .iter()
            .map(|(_, d)| {
                let mut models = HashMap::new();
                for m in d.models() {
                    if let Some(id) = m.element.id.as_deref() {
                        models.entry(id).or_insert(m);
                    }
                }
                models
            })
            .collect::<Vec<_>>();
        let definitions = sources
            .iter()
            .map(|(_, d)| {
                let mut out = HashMap::new();
                for e in children(&d.element, "comp", "listOfExternalModelDefinitions") {
                    if let Some(id) = e.element.id.as_deref() {
                        out.entry(id).or_insert(e);
                    }
                }
                out
            })
            .collect::<Vec<HashMap<&str, &Extension>>>();
        let locations = sources
            .iter()
            .enumerate()
            .filter_map(|(i, (name, _))| normalize_location(name).map(|name| (name, i)))
            .collect::<HashMap<_, _>>();
        let mut this = Self {
            models,
            resolved: vec![HashMap::new(); sources.len()],
            budget,
        };
        for (home, roots) in definitions.iter().enumerate() {
            for root in roots.keys() {
                let mut path = vec![];
                let mut seen = HashSet::new();
                let (mut entry, mut model_ref) = (home, *root);
                let result = loop {
                    if let Some(model) = this.models[entry].get(model_ref) {
                        break Some((entry, *model));
                    }
                    if let Some(target) = this.resolved[entry].get(model_ref) {
                        break *target;
                    }
                    if !seen.insert((entry, model_ref)) {
                        break None;
                    }
                    path.push((entry, model_ref));
                    let Some(emd) = definitions[entry].get(model_ref) else {
                        break None;
                    };
                    let Some(target) = attr(emd, "source")
                        .and_then(|source| resolve_source(&sources[entry].0, source))
                        .and_then(|location| locations.get(&location).copied())
                    else {
                        break None;
                    };
                    let Some(target_ref) = attr(emd, "modelRef").filter(|s| !s.is_empty()) else {
                        break sources[target]
                            .1
                            .model
                            .as_ref()
                            .map(|model| (target, model));
                    };
                    entry = target;
                    model_ref = target_ref;
                };
                for (entry, model_ref) in path {
                    this.resolved[entry].insert(model_ref, result);
                }
            }
        }
        this
    }
}
impl<'a> ModelResolver<'a> for Resolver<'a> {
    fn model(&self, entry: usize, model_ref: &str) -> Option<(usize, &'a Model)> {
        self.models[entry]
            .get(model_ref)
            .map(|m| (entry, *m))
            .or_else(|| self.resolved[entry].get(model_ref).copied().flatten())
    }
    fn budget(&self) -> usize {
        self.budget
    }
}

impl OdeSystem {
    /// Read a source together with an explicit, closed set of companion documents.
    /// Neither a URI nor a filesystem path can cause the core to fetch a document.
    pub fn read_with_sources(
        source: &[u8],
        location: &str,
        documents: &[(String, Vec<u8>)],
        budget: usize,
    ) -> Result<Self, String> {
        const MAX_BYTES: usize = 100 * 1024 * 1024;
        if documents.len() >= 1000 {
            return Err("The source set exceeds 1000 documents.".into());
        }
        let total = documents
            .iter()
            .try_fold(source.len(), |total, (_, bytes)| {
                total.checked_add(bytes.len())
            })
            .ok_or("The source set exceeds its byte budget.")?;
        if total > MAX_BYTES {
            return Err("The source set exceeds 100 MB.".into());
        }
        let mut sources = Vec::with_capacity(documents.len() + 1);
        let mut seen = HashSet::new();
        for (name, bytes) in std::iter::once((location, source)).chain(
            documents
                .iter()
                .map(|(name, bytes)| (name.as_str(), bytes.as_slice())),
        ) {
            let normalized = normalize_location(name)
                .ok_or_else(|| format!("Invalid source location '{name}'"))?;
            if !seen.insert(normalized) {
                continue;
            }
            sources.push((
                name.to_owned(),
                sbmlsim_reader::read(bytes).map_err(|e| format!("{name}: {e}"))?,
            ));
        }
        Self::from_documents(sources, budget)
    }

    pub(super) fn from_documents(
        mut sources: Vec<(String, Document)>,
        budget: usize,
    ) -> Result<Self, String> {
        let original = sources[0]
            .1
            .model
            .as_ref()
            .ok_or("The SBML document has no model.")?;
        let flattened = if !children(&original.element, "comp", "listOfSubmodels").is_empty() {
            let resolver = Resolver::new(&sources, budget);
            Some(flatten(&sources[0].1, Some((&resolver, 0)))?)
        } else {
            None
        };
        let mut document = sources.swap_remove(0).1;
        let original = document.model.take().unwrap();
        let model = flattened.unwrap_or(original);
        let model = if document.level < 3 {
            normalize(&document, &model).into_owned()
        } else {
            model
        };
        let metadata = Document {
            level: document.level,
            version: document.version,
            ..Default::default()
        };
        Ok(Self {
            document: metadata,
            model,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(model: &str, external: &str) -> String {
        format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" level="3" version="2" comp:required="true">{external}{model}</sbml>"#
        )
    }
    #[test]
    fn equations_resolve_only_explicit_companion_documents() {
        let model = r#"<model id="main"><comp:listOfSubmodels><comp:submodel comp:id="child" comp:modelRef="external"/></comp:listOfSubmodels></model>"#;
        let external = r#"<comp:listOfExternalModelDefinitions><comp:externalModelDefinition comp:id="external" comp:source="other.xml"/></comp:listOfExternalModelDefinitions>"#;
        let source = document(model, external);
        assert!(OdeSystem::read(source.as_bytes()).is_err());
        let companion = document(
            r#"<model id="other"><listOfParameters><parameter id="p" value="2" constant="false"/></listOfParameters><listOfRules><rateRule variable="p"><math xmlns="http://www.w3.org/1998/Math/MathML"><cn>1</cn></math></rateRule></listOfRules></model>"#,
            "",
        );
        let system = OdeSystem::read_with_sources(
            source.as_bytes(),
            "./main.xml",
            &[("./other.xml".into(), companion.into_bytes())],
            10000,
        )
        .unwrap();
        let code = system
            .render(
                "python",
                &RenderOptions {
                    simulator: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(code.content.contains("child__p"));
    }
}
