pragma Singleton
import QtQuick
import PSHIDBridge

// The warnings that concern the whole app and need something done, with a
// key each: AppWarnings shows them (each page in full the ones that belong
// to it, the rest in one line), the sidebar marks Status with their number.
QtObject {
    // Selected devices the app may not read: from the device list (also
    // while the bridge service does not run) and from the service (a device
    // plugged in since the list was made).
    readonly property int selectedBlocked: Math.max(
        AppController.devicesBlocked,
        JSON.parse(AppController.devicesJson).filter(g => g.selected && !g.accessible).length)
    readonly property int keyboardsBlocked: AppController.otherKeyboardsBlocked
    readonly property bool access: selectedBlocked > 0 || keyboardsBlocked > 0 || AppController.accessMessage !== ""
    readonly property bool adapter: AppController.adapterMode === "delaying"

    /// The active ones, in the order of the banners: {key, level, name}
    /// (name: a few words for the line that sums them up).
    readonly property var list: [
        AppController.lastError !== "" ? { key: "error", level: "bad", name: I18n.tr("Error") } : null,
        access ? { key: "access", level: "warn", name: I18n.tr("Device access") } : null,
        AppController.boardWarning !== "" ? { key: "board", level: "warn", name: I18n.tr("Board refuses packets") } : null,
        AppController.counterError !== "" ? { key: "counter", level: "warn", name: I18n.tr("Packet counter") } : null,
        adapter ? { key: "adapter", level: "warn", name: I18n.tr("Network adapter") } : null
    ].filter(w => w !== null)
    readonly property int count: list.length
    /// "bad" with an error among them, else "warn" ("" without any).
    readonly property string level: list.some(w => w.level === "bad") ? "bad" : count > 0 ? "warn" : ""

    function active(key) {
        return list.some(w => w.key === key);
    }
}
