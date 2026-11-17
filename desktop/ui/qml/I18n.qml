pragma Singleton
import QtQuick
import PSHIDBridge

// The app's texts in the chosen language (Settings: language). English is
// written in the QML itself; the catalog (ui/i18n/de.json, through the
// controller) maps it to German. Bindings that call tr() depend on `map`,
// so every text follows a change of language at once.
//
//   text: I18n.tr("Bridge on")
//   text: I18n.trf("%1 of %2 devices", open, total)
QtObject {
    readonly property var map: JSON.parse(AppController.translationsJson)

    function tr(text) {
        const t = map[text];
        return t !== undefined && t !== "" ? t : text;
    }

    function trf(text, ...args) {
        let out = tr(text);
        for (let i = args.length; i >= 1; i--)
            out = out.split("%" + i).join(String(args[i - 1]));
        return out;
    }
}
