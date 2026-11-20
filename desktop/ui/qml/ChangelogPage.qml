import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// What changed in each version, newest first, the installed one marked.
// Made from git when the app is built (tools/changelog.py): the entries are
// the commit subjects, their details the commit texts (English); only the
// page's own texts are translated.
AppPage {
    id: page

    title: I18n.tr("Changelog")
    subtitle: I18n.trf("Installed: version %1. What changed in each version, newest first.", AppController.version)

    readonly property var log: {
        try {
            return JSON.parse(AppController.changelogJson);
        } catch (e) {
            return ({});
        }
    }
    readonly property var releases: log.releases ?? []
    readonly property string repo: log.repo ?? ""
    // Filter: "all", "app", "board" or "other".
    property string group: "all"
    // Versions shown (more on request: there are many).
    property int shown: 5
    // Every change's details open (else each opens on its own).
    property bool allDetails: false
    // [{release, changes}] with the changes of the filter, versions without any left out.
    readonly property var matching: releases
        .map(r => ({ release: r, changes: group === "all" ? r.changes : r.changes.filter(c => c.group === group) }))
        .filter(m => m.changes.length > 0)

    function dateText(iso) {
        const d = new Date(iso + "T12:00:00");
        if (isNaN(d.getTime()))
            return iso;
        return AppController.language === "de" ? d.toLocaleDateString(Qt.locale("de_DE"), "d. MMMM yyyy")
                                               : d.toLocaleDateString(Qt.locale("en_GB"), "d MMMM yyyy");
    }
    function openOnGitHub(path) {
        Qt.openUrlExternally("https://github.com/" + page.repo + "/" + path);
    }

    Banner {
        kind: "info"
        visible: AppController.appUpdateAvailable
        text: I18n.trf("Version %1 is available for the app.", AppController.latestVersion)
        actions: AppButton {
            text: I18n.tr("Update app")
            highlighted: true
            visible: AppController.canUpdateApp
            enabled: !AppController.updateBusy
            onClicked: AppController.updateApp()
        }
    }

    ColumnLayout {
        Layout.fillWidth: true
        visible: page.releases.length > 0
        spacing: 6
        // Wraps below each other in a narrow window.
        Flow {
            Layout.fillWidth: true
            spacing: Style.gap
            Segmented {
                id: filter
                model: [
                    { value: "all", text: I18n.tr("All") },
                    { value: "app", text: I18n.tr("App") },
                    { value: "board", text: I18n.tr("Board") },
                    { value: "other", text: I18n.tr("Other") }
                ]
                value: page.group
                onActivated: value => {
                    page.group = value;
                    page.shown = 5;
                }
            }
            AppSwitch {
                height: filter.height  // centred next to the filter
                text: I18n.tr("Show all details")
                checked: page.allDetails
                onToggled: page.allDetails = checked
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.language !== "en"
            text: I18n.tr("The entries are the titles and texts of the changes in development, in English.")
            color: Style.muted
            wrapMode: Text.WordWrap
        }
    }

    Card {
        visible: page.releases.length === 0
        title: I18n.tr("No changelog in this build")
        icon: "changelog"
        Controls.Label {
            Layout.fillWidth: true
            text: I18n.tr("The changelog is made from the git history when the app is built (make desktop).")
            color: Style.muted
            wrapMode: Text.WordWrap
        }
    }

    Repeater {
        model: page.matching.slice(0, page.shown)
        delegate: Card {
            id: version
            required property var modelData
            readonly property var release: modelData.release
            readonly property bool installed: release.version === AppController.version
            // A narrow window: the link only as an icon, the title keeps its room.
            readonly property bool narrow: width < 560

            title: I18n.trf("Version %1", release.version)
            subtitle: page.dateText(release.date) + " · "
                      + (modelData.changes.length === 1 ? I18n.tr("1 change")
                                                         : I18n.trf("%1 changes", modelData.changes.length))
            icon: installed ? "check" : "changelog"
            iconColor: installed ? Style.ok : Style.accent
            actions: [
                Pill {
                    visible: version.installed
                    text: I18n.tr("installed")
                    level: "ok"
                },
                AppButton {
                    visible: version.release.compare !== "" && page.repo !== ""
                    text: version.narrow ? "" : I18n.tr("Compare on GitHub")
                    flat: true
                    icon.name: "external"
                    icon.color: Style.text
                    Accessible.name: I18n.tr("Compare on GitHub")
                    Controls.ToolTip.visible: version.narrow && hovered
                    Controls.ToolTip.text: I18n.tr("Compare on GitHub")
                    Controls.ToolTip.delay: 500
                    onClicked: page.openOnGitHub("compare/" + version.release.compare)
                }
            ]

            Repeater {
                model: version.modelData.changes
                delegate: RowLayout {
                    id: change
                    required property var modelData
                    // Its details: opened here, or all of them above.
                    property bool open: false
                    readonly property bool expanded: (page.allDetails || open) && modelData.details.length > 0
                    readonly property color tone: modelData.group === "board" ? (Style.dark ? Style.series[1] : Qt.darker(Style.series[1], 1.35))
                                                  : modelData.group === "app" ? Style.accent : Style.muted
                    Layout.fillWidth: true
                    spacing: 10

                    // The area as a chip, coloured by the filter's group.
                    Rectangle {
                        Layout.alignment: Qt.AlignTop
                        Layout.maximumWidth: 180
                        implicitWidth: areaLabel.implicitWidth + 16
                        implicitHeight: areaLabel.implicitHeight + 6
                        radius: height / 2
                        color: Style.tint(change.tone, 0.14)
                        Controls.Label {
                            id: areaLabel
                            anchors.centerIn: parent
                            width: Math.min(implicitWidth, parent.width - 16)
                            text: change.modelData.area !== "" ? change.modelData.area : I18n.tr("General")
                            textFormat: Text.PlainText
                            color: change.tone
                            elide: Text.ElideRight
                            font.weight: Font.DemiBold
                            font.pointSize: Style.fontSize * 0.85
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Repeater {
                            model: change.modelData.items
                            delegate: RowLayout {
                                required property string modelData
                                Layout.fillWidth: true
                                spacing: 8
                                // A dot before each point when there are several.
                                Rectangle {
                                    visible: change.modelData.items.length > 1
                                    Layout.alignment: Qt.AlignTop
                                    Layout.topMargin: item.font.pixelSize * 0.55
                                    implicitWidth: 5
                                    implicitHeight: 5
                                    radius: 2.5
                                    color: Style.muted
                                }
                                Controls.Label {
                                    id: item
                                    Layout.fillWidth: true
                                    text: modelData
                                    // Commit subjects: never read as HTML.
                                    textFormat: Text.PlainText
                                    color: Style.text
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }
                        // The commit's text, folded away until asked for.
                        Controls.AbstractButton {
                            id: more
                            visible: change.modelData.details.length > 0 && !page.allDetails
                            hoverEnabled: true
                            padding: 0
                            topPadding: 1
                            text: change.open ? I18n.tr("Less") : I18n.tr("Details")
                            Accessible.name: text
                            contentItem: Controls.Label {
                                text: more.text + (change.open ? " ▴" : " ▾")
                                color: Style.accent
                                font.pointSize: Style.fontSize * 0.9
                                font.underline: more.hovered || more.visualFocus
                            }
                            HoverHandler {
                                cursorShape: Qt.PointingHandCursor
                            }
                            onClicked: change.open = !change.open
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            Layout.topMargin: 2
                            visible: change.expanded
                            spacing: 4
                            Repeater {
                                // Built only once shown: many changes have long texts.
                                model: change.expanded ? change.modelData.details : []
                                delegate: RowLayout {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    spacing: 8
                                    Rectangle {
                                        visible: modelData.bullet
                                        Layout.alignment: Qt.AlignTop
                                        Layout.topMargin: detail.font.pixelSize * 0.55
                                        implicitWidth: 4
                                        implicitHeight: 4
                                        radius: 2
                                        color: Style.muted
                                    }
                                    Controls.Label {
                                        id: detail
                                        Layout.fillWidth: true
                                        text: modelData.text
                                        textFormat: Text.PlainText
                                        color: Style.muted
                                        wrapMode: Text.WordWrap
                                        font.pointSize: Style.fontSize * 0.92
                                    }
                                }
                            }
                        }
                    }
                    // The commit, opened on GitHub.
                    Controls.Label {
                        Layout.alignment: Qt.AlignTop
                        visible: page.repo !== ""
                        text: change.modelData.hash
                        color: hashHover.hovered ? Style.accent : Style.muted
                        font.family: "monospace"
                        font.pointSize: Style.fontSize * 0.85
                        font.underline: hashHover.hovered
                        Accessible.name: I18n.trf("Change %1 on GitHub", change.modelData.hash)
                        Controls.ToolTip.visible: hashHover.hovered
                        Controls.ToolTip.text: I18n.tr("Open this change on GitHub")
                        Controls.ToolTip.delay: 500
                        HoverHandler {
                            id: hashHover
                            cursorShape: Qt.PointingHandCursor
                        }
                        TapHandler {
                            onTapped: page.openOnGitHub("commit/" + change.modelData.hash)
                        }
                    }
                }
            }
        }
    }

    AppButton {
        Layout.alignment: Qt.AlignHCenter
        visible: page.matching.length > page.shown
        text: I18n.trf("Show older versions (%1 more)", page.matching.length - page.shown)
        onClicked: page.shown += 10
    }
}
