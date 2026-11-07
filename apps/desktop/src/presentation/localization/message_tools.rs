//! Message tools: own interface copy, sorted by German source key.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Beginn entfernen",
        "Remove start time",
        "Quitar la hora de inicio",
    ),
    (
        "Beginn festlegen",
        "Set start time",
        "Establecer hora de inicio",
    ),
    ("Beginn: {time}", "Starts: {time}", "Comienza: {time}"),
    (
        "Dieser Link kann gerade nicht geöffnet werden.",
        "This link cannot be opened right now.",
        "Este enlace no se puede abrir ahora.",
    ),
    ("Geplant ({n})", "Scheduled ({n})", "Programados ({n})"),
    ("In 20 Minuten", "In 20 minutes", "Dentro de 20 minutos"),
    ("In 3 Stunden", "In 3 hours", "Dentro de 3 horas"),
    ("In Karte öffnen", "Open in map", "Abrir en el mapa"),
    ("Senden fehlgeschlagen", "Sending failed", "Error al enviar"),
    ("Später senden", "Send later", "Enviar más tarde"),
    ("Zeit speichern", "Save time", "Guardar hora"),
    (
        "Zum Planen eine Nachricht ohne Dateien verfassen.",
        "Write a message without files to schedule it.",
        "Escribe un mensaje sin archivos para programarlo.",
    ),
];
