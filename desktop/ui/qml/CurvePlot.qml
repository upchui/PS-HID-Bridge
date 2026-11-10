import QtQuick
import org.kde.kirigami as Kirigami

// Gain over mouse speed: gain = sensitivity * (1 + accel * max(0, speed - 2)), capped.
Canvas {
    id: plot

    property real sensitivity: 1.0
    property real accel: 0.0
    readonly property real maxSpeed: 40
    readonly property real threshold: 2
    readonly property real maxGain: Math.max(8, sensitivity)

    implicitHeight: Kirigami.Units.gridUnit * 10

    function gain(speed) {
        return Math.min(sensitivity * (1 + accel * Math.max(0, speed - threshold)), maxGain);
    }

    onSensitivityChanged: requestPaint()
    onAccelChanged: requestPaint()
    onWidthChanged: requestPaint()

    onPaint: {
        const ctx = getContext("2d");
        const w = width, h = height, pad = 28;
        ctx.reset();
        const top = Math.max(2, gain(maxSpeed) * 1.15);
        const x = s => pad + (w - pad - 6) * s / maxSpeed;
        const y = g => h - pad - (h - pad - 6) * g / top;

        ctx.strokeStyle = Kirigami.Theme.disabledTextColor;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(pad, 4);
        ctx.lineTo(pad, h - pad);
        ctx.lineTo(w - 4, h - pad);
        ctx.stroke();

        ctx.fillStyle = Kirigami.Theme.textColor;
        ctx.font = "11px sans-serif";
        ctx.fillText("speed", w - 40, h - 8);
        ctx.fillText("× " + top.toFixed(1), 2, 12);
        ctx.fillText("1×", 6, y(1) + 4);

        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(pad, y(1));
        ctx.lineTo(w - 4, y(1));
        ctx.stroke();
        ctx.setLineDash([]);

        ctx.strokeStyle = Kirigami.Theme.highlightColor;
        ctx.lineWidth = 2;
        ctx.beginPath();
        for (let i = 0; i <= 80; i++) {
            const s = maxSpeed * i / 80;
            if (i === 0) ctx.moveTo(x(s), y(gain(s)));
            else ctx.lineTo(x(s), y(gain(s)));
        }
        ctx.stroke();
    }
}
