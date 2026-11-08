//! Application-shell status and navigation copy: connection state, Activity, list menus and
//! room-level access states. Keys are sorted for binary search.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    ("Aktivität", "Activity", "Actividad"),
    ("Alles gelesen.", "All caught up.", "Todo al día."),
    ("Ansehen", "View", "Ver"),
    ("Anzeigen", "Show", "Mostrar"),
    (
        "Dein Zugang wurde möglicherweise geändert oder der Link ist nicht mehr gültig.",
        "Your access may have changed, or the link is no longer valid.",
        "Es posible que tu acceso haya cambiado o que el enlace ya no sea válido.",
    ),
    (
        "Der bestätigte Stand bleibt sichtbar. Neue Inhalte erscheinen nach der Verbindung.",
        "The last confirmed state stays visible. New content appears once you are connected.",
        "El último estado confirmado sigue visible. El contenido nuevo aparecerá al conectarte.",
    ),
    (
        "Der bestätigte Stand bleibt sichtbar. Talk versucht es automatisch erneut.",
        "The last confirmed state stays visible. Talk keeps retrying automatically.",
        "El último estado confirmado sigue visible. Talk lo reintenta automáticamente.",
    ),
    (
        "Die Nachrichten erscheinen, sobald du wieder online bist.",
        "Messages appear once you are back online.",
        "Los mensajes aparecen cuando vuelvas a conectarte.",
    ),
    (
        "Du kannst hier nicht schreiben.",
        "You can't write here.",
        "No puedes escribir aquí.",
    ),
    (
        "Du kannst hier nicht schreiben. Dein Entwurf bleibt erhalten.",
        "You can't write here. Your draft is kept.",
        "No puedes escribir aquí. Tu borrador se conserva.",
    ),
    (
        "Einladungen von anderen Servern",
        "Invitations from other servers",
        "Invitaciones de otros servidores",
    ),
    ("Getrennt seit", "Offline for", "Sin conexión desde hace"),
    (
        "Hier noch nicht verfügbar",
        "Not available here yet",
        "Aún no disponible aquí",
    ),
    (
        "Jetzt erneut verbinden",
        "Reconnect now",
        "Reconectar ahora",
    ),
    (
        "Keine neuen Erwähnungen.",
        "No new mentions.",
        "Sin menciones nuevas.",
    ),
    ("Liste anpassen", "List options", "Opciones de la lista"),
    (
        "Nachrichten konnten nicht geladen werden.",
        "Messages could not be loaded.",
        "No se pudieron cargar los mensajes.",
    ),
    (
        "Neues Gespräch oder Verzeichnis",
        "New conversation or directory",
        "Nueva conversación o directorio",
    ),
    (
        "Noch keine Verbindung",
        "Not connected yet",
        "Aún sin conexión",
    ),
    ("Nur Lesezugriff", "Read-only", "Solo lectura"),
    ("Nur Ungelesene", "Unread only", "Solo no leídas"),
    (
        "Offene Gespräche durchsuchen",
        "Browse open conversations",
        "Explorar conversaciones abiertas",
    ),
    (
        "Prüfe deine Verbindung und versuche es erneut.",
        "Check your connection and try again.",
        "Revisa tu conexión e inténtalo de nuevo.",
    ),
    (
        "Talk versucht es automatisch erneut.",
        "Talk keeps retrying automatically.",
        "Talk lo reintenta automáticamente.",
    ),
    (
        "Ungelesene Gespräche erscheinen hier.",
        "Unread conversations show up here.",
        "Las conversaciones no leídas aparecen aquí.",
    ),
    ("Verbinde …", "Connecting …", "Conectando …"),
    (
        "Verbindung wird wiederhergestellt",
        "Reconnecting",
        "Restableciendo la conexión",
    ),
    ("Wartet auf Senden", "Waiting to send", "Pendiente de envío"),
    (
        "Wenn dich jemand erwähnt, erscheint es hier.",
        "When someone mentions you, it shows up here.",
        "Cuando alguien te mencione, aparecerá aquí.",
    ),
    (
        "weniger als einer Minute",
        "less than a minute",
        "menos de un minuto",
    ),
    (
        "{n} Einladung wartet auf deine Antwort.",
        "{n} invitation is waiting for your reply.",
        "{n} invitación espera tu respuesta.",
    ),
    (
        "{n} Einladungen warten auf deine Antwort.",
        "{n} invitations are waiting for your reply.",
        "{n} invitaciones esperan tu respuesta.",
    ),
    ("{n} Minute", "{n} minute", "{n} minuto"),
    ("{n} Minuten", "{n} minutes", "{n} minutos"),
    ("{n} Änderung", "{n} change", "{n} cambio"),
    ("{n} Änderungen", "{n} changes", "{n} cambios"),
    (
        "Änderungen werden gesendet, sobald du wieder online bist.",
        "Changes will be sent when you are back online.",
        "Los cambios se enviarán cuando vuelvas a conectarte.",
    ),
];
