//! The platform shell: the parts of Kodo that live outside its own window.
//!
//! Two surfaces, both drawn by the OS rather than by the webview — the macOS
//! menu bar across the top of the screen and the menu-bar status item in the
//! top-right corner — plus the window behaviors that go with them (closing
//! hides, clicking the dock icon comes back).
//!
//! The frontend renders none of this. Menu labels are written here in Rust
//! because a native menu cannot read `i18n.ts`, and the boundary carries an
//! opaque action id rather than a string, so the GUI never holds a second copy
//! of a label that could drift. See `docs/design/COMPONENTS.md` §17.

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItem, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "macos")]
use tauri::menu::{
    AboutMetadata, CheckMenuItemBuilder, IsMenuItem, MenuItemKind, PredefinedMenuItem,
    SubmenuBuilder,
};

/// The status item declared by `tauri.conf.json` → `app.trayIcon.id`.
const TRAY_ID: &str = "main";

/// The window declared by `tauri.conf.json` → `app.windows[0].label`.
const WINDOW_LABEL: &str = "main";

const TOOLTIP_IDLE: &str = "Kodo";
const TOOLTIP_RUNNING: &str = "Kodo — 运行中";
const STATUS_IDLE: &str = "运行状态：空闲";
const STATUS_RUNNING: &str = "运行状态：运行中";

/// Channel carrying a menu or tray action to the webview.
pub const ACTION_EVENT: &str = "menu:action";

/// Menu item ids, dotted so a leaf reads as its own path. `Menu::get` and
/// `Submenu::get` are shallow — they only look at direct children — so each
/// segment is resolved one level at a time (see `apply_menu_state`).
pub mod id {
    pub const SETTINGS: &str = "kodo.app.settings";
    pub const NEW_TASK: &str = "kodo.file.new-task";
    pub const ADD_PROJECT: &str = "kodo.file.add-project";
    pub const VIEW: &str = "kodo.view";
    pub const SIDEBAR: &str = "kodo.view.sidebar";
    pub const INSPECTOR: &str = "kodo.view.inspector";
    pub const APPEARANCE: &str = "kodo.view.appearance";
    pub const THEME_LIGHT: &str = "kodo.view.appearance.light";
    pub const THEME_DARK: &str = "kodo.view.appearance.dark";
    pub const THEME_SYSTEM: &str = "kodo.view.appearance.system";
    pub const DEVTOOLS: &str = "kodo.view.devtools";
    pub const SHORTCUTS: &str = "kodo.help.shortcuts";
    pub const STATE_DIR: &str = "kodo.help.state-dir";
    pub const TRAY_SHOW: &str = "kodo.tray.show";
    pub const TRAY_NEW_TASK: &str = "kodo.tray.new-task";
    pub const TRAY_STATUS: &str = "kodo.tray.status";
    pub const TRAY_QUIT: &str = "kodo.tray.quit";
}

/// Action ids the webview receives. Deliberately few and stable: the frontend
/// dispatches them onto handles it already has, so a new menu item costs one
/// line in `App.tsx` — never a new component.
pub mod action {
    pub const NEW_TASK: &str = "new-task";
    pub const ADD_PROJECT: &str = "add-project";
    pub const OPEN_SETTINGS: &str = "open-settings";
    pub const TOGGLE_SIDEBAR: &str = "toggle-sidebar";
    pub const TOGGLE_INSPECTOR: &str = "toggle-inspector";
    pub const THEME_LIGHT: &str = "theme-light";
    pub const THEME_DARK: &str = "theme-dark";
    pub const THEME_SYSTEM: &str = "theme-system";
    pub const SHORTCUTS: &str = "shortcuts";
}

#[derive(Clone, serde::Serialize)]
struct ActionDto {
    action: String,
}

/// The status row inside the tray menu is the one item whose text changes after
/// the menu is built; `TrayIcon` gives back the menu it was handed but not the
/// items inside it, so the handle is parked here at boot.
#[derive(Clone)]
struct TrayStatus(MenuItem<tauri::Wry>);

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// Bring the main window back in front. Safe from any thread; the item runs on
/// the main thread via the channel.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Left-clicking the status item toggles the window.
fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    // Ask the window, not a cached flag: a window the user hid with ⌘H is
    // invisible and must come back rather than be "toggled away" again.
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        show_main(app);
    }
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

/// Attach the tray menu and tooltip to the status item declared in
/// `tauri.conf.json`. The icon itself is embedded at compile time by the config
/// (see the `trayIcon` block), so nothing is loaded at runtime here.
///
/// A missing status item is not an error: on a platform without one the app
/// still runs, it simply has no tray.
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let show = MenuItemBuilder::with_id(id::TRAY_SHOW, "显示 Kodo").build(app)?;
    let new_task = MenuItemBuilder::with_id(id::TRAY_NEW_TASK, "新建任务").build(app)?;
    // Read-only: the row reports, it is not a command.
    let status = MenuItemBuilder::with_id(id::TRAY_STATUS, STATUS_IDLE)
        .enabled(false)
        .build(app)?;
    let quit = MenuItemBuilder::with_id(id::TRAY_QUIT, "退出 Kodo").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&new_task)
        .separator()
        .item(&status)
        .separator()
        .item(&quit)
        .build()?;
    tray.set_menu(Some(menu))?;
    tray.set_tooltip(Some(TOOLTIP_IDLE))?;
    app.manage(TrayStatus(status));
    Ok(())
}

