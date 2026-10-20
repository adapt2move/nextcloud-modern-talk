//! Device-local update progress and explicit installation action.
pub const MESSAGES: &[(&str, &str, &str)] = &[
    ("App beenden", "Quit app", "Salir de la aplicación"),
    (
        "Beende die App und öffne sie erneut, bevor du weiterarbeitest.",
        "Quit and reopen the app before continuing.",
        "Cierra la aplicación y vuelve a abrirla antes de continuar.",
    ),
    (
        "Beende zuerst den Anruf, um das Update zu installieren.",
        "End the call before installing the update.",
        "Finaliza la llamada antes de instalar la actualización.",
    ),
    (
        "Das Update ist installiert. Der Neustart wird vorbereitet …",
        "The update is installed. Preparing to restart…",
        "La actualización está instalada. Preparando el reinicio…",
    ),
    (
        "Das Update ist zur Installation bereit.",
        "The update is ready to install.",
        "La actualización está lista para instalar.",
    ),
    (
        "Das Update konnte nicht abgeschlossen werden.",
        "The update could not be completed.",
        "No se pudo completar la actualización.",
    ),
    (
        "Das Update konnte nicht auf diesem Computer gespeichert werden.",
        "The update could not be saved on this computer.",
        "No se pudo guardar la actualización en este equipo.",
    ),
    (
        "Das Update konnte nicht installiert werden.",
        "The update could not be installed.",
        "No se pudo instalar la actualización.",
    ),
    (
        "Das Update konnte nicht verifiziert werden.",
        "The update could not be verified.",
        "No se pudo verificar la actualización.",
    ),
    (
        "Das Update wird geprüft …",
        "Verifying the update…",
        "Verificando la actualización…",
    ),
    (
        "Das Update wird heruntergeladen …",
        "Downloading the update…",
        "Descargando la actualización…",
    ),
    (
        "Das Update wird installiert …",
        "Installing the update…",
        "Instalando la actualización…",
    ),
    (
        "Das Update überschreitet die unterstützte Größe.",
        "The update exceeds the supported size.",
        "La actualización supera el tamaño admitido.",
    ),
    (
        "Das Updatepaket hat ein ungültiges Format.",
        "The update package has an invalid format.",
        "El paquete de actualización tiene un formato no válido.",
    ),
    (
        "Der Installationsstatus ist unklar. Prüfe die installierte Version.",
        "The installation status is uncertain. Check the installed version.",
        "El estado de la instalación es incierto. Comprueba la versión instalada.",
    ),
    (
        "Der Paketmanager hat die Installation nicht bestätigt. Beende die App und prüfe die Installation, bevor du sie erneut öffnest.",
        "The package manager did not confirm installation. Quit the app and check the installation before reopening it.",
        "El gestor de paquetes no confirmó la instalación. Cierra la aplicación y comprueba la instalación antes de volver a abrirla.",
    ),
    (
        "Der Updatedienst wurde beendet.",
        "The update service has stopped.",
        "El servicio de actualizaciones se ha detenido.",
    ),
    (
        "Die App bereitet die Installation vor …",
        "The app is preparing the installation…",
        "La aplicación está preparando la instalación…",
    ),
    (
        "Die Paketverwaltung ist nicht verfügbar.",
        "The package manager is unavailable.",
        "El gestor de paquetes no está disponible.",
    ),
    (
        "Die Paketverwaltung konnte das Update nicht installieren.",
        "The package manager could not install the update.",
        "El gestor de paquetes no pudo instalar la actualización.",
    ),
    (
        "Die Updateaktion wurde abgebrochen.",
        "The update action was cancelled.",
        "La acción de actualización se ha cancelado.",
    ),
    (
        "Die Updateauswahl hat sich geändert. Bitte erneut prüfen.",
        "The selected update has changed. Check again.",
        "La actualización seleccionada ha cambiado. Comprueba de nuevo.",
    ),
    (
        "Die vorherige Version wurde geprüft und kann erneut gestartet werden.",
        "The previous version was verified and can be started again.",
        "Se ha verificado la versión anterior y se puede iniciar de nuevo.",
    ),
    (
        "Diese Installation unterstützt keine automatischen Updates.",
        "This installation does not support automatic updates.",
        "Esta instalación no admite actualizaciones automáticas.",
    ),
    (
        "Eine Updateaktion läuft bereits.",
        "An update action is already running.",
        "Ya hay una acción de actualización en curso.",
    ),
    (
        "Schließe ungesendete Nachrichten vor dem Neustart ab oder leere sie.",
        "Finish or clear unsent messages before restarting.",
        "Termina o borra los mensajes sin enviar antes de reiniciar.",
    ),
    ("Update", "Update", "Actualizar"),
    (
        "Update installieren und neu starten",
        "Install update and restart",
        "Instalar la actualización y reiniciar",
    ),
    (
        "Updates werden automatisch für den gewählten Kanal geprüft und heruntergeladen. Du entscheidest, wann installiert und neu gestartet wird.",
        "Updates for the selected channel are checked and downloaded automatically. You choose when to install and restart.",
        "Las actualizaciones del canal seleccionado se buscan y descargan automáticamente. Tú decides cuándo instalarlas y reiniciar.",
    ),
    (
        "Vorherige Version starten",
        "Start previous version",
        "Iniciar la versión anterior",
    ),
    (
        "{received} von {total} MiB heruntergeladen.",
        "{received} of {total} MiB downloaded.",
        "{received} de {total} MiB descargados.",
    ),
];
