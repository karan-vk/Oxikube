//! The closed set of Lucide icons Oxikube ships.
//!
//! [`IconName`] is generated from one list: each entry names a variant and the Lucide file stem.
//! The SVG bytes are pulled in with `include_bytes!`, so an icon that is not in the list is not in
//! the binary. To add an icon, copy `<stem>.svg` from the Lucide set into `assets/icons/` (see
//! `assets/icons/LICENSE`, ISC) and add one line to the list below.

macro_rules! icon_names {
    ($($variant:ident => $stem:literal,)*) => {
        /// A Lucide icon embedded in the binary. Views never spell paths: they take an
        /// `IconName` (through `oxikube_ui::Icon`) and the asset source resolves it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum IconName {
            $(
                #[doc = concat!("Lucide `", $stem, "`.")]
                $variant,
            )*
        }

        impl IconName {
            /// Every embedded icon, in declaration order.
            pub const ALL: &'static [IconName] = &[$(IconName::$variant,)*];

            /// Asset path the icon is served under, e.g. `icons/box.svg`.
            pub const fn path(self) -> &'static str {
                match self {
                    $(IconName::$variant => concat!("icons/", $stem, ".svg"),)*
                }
            }

            /// The SVG source bytes (static; loading copies nothing).
            pub const fn svg(self) -> &'static [u8] {
                match self {
                    $(IconName::$variant => include_bytes!(concat!("../assets/icons/", $stem, ".svg")),)*
                }
            }
        }
    };
}

icon_names! {
    Box => "box",
    Boxes => "boxes",
    Container => "container",
    Layers => "layers",
    Server => "server",
    Network => "network",
    Globe => "globe",
    Database => "database",
    HardDrive => "hard-drive",
    KeyRound => "key-round",
    Lock => "lock",
    Shield => "shield",
    ShieldCheck => "shield-check",
    Settings => "settings",
    FileText => "file-text",
    FileCode => "file-code",
    Terminal => "terminal",
    SquareTerminal => "square-terminal",
    Search => "search",
    Funnel => "funnel",
    RefreshCw => "refresh-cw",
    Plus => "plus",
    Minus => "minus",
    X => "x",
    Check => "check",
    ChevronRight => "chevron-right",
    ChevronDown => "chevron-down",
    ChevronUp => "chevron-up",
    ChevronLeft => "chevron-left",
    ArrowUp => "arrow-up",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    Trash => "trash",
    Pencil => "pencil",
    Copy => "copy",
    Clipboard => "clipboard",
    ExternalLink => "external-link",
    Play => "play",
    Pause => "pause",
    Square => "square",
    RotateCcw => "rotate-ccw",
    Scaling => "scaling",
    Eye => "eye",
    EyeOff => "eye-off",
    Star => "star",
    Bookmark => "bookmark",
    Clock => "clock",
    Activity => "activity",
    Cpu => "cpu",
    MemoryStick => "memory-stick",
    Gauge => "gauge",
    TriangleAlert => "triangle-alert",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleX => "circle-x",
    Info => "info",
    Bell => "bell",
    Ellipsis => "ellipsis",
    EllipsisVertical => "ellipsis-vertical",
    Folder => "folder",
    FolderOpen => "folder-open",
    PanelLeft => "panel-left",
    PanelRight => "panel-right",
    PanelBottom => "panel-bottom",
    Palette => "palette",
    Sun => "sun",
    Moon => "moon",
    Command => "command",
    Keyboard => "keyboard",
    List => "list",
    Table => "table",
    ChartLine => "chart-line",
    ChartBar => "chart-bar",
    Plug => "plug",
    Unplug => "unplug",
    Link => "link",
    Cloud => "cloud",
    Workflow => "workflow",
    GitBranch => "git-branch",
    Package => "package",
    Download => "download",
    Upload => "upload",
    User => "user",
    Users => "users",
    Tag => "tag",
    Hash => "hash",
    Bug => "bug",
    Zap => "zap",
    Wifi => "wifi",
    Maximize => "maximize",
    Minimize => "minimize",
    LayoutDashboard => "layout-dashboard",
    SlidersHorizontal => "sliders-horizontal",
    LoaderCircle => "loader-circle",
    Circle => "circle",
}

impl IconName {
    /// The icon served at `path` (`icons/<stem>.svg`), if any.
    pub fn from_path(path: &str) -> Option<IconName> {
        Self::ALL.iter().copied().find(|icon| icon.path() == path)
    }
}

#[cfg(test)]
mod tests;
