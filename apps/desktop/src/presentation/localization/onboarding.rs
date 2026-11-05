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
        "Das Laufwerk wird jetzt unter einer anderen Kennung geführt. Benutzerkonto und Datenordner stimmen überein. Wähle „Kontospeicher wieder verwenden“, um deine Konten wieder zu laden.",
        "The drive now has a different identifier. The user account and data folder match. Choose “Use account storage again” to reload your accounts.",
        "La unidad ahora tiene otro identificador. La cuenta de usuario y la carpeta de datos coinciden. Elige «Volver a usar el almacenamiento de cuentas» para volver a cargar tus cuentas.",
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
        "Der App-Datenordner liegt außerhalb des unterstützten lokalen Benutzerordners. Stelle den ursprünglichen Speicherort wieder her und lade die Konten erneut.",
        "The app data folder is outside the supported local user folder. Restore its original location and reload accounts.",
        "La carpeta de datos de la aplicación está fuera de la carpeta local de usuario compatible. Restaura su ubicación original y vuelve a cargar las cuentas.",
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
        "Die Kontodaten sind beschädigt oder haben ein unerwartetes Format. Stelle den App-Datenordner aus einer Sicherung wieder her. Ohne Sicherung benötigst du Hilfe zur Wiederherstellung.",
        "The account data is damaged or has an unexpected format. Restore the app data folder from a backup. Without a backup, you need help with recovery.",
        "Los datos de las cuentas están dañados o tienen un formato inesperado. Restaura la carpeta de datos de la aplicación desde una copia de seguridad. Sin una copia, necesitarás ayuda para recuperarlos.",
    ),
    (
        "Die lokalen Kontodaten passen nicht zu diesem App-Datenordner. Das kann nach Kopieren oder Wiederherstellen auftreten. Stelle den ursprünglichen Ordner wieder her und lade die Konten erneut.",
        "The local account data does not match this app data folder. This can happen after copying or restoring it. Restore the original folder and reload accounts.",
        "Los datos locales de las cuentas no coinciden con esta carpeta de datos de la aplicación. Esto puede ocurrir después de copiarla o restaurarla. Restaura la carpeta original y vuelve a cargar las cuentas.",
    ),
    (
        "Diese App-Version kann das Format der gespeicherten Konten nicht lesen. Verwende die zuletzt funktionierende oder eine neuere Version von Talk.",
        "This app version cannot read the saved account format. Use the last working version or a newer version of Talk.",
        "Esta versión de la aplicación no puede leer el formato de las cuentas guardadas. Usa la última versión que funcionaba o una versión más reciente de Talk.",
    ),
    (
        "Dieses Konto ist bereits auf diesem Gerät eingetragen. Öffne die Konten und verbinde es dort erneut.",
        "This account is already registered on this device. Open accounts and reconnect it there.",
        "Esta cuenta ya está registrada en este dispositivo. Abre las cuentas y vuelve a conectarla allí.",
    ),
    (
        "Ein früherer Speicherversuch ist noch ungeklärt. Öffne „Konten“, um den Speicherstatus zu prüfen, bevor du dich erneut anmeldest.",
        "An earlier save attempt is unresolved. Open Accounts to check the storage status before signing in again.",
        "Un intento anterior de guardado sigue sin resolverse. Abre Cuentas para comprobar el estado del almacenamiento antes de iniciar sesión de nuevo.",
    ),
    (
        "Eine andere Instanz von Talk verwendet die gespeicherten Konten. Beende sie und klicke auf „Konten erneut laden“.",
        "Another instance of Talk is using the saved accounts. Close it and click “Reload accounts”.",
        "Otra instancia de Talk está usando las cuentas guardadas. Ciérrala y pulsa «Volver a cargar las cuentas».",
    ),
    (
        "Gespeicherte Konten konnten nicht geladen werden.",
        "Could not load saved accounts.",
        "No se pudieron cargar las cuentas guardadas.",
    ),
    (
        "Gespeicherte Konten nicht erreichbar.",
        "Saved accounts are unavailable.",
        "Las cuentas guardadas no están disponibles.",
    ),
    (
        "Im Kontospeicher fehlt eine benötigte Datei. Stelle den vollständigen App-Datenordner aus einer Sicherung wieder her und lade die Konten erneut.",
        "A required file is missing from account storage. Restore the complete app data folder from a backup and reload accounts.",
        "Falta un archivo necesario en el almacenamiento de cuentas. Restaura la carpeta completa de datos de la aplicación desde una copia de seguridad y vuelve a cargar las cuentas.",
    ),
    (
        "Kein Zugriff auf gespeicherte Konten.",
        "Cannot access saved accounts.",
        "No se puede acceder a las cuentas guardadas.",
    ),
    (
        "Konten erneut laden",
        "Reload accounts",
        "Volver a cargar las cuentas",
    ),
    ("Konten öffnen", "Open accounts", "Abrir cuentas"),
    (
        "Konto bereits vorhanden.",
        "Account already exists.",
        "La cuenta ya existe.",
    ),
    (
        "Kontospeicher am falschen Speicherort.",
        "Account storage is in an unsupported location.",
        "El almacenamiento de cuentas está en una ubicación no compatible.",
    ),
    (
        "Kontospeicher benötigt eine andere App-Version.",
        "Account storage requires a different app version.",
        "El almacenamiento de cuentas requiere otra versión de la aplicación.",
    ),
    (
        "Kontospeicher gehört zu einem anderen Profil.",
        "Account storage belongs to a different profile.",
        "El almacenamiento de cuentas pertenece a otro perfil.",
    ),
    (
        "Kontospeicher muss neu zugeordnet werden.",
        "Account storage needs to be reassociated.",
        "Es necesario volver a asociar el almacenamiento de cuentas.",
    ),
    (
        "Kontospeicher wieder verwenden",
        "Use account storage again",
        "Volver a usar el almacenamiento de cuentas",
    ),
    (
        "Kontospeicher wird bereits verwendet.",
        "Account storage is already in use.",
        "El almacenamiento de cuentas ya está en uso.",
    ),
    (
        "Lokale Kontodaten konnten nicht gelesen werden.",
        "Could not read local account data.",
        "No se pudieron leer los datos locales de las cuentas.",
    ),
    (
        "Lokale Kontodaten sind unvollständig.",
        "Local account data is incomplete.",
        "Los datos locales de las cuentas están incompletos.",
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
        "Talk kann den lokalen Kontospeicher gerade nicht öffnen. Prüfe, ob dein Benutzerordner verfügbar und genügend Speicherplatz frei ist. Lade die Konten anschließend erneut.",
        "Talk cannot open local account storage right now. Check that your user folder is available and there is enough free disk space. Then reload accounts.",
        "Talk no puede abrir el almacenamiento local de cuentas en este momento. Comprueba que tu carpeta de usuario esté disponible y que haya suficiente espacio libre. Después vuelve a cargar las cuentas.",
    ),
    (
        "Talk kann den lokalen Kontospeicher wegen seiner Zugriffsrechte nicht öffnen. Prüfe die Rechte des App-Datenordners und lade die Konten erneut.",
        "Talk cannot open local account storage because of its access permissions. Check the app data folder permissions and reload accounts.",
        "Talk no puede abrir el almacenamiento local de cuentas debido a sus permisos de acceso. Revisa los permisos de la carpeta de datos de la aplicación y vuelve a cargar las cuentas.",
    ),
    (
        "Talk kann die lokalen Kontodaten nicht zuordnen und startet deshalb keine Anmeldung. Lade die Konten erneut. Bleibt der Fehler bestehen, benötigst du Hilfe zur Wiederherstellung.",
        "Talk cannot identify the local account data and cannot start sign-in. Reload accounts. If the error persists, you need help with recovery.",
        "Talk no puede identificar los datos locales de las cuentas y no puede iniciar el acceso. Vuelve a cargar las cuentas. Si el error persiste, necesitarás ayuda para recuperarlos.",
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
