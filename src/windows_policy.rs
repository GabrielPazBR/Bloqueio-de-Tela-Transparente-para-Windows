#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    OpenMenu,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAction {
    Lock,
    OpenSettings,
    ShowStatus,
    Shutdown,
    Ignore,
}

pub const fn tray_menu_action(command: usize) -> TrayMenuAction {
    match command {
        1001 => TrayMenuAction::Lock,
        1002 => TrayMenuAction::OpenSettings,
        1003 => TrayMenuAction::ShowStatus,
        1004 => TrayMenuAction::Shutdown,
        _ => TrayMenuAction::Ignore,
    }
}

pub fn tray_action(encoded_event: u32) -> TrayAction {
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const NIN_SELECT: u32 = 0x0400;
    const NIN_KEYSELECT: u32 = 0x0401;

    match encoded_event & 0xffff {
        WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT => TrayAction::OpenSettings,
        WM_RBUTTONUP | WM_CONTEXTMENU | NIN_KEYSELECT => TrayAction::OpenMenu,
        _ => TrayAction::Ignore,
    }
}

pub fn should_restore_tray_icon(message: u32, taskbar_created_message: u32) -> bool {
    taskbar_created_message != 0 && message == taskbar_created_message
}

pub fn trusted_agent_process(expected_process_id: Option<u32>, actual_process_id: u32) -> bool {
    expected_process_id.is_some_and(|expected| expected == actual_process_id)
}

pub const fn should_quit_on_window_destroy(window: isize, manager_window: isize) -> bool {
    manager_window != 0 && window == manager_window
}

pub fn dimming_alpha(percent: u8) -> u8 {
    (((u16::from(percent.min(100)) * 255) + 50) / 100).max(1) as u8
}

pub fn inactivity_lock_due(
    timeout_minutes: u16,
    idle_duration: std::time::Duration,
    already_locked: bool,
) -> bool {
    !already_locked
        && timeout_minutes > 0
        && idle_duration >= std::time::Duration::from_secs(u64::from(timeout_minutes) * 60)
}

pub fn should_enforce_lock_foreground(locked: bool, windows_hello_in_progress: bool) -> bool {
    locked && !windows_hello_in_progress
}

pub fn should_block_lock_input(locked: bool, windows_hello_in_progress: bool) -> bool {
    locked && !windows_hello_in_progress
}

pub fn should_suspend_hooks_for_windows_hello(win_l_replacement_enabled: bool) -> bool {
    !win_l_replacement_enabled
}

