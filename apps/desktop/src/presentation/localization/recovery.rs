//! Shared collection recovery copy; never server or user content.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Aktualisierung fehlgeschlagen.",
        "Refresh failed.",
        "No se pudo actualizar.",
    ),
    (
        "Beende zuerst die aktuelle Bearbeitung.",
        "Finish the current edit first.",
        "Termina primero la edición actual.",
    ),
    (
        "Bitte warte einen Moment.",
        "Please wait a moment.",
        "Espera un momento.",
    ),
    (
        "Deine Berechtigungen können sich geändert haben. Prüfe den Zugriff erneut.",
        "Your permissions may have changed. Check access again.",
        "Es posible que tus permisos hayan cambiado. Vuelve a comprobar el acceso.",
    ),
    (
        "Der bestätigte Stand bleibt sichtbar.",
        "Previously loaded content stays visible.",
        "El contenido cargado anteriormente sigue visible.",
    ),
    (
        "Der bestätigte Stand bleibt sichtbar. Bitte erneut versuchen.",
        "Previously loaded content stays visible. Please try again.",
        "El contenido cargado anteriormente sigue visible. Vuelve a intentarlo.",
    ),
    (
        "Der bestätigte Stand bleibt sichtbar. Verbinde dich erneut und lade die Inhalte neu.",
        "Previously loaded content stays visible. Reconnect and reload the content.",
        "El contenido cargado anteriormente sigue visible. Vuelve a conectarte y recarga el contenido.",
    ),
    (
        "Inhalte konnten nicht geladen werden. Bitte erneut versuchen.",
        "Content could not be loaded. Please try again.",
        "No se pudo cargar el contenido. Vuelve a intentarlo.",
    ),
    (
        "Inhalte konnten noch nicht geladen werden. Prüfe deine Verbindung und versuche es erneut.",
        "Content has not been loaded yet. Check your connection and try again.",
        "Todavía no se ha cargado el contenido. Comprueba la conexión y vuelve a intentarlo.",
    ),
    ("Kein Zugriff.", "No access.", "Sin acceso."),
    ("Wird aktualisiert …", "Refreshing…", "Actualizando…"),
    ("Wird geladen …", "Loading…", "Cargando…"),
];
