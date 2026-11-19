//! Platform specific settings for Windows.

/// The platform specific window settings of an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSpecific {
    /// Drag and drop support
    pub drag_and_drop: bool,

    /// Whether show or hide the window icon in the taskbar.
    pub skip_taskbar: bool,

    /// Shows or hides the background drop shadow for undecorated windows.
    ///
    /// The shadow is hidden by default.
    /// Enabling the shadow causes a thin 1px line to appear on the top of the window.
    pub undecorated_shadow: bool,

    /// Sets the preferred style of the window corners.
    ///
    /// Supported starting with Windows 11 Build 22000.
    pub corner_preference: CornerPreference,

    /// The material Windows draws behind the window (Mica, Acrylic…),
    /// seen where a transparent window draws see-through.
    ///
    /// Supported starting with Windows 11 Build 22621.
    pub system_backdrop: SystemBackdrop,

    /// Leaves out the window's GDI redirection surface
    /// (`WS_EX_NOREDIRECTIONBITMAP`), for a window drawn only through
    /// DirectComposition, so its see-through parts show the backdrop.
    pub no_redirection_bitmap: bool,
}

impl Default for PlatformSpecific {
    fn default() -> Self {
        Self {
            drag_and_drop: true,
            skip_taskbar: false,
            undecorated_shadow: false,
            corner_preference: Default::default(),
            system_backdrop: Default::default(),
            no_redirection_bitmap: false,
        }
    }
}

/// A material Windows draws behind a window (`DWM_SYSTEMBACKDROP_TYPE`).
#[repr(i32)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemBackdrop {
    /// Corresponds to `DWMSBT_AUTO`: the system decides.
    #[default]
    Auto = 0,
    /// Corresponds to `DWMSBT_NONE`.
    None = 1,
    /// Corresponds to `DWMSBT_MAINWINDOW`: Mica.
    Mica = 2,
    /// Corresponds to `DWMSBT_TRANSIENTWINDOW`: Acrylic.
    Acrylic = 3,
    /// Corresponds to `DWMSBT_TABBEDWINDOW`: Mica Alt.
    MicaAlt = 4,
}

/// Describes how the corners of a window should look like.
#[repr(i32)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CornerPreference {
    /// Corresponds to `DWMWCP_DEFAULT`.
    ///
    /// Let the system decide when to round window corners.
    #[default]
    Default = 0,

    /// Corresponds to `DWMWCP_DONOTROUND`.
    ///
    /// Never round window corners.
    DoNotRound = 1,

    /// Corresponds to `DWMWCP_ROUND`.
    ///
    /// Round the corners, if appropriate.
    Round = 2,

    /// Corresponds to `DWMWCP_ROUNDSMALL`.
    ///
    /// Round the corners if appropriate, with a small radius.
    RoundSmall = 3,
}
