//! The app's asset source: gpui-kit's default icon bundle plus the Lucide
//! icons Proxymate's widgets draw (gpui-kit embeds only the icons its own
//! components use; the full catalog would add ~1,600 SVGs to the binary).

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(
    WidgetIcons,
    [
        Lock,
        LockOpen,
        ShieldAlert,
        Pin,
        MessageSquareText,
        PencilLine,
        Bot,
        Repeat,
        Repeat2,
        Ellipsis,
        ArrowUp,
        ArrowDown,
        // The shell chrome (G2).
        Square,
        Play,
        Plus,
        Trash,
        Search,
        X,
        ChevronDown,
        ChevronRight,
        Globe,
        Layers,
        Radio,
        Star,
        FolderOpen,
        FileBraces,
        Archive,
        ShieldOff,
        Funnel,
        ArrowDownToLine,
        LoaderCircle,
        ScanSearch,
        CircleCheck,
        CircleAlert,
        HardDriveDownload,
        EyeOff,
        Copy,
        // The inspector (G3).
        TriangleAlert,
        // The body viewers (G4).
        ChevronUp,
        TextWrap,
        WandSparkles,
        Download,
        // Rules (G7): the toolbar's tools and the Rules window.
        ListChecks,
        ArchiveX,
        Cookie,
        Gauge,
        OctagonPause,
        GripVertical,
        FolderPlus,
        FlaskConical,
        CircleX,
        Save,
        CaseSensitive,
        Check,
        // The stream panels (G6).
        Send,
        Unplug,
        RotateCcw,
        // The Settings window (G12).
        Wifi,
        Network,
        ShieldCheck,
        Laptop,
        HardDrive,
        Braces,
        Boxes,
        Palette,
        SlidersHorizontal,
        RefreshCw,
        Settings,
        Smartphone,
        FileBox,
        FileCode,
        KeyRound,
        Monitor,
        Sun,
        Moon,
        // Editing (G8): the JSON and name/value editors, the Breakpoint
        // Editor and Compose.
        Undo2,
        Redo2,
        ChevronsUpDown,
        ChevronsDownUp,
        CirclePlay,
        Reply,
        Binary,
        // macOS integration (G13): certificate, system proxy, device setup,
        // Intercept and the MCP indicator.
        ShieldCheck,
        ShieldPlus,
        Crosshair,
        Smartphone,
        MonitorSmartphone,
        RefreshCw,
        SquareTerminal,
        AppWindow,
        Wifi,
    ]
);

/// gpui-kit's assets and Proxymate's icons, for `Application::with_assets`
/// and offscreen tests.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = WidgetIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(WidgetIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_icons_and_kit_icons_both_load() {
        let lock = gpui_kit::assets::IconName::LockOpen.path();
        assert!(Assets.load(&lock).unwrap().is_some(), "{lock}");
        let kit = gpui_kit::assets::IconName::Ellipsis.path();
        assert!(Assets.load(&kit).unwrap().is_some(), "{kit}");
    }
}
