#pragma once

#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>

// The app icon for every window. Under Wayland the compositor gets it through
// xdg-toplevel-icon -- otherwise it looks the icon up in the .desktop file
// named like the app id, which an AppImage does not install. Under X11 it
// becomes _NET_WM_ICON.
inline void set_window_icon()
{
    QGuiApplication::setWindowIcon(QIcon(QStringLiteral(":/ps-hid-bridge/app-icon.svg")));
}
