pragma Singleton
import QtQuick
import PSHIDBridge

// Colours and sizes of the app, light and dark (the same colours as
// the board's web interface).
QtObject {
    // "system" follows the desktop (through the XDG desktop portal).
    readonly property bool dark: AppController.appearance === "dark"
        || (AppController.appearance !== "light" && Application.styleHints.colorScheme === Qt.ColorScheme.Dark)

    readonly property color window: dark ? "#12161d" : "#f4f6fa"
    readonly property color sidebar: dark ? "#161b23" : "#e9edf3"
    readonly property color surface: dark ? "#1b212b" : "#ffffff"
    readonly property color surfaceAlt: dark ? "#222935" : "#f0f3f8"
    readonly property color field: dark ? "#141921" : "#ffffff"
    readonly property color button: dark ? "#262e3b" : "#f6f8fb"
    readonly property color border: dark ? "#2c3442" : "#dde2ea"
    readonly property color text: dark ? "#e6eaf1" : "#1b2333"
    readonly property color muted: dark ? "#9aa4b5" : "#5c6678"
    readonly property color disabled: dark ? "#5d6677" : "#a4acb9"
    readonly property color accent: dark ? "#5b8cff" : "#2f6fed"
    readonly property color accentText: dark ? "#0b1220" : "#ffffff"
    readonly property color ok: dark ? "#4cc382" : "#1d7f45"
    readonly property color warn: dark ? "#e0a83a" : "#9a6200"
    readonly property color bad: dark ? "#ff7b72" : "#b3261e"

    // Chart series, in this order (checked with the dataviz palette
    // validator against `surface` in both modes: CVD separation, lightness,
    // contrast).
    readonly property var series: dark ? ["#3987e5", "#d95926", "#199e70", "#c98500"]
                                       : ["#2a78d6", "#eb6834", "#1baf7a", "#eda100"]
    function onSeries(i) {
        // White or near-black, whichever stands out more from the fill.
        const c = Qt.lighter(series[i % series.length], 1.0);
        const lin = v => v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
        const l = 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
        return 1.05 / (l + 0.05) >= (l + 0.05) / 0.0535 ? "#ffffff" : "#0b0b0b";
    }
    /// Gridlines and axis rules: one step off the surface.
    readonly property color grid: dark ? "#2a313d" : "#e8ecf2"

    readonly property int radius: 8
    readonly property int gap: 12
    readonly property int pad: 16
    /// Content does not get wider than this (easier to read).
    readonly property int maxWidth: 920
    readonly property real fontSize: Qt.application.font.pointSize > 0 ? Qt.application.font.pointSize : 10

    function tint(c, alpha) {
        const k = Qt.lighter(c, 1.0);  // also takes a colour given as text ("#2a78d6")
        return Qt.rgba(k.r, k.g, k.b, alpha === undefined ? 0.15 : alpha);
    }

    /// Colour for a state: "ok", "warn", "bad", "accent" or anything else (muted).
    function stateColor(state) {
        return state === "ok" ? ok : state === "warn" ? warn : state === "bad" ? bad
             : state === "accent" ? accent : muted;
    }
}
