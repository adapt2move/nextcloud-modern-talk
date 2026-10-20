//! Private per-account layout fallback. Keys are durable lookup keys, never authority.
//! Upserts touch one item; a partial server page cannot replace or prune a layout.
use crate::bounded::Text;

pub const MAX_ACCOUNTS: usize = 8;
pub const MAX_GROUPS: usize = 32;
pub const MAX_PLACEMENTS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationError {
    InvalidKey,
    InvalidGroup,
    MissingGroup,
    Capacity,
    RemoteSupportUnknown,
    RemoteSupported,
}

fn lower_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationAccountKey(Text<128>);
impl NavigationAccountKey {
    pub fn new(value: &str) -> Result<Self, NavigationError> {
        let mut parts = value.split(':');
        if parts.next() != Some("a1")
            || !parts.next().is_some_and(lower_hex)
            || !parts.next().is_some_and(lower_hex)
            || parts.next().is_some()
        {
            return Err(NavigationError::InvalidKey);
        }
        Text::new(value)
            .map(Self)
            .map_err(|_| NavigationError::InvalidKey)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationRoomKey(Text<256>);
impl NavigationRoomKey {
    pub fn new(value: &str) -> Result<Self, NavigationError> {
        if !value.strip_prefix("r1:").is_some_and(lower_hex) {
            return Err(NavigationError::InvalidKey);
        }
        Text::new(value)
            .map(Self)
            .map_err(|_| NavigationError::InvalidKey)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Capability for the feature being edited, resolved by the account owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteNavigationSupport {
    Unknown,
    Supported,
    Unsupported,
}
impl RemoteNavigationSupport {
    fn local(self) -> Result<(), NavigationError> {
        match self {
            Self::Unknown => Err(NavigationError::RemoteSupportUnknown),
            Self::Supported => Err(NavigationError::RemoteSupported),
            Self::Unsupported => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NavigationGroupId(u64);
impl NavigationGroupId {
    pub fn new(value: u64) -> Result<Self, NavigationError> {
        if value == 0 {
            Err(NavigationError::InvalidGroup)
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationGroup {
    pub id: NavigationGroupId,
    pub name: Text<128>,
    pub order: u16,
    pub collapsed: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationPlacement {
    pub room: NavigationRoomKey,
    pub group: Option<NavigationGroupId>,
    pub order: u16,
    /// None leaves the server's favorite alone. Never infer unsupported from unknown.
    pub favorite: Option<bool>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationEdit {
    SetSectionCollapsed {
        section: NavigationSection,
        collapsed: bool,
    },
    UpsertGroup(NavigationGroup),
    RemoveGroup(NavigationGroupId),
    /// Layout and favorite capability are checked independently.
    PlaceRoom {
        room: NavigationRoomKey,
        group: Option<NavigationGroupId>,
        order: u16,
    },
    SetFavorite {
        room: NavigationRoomKey,
        favorite: Option<bool>,
    },
    RemoveRoom(NavigationRoomKey),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountNavigation {
    account: NavigationAccountKey,
    collapsed_sections: u8,
    groups: Vec<NavigationGroup>,
    placements: Vec<NavigationPlacement>,
}
impl AccountNavigation {
    pub fn account(&self) -> &NavigationAccountKey {
        &self.account
    }
    pub fn groups(&self) -> &[NavigationGroup] {
        &self.groups
    }
    pub fn placements(&self) -> &[NavigationPlacement] {
        &self.placements
    }
    pub fn collapsed(&self, section: NavigationSection) -> bool {
        self.collapsed_sections & section.bit() != 0
    }
    fn placement(
        &mut self,
        room: NavigationRoomKey,
    ) -> Result<&mut NavigationPlacement, NavigationError> {
        let index = if let Some(index) = self.placements.iter().position(|p| p.room == room) {
            index
        } else {
            if self.placements.len() == MAX_PLACEMENTS {
                return Err(NavigationError::Capacity);
            }
            self.placements.push(NavigationPlacement {
                room,
                group: None,
                order: 0,
                favorite: None,
            });
            self.placements.len() - 1
        };
        Ok(&mut self.placements[index])
    }
    fn apply(&mut self, edit: NavigationEdit) -> Result<(), NavigationError> {
        match edit {
            NavigationEdit::SetSectionCollapsed { section, collapsed } => {
                if collapsed {
                    self.collapsed_sections |= section.bit();
                } else {
                    self.collapsed_sections &= !section.bit();
                }
            }
            NavigationEdit::UpsertGroup(group) => {
                if group.name.as_str().trim().is_empty()
                    || group.name.as_str().chars().any(char::is_control)
                {
                    return Err(NavigationError::InvalidGroup);
                }
                if let Some(current) = self.groups.iter_mut().find(|g| g.id == group.id) {
                    *current = group;
                } else {
                    if self.groups.len() == MAX_GROUPS {
                        return Err(NavigationError::Capacity);
                    }
                    self.groups.push(group);
                }
            }
            NavigationEdit::RemoveGroup(id) => {
                self.groups.retain(|g| g.id != id);
                for placement in &mut self.placements {
                    if placement.group == Some(id) {
                        placement.group = None;
                    }
                }
            }
            NavigationEdit::PlaceRoom { room, group, order } => {
                if group.is_some_and(|id| !self.groups.iter().any(|g| g.id == id)) {
                    return Err(NavigationError::MissingGroup);
                }
                let placement = self.placement(room)?;
                placement.group = group;
                placement.order = order;
            }
            NavigationEdit::SetFavorite { room, favorite } => {
                self.placement(room)?.favorite = favorite
            }
            NavigationEdit::RemoveRoom(room) => self.placements.retain(|p| p.room != room),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NavigationPreferences {
    accounts: Vec<AccountNavigation>,
}
impl NavigationPreferences {
    pub fn accounts(&self) -> &[AccountNavigation] {
        &self.accounts
    }
    pub fn account(&self, key: &NavigationAccountKey) -> Option<&AccountNavigation> {
        self.accounts.iter().find(|a| &a.account == key)
    }
    /// Pass the grouping/ordering or favorite capability matching the edit.
    /// At capacity existing entries remain editable; new entries fail without eviction.
    pub fn apply(
        &mut self,
        account: NavigationAccountKey,
        support: RemoteNavigationSupport,
        edit: NavigationEdit,
    ) -> Result<(), NavigationError> {
        support.local()?;
        if let Some(layout) = self.accounts.iter_mut().find(|a| a.account == account) {
            return layout.apply(edit);
        }
        if matches!(
            edit,
            NavigationEdit::RemoveRoom(_) | NavigationEdit::RemoveGroup(_)
        ) {
            return Ok(());
        }
        if self.accounts.len() == MAX_ACCOUNTS {
            return Err(NavigationError::Capacity);
        }
        let mut layout = AccountNavigation {
            account,
            collapsed_sections: 0,
            groups: Vec::new(),
            placements: Vec::new(),
        };
        layout.apply(edit)?;
        self.accounts.push(layout);
        Ok(())
    }
    /// Explicit account-forget only. Logout/runtime replacement does not erase layout.
    pub fn forget_account(&mut self, key: &NavigationAccountKey) {
        self.accounts.retain(|a| &a.account != key);
    }
}

/// Stable built-in sections, independent of localized labels or server room names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationSection {
    Personal,
    Channels,
    Conferences,
    Huddles,
    Other,
}
impl NavigationSection {
    pub const ALL: [Self; 5] = [
        Self::Personal,
        Self::Channels,
        Self::Conferences,
        Self::Huddles,
        Self::Other,
    ];
    pub fn bit(self) -> u8 {
        match self {
            Self::Personal => 1,
            Self::Channels => 2,
            Self::Conferences => 4,
            Self::Huddles => 8,
            Self::Other => 16,
        }
    }
}
