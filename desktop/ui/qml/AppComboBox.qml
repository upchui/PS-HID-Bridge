import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Drop-down list in the app's style: closed it looks like an AppButton with
// a chevron; the list opens on a card with the chosen entry ticked. Works
// with a list of texts and with objects and textRole.
Controls.ComboBox {
    id: box

    hoverEnabled: true
    leftPadding: 10
    rightPadding: 10 + 16 + 6

    function entryText(data) {
        return box.textRole !== "" && data !== null && typeof data === "object" ? data[box.textRole] : data;
    }

    background: Rectangle {
        implicitWidth: 140
        implicitHeight: 32
        radius: 7
        color: !box.enabled ? Style.tint(Style.text, 0.04)
             : box.pressed || box.popup.visible ? Style.tint(Style.text, 0.14)
             : box.hovered ? Style.tint(Style.text, 0.08) : Style.button
        border.color: box.popup.visible ? Style.accent : Style.border
        border.width: 1
        // Keyboard focus.
        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: parent.radius + 3
            color: "transparent"
            border.color: Style.accent
            border.width: 2
            visible: box.visualFocus
        }
    }

    contentItem: Controls.Label {
        text: box.displayText
        color: box.enabled ? Style.text : Style.disabled
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    indicator: AppIcon {
        x: box.width - width - 10
        y: (box.height - height) / 2
        implicitWidth: 16
        implicitHeight: 16
        name: "chevron"
        color: box.enabled ? Style.muted : Style.disabled
        rotation: box.popup.visible ? 180 : 0
    }

    delegate: Controls.ItemDelegate {
        id: entry
        required property var modelData
        required property int index
        readonly property bool chosen: box.currentIndex === index

        width: ListView.view ? ListView.view.width : implicitWidth
        hoverEnabled: true
        highlighted: box.highlightedIndex === index
        leftPadding: 10
        rightPadding: 10
        topPadding: 6
        bottomPadding: 6

        background: Rectangle {
            radius: 5
            color: entry.pressed ? Style.tint(Style.accent, 0.24)
                 : entry.highlighted ? Style.tint(Style.accent, 0.14) : "transparent"
        }
        contentItem: RowLayout {
            spacing: 8
            Controls.Label {
                Layout.fillWidth: true
                text: box.entryText(entry.modelData)
                color: Style.text
                font.weight: entry.chosen ? Font.DemiBold : Font.Normal
                elide: Text.ElideRight
            }
            AppIcon {
                name: "check"
                color: Style.accent
                implicitWidth: 16
                implicitHeight: 16
                opacity: entry.chosen ? 1 : 0
            }
        }
    }

    popup: Controls.Popup {
        y: box.height + 4
        width: Math.max(box.width, implicitContentWidth + leftPadding + rightPadding)
        // Stays inside the window; a long list scrolls.
        margins: 8
        padding: 4
        implicitHeight: contentItem.implicitHeight + topPadding + bottomPadding

        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            implicitWidth: 200
            model: box.popup.visible ? box.delegateModel : null
            currentIndex: box.highlightedIndex
            spacing: 1
            Controls.ScrollIndicator.vertical: Controls.ScrollIndicator {}
        }

        background: Rectangle {
            radius: Style.radius
            color: Style.surface
            border.color: Style.border
        }
    }
}
