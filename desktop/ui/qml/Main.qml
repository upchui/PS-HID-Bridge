import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ApplicationWindow {
    id: root

    title: "PS5 HID Bridge"
    width: Kirigami.Units.gridUnit * 50
    height: Kirigami.Units.gridUnit * 36
    minimumWidth: Kirigami.Units.gridUnit * 30
    minimumHeight: Kirigami.Units.gridUnit * 24

    readonly property var pageNames: ["status", "devices", "triggers", "mouse", "board"]
    readonly property var pageComponents: ({
        "status": statusPage,
        "devices": devicesPage,
        "triggers": triggersPage,
        "mouse": mousePage,
        "board": boardPage,
        "wizard": wizardPage
    })
    property string currentPage: ""

    property var currentItem: null

    // Pages are created with a parent and owned here (pushing a bare
    // Component makes Kirigami create a parentless item first).
    function show(name) {
        if (currentPage === name)
            return;
        currentPage = name;
        pageStack.clear();
        if (currentItem)
            currentItem.destroy();
        currentItem = pageComponents[name].createObject(root.contentItem);
        pageStack.push(currentItem);
    }

    Component { id: statusPage; StatusPage {} }
    Component { id: devicesPage; DevicesPage {} }
    Component { id: triggersPage; TriggersPage {} }
    Component { id: mousePage; MousePage {} }
    Component { id: boardPage; BoardPage {} }
    Component { id: wizardPage; WizardPage { onFinished: root.show("status") } }

    globalDrawer: Kirigami.GlobalDrawer {
        title: "PS5 HID Bridge"
        isMenu: false
        modal: false
        collapsible: false
        width: Kirigami.Units.gridUnit * 11
        enabled: AppController.setupDone
        actions: [
            Kirigami.Action { text: "Status"; checked: root.currentPage === "status"; onTriggered: root.show("status") },
            Kirigami.Action { text: "Geräte"; checked: root.currentPage === "devices"; onTriggered: root.show("devices") },
            Kirigami.Action { text: "Auslöser"; checked: root.currentPage === "triggers"; onTriggered: root.show("triggers") },
            Kirigami.Action { text: "Maus"; checked: root.currentPage === "mouse"; onTriggered: root.show("mouse") },
            Kirigami.Action { text: "Board"; checked: root.currentPage === "board"; onTriggered: root.show("board") },
            Kirigami.Action { text: "Einrichtung wiederholen"; onTriggered: root.show("wizard") }
        ]
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
                const ok = root.pageStack.grabToImage(function (result) {
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
