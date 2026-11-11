import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

Controls.ApplicationWindow {
    id: root

    title: "PS HID Bridge"
    width: Kirigami.Units.gridUnit * 56
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 34
    minimumHeight: Kirigami.Units.gridUnit * 26
    visible: true
    color: Style.window

    // Colours of the standard controls (buttons, fields, switches ...).
    palette {
        window: Style.window
        base: Style.field
        alternateBase: Style.surfaceAlt
        button: Style.button
        brightText: Style.text
        active.windowText: Style.text
        active.text: Style.text
        active.buttonText: Style.text
        inactive.windowText: Style.text
        inactive.text: Style.text
        inactive.buttonText: Style.text
        highlight: Style.accent
        highlightedText: Style.accentText
        placeholderText: Style.muted
        toolTipBase: Style.surfaceAlt
        toolTipText: Style.text
        link: Style.accent
        light: Style.surface
        midlight: Style.surfaceAlt
        mid: Style.border
        dark: Style.border
        shadow: Style.dark ? "#000000" : "#9aa3b2"
        disabled.text: Style.disabled
        disabled.buttonText: Style.disabled
        disabled.windowText: Style.disabled
    }

    readonly property var pages: [
        { name: "status", text: "Status", icon: "status" },
        { name: "input", text: "Input", icon: "input" },
        { name: "triggers", text: "Triggers", icon: "bolt" },
        { name: "board", text: "Board", icon: "board" },
        { name: "firmware", text: "Firmware & updates", icon: "update" }
    ]
    readonly property var pageNames: pages.map(p => p.name)
    readonly property var pageComponents: ({
        "status": statusPage,
        "input": inputPage,
        "triggers": triggersPage,
        "board": boardPage,
        "firmware": firmwarePage,
        "wizard": wizardPage
    })
    property string currentPage: ""
    property var currentItem: null

    function show(name) {
        if (currentPage === name)
            return;
        currentPage = name;
        if (currentItem)
            currentItem.destroy();
        currentItem = pageComponents[name].createObject(pageHost);
    }

    Component { id: statusPage; StatusPage {} }
    Component { id: inputPage; InputPage {} }
    Component { id: triggersPage; TriggersPage {} }
    Component { id: boardPage; BoardPage {} }
    Component { id: firmwarePage; FirmwarePage {} }
    Component { id: wizardPage; WizardPage { onFinished: root.show("status") } }

    RowLayout {
        id: shell
        anchors.fill: parent
        spacing: 0

        // ---------------------------------------------------- sidebar
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 12
            visible: root.currentPage !== "wizard"
            color: Style.sidebar

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 10
                spacing: 4

                RowLayout {
                    Layout.margins: 6
                    Layout.bottomMargin: 14
                    spacing: 10
                    Rectangle {
                        implicitWidth: 34
                        implicitHeight: 34
                        radius: 9
                        color: Style.accent
                        AppIcon {
                            anchors.centerIn: parent
                            name: "gamepad"
                            color: Style.accentText
                            implicitWidth: 22
                            implicitHeight: 22
                        }
                    }
                    ColumnLayout {
                        spacing: 0
                        Controls.Label {
                            text: "PS HID Bridge"
                            color: Style.text
                            font.weight: Font.DemiBold
                        }
                        RowLayout {
                            spacing: 5
                            Rectangle {
                                implicitWidth: 8
                                implicitHeight: 8
                                radius: 4
                                color: AppController.active ? Style.ok : AppController.connected ? Style.accent : Style.muted
                            }
                            Controls.Label {
                                text: AppController.active ? "Bridge on" : AppController.connected ? "Ready" : "Not connected"
                                color: Style.muted
                                font.pointSize: Style.fontSize * 0.9
                            }
                        }
                    }
                }

                Repeater {
                    model: root.pages
                    delegate: NavItem {
                        required property var modelData
                        text: modelData.text
                        iconName: modelData.icon
                        current: root.currentPage === modelData.name
                        onClicked: root.show(modelData.name)
                    }
                }

                Item { Layout.fillHeight: true }

                NavItem {
                    text: "Run setup again"
                    iconName: "setup"
                    onClicked: root.show("wizard")
                }
                Segmented {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: 6
                    value: AppController.appearance
                    model: [
                        { value: "light", icon: "sun" },
                        { value: "system", text: "Auto" },
                        { value: "dark", icon: "moon" }
                    ]
                    onActivated: v => AppController.chooseAppearance(v)
                }
            }
            Rectangle {
                anchors.right: parent.right
                width: 1
                height: parent.height
                color: Style.border
            }
        }

        // ---------------------------------------------------- page
        Rectangle {
            id: pageHost
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Style.window
        }
    }

    // --self-test / --screenshot DIR: render every page once, then quit.
    property var autoPages: []
    property string screenshotDir: ""

    Timer {
        id: autoTimer
        interval: 300
        onTriggered: {
            if (root.screenshotDir !== "") {
                const name = root.currentPage;
                const ok = shell.grabToImage(function (result) {
                    result.saveToFile(root.screenshotDir + "/" + name + ".png");
                    root.nextAutoPage();
                });
                if (!ok)
                    root.nextAutoPage();
            } else {
                root.nextAutoPage();
            }
        }
    }

    function nextAutoPage() {
        if (autoPages.length === 0) {
            Qt.quit();
            return;
        }
        show(autoPages.shift());
        autoTimer.start();
    }

    Component.onCompleted: {
        const args = Qt.application.arguments;
        const shot = args.indexOf("--screenshot");
        if (shot >= 0 || args.indexOf("--self-test") >= 0) {
            screenshotDir = shot >= 0 && shot + 1 < args.length ? args[shot + 1] : "";
            autoPages = ["wizard"].concat(pageNames);
            Qt.callLater(nextAutoPage);
            return;
        }
        Qt.callLater(show, AppController.setupDone ? "status" : "wizard");
    }
}
