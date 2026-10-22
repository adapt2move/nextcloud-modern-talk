//! Account connection failures shared by login, accounts and the composer.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Dein Entwurf bleibt erhalten. Nachrichten werden nicht automatisch gesendet.",
        "Your draft is preserved. Messages will not be sent automatically.",
        "Se conserva tu borrador. Los mensajes no se enviarán automáticamente.",
    ),
    (
        "Der Server begrenzt die Anfragen. Bitte warte und versuche es später erneut.",
        "The server is limiting requests. Wait and try again later.",
        "El servidor está limitando las solicitudes. Espera y vuelve a intentarlo más tarde.",
    ),
    (
        "Der Server hat die Anfrage mit HTTP 403 abgelehnt. Prüfe den Serverzugriff.",
        "The server refused the request with HTTP 403. Check access to the server.",
        "El servidor rechazó la solicitud con HTTP 403. Revisa el acceso al servidor.",
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
