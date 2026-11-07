//! Server-provided message translation copy.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Die Übersetzung ist gerade nicht verfügbar.",
        "Translation is unavailable right now.",
        "La traducción no está disponible en este momento.",
    ),
    (
        "Die Übersetzung wird bereits geladen.",
        "Translation is already in progress.",
        "La traducción ya está en curso.",
    ),
    ("Original anzeigen", "Show original", "Mostrar original"),
    ("Wird übersetzt …", "Translating …", "Traduciendo …"),
    ("Übersetzen", "Translate", "Traducir"),
    ("Übersetzung", "Translation", "Traducción"),
    (
        "Übersetzung anzeigen",
        "Show translation",
        "Mostrar traducción",
    ),
];
