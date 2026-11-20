import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Started as an AppImage on a system with Shelly (CachyOS): install it
// there, start the installed copy instead, or restart from it after
// installing. The app keeps updating itself either way. Looks like a
// Banner, with its buttons below the text (they wrap in a narrow window).
Rectangle {
    id: banner

    readonly property string offer: AppController.installOffer
    readonly property string level: AppController.installLevel === "bad" ? "warn"
                                    : offer === "restart-installed" ? "ok" : "accent"
    readonly property string text: AppController.installBusy ? I18n.tr("Installing with Shelly …")
          : AppController.installMessage !== "" && offer !== "restart-installed" ? AppController.installMessage
          : offer === "install"
          ? I18n.trf("PS HID Bridge runs from %1. Install it with Shelly? It is then in the application menu and keeps updating itself.", AppController.installFolder)
          : offer === "start-installed"
          ? I18n.trf("PS HID Bridge is installed already (version %1). Start the installed one instead of this file?", AppController.installedVersion)
          : offer === "restart-installed"
          ? I18n.trf("%1 Restart from the installed copy now?", AppController.installMessage)
          : ""

    visible: text !== ""
    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 20
    radius: Style.radius
    color: Style.tint(Style.stateColor(level), 0.12)
    border.color: Style.tint(Style.stateColor(level), 0.45)

    RowLayout {
        anchors.fill: parent
        anchors.margins: 10
        anchors.leftMargin: 14
        spacing: 10

        AppIcon {
            Layout.alignment: Qt.AlignTop
            name: banner.level === "ok" ? "check" : banner.level === "warn" ? "warning" : "download"
            color: Style.stateColor(banner.level)
            implicitWidth: 18
            implicitHeight: 18
        }
        ColumnLayout {
            id: column
            Layout.fillWidth: true
            spacing: 8
            Controls.Label {
                Layout.fillWidth: true
                text: banner.text
                color: Style.text
                wrapMode: Text.WordWrap
            }
            Flow {
                Layout.fillWidth: true
                visible: !AppController.installBusy
                spacing: 6
                AppButton {
                    visible: banner.offer === "install"
                    text: I18n.tr("Install with Shelly")
                    highlighted: true
                    onClicked: AppController.installWithShelly()
                }
                AppButton {
                    visible: banner.offer === "start-installed" || banner.offer === "restart-installed"
                    text: banner.offer === "start-installed" ? I18n.tr("Start the installed one") : I18n.tr("Restart")
                    highlighted: true
                    onClicked: AppController.startInstalled()
                }
                AppButton {
                    visible: banner.offer !== ""
                    text: I18n.tr("Later")
                    flat: true
                    onClicked: AppController.dismissInstall()
                }
                AppButton {
                    visible: banner.offer === "" && AppController.installMessage !== ""
                    text: I18n.tr("Dismiss")
                    flat: true
                    onClicked: AppController.installMessage = ""
                }
            }
        }
    }
}
