//! 书签展示快照，路径始终是身份，别名只影响显示

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    path: String,
    alias: Option<String>,
}

impl Bookmark {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            alias: None,
        }
    }

    pub fn with_alias(mut self, alias: Option<&str>) -> Self {
        self.alias = normalize_alias(alias);
        self
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn label(&self) -> &str {
        self.alias().unwrap_or(self.path())
    }

    /// 所有关键词均出现在别名或完整路径中，不受显示省略影响
    pub fn matches(&self, query: &str) -> bool {
        let path = self.path.to_lowercase();
        let alias = self.alias.as_deref().unwrap_or_default().to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| path.contains(word) || alias.contains(word))
    }
}

pub(crate) fn normalize_alias(alias: Option<&str>) -> Option<String> {
    alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(str::to_owned)
}
