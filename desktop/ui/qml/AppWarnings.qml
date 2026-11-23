import QtQuick
import QtQuick.Layouts
import PSHIDBridge

// Warnings that concern the whole app and need something done (Warnings),
// on top of every page (AppPage), each once. A page shows the ones that
// belong to it (`own`) in full, errors always; the others are summed up in
// one line that opens them. Messages about something done on one page stay
// on that page.
ColumnLayout {
    id: warnings

    /// Keys (Warnings.list) shown in full on this page.
    property var own: []
    /// The summed-up ones opened.
    property bool expanded: false

    readonly property int selectedBlocked: Warnings.selectedBlocked
    readonly property int keyboardsBlocked: Warnings.keyboardsBlocked
    // The ones in the line.
    readonly property var folded: Warnings.list.filter(w => w.key !== "error" && own.indexOf(w.key) < 0)
    // Something to show (empty, the column takes no room on the page).
    readonly property bool any: Warnings.count > 0

    function shows(key) {
        return Warnings.active(key) && (key === "error" || own.indexOf(key) >= 0 || expanded);
    }

    spacing: Style.gap

    Banner {
        kind: "error"
        visible: warnings.shows("error")
        text: AppController.lastError
        actions: AppButton {
            text: I18n.tr("Dismiss")
            flat: true
            onClicked: AppController.lastError = ""
        }
    }

    Banner {
        kind: "warn"
        visible: warnings.folded.length > 0
        oneLine: true
        text: warnings.folded.length === 1
              ? I18n.trf("Notice: %1", warnings.folded[0].name)
              : I18n.trf("%1 notices: %2", warnings.folded.length, warnings.folded.map(w => w.name).join(" · "))
        actions: AppButton {
            text: warnings.expanded ? I18n.tr("Hide") : I18n.tr("Show")
            flat: true
            onClicked: warnings.expanded = !warnings.expanded
        }
    }

    // Device access: selected devices (their input) and other keyboards
    // (triggers and the emergency stop work there too), one setup for both.
    Banner {
        kind: "warn"
        visible: warnings.shows("access") && (warnings.selectedBlocked > 0 || warnings.keyboardsBlocked > 0)
        text: warnings.selectedBlocked > 0 && warnings.keyboardsBlocked > 0
              ? I18n.trf("%1 selected devices and %2 other keyboards (for triggers and the emergency stop) cannot be read yet. The app sets up access once (administrator password).",
                         warnings.selectedBlocked, warnings.keyboardsBlocked)
              : warnings.selectedBlocked === 1
              ? I18n.tr("1 selected device cannot be read yet. The app sets up access once (administrator password).")
              : warnings.selectedBlocked > 1
              ? I18n.trf("%1 selected devices cannot be read yet. The app sets up access once (administrator password).",
                         warnings.selectedBlocked)
              : warnings.keyboardsBlocked === 1
              ? I18n.tr("Triggers and the emergency stop work on every keyboard, also on ones that are not selected. "
                        + "1 keyboard cannot be read yet; the app sets up access once (administrator password).")
              : I18n.trf("Triggers and the emergency stop work on every keyboard, also on ones that are not selected. "
                         + "%1 keyboards cannot be read yet; the app sets up access once (administrator password).",
                         warnings.keyboardsBlocked)
        actions: AppButton {
            text: AppController.accessBusy ? I18n.tr("Setting up …") : I18n.tr("Set up access")
            enabled: !AppController.accessBusy
            onClicked: AppController.installAccess()
        }
    }
    Banner {
        kind: "warn"
        visible: warnings.shows("access") && AppController.accessMessage !== ""
        text: AppController.accessMessage
    }

    Banner {
        kind: "warn"
        visible: warnings.shows("board")
        text: AppController.boardWarning
        actions: AppButton {
            text: I18n.tr("Dismiss")
            flat: true
            onClicked: AppController.dismissBoardWarning()
        }
    }
    Banner {
        kind: "warn"
        visible: warnings.shows("counter")
        text: I18n.trf("The packet counter cannot be saved (%1). Input still works; "
                       + "free up disk space or fix the folder's permissions, the app keeps trying. Until then, a restart "
                       + "with the computer's clock set back could reuse values that protect your input.", AppController.counterError)
    }
    Banner {
        kind: "warn"
        visible: warnings.shows("adapter")
        text: I18n.trf("The network adapter %1 holds every packet back up to about %2 before sending it, to bundle packets (USB adapter, cdc_ncm). "
                       + "This comes on top of “Input → network” and shows in the round trip. "
                       + "Set up once (administrator password); after that the adapter sends at once while the app runs and bundles again when it is closed, without a password.",
                       AppController.adapterIface, Style.us(AppController.adapterDelayMs * 1000))
              + (AppController.latencySetupMessage !== "" ? "\n" + AppController.latencySetupMessage : "")
        actions: AppButton {
            text: AppController.latencySetupBusy ? I18n.tr("Setting up …") : I18n.tr("Set up")
            enabled: !AppController.latencySetupBusy
            onClicked: AppController.setupLowLatency()
            Accessible.description: I18n.tr("Asks for the administrator password")
        }
    }
}
