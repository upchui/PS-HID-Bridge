import QtQuick
import QtQuick.Shapes
import PSHIDBridge

// One of the app's line icons (Icons.qml, from desktop/ui/icons/*.svg) in
// any colour and size. Drawn as shapes, so it stays sharp and needs no icon
// theme.
Item {
    id: root

    /// Icon name, e.g. "board"; empty draws nothing.
    property string name
    property color color: Style.text

    readonly property var paths: Icons.paths[name] ?? { s: "", f: "" }

    implicitWidth: 20
    implicitHeight: 20

    Shape {
        visible: root.name !== ""
        width: 24
        height: 24
        anchors.centerIn: parent
        scale: Math.min(root.width, root.height) / 24
        preferredRendererType: Shape.CurveRenderer

        ShapePath {
            strokeColor: root.color
            strokeWidth: 2
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: root.paths.s !== "" ? root.paths.s : "M0 0" }
        }
        ShapePath {
            strokeColor: "transparent"
            fillColor: root.color
            PathSvg { path: root.paths.f !== "" ? root.paths.f : "M0 0" }
        }
    }
}
