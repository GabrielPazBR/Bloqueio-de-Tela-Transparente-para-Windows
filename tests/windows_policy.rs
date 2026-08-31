use bloqueio_transparente::windows_policy::{
    ClockWidgetLayout, ImageLayout, KeyDecision, KeyEvent, LayeredSurface, MonitorRect,
    OverlayLayout, TrayAction, TrayMenuAction, VirtualKey, WidgetLayout, clock_date_label,
    dimming_alpha, layered_surface_alpha, should_quit_on_window_destroy, tray_action,
    tray_menu_action, trusted_agent_process,
};
use std::time::Duration;

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
fn tray_exit_command_requests_a_complete_shutdown() {
    assert_eq!(tray_menu_action(1004), TrayMenuAction::Shutdown);
    assert_eq!(tray_menu_action(1001), TrayMenuAction::Lock);
    assert_eq!(tray_menu_action(9999), TrayMenuAction::Ignore);
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
fn widget_surface_stays_opaque_independently_from_screen_dimming() {
    assert_eq!(layered_surface_alpha(LayeredSurface::Dimming, 0), 1);
    assert_eq!(layered_surface_alpha(LayeredSurface::Dimming, 75), 191);
    assert_eq!(layered_surface_alpha(LayeredSurface::Widget, 0), 255);
    assert_eq!(layered_surface_alpha(LayeredSurface::Widget, 75), 255);
    assert_eq!(layered_surface_alpha(LayeredSurface::Widget, 100), 255);
}

#[test]
fn widget_transparency_fades_a_color_channel_as_the_slider_increases() {
    use bloqueio_transparente::windows_policy::{
        apply_widget_opacity, blend_channel_over_background, unlock_logo_channel,
        widget_text_channel,
    };

    assert_eq!(apply_widget_opacity(255, 0), 255);
    assert_eq!(apply_widget_opacity(255, 40), 153);
    assert_eq!(apply_widget_opacity(200, 50), 100);
    assert_eq!(apply_widget_opacity(255, 100), 0);
    assert_eq!(widget_text_channel(0), 255);
    assert_eq!(widget_text_channel(40), 153);
    assert_eq!(widget_text_channel(100), 0);
    assert_eq!(unlock_logo_channel(0), 0);
    assert_eq!(unlock_logo_channel(127), 127);
    assert_eq!(unlock_logo_channel(255), 255);
    assert_eq!(blend_channel_over_background(255, 20, 0), 20);
    assert_eq!(blend_channel_over_background(255, 20, 255), 255);
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

#[test]
fn inactivity_lock_is_due_only_after_the_configured_timeout() {
    use bloqueio_transparente::windows_policy::inactivity_lock_due;

    assert!(!inactivity_lock_due(0, Duration::from_secs(3_600), false));
    assert!(!inactivity_lock_due(5, Duration::from_secs(299), false));
    assert!(inactivity_lock_due(5, Duration::from_secs(300), false));
    assert!(!inactivity_lock_due(5, Duration::from_secs(600), true));
}

#[test]
fn transparent_window_does_not_reclaim_focus_during_windows_hello() {
    use bloqueio_transparente::windows_policy::should_enforce_lock_foreground;

    assert!(should_enforce_lock_foreground(true, false));
    assert!(!should_enforce_lock_foreground(true, true));
    assert!(!should_enforce_lock_foreground(false, false));
}

#[test]
fn input_hooks_release_events_while_windows_hello_is_open() {
    use bloqueio_transparente::windows_policy::should_block_lock_input;

    assert!(should_block_lock_input(true, true, false));
    assert!(!should_block_lock_input(true, true, true));
    assert!(!should_block_lock_input(false, true, false));
    assert!(!should_block_lock_input(true, false, false));
}

#[test]
fn windows_hello_keeps_the_permanent_win_l_hook_installed() {
    use bloqueio_transparente::windows_policy::should_suspend_hooks_for_windows_hello;

    assert!(!should_suspend_hooks_for_windows_hello(true));
    assert!(should_suspend_hooks_for_windows_hello(false));
}

#[test]
fn win_l_starts_transparent_lock_only_when_replacement_is_enabled_and_screen_is_free() {
    use bloqueio_transparente::windows_policy::should_trigger_transparent_lock;

    assert!(should_trigger_transparent_lock(
        true, false, 0x4c, true, true
    ));
    assert!(!should_trigger_transparent_lock(
        false, false, 0x4c, true, true
    ));
    assert!(!should_trigger_transparent_lock(
        true, true, 0x4c, true, true
    ));
    assert!(!should_trigger_transparent_lock(
        true, false, 0x4c, false, true
    ));
    assert!(!should_trigger_transparent_lock(
        true, false, 0x4c, true, false
    ));
}

#[test]
fn win_l_uses_the_key_events_reported_by_the_low_level_hook() {
    use bloqueio_transparente::windows_policy::{
        next_windows_key_mask, should_forward_windows_key_up,
    };

    let mut windows_keys = 0;
    windows_keys = next_windows_key_mask(windows_keys, 0x5b, true, false);
    assert_ne!(windows_keys, 0);
    assert!(
        bloqueio_transparente::windows_policy::should_trigger_transparent_lock(
            true,
            false,
            0x4c,
            windows_keys != 0,
            true,
        )
    );
    windows_keys = next_windows_key_mask(windows_keys, 0x5b, false, true);
    assert_eq!(windows_keys, 0);

    windows_keys = next_windows_key_mask(windows_keys, 0x5c, true, false);
    assert_ne!(windows_keys, 0);
    windows_keys = next_windows_key_mask(windows_keys, 0x5c, false, true);
    assert_eq!(windows_keys, 0);

    let passed_through_before_lock = 1;
    assert!(should_forward_windows_key_up(
        passed_through_before_lock,
        0x5b,
        true,
    ));
    assert!(!should_forward_windows_key_up(0, 0x5b, true));
    assert!(!should_forward_windows_key_up(
        passed_through_before_lock,
        0x41,
        true,
    ));
}

#[test]
fn win_l_policy_restores_the_value_that_existed_before_replacement() {
    use bloqueio_transparente::windows_policy::{WinLRestoreAction, win_l_restore_action};

    assert_eq!(
        win_l_restore_action(Some(2), Some(1)),
        WinLRestoreAction::Delete
    );
    assert_eq!(
        win_l_restore_action(Some(0), Some(1)),
        WinLRestoreAction::Write(0)
    );
    assert_eq!(
        win_l_restore_action(Some(1), Some(1)),
        WinLRestoreAction::Write(1)
    );
    assert_eq!(
        win_l_restore_action(None, Some(1)),
        WinLRestoreAction::Write(0)
    );
    assert_eq!(
        win_l_restore_action(None, None),
        WinLRestoreAction::NoChange
    );
}
