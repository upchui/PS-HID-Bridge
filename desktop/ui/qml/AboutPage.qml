import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import QtQuick.Shapes
import PSHIDBridge

// What the project is and where it lives, who made it, its license and
// what it is built with.
AppPage {
    id: page

    readonly property string repoUrl: "https://github.com/" + AppController.repo
    // Everyone who took part, from the project's history when the app was
    // built (tools/changelog.py): [{name, aka, github}], oldest first.
    readonly property var contributors: {
        try {
            return JSON.parse(AppController.changelogJson).contributors ?? [];
        } catch (e) {
            return [];
        }
    }
    // Their GitHub pictures fetched so far: {"name": "file:///…"}.
    readonly property var avatars: {
        try {
            return JSON.parse(AppController.avatars);
        } catch (e) {
            return ({});
        }
    }
    // Self-test and screenshots: nothing is fetched.
    readonly property bool offline: Qt.application.arguments.indexOf("--self-test") >= 0
                                    || Qt.application.arguments.indexOf("--screenshot") >= 0
    // Not a text: the same in every language.
    readonly property string copyright: "Copyright © 2026 Alexander Huthansl"
    readonly property string licenseId: "GPL-2.0-or-later"

    // A link as a button that opens it in the browser.
    component LinkButton: AppButton {
        property string url
        icon.name: "external"
        icon.color: Style.text
        onClicked: Qt.openUrlExternally(url)
        Accessible.description: url
    }

    title: I18n.tr("About")
    subtitle: I18n.tr("Mouse and keyboard of this computer on the PlayStation.")
    warnings: false

    Card {
        title: "PS HID Bridge"
        icon: "gamepad"
        subtitle: AppController.boardFirmware !== ""
                  ? I18n.trf("App %1 · board firmware %2", AppController.version, AppController.boardFirmware)
                  : I18n.trf("App %1", AppController.version)

        Controls.Label {
            Layout.fillWidth: true
            text: I18n.tr("The app forwards mouse and keyboard, encrypted over the local network, to a small board (RP2350 with a W5500 network chip) on the PlayStation's USB port. The board passes them on as a USB mouse and keyboard; the picture comes in via Remote Play, e.g. with chiaki-ng.")
            color: Style.text
            wrapMode: Text.WordWrap
        }
        Flow {
            Layout.fillWidth: true
            spacing: 8
            LinkButton {
                text: I18n.tr("Project on GitHub")
                url: page.repoUrl
            }
            LinkButton {
                text: I18n.tr("Releases")
                url: page.repoUrl + "/releases"
            }
            LinkButton {
                text: I18n.tr("Documentation")
                url: page.repoUrl + "#readme"
            }
            LinkButton {
                text: I18n.tr("Report a problem")
                url: page.repoUrl + "/issues"
            }
        }
    }

    Card {
        title: I18n.tr("Contributors")
        icon: "person"

        // Each the same way: the GitHub picture, the GitHub name (or the
        // name in git), below it further names.
        Repeater {
            model: page.contributors
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 12
                Component.onCompleted: {
                    if (modelData.github !== "" && !page.offline)
                        AppController.fetchAvatar(modelData.github);
                }
                Item {
                    implicitWidth: 40
                    implicitHeight: 40
                    Layout.alignment: Qt.AlignVCenter
                    // Until the picture is there, or without one: a person.
                    Rectangle {
                        anchors.fill: parent
                        radius: width / 2
                        color: Style.surfaceAlt
                        border.color: Style.border
                        visible: picture.status !== Image.Ready
                        AppIcon {
                            anchors.centerIn: parent
                            name: "person"
                            color: Style.muted
                            implicitWidth: 22
                            implicitHeight: 22
                        }
                    }
                    Image {
                        id: picture
                        anchors.fill: parent
                        source: page.avatars[modelData.github] ?? ""
                        sourceSize: Qt.size(80, 80)
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        smooth: true
                    }
                    // Round like on GitHub: the corners in the card's colour.
                    Shape {
                        anchors.fill: parent
                        visible: picture.status === Image.Ready
                        preferredRendererType: Shape.CurveRenderer
                        ShapePath {
                            fillColor: Style.surface
                            strokeColor: "transparent"
                            fillRule: ShapePath.OddEvenFill
                            startX: 0
                            startY: 0
                            PathLine { x: 40; y: 0 }
                            PathLine { x: 40; y: 40 }
                            PathLine { x: 0; y: 40 }
                            PathLine { x: 0; y: 0 }
                            PathMove { x: 40; y: 20 }
                            PathArc { x: 0; y: 20; radiusX: 20; radiusY: 20 }
                            PathArc { x: 40; y: 20; radiusX: 20; radiusY: 20 }
                        }
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        text: modelData.name
                        color: Style.text
                        font.weight: Font.DemiBold
                        wrapMode: Text.WordWrap
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        visible: modelData.aka.length > 0
                        text: modelData.aka.join(" · ")
                        color: Style.muted
                        wrapMode: Text.WordWrap
                    }
                }
                LinkButton {
                    visible: modelData.github !== ""
                    text: I18n.tr("GitHub profile")
                    flat: true
                    url: "https://github.com/" + modelData.github
                }
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: page.contributors.length === 0
            text: I18n.tr("This build has no list: it is made from the project's history when the app is built.")
            color: Style.muted
            wrapMode: Text.WordWrap
        }
    }

    Card {
        title: I18n.tr("License")
        icon: "info"
        subtitle: page.licenseId

        Controls.Label {
            Layout.fillWidth: true
            text: I18n.tr("PS HID Bridge is free software: you may pass it on and change it under the terms of the GNU General Public License as published by the Free Software Foundation, version 2 or (at your option) any later version. It comes without any warranty.")
            color: Style.text
            wrapMode: Text.WordWrap
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            Controls.Label {
                Layout.fillWidth: true
                text: page.copyright
                color: Style.muted
                wrapMode: Text.WordWrap
            }
            LinkButton {
                text: I18n.tr("License text")
                flat: true
                url: page.repoUrl + "/blob/main/LICENSE"
            }
        }
    }

    Card {
        title: I18n.tr("Built with")
        icon: "board"
        subtitle: I18n.tr("Free software by others, each under its own license.")

        Repeater {
            model: [
                { name: "Qt 6", what: I18n.tr("user interface"), license: "LGPL-3.0", url: "https://www.qt.io" },
                { name: "CXX-Qt", what: I18n.tr("Rust and Qt together"), license: "MIT / Apache-2.0",
                  url: "https://github.com/KDAB/cxx-qt" },
                { name: "Rust", what: I18n.tr("libraries such as rustls and ring (HTTPS), zbus (D-Bus), evdev and clap"),
                  license: "MIT, Apache-2.0, ISC\u00a0…", url: page.repoUrl + "/blob/main/desktop/Cargo.lock" },
                { name: "Raspberry Pi Pico SDK", what: I18n.tr("the board's firmware"), license: "BSD-3-Clause",
                  url: "https://github.com/raspberrypi/pico-sdk" },
                { name: "TinyUSB", what: I18n.tr("USB on the board"), license: "MIT",
                  url: "https://github.com/hathach/tinyusb" },
                { name: "WIZnet ioLibrary", what: I18n.tr("the W5500 network chip"), license: "MIT",
                  url: "https://github.com/Wiznet/ioLibrary_Driver" },
                { name: "Monocypher", what: I18n.tr("encryption on the board"), license: "BSD-2-Clause / CC0-1.0",
                  url: "https://monocypher.org" }
            ]
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 12
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        text: modelData.name
                        color: Style.text
                        font.weight: Font.DemiBold
                        wrapMode: Text.WordWrap
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: modelData.what + " · " + modelData.license
                        color: Style.muted
                        wrapMode: Text.WordWrap
                    }
                }
                LinkButton {
                    flat: true
                    url: modelData.url
                    Accessible.name: I18n.trf("Open %1", modelData.name)
                    Controls.ToolTip.visible: hovered
                    Controls.ToolTip.text: modelData.url
                    Controls.ToolTip.delay: 500
                }
            }
        }
    }
}