pub fn should_trigger_transparent_lock(
    replacement_enabled: bool,
    locked: bool,
    virtual_key: u32,
    windows_key_down: bool,
    key_down: bool,
) -> bool {
    replacement_enabled && !locked && virtual_key == 0x4c && windows_key_down && key_down
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl MonitorRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayLayout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetLayout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub const fn apply_widget_opacity(channel: u8, opacity_percentage: u8) -> u8 {
    let bounded = if opacity_percentage > 100 {
        100
    } else {
        opacity_percentage
    };
    let visibility = 100 - bounded;
    ((channel as u16 * visibility as u16 + 50) / 100) as u8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockWidgetLayout {
    pub outer: WidgetLayout,
    pub inner: WidgetLayout,
    pub time: WidgetLayout,
    pub date: WidgetLayout,
    pub corner_radius: i32,
    pub time_font_size: i32,
    pub date_font_size: i32,
}

impl ClockWidgetLayout {
    pub fn from_widget(widget: WidgetLayout) -> Self {
        let shortest = widget.width.min(widget.height).max(1);
        let shell = (shortest / 22).clamp(3, 8);
        let padding_x = (widget.width / 18).clamp(12, 36);
        let padding_y = (widget.height / 12).clamp(6, 18);
        let inner = inset(widget, shell);
        let content = WidgetLayout {
            x: inner.x + padding_x,
            y: inner.y + padding_y,
            width: (inner.width - padding_x * 2).max(1),
            height: (inner.height - padding_y * 2).max(1),
        };
        let time_height = (content.height * 68 / 100).max(1);
        let date_height = (content.height - time_height).max(1);
        let time = WidgetLayout {
            height: time_height,
            ..content
        };
        let date = WidgetLayout {
            x: content.x,
            y: content.y + time_height,
            width: content.width,
            height: date_height,
        };
        Self {
            outer: widget,
            inner,
            time,
            date,
            corner_radius: (shortest / 7).clamp(10, 30),
            time_font_size: (time.height * 88 / 100).clamp(30, 112),
            date_font_size: (date.height * 58 / 100).clamp(11, 24),
        }
    }
}

fn inset(layout: WidgetLayout, amount: i32) -> WidgetLayout {
    WidgetLayout {
        x: layout.x + amount,
        y: layout.y + amount,
        width: (layout.width - amount * 2).max(1),
        height: (layout.height - amount * 2).max(1),
    }
}

pub fn clock_date_label(day_of_week: u16, day: u16, month: u16, year: u16) -> String {
    const DAYS: [&str; 7] = ["DOM", "SEG", "TER", "QUA", "QUI", "SEX", "SÁB"];
    const MONTHS: [&str; 12] = [
        "JAN", "FEV", "MAR", "ABR", "MAI", "JUN", "JUL", "AGO", "SET", "OUT", "NOV", "DEZ",
    ];
    let weekday = DAYS.get(usize::from(day_of_week)).copied().unwrap_or("");
    let month = month
        .checked_sub(1)
        .and_then(|index| MONTHS.get(usize::from(index)))
        .copied()
        .unwrap_or("");
    format!("{weekday}  ·  {day:02} {month} {year:04}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLayout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ImageLayout {
    pub fn contain(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        source_width: u32,
        source_height: u32,
    ) -> Option<Self> {
        if width <= 0 || height <= 0 || source_width == 0 || source_height == 0 {
            return None;
        }

        let box_width = i64::from(width);
        let box_height = i64::from(height);
        let source_width = i64::from(source_width);
        let source_height = i64::from(source_height);
        let (fitted_width, fitted_height) =
            if box_width * source_height <= box_height * source_width {
                (box_width, (box_width * source_height / source_width).max(1))
            } else {
                (
                    (box_height * source_width / source_height).max(1),
                    box_height,
                )
            };
        let fitted_width = i32::try_from(fitted_width).ok()?;
        let fitted_height = i32::try_from(fitted_height).ok()?;

        Some(Self {
            x: x + (width - fitted_width) / 2,
            y: y + (height - fitted_height) / 2,
            width: fitted_width,
            height: fitted_height,
        })
    }
}

impl WidgetLayout {
    pub fn place(
        monitor: MonitorRect,
        width: i32,
        height: i32,
        x_percent: u8,
        y_percent: u8,
    ) -> Self {
        let monitor_width = monitor.right - monitor.left;
        let monitor_height = monitor.bottom - monitor.top;
        let center_x = monitor.left + monitor_width * i32::from(x_percent.min(100)) / 100;
        Self {
            x: center_x - width / 2,
            y: monitor.top + monitor_height * i32::from(y_percent.min(100)) / 100,
            width,
            height,
        }
    }
}

pub fn central_monitor(monitors: &[MonitorRect]) -> Option<MonitorRect> {
    let left = monitors.iter().map(|monitor| monitor.left).min()?;
    let right = monitors.iter().map(|monitor| monitor.right).max()?;
    let top = monitors.iter().map(|monitor| monitor.top).min()?;
    let bottom = monitors.iter().map(|monitor| monitor.bottom).max()?;
    let center_x = i64::from(left) + i64::from(right - left) / 2;
    let center_y = i64::from(top) + i64::from(bottom - top) / 2;
    monitors.iter().copied().min_by_key(|monitor| {
        let monitor_x = i64::from(monitor.left) + i64::from(monitor.right - monitor.left) / 2;
        let monitor_y = i64::from(monitor.top) + i64::from(monitor.bottom - monitor.top) / 2;
        (monitor_x - center_x).pow(2) + (monitor_y - center_y).pow(2)
    })
}

impl OverlayLayout {
    pub fn from_monitors(monitors: &[MonitorRect]) -> Vec<Self> {
        monitors
            .iter()
            .filter_map(|monitor| {
                let width = monitor.right.checked_sub(monitor.left)?;
                let height = monitor.bottom.checked_sub(monitor.top)?;
                (width > 0 && height > 0).then_some(Self {
                    x: monitor.left,
                    y: monitor.top,
                    width,
                    height,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualKey {
    Tab,
    Escape,
    LWin,
    RWin,
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: VirtualKey,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub key_down: bool,
}

impl KeyEvent {
    pub const fn down(key: VirtualKey) -> Self {
        Self {
            key,
            control: false,
            alt: false,
            shift: false,
            key_down: true,
        }
    }

    pub const fn with_control(mut self) -> Self {
        self.control = true;
        self
    }

    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn decision(self, locked: bool, _lock_window_foreground: bool) -> KeyDecision {
        if !locked {
            return KeyDecision::PassThrough;
        }
        let system_shortcut = matches!(self.key, VirtualKey::LWin | VirtualKey::RWin)
            || (self.alt && self.key == VirtualKey::Tab)
            || (self.control && self.key == VirtualKey::Escape);
        if system_shortcut {
            KeyDecision::Consume
        } else {
            KeyDecision::ForwardToLockWindow
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDecision {
    Consume,
    ForwardToLockWindow,
    PassThrough,
}

pub fn win_l_registry_update_needed(current: Option<u32>, enabled: bool) -> bool {
    match (current, enabled) {
        (None, false) => false,
        (Some(value), expected) => value != u32::from(expected),
        (None, true) => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinLRestoreAction {
    NoChange,
    Delete,
    Write(u32),
}

pub fn win_l_restore_action(
    saved_original: Option<u32>,
    current: Option<u32>,
) -> WinLRestoreAction {
    match saved_original {
        Some(2) => WinLRestoreAction::Delete,
        Some(value) => WinLRestoreAction::Write(value),
        None if current.is_some_and(|value| value != 0) => WinLRestoreAction::Write(0),
        None => WinLRestoreAction::NoChange,
    }
}
