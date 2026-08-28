use bloqueio_transparente::windows_policy::{
    ClockWidgetLayout, ImageLayout, KeyDecision, KeyEvent, MonitorRect, OverlayLayout, TrayAction,
    VirtualKey, WidgetLayout, clock_date_label, dimming_alpha, should_quit_on_window_destroy,
    tray_action, trusted_agent_process,
};

#[test]
fn tray_clicks_open_settings_or_the_context_menu() {
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_RBUTTONUP: u32 = 0x0205;
    const NIN_SELECT: u32 = 0x0400;

    assert_eq!(tray_action(WM_LBUTTONUP), TrayAction::OpenSettings);
    assert_eq!(tray_action(NIN_SELECT), TrayAction::OpenSettings);
    assert_eq!(tray_action(WM_RBUTTONUP), TrayAction::OpenMenu);
}

#[test]
fn tray_event_ignores_the_icon_id_in_the_high_word() {
    const WM_RBUTTONUP: u32 = 0x0205;
    let version_four_event = (1 << 16) | WM_RBUTTONUP;

    assert_eq!(tray_action(version_four_event), TrayAction::OpenMenu);
}

#[test]
fn only_destroying_the_manager_window_ends_the_agent() {
    assert!(!should_quit_on_window_destroy(200, 100));
    assert!(should_quit_on_window_destroy(100, 100));
    assert!(!should_quit_on_window_destroy(0, 0));
}

#[test]
fn custom_images_keep_their_aspect_ratio_and_are_centered() {
    assert_eq!(
        ImageLayout::contain(100, 20, 160, 72, 1200, 600),
        Some(ImageLayout {
            x: 108,
            y: 20,
            width: 144,
            height: 72,
        })
    );
    assert_eq!(
        ImageLayout::contain(100, 20, 160, 72, 600, 1200),
        Some(ImageLayout {
            x: 162,
            y: 20,
            width: 36,
            height: 72,
        })
    );
    assert_eq!(
        ImageLayout::contain(100, 20, 160, 72, 600, 600),
        Some(ImageLayout {
            x: 144,
            y: 20,
            width: 72,
            height: 72,
        })
    );
}

#[test]
fn invalid_image_dimensions_are_not_drawn() {
    assert_eq!(ImageLayout::contain(0, 0, 160, 72, 0, 600), None);
    assert_eq!(ImageLayout::contain(0, 0, 0, 72, 600, 600), None);
}

#[test]
fn explorer_recreation_requests_the_tray_icon_again() {
    use bloqueio_transparente::windows_policy::should_restore_tray_icon;
    assert!(should_restore_tray_icon(0xc123, 0xc123));
    assert!(!should_restore_tray_icon(0xc124, 0xc123));
    assert!(!should_restore_tray_icon(0, 0));
}

#[test]
fn privileged_agent_messages_require_the_spawned_process_id() {
    assert!(trusted_agent_process(Some(4242), 4242));
    assert!(!trusted_agent_process(Some(4242), 99));
    assert!(!trusted_agent_process(None, 4242));
}

#[test]
fn dimming_percentage_maps_to_layered_window_alpha() {
    assert_eq!(dimming_alpha(0), 1);
    assert_eq!(dimming_alpha(50), 128);
    assert_eq!(dimming_alpha(100), 255);
}

#[test]
fn one_overlay_is_created_for_each_monitor_including_negative_coordinates() {
    let monitors = vec![
        MonitorRect::new(-1920, 0, 0, 1080),
        MonitorRect::new(0, 0, 2560, 1440),
    ];

    let overlays = OverlayLayout::from_monitors(&monitors);

    assert_eq!(overlays.len(), 2);
    assert_eq!(overlays[0].x, -1920);
    assert_eq!(overlays[0].width, 1920);
    assert_eq!(overlays[1].height, 1440);
}

#[test]
fn dangerous_system_shortcuts_are_always_consumed_while_locked() {
    let cases = [
        KeyEvent::down(VirtualKey::Tab).with_alt(),
        KeyEvent::down(VirtualKey::Escape).with_control(),
        KeyEvent::down(VirtualKey::LWin),
        KeyEvent::down(VirtualKey::RWin),
        KeyEvent::down(VirtualKey::Escape)
            .with_control()
            .with_shift(),
    ];

    for event in cases {
        assert_eq!(event.decision(true, true), KeyDecision::Consume);
    }
}

#[test]
fn ordinary_keys_are_forwarded_even_when_the_lock_window_loses_foreground() {
    let letter = KeyEvent::down(VirtualKey::Other(0x41));
    assert_eq!(
        letter.decision(true, true),
        KeyDecision::ForwardToLockWindow
    );
    assert_eq!(
        letter.decision(true, false),
        KeyDecision::ForwardToLockWindow
    );
    assert_eq!(letter.decision(false, false), KeyDecision::PassThrough);
}

#[test]
fn widget_position_uses_percentages_inside_the_primary_monitor() {
    use bloqueio_transparente::windows_policy::WidgetLayout;
    let monitor = MonitorRect::new(0, 0, 1920, 1080);
    assert_eq!(
        WidgetLayout::place(monitor, 400, 120, 50, 5),
        WidgetLayout {
            x: 760,
            y: 54,
            width: 400,
            height: 120
        }
    );
}

#[test]
fn widget_uses_the_geometrically_central_monitor() {
    use bloqueio_transparente::windows_policy::central_monitor;
    let monitors = [
        MonitorRect::new(-1920, 0, 0, 1080),
        MonitorRect::new(0, 0, 1920, 1080),
        MonitorRect::new(1920, 0, 3840, 1080),
    ];
    assert_eq!(central_monitor(&monitors), Some(monitors[1]));
}

#[test]
fn clock_widget_gives_the_time_more_space_and_typographic_weight() {
    let layout = ClockWidgetLayout::from_widget(WidgetLayout {
        x: 760,
        y: 54,
        width: 400,
        height: 120,
    });

    assert!(layout.time.height > layout.date.height);
    assert!(layout.time_font_size >= layout.date_font_size * 3);
    assert!(layout.inner.width < layout.outer.width);
    assert!(layout.inner.height < layout.outer.height);
}

#[test]
fn clock_widget_formats_a_compact_portuguese_date() {
    assert_eq!(clock_date_label(5, 28, 8, 2026), "SEX  ·  28 AGO 2026");
}

#[test]
fn matching_win_l_registry_value_does_not_require_a_write() {
    use bloqueio_transparente::windows_policy::win_l_registry_update_needed;
    assert!(!win_l_registry_update_needed(Some(0), false));
    assert!(!win_l_registry_update_needed(Some(1), true));
    assert!(win_l_registry_update_needed(Some(0), true));
    assert!(!win_l_registry_update_needed(None, false));
}