/// Left click toggles the window — which is why `showMenuOnLeftClick` is off in
/// `tauri.conf.json`. The menu stays on right click.
pub fn on_tray_event(app: &AppHandle, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        toggle_main(app);
    }
}

/// Mirror a run's lifecycle onto the tray: the tooltip and the status row. The
/// window may well be hidden while a run is in flight, so this is the only
/// ambient signal that the agent is still working.
pub fn set_run_status(app: &AppHandle, running: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tooltip = if running {
            TOOLTIP_RUNNING
        } else {
            TOOLTIP_IDLE
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
    if let Some(status) = app.try_state::<TrayStatus>() {
        let text = if running { STATUS_RUNNING } else { STATUS_IDLE };
        let _ = status.0.set_text(text);
    }
}

// ---------------------------------------------------------------------------
// Menu events
// ---------------------------------------------------------------------------

/// Route every menu click — app menu bar and tray menu share this channel.
///
/// Window-level items are done here; anything the webview owns is forwarded as
/// an action id. The split is "does the GUI already have this handle?", not
/// "which menu did it come from".
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().0.as_str() {
        id::TRAY_SHOW => toggle_main(app),
        id::TRAY_NEW_TASK => {
            show_main(app);
            emit(app, action::NEW_TASK);
        }
        id::TRAY_QUIT => app.exit(0),
        id::NEW_TASK => emit(app, action::NEW_TASK),
        id::ADD_PROJECT => emit(app, action::ADD_PROJECT),
        id::SETTINGS => emit(app, action::OPEN_SETTINGS),
        id::SIDEBAR => emit(app, action::TOGGLE_SIDEBAR),
        id::INSPECTOR => emit(app, action::TOGGLE_INSPECTOR),
        id::THEME_LIGHT => emit(app, action::THEME_LIGHT),
        id::THEME_DARK => emit(app, action::THEME_DARK),
        id::THEME_SYSTEM => emit(app, action::THEME_SYSTEM),
        id::SHORTCUTS => emit(app, action::SHORTCUTS),
        id::STATE_DIR => reveal_state_dir(app),
        #[cfg(debug_assertions)]
        id::DEVTOOLS => {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                window.open_devtools();
            }
        }
        // Submenu titles, predefined roles (Quit, Copy, …) and anything added
        // later that the GUI does not need to hear about.
        _ => {}
    }
}

fn emit(app: &AppHandle, action: &str) {
    let _ = app.emit(
        ACTION_EVENT,
        ActionDto {
            action: action.to_string(),
        },
    );
}

/// Open the state directory in Finder — "where is my data?" answered from the
/// Help menu. Same idea as the GUI's `reveal_project`.
pub fn reveal_state_dir(app: &AppHandle) {
    use tauri_plugin_opener::OpenerExt;

    if let Some(dir) = kodo_core::settings::state_dir() {
        let _ = app.opener().reveal_item_in_dir(dir);
    }
}

// ---------------------------------------------------------------------------
// Application menu bar (macOS)
// ---------------------------------------------------------------------------

