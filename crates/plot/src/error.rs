use std::fmt;

/// Marks the sub path an element reports, see [`SpecError::from_serde`].
pub(crate) const SUB_PATH: &str = "\u{1}";

/// What is wrong with a figure spec and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    /// The path of the offending field, e.g. `plots[0].elements[1].y`; `.` for the document.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl SpecError {
    /// Creates an error at a path.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        SpecError {
            path: path.into(),
            message: message.into(),
        }
    }

    /// An error of serde. An element deserializes its fields itself and reports the path
    /// inside it as `SUB_PATH path SUB_PATH message`, which is joined to the path of the element.
    /// An unknown field names itself in the path.
    pub(crate) fn from_serde(path: String, message: String) -> Self {
        let (path, message) = match message
            .strip_prefix(SUB_PATH)
            .and_then(|m| m.split_once(SUB_PATH))
        {
            Some((sub, rest)) => (join(&path, sub), rest.to_string()),
            None => (path, message),
        };
        if let Some(field) = message
            .strip_prefix("unknown field `")
            .and_then(|m| m.split_once('`'))
            .map(|(f, _)| f.to_string())
        {
            let named = path.rsplit('.').next() == Some(field.as_str());
            let path = if named { path } else { join(&path, &field) };
            return SpecError { path, message };
        }
        SpecError { path, message }
    }
}

fn join(path: &str, sub: &str) -> String {
    if sub.is_empty() || sub == "." {
        path.to_string()
    } else if path.is_empty() || path == "." {
        sub.to_string()
    } else if sub.starts_with('[') {
        format!("{path}{sub}")
    } else {
        format!("{path}.{sub}")
    }
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for SpecError {}
