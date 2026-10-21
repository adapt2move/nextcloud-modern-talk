//! Setup and secure-storage recovery copy: German, English, Spanish.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "Bereit für deine Gespräche.",
        "Ready for your conversations.",
        "Todo listo para tus conversaciones.",
    ),
    (
        "Bestätige die Anfrage von macOS, wenn sie erscheint. Nach der Freigabe geht es automatisch weiter.",
        "Confirm the macOS request when it appears. Setup continues automatically after access is allowed.",
        "Confirma la solicitud de macOS cuando aparezca. La configuración continúa automáticamente al permitir el acceso.",
    ),
    (
        "Das sichere Speichern wurde abgebrochen. Bitte erneut versuchen oder die private Datei wählen.",
        "Secure saving was cancelled. Retry or choose the private file.",
        "Se canceló el guardado seguro. Reintenta o elige el archivo privado.",
    ),
    (
        "Dein Konto ist verbunden. Optionale Freigaben kannst du jederzeit in den Einstellungen ändern.",
        "Your account is connected. You can change optional permissions in Settings at any time.",
        "Tu cuenta está conectada. Puedes cambiar los permisos opcionales en Ajustes cuando quieras.",
    ),
    (
        "Deine Anmeldung wird dauerhaft auf diesem Gerät gespeichert.",
        "Your sign-in is being saved on this device.",
        "Se está guardando tu inicio de sesión en este dispositivo.",
    ),
    (
        "Der Einrichtungsfortschritt konnte nicht gespeichert werden. Bitte erneut versuchen.",
        "Setup progress could not be saved. Please try again.",
        "No se pudo guardar el progreso de la configuración. Vuelve a intentarlo.",
    ),
    (
        "Der Schlüsselspeicher ist vorübergehend nicht erreichbar. Bitte erneut versuchen oder die private Datei wählen.",
        "The secure store is temporarily unavailable. Retry or choose the private file.",
        "El almacén seguro no está disponible temporalmente. Reintenta o elige el archivo privado.",
    ),
    (
        "Der Schlüsselspeicher meldet beschädigte Daten. Bitte repariere ihn oder wähle die private Datei.",
        "The secure store reports damaged data. Repair it or choose the private file.",
        "El almacén seguro informa de datos dañados. Repáralo o elige el archivo privado.",
    ),
    (
        "Der Speichervorgang ist noch nicht bestätigt und kann noch abgeschlossen werden. Bitte warte kurz und versuche es erneut.",
        "Saving has not been confirmed and may still finish. Wait a moment and try again.",
        "El guardado no se ha confirmado y aún puede completarse. Espera un momento y vuelve a intentarlo.",
    ),
    (
        "Der Speicherzustand hat sich geändert. Bitte erneut versuchen.",
        "The storage state changed. Please try again.",
        "El estado del almacenamiento ha cambiado. Vuelve a intentarlo.",
    ),
    (
        "Der Zugriff auf den lokalen Speicher wurde verweigert. Prüfe die Dateirechte und versuche es erneut.",
        "Access to local storage was denied. Check file permissions and try again.",
        "Se denegó el acceso al almacenamiento local. Revisa los permisos de los archivos y reinténtalo.",
    ),
    (
        "Der lokale Kontozustand konnte nicht sicher bestätigt werden. Deine Anmeldung gilt noch nicht als gespeichert.",
        "The local account state could not be safely confirmed. Your sign-in is not yet confirmed as saved.",
        "No se pudo confirmar con seguridad el estado local de la cuenta. Aún no se ha confirmado que tu inicio de sesión esté guardado.",
    ),
    (
        "Der lokale Speicher ist beschädigt oder unsicher. Repariere ihn, bevor du die Anmeldung speicherst.",
        "Local storage is damaged or unsafe. Repair it before saving the sign-in.",
        "El almacenamiento local está dañado o no es seguro. Repáralo antes de guardar el inicio de sesión.",
    ),
    (
        "Der lokale Speicher ist nicht verfügbar. Prüfe den Speicherort und versuche es erneut.",
        "Local storage is unavailable. Check the storage location and try again.",
        "El almacenamiento local no está disponible. Revisa su ubicación y reinténtalo.",
    ),
    (
        "Die Abmeldung konnte noch nicht dauerhaft bestätigt werden. Der lokale Speicherstatus muss zuerst geklärt werden.",
        "Sign-out could not yet be confirmed as persistent. The local storage state must be resolved first.",
        "Aún no se pudo confirmar que el cierre de sesión sea permanente. Primero debe resolverse el estado del almacenamiento local.",
    ),
    (
        "Die Anmeldung konnte nicht sicher gespeichert werden. Bitte erneut versuchen oder die private Datei wählen.",
        "The sign-in could not be saved securely. Retry or choose the private file.",
        "No se pudo guardar el inicio de sesión de forma segura. Reintenta o elige el archivo privado.",
    ),
    (
        "Dieses Konto ist bereits auf diesem Gerät eingetragen. Öffne die Konten und verbinde es dort erneut.",
        "This account is already registered on this device. Open accounts and reconnect it there.",
        "Esta cuenta ya está registrada en este dispositivo. Abre las cuentas y vuelve a conectarla allí.",
    ),
    (
        "Einrichtungsfortschritt wird gespeichert …",
        "Saving setup progress…",
        "Guardando el progreso de la configuración…",
    ),
    ("Konten öffnen", "Open accounts", "Abrir cuentas"),
    (
        "Konto bereits vorhanden.",
        "Account already exists.",
        "La cuenta ya existe.",
    ),
    (
        "Sicheres Speichern erneut versuchen",
        "Retry secure saving",
        "Reintentar el guardado seguro",
    ),
    (
        "Sicheres Speichern nicht möglich.",
        "Secure saving is unavailable.",
        "El guardado seguro no está disponible.",
    ),
    (
        "Weiter ohne Freigabe",
        "Continue without permission",
        "Continuar sin permiso",
    ),
    (
        "Windows verwaltet den Gerätezugriff in „Datenschutz & Sicherheit“. Bereits erlaubter Zugriff wird automatisch erkannt.",
        "Windows manages device access in Privacy & security. Access already allowed is detected automatically.",
        "Windows gestiona el acceso a los dispositivos en Privacidad y seguridad. El acceso ya permitido se detecta automáticamente.",
    ),
    (
        "macOS fragt nach der Bildschirmfreigabe. Falls es einen Neustart verlangt, geht es danach hier weiter.",
        "macOS requests screen sharing access. If it requires a restart, setup resumes here afterwards.",
        "macOS solicita acceso para compartir la pantalla. Si requiere reiniciar, la configuración continúa aquí después.",
    ),
    (
        "macOS hat die Freigabe bestätigt. Starte die App neu, damit dieser Prozess sie verwenden kann.",
        "macOS confirmed access. Restart the app so this process can use it.",
        "macOS confirmó el acceso. Reinicia la aplicación para que este proceso pueda utilizarlo.",
    ),
];