/// Build the app menu bar. macOS-only: `Menu::default` was the only thing that
/// ever installed a menu there, so Linux has never had one and must not gain
/// one by accident.
#[cfg(target_os = "macos")]
pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let about = PredefinedMenuItem::about(app, Some("关于 Kodo"), Some(about_metadata(app)))?;
    let settings = MenuItemBuilder::with_id(id::SETTINGS, "设置…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;

    let app_menu = SubmenuBuilder::new(app, "Kodo")
        .item(&about)
        .separator()
        .item(&PredefinedMenuItem::services(app, Some("服务"))?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some("隐藏 Kodo"))?)
        .item(&PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?)
        .item(&PredefinedMenuItem::show_all(app, Some("全部显示"))?)
        .separator()
        .item(&settings)
        .separator()
        .item(&PredefinedMenuItem::quit(app, Some("退出 Kodo"))?)
        .build()?;

    let new_task = MenuItemBuilder::with_id(id::NEW_TASK, "新建任务")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let add_project = MenuItemBuilder::with_id(id::ADD_PROJECT, "添加项目…")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let file_menu = SubmenuBuilder::new(app, "文件")
        .item(&new_task)
        .item(&add_project)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, Some("关闭窗口"))?)
        .build()?;

    // Not decoration: a custom menu that drops the Edit roles takes ⌘C, ⌘V and
    // ⌘A away from the webview with it. These must stay predefined.
    let edit_menu = SubmenuBuilder::new(app, "编辑")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let sidebar = CheckMenuItemBuilder::with_id(id::SIDEBAR, "侧边栏").build(app)?;
    let inspector = CheckMenuItemBuilder::with_id(id::INSPECTOR, "检查器").build(app)?;
    let appearance = SubmenuBuilder::with_id(app, id::APPEARANCE, "外观")
        .item(&CheckMenuItemBuilder::with_id(id::THEME_LIGHT, "浅色").build(app)?)
        .item(&CheckMenuItemBuilder::with_id(id::THEME_DARK, "深色").build(app)?)
        .item(&CheckMenuItemBuilder::with_id(id::THEME_SYSTEM, "跟随系统").build(app)?)
        .build()?;
    let before_appearance = PredefinedMenuItem::separator(app)?;
    let before_fullscreen = PredefinedMenuItem::separator(app)?;
    let fullscreen = PredefinedMenuItem::fullscreen(app, Some("进入全屏"))?;
    #[cfg(debug_assertions)]
    let devtools = MenuItemBuilder::with_id(id::DEVTOOLS, "开发者工具").build(app)?;
    let mut view_items: Vec<&dyn IsMenuItem<_>> = vec![
        &sidebar,
        &inspector,
        &before_appearance,
        &appearance,
        &before_fullscreen,
        &fullscreen,
    ];
    #[cfg(debug_assertions)]
    view_items.push(&devtools);
    let view_menu = SubmenuBuilder::with_id(app, id::VIEW, "显示")
        .items(&view_items)
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "窗口")
        .item(&PredefinedMenuItem::minimize(app, Some("最小化"))?)
        .item(&PredefinedMenuItem::maximize(app, Some("缩放"))?)
        .build()?;

    let shortcuts = MenuItemBuilder::with_id(id::SHORTCUTS, "键盘快捷键").build(app)?;
    let state_dir = MenuItemBuilder::with_id(id::STATE_DIR, "打开状态目录").build(app)?;
    let help_menu = SubmenuBuilder::new(app, "帮助")
        .item(&shortcuts)
        .item(&state_dir)
        .build()?;

    let items: [&dyn IsMenuItem<_>; 6] = [
        &app_menu,
        &file_menu,
        &edit_menu,
        &view_menu,
        &window_menu,
        &help_menu,
    ];
    MenuBuilder::new(app).items(&items).build()
}

/// Contents of the native About panel. Version comes from
/// `PackageInfo`, which falls back to `CARGO_PKG_VERSION` now that
/// `tauri.conf.json` no longer carries a `version` — so this panel and
/// `kodo_core::info()` can never disagree.
#[cfg(target_os = "macos")]
fn about_metadata(app: &AppHandle) -> AboutMetadata<'_> {
    let mut credits = String::from("Rust · Tauri 2 · React 19\nMIT License");
    if let Some(dir) = kodo_core::settings::state_dir() {
        credits.push_str(&format!("\n状态目录：{}", dir.display()));
    }
    // Name and copyright are read from the config rather than repeated here:
    // `productName` and `bundle.copyright` are the values the bundle itself
    // carries, so the About panel cannot describe a different app than the one
    // that was installed.
    AboutMetadata {
        name: Some(app.package_info().name.clone()),
        version: Some(app.package_info().version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        credits: Some(credits),
        icon: app.default_window_icon().cloned(),
        ..Default::default()
    }
}

/// Push the GUI's real state onto the menu's check marks. Called on every
/// change of sidebar, inspector or theme, so the menu can never show a stale
/// check: the webview is the source of truth and the menu only mirrors it.
#[cfg(target_os = "macos")]
pub fn apply_menu_state(app: &AppHandle, sidebar: bool, inspector: bool, theme: &str) {
    let Some(menu) = app.menu() else {
        return;
    };
    apply_check(menu.get(id::SIDEBAR), sidebar);
    apply_check(menu.get(id::INSPECTOR), inspector);
    // Two shallow hops: `get` never recurses, so 显示 → 外观 → leaf.
    let Some(MenuItemKind::Submenu(view)) = menu.get(id::VIEW) else {
        return;
    };
    let Some(MenuItemKind::Submenu(appearance)) = view.get(id::APPEARANCE) else {
        return;
    };
    apply_check(appearance.get(id::THEME_LIGHT), theme == "light");
    apply_check(appearance.get(id::THEME_DARK), theme == "dark");
    apply_check(appearance.get(id::THEME_SYSTEM), theme == "system");
}

#[cfg(target_os = "macos")]
fn apply_check(item: Option<MenuItemKind<tauri::Wry>>, checked: bool) {
    if let Some(MenuItemKind::Check(item)) = item {
        let _ = item.set_checked(checked);
    }
}

/// Non-macOS builds have no menu bar; the GUI still calls this on every state
/// change, so it has to exist.
#[cfg(not(target_os = "macos"))]
pub fn apply_menu_state(_app: &AppHandle, _sidebar: bool, _inspector: bool, _theme: &str) {}
