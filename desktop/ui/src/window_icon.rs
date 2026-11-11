//! The app icon on the windows (see window_icon.h).

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("window_icon.h");
        /// Call once the QGuiApplication exists, before the first window.
        fn set_window_icon();
    }
}

pub use ffi::set_window_icon;
