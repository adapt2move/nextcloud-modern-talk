//! One inspectable key registry. Presentation uses the same entries to dispatch;
//! no configurable value is persisted until a real rebind mechanism exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionId {
    OpenSettings,
    Search,
    ConversationSearch,
    Close,
    SendMessage,
    NewLine,
    NextControl,
    PreviousControl,
    Undo,
    Redo,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyContext {
    Application,
    Composer,
    TextInput,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Modifiers {
    None,
    Shift,
    Primary,
    PrimaryShift,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub action: ActionId,
    pub key: &'static str,
    pub modifiers: Modifiers,
    pub context: KeyContext,
}
pub const SHORTCUTS: [Shortcut; 10] = [
    Shortcut {
        action: ActionId::OpenSettings,
        key: ",",
        modifiers: Modifiers::Primary,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::Search,
        key: "k",
        modifiers: Modifiers::Primary,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::ConversationSearch,
        key: "f",
        modifiers: Modifiers::Primary,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::Close,
        key: "escape",
        modifiers: Modifiers::None,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::SendMessage,
        key: "enter",
        modifiers: Modifiers::None,
        context: KeyContext::Composer,
    },
    Shortcut {
        action: ActionId::NewLine,
        key: "enter",
        modifiers: Modifiers::Shift,
        context: KeyContext::Composer,
    },
    Shortcut {
        action: ActionId::NextControl,
        key: "tab",
        modifiers: Modifiers::None,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::PreviousControl,
        key: "tab",
        modifiers: Modifiers::Shift,
        context: KeyContext::Application,
    },
    Shortcut {
        action: ActionId::Undo,
        key: "z",
        modifiers: Modifiers::Primary,
        context: KeyContext::TextInput,
    },
    Shortcut {
        action: ActionId::Redo,
        key: "z",
        modifiers: Modifiers::PrimaryShift,
        context: KeyContext::TextInput,
    },
];
pub fn matching(key: &str, modifiers: Modifiers, context: KeyContext) -> Option<ActionId> {
    SHORTCUTS
        .iter()
        .find(|binding| {
            binding.key == key && binding.modifiers == modifiers && binding.context == context
        })
        .map(|binding| binding.action)
}
