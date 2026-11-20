import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

Controls.ApplicationWindow {
    id: root

    title: "PS HID Bridge"
    width: Kirigami.Units.gridUnit * 56
    // --screenshot-height N: taller window, so a screenshot shows a whole page.
    height: {
        const args = Qt.application.arguments;
        const i = args.indexOf("--screenshot-height");
        return i >= 0 && i + 1 < args.length ? Number(args[i + 1]) : Kirigami.Units.gridUnit * 40;
    }
    minimumWidth: Kirigami.Units.gridUnit * 34
    minimumHeight: Kirigami.Units.gridUnit * 26

    Connections {
        target: AppController
        function onShowWindowRequested() {
            root.show();
            if (root.visibility === Window.Minimized) root.showNormal();
            root.raise();
            root.requestActivate();
        }
    }
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
        { name: "status", text: I18n.tr("Status"), icon: "status" },
        { name: "input", text: I18n.tr("Input"), icon: "input" },
        { name: "triggers", text: I18n.tr("Triggers"), icon: "bolt" },
        { name: "board", text: I18n.tr("Board"), icon: "board" },
        { name: "latency", text: I18n.tr("Latency"), icon: "latency" },
        { name: "firmware", text: I18n.tr("Firmware & updates"), icon: "update" },
        { name: "changelog", text: I18n.tr("Changelog"), icon: "changelog" }
    ]
    readonly property var pageNames: pages.map(p => p.name)
    readonly property var pageComponents: ({
        "status": statusPage,
        "input": inputPage,
        "triggers": triggersPage,
        "board": boardPage,
        "latency": latencyPage,
        "firmware": firmwarePage,
        "changelog": changelogPage,
        "wizard": wizardPage
    })
    property string currentPage: ""
    property var currentItem: null

    // Not "show": that would hide the window's own show() (tray "Show window").
    function showPage(name) {
        if (currentPage === name)
            return;
        currentPage = name;
        if (currentItem)
            currentItem.destroy();
        currentItem = pageComponents[name].createObject(pageHost);
    }

    Component { id: statusPage; StatusPage { onOpenPage: name => root.showPage(name) } }
    Component { id: inputPage; InputPage {} }
    Component { id: triggersPage; TriggersPage {} }
    Component { id: boardPage; BoardPage {} }
    Component { id: latencyPage; LatencyPage {} }
    Component { id: firmwarePage; FirmwarePage {} }
    Component { id: changelogPage; ChangelogPage {} }
    Component { id: wizardPage; WizardPage { onFinished: root.showPage("status") } }

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
                        Layout.fillWidth: true
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
                                color: AppController.active ? Style.ok
                                     : AppController.pending || AppController.paused ? Style.warn
                                     : AppController.connected ? Style.accent : Style.muted
                            }
                            Controls.Label {
                                // The same words as the Status page (also "Paused",
                                // "Please release all keys …", "Connecting …").
                                Layout.fillWidth: true
                                text: AppController.statusText !== "" ? AppController.statusText
                                      : AppController.active ? I18n.tr("Bridge on") : AppController.connected ? I18n.tr("Ready")
                                      : I18n.tr("Not connected")
                                elide: Text.ElideRight
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
                        onClicked: root.showPage(modelData.name)
                    }
                }

                Item { Layout.fillHeight: true }

                NavItem {
                    text: I18n.tr("Run setup again")
                    iconName: "setup"
                    onClicked: root.showPage("wizard")
                }
                Segmented {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: 6
                    value: AppController.appearance
                    model: [
                        { value: "light", icon: "sun", tip: I18n.tr("Light") },
                        { value: "system", text: I18n.tr("Auto"), tip: I18n.tr("Like the desktop") },
                        { value: "dark", icon: "moon", tip: I18n.tr("Dark") }
                    ]
                    onActivated: v => AppController.chooseAppearance(v)
                }
                // Language names in their own language.
                Segmented {
                    Layout.alignment: Qt.AlignHCenter
                    value: AppController.language
                    model: [
                        { value: "en", text: "EN", tip: "English" },
                        { value: "de", text: "DE", tip: "Deutsch" }
                    ]
                    onActivated: v => AppController.setAppLanguage(v)
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

    // --screenshot-live: connected to the board; the bridge goes on after a
    // few seconds and the status page collects values before the pictures.
    Timer {
        id: liveOn
        interval: 4000
        onTriggered: AppController.setOn(true)
    }
    Timer {
        id: liveWarmup
        interval: 25000
        onTriggered: root.nextAutoPage()
    }

    function nextAutoPage() {
        if (autoPages.length === 0) {
            Qt.quit();
            return;
        }
        showPage(autoPages.shift());
        autoTimer.start();
    }

    Component.onCompleted: {
        const args = Qt.application.arguments;
        const shot = args.indexOf("--screenshot");
        if (shot >= 0 || args.indexOf("--self-test") >= 0) {
            screenshotDir = shot >= 0 && shot + 1 < args.length ? args[shot + 1] : "";
            if (shot >= 0 && args.indexOf("--screenshot-live") >= 0) {
                autoPages = pageNames.slice();
                autoTimer.interval = 3000; // the page fetches from the board
                showPage("status");
                liveOn.start();
                liveWarmup.start();
                return;
            }
            autoPages = ["wizard"].concat(pageNames);
            Qt.callLater(nextAutoPage);
            return;
        }
        Qt.callLater(showPage, AppController.setupDone ? "status" : "wizard");
    }
}
