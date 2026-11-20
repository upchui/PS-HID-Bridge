import QtQuick
import QtQuick.Layouts
import PSHIDBridge

// Warnings that concern the whole app and need something done: on top of
// every page (AppPage), each once. Messages about something done on one
// page stay on that page.
ColumnLayout {
    id: warnings

    // Selected devices the app may not read: from the device list (also
    // while the bridge service does not run) and from the service (a device
    // plugged in since the list was made).
    readonly property int selectedBlocked: Math.max(
        AppController.devicesBlocked,
        JSON.parse(AppController.devicesJson).filter(g => g.selected && !g.accessible).length)
    readonly property int keyboardsBlocked: AppController.otherKeyboardsBlocked
    readonly property bool access: selectedBlocked > 0 || keyboardsBlocked > 0 || AppController.accessMessage !== ""
    readonly property bool adapter: AppController.adapterMode === "delaying"
    // Something to show (empty, the column takes no room on the page).
    readonly property bool any: access || adapter || AppController.lastError !== ""
                                || AppController.boardWarning !== "" || AppController.counterError !== ""

    spacing: Style.gap

    Banner {
        kind: "error"
        visible: AppController.lastError !== ""
        text: AppController.lastError
        actions: AppButton {
            text: I18n.tr("Dismiss")
            flat: true
            onClicked: AppController.lastError = ""
        }
    }

    // Device access: selected devices (their input) and other keyboards
    // (triggers and the emergency stop work there too), one setup for both.
    Banner {
        kind: "warn"
        visible: warnings.selectedBlocked > 0 || warnings.keyboardsBlocked > 0
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
        visible: AppController.accessMessage !== ""
        text: AppController.accessMessage
    }

    Banner {
        kind: "warn"
        visible: AppController.boardWarning !== ""
        text: AppController.boardWarning
        actions: AppButton {
            text: I18n.tr("Dismiss")
            flat: true
            onClicked: AppController.dismissBoardWarning()
        }
    }
    Banner {
        kind: "warn"
        visible: AppController.counterError !== ""
        text: I18n.trf("The packet counter cannot be saved (%1). Input still works; "
                       + "free up disk space or fix the folder's permissions, the app keeps trying. Until then, a restart "
                       + "with the computer's clock set back could reuse values that protect your input.", AppController.counterError)
    }
    Banner {
        kind: "warn"
        visible: warnings.adapter
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
