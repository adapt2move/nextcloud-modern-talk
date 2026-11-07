//! Private worker input and bounded resolved metadata for a message link.
use super::content::ImageBytes;
use crate::bounded::Text;

#[derive(Clone, Eq, PartialEq)]
pub struct ReferenceUrl(Text<2048>);
impl ReferenceUrl {
    pub fn parse(value: &str) -> Option<Self> {
        crate::markup::web_destination(value).map(Self)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl std::fmt::Debug for ReferenceUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReferenceUrl([redacted])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub title: Text<512>,
    pub description: Option<Text<2048>>,
    pub thumbnail: Option<ImageBytes>,
}
