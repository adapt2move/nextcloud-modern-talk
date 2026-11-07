//! Account-scoped room creation inputs. Directory identities remain opaque.
use crate::{
    bounded::{Page, Text},
    policy::Decision,
    protocol::{FeatureSupport, conversation::RecipientId},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preset {
    pub id: Text<128>,
    pub name: Text<256>,
    pub description: Text<1024>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    pub group: Decision,
    pub extended: FeatureSupport,
    pub public: FeatureSupport,
    pub private_allowed: bool,
    pub public_allowed: bool,
    pub presets: Page<Preset, 30>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Room {
    pub name: Text<256>,
    pub description: Text<2000>,
    pub public: bool,
    pub preset: Option<Text<128>>,
    pub participants: Page<RecipientId, 20>,
}
