//! Account connection failures shared by login, accounts and the composer.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Auf diesem Server nicht verfügbar.",
        "This feature is unavailable on this server.",
        "Esta función no está disponible en este servidor.",
    ),
    (
        "Dein Entwurf bleibt erhalten. Nachrichten werden nicht automatisch gesendet.",
        "Your draft is preserved. Messages will not be sent automatically.",
        "Se conserva tu borrador. Los mensajes no se enviarán automáticamente.",
    ),
    (
        "Der Server begrenzt die Anfragen. Bitte später erneut versuchen.",
        "The server is limiting requests. Please try again later.",
        "El servidor está limitando las solicitudes. Vuelve a intentarlo más tarde.",
    ),
    (
        "Der Server meldet einen Fehler. Bitte versuche es später erneut.",
        "The server reported an error. Please try again later.",
        "El servidor indicó un error. Vuelve a intentarlo más tarde.",
    ),
    (
        "Die Serverantwort konnte nicht verarbeitet werden. Prüfe die Serveradresse oder versuche es später erneut.",
        "The server response could not be processed. Check the server address or try again later.",
        "No se pudo procesar la respuesta del servidor. Revisa su dirección o vuelve a intentarlo más tarde.",
    ),
    (
        "Die Verbindung hat zu lange gedauert. Bitte versuche es später erneut.",
        "The connection timed out. Please try again later.",
        "Se agotó el tiempo de espera de la conexión. Vuelve a intentarlo más tarde.",
    ),
    (
        "Die Verbindung zum Server ist fehlgeschlagen. Prüfe Netzwerk, VPN und Serveradresse.",
        "The connection to the server failed. Check your network, VPN and server address.",
        "No se pudo conectar con el servidor. Revisa la red, la VPN y la dirección del servidor.",
    ),
    (
        "Dieser Entwurf bleibt nur für diese Sitzung erhalten.",
        "This draft is kept for this session only.",
        "Este borrador se conserva solo durante esta sesión.",
    ),
    (
        "Offline – Änderungen werden gesendet, sobald du wieder online bist",
        "Offline – changes will be sent when you are back online",
        "Sin conexión – los cambios se enviarán cuando vuelvas a conectarte",
    ),
    (
        "Prüfe, ob du auf diesen Server zugreifen darfst.",
        "Check whether you have access to this server.",
        "Comprueba si tienes acceso a este servidor.",
    ),
    (
        "Server antwortet nicht rechtzeitig.",
        "The server did not respond in time.",
        "El servidor no respondió a tiempo.",
    ),
    (
        "Server verweigert den Zugriff.",
        "The server denied access.",
        "El servidor denegó el acceso.",
    ),
    (
        "Server vorübergehend nicht verfügbar.",
        "Server temporarily unavailable.",
        "Servidor no disponible temporalmente.",
    ),
    (
        "Unerwartete Serverantwort.",
        "Unexpected server response.",
        "Respuesta inesperada del servidor.",
    ),
    (
        "Zu viele Anfragen.",
        "Too many requests.",
        "Demasiadas solicitudes.",
    ),
];
