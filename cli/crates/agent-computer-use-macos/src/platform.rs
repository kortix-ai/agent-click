use agent_computer_use_core::action::{Action, ActionResult};
use agent_computer_use_core::node::AccessibilityNode;
use agent_computer_use_core::platform::{AppInfo, Platform, WindowInfo};
use agent_computer_use_core::selector::Selector;
use agent_computer_use_core::{Error, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::ax;
use crate::input;

const PID_CACHE_TTL: Duration = Duration::from_secs(30);

struct PidEntry {
    pid: i32,
    expires: Instant,
}

pub struct MacOSPlatform {
    pid_cache: Mutex<HashMap<String, PidEntry>>,
}

impl MacOSPlatform {
    pub fn new() -> Self {
        Self {
            pid_cache: Mutex::new(HashMap::new()),
        }
    }

    fn running_apps(&self) -> Vec<(i32, String)> {
        running_apps_native()
    }

    fn activate_app(&self, app_name: &str) -> Result<()> {
        let pid = self.find_app_pid(app_name)?;

        ax::raise_window(pid);

        if parse_pid_target(app_name).is_some() {
            std::thread::sleep(Duration::from_millis(200));
            return Ok(());
        }

        let output = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!(r#"tell application "{app_name}" to activate"#),
            ])
            .output()
            .map_err(|e| Error::PlatformError {
                message: format!("failed to activate {app_name}: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::PlatformError {
                message: format!("failed to activate {app_name}: {stderr}"),
            });
        }

        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    fn find_app_pid(&self, app_name: &str) -> Result<i32> {
        if let Some(pid) = parse_pid_target(app_name) {
            return Ok(pid);
        }

        let lower = app_name.to_lowercase();

        {
            let cache = self.pid_cache.lock().unwrap();
            if let Some(entry) = cache.get(&lower) {
                if entry.expires > Instant::now() {
                    return Ok(entry.pid);
                }
            }
        }

        let apps = self.running_apps();
        let found = apps
            .iter()
            .find(|(_, name)| name.to_lowercase() == lower)
            .or_else(|| {
                apps.iter()
                    .find(|(_, name)| name.to_lowercase().starts_with(&lower))
            })
            .map(|(pid, _)| *pid)
            .ok_or_else(|| Error::ApplicationNotFound {
                name: app_name.to_string(),
            })?;

        {
            let mut cache = self.pid_cache.lock().unwrap();
            let expires = Instant::now() + PID_CACHE_TTL;
            for (pid, name) in &apps {
                cache.insert(name.to_lowercase(), PidEntry { pid: *pid, expires });
            }
        }

        Ok(found)
    }

    fn find_app_pids(&self, app_name: &str) -> Result<Vec<(i32, String)>> {
        if let Some(pid) = parse_pid_target(app_name) {
            let name = self
                .running_apps()
                .into_iter()
                .find(|(app_pid, _)| *app_pid == pid)
                .map(|(_, name)| name)
                .unwrap_or_else(|| app_name.to_string());
            return Ok(vec![(pid, name)]);
        }

        let lower = app_name.to_lowercase();
        let apps = self.running_apps();
        let mut matches: Vec<(i32, String)> = apps
            .iter()
            .filter(|(_, name)| name.to_lowercase() == lower)
            .cloned()
            .collect();

        if matches.is_empty() {
            matches = apps
                .iter()
                .filter(|(_, name)| name.to_lowercase().starts_with(&lower))
                .cloned()
                .collect();
        }

        if matches.is_empty() {
            return Err(Error::ApplicationNotFound {
                name: app_name.to_string(),
            });
        }

        Ok(matches)
    }
}

fn parse_pid_target(app_name: &str) -> Option<i32> {
    let pid = app_name.strip_prefix("pid:")?.parse::<i32>().ok()?;
    (pid > 0).then_some(pid)
}

impl MacOSPlatform {
    pub fn ax_press(&self, selector: &Selector) -> Option<()> {
        let root = match &selector.app {
            Some(name) => {
                let pid = self.find_app_pid(name).ok()?;
                ax::application_element(pid)
            }
            None => ax::system_wide_element(),
        };
        if ax::press_element(root, selector) {
            Some(())
        } else {
            None
        }
    }
}

impl Default for MacOSPlatform {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for MacOSPlatform {}
unsafe impl Sync for MacOSPlatform {}

#[async_trait]
impl Platform for MacOSPlatform {
    async fn tree(&self, app: Option<&str>, max_depth: Option<u32>) -> Result<AccessibilityNode> {
        if !ax::is_trusted() {
            return Err(Error::PermissionDenied {
                message: "Accessibility permission not granted. \
                          Go to System Settings > Privacy & Security > Accessibility \
                          and add this application."
                    .into(),
            });
        }

        match app {
            Some(name) => {
                let pid = self.find_app_pid(name)?;
                let element = ax::application_element(pid);
                Ok(ax::element_to_node(element, max_depth, 0))
            }
            None => {
                let apps = self.running_apps();
                let children: Vec<AccessibilityNode> = apps
                    .into_iter()
                    .map(|(pid, _)| {
                        let element = ax::application_element(pid);
                        ax::element_to_node(element, max_depth, 1)
                    })
                    .collect();

                Ok(AccessibilityNode {
                    role: agent_computer_use_core::node::Role::SystemWide,
                    name: Some("System".into()),
                    value: None,
                    description: None,
                    id: None,
                    position: None,
                    size: None,
                    focused: None,
                    enabled: None,
                    pid: None,
                    children,
                })
            }
        }
    }

    async fn find(&self, selector: &Selector) -> Result<Vec<AccessibilityNode>> {
        if !ax::is_trusted() {
            return Err(Error::PermissionDenied {
                message: "Accessibility permission not granted. \
                          Go to System Settings > Privacy & Security > Accessibility \
                          and add this application."
                    .into(),
            });
        }

        match &selector.app {
            Some(name) => {
                let root = ax::application_element(self.find_app_pid(name)?);
                Ok(ax::find_all_native(root, selector))
            }
            None => {
                let mut all_results = Vec::new();
                for (pid, _) in self.running_apps() {
                    let root = ax::application_element(pid);
                    all_results.extend(ax::find_all_native(root, selector));
                }
                Ok(all_results)
            }
        }
    }

    async fn find_one(&self, selector: &Selector) -> Result<AccessibilityNode> {
        if !ax::is_trusted() {
            return Err(Error::PermissionDenied {
                message: "Accessibility permission not granted. \
                          Go to System Settings > Privacy & Security > Accessibility \
                          and add this application."
                    .into(),
            });
        }

        match &selector.app {
            Some(name) => {
                let root = ax::application_element(self.find_app_pid(name)?);
                ax::find_first_native(root, selector).ok_or_else(|| Error::ElementNotFound {
                    message: format!("{selector:?}"),
                })
            }
            None => {
                for (pid, _) in self.running_apps() {
                    let root = ax::application_element(pid);
                    if let Some(node) = ax::find_first_native(root, selector) {
                        return Ok(node);
                    }
                }
                Err(Error::ElementNotFound {
                    message: format!("{selector:?}"),
                })
            }
        }
    }

    async fn perform(&self, action: &Action) -> Result<ActionResult> {
        match action {
            Action::Click {
                selector,
                coordinates,
                button,
                count,
            } => {
                let (point, target_pid) = match (selector, coordinates) {
                    (_, Some(coords)) => {
                        let pid = selector
                            .as_ref()
                            .and_then(|s| s.app.as_ref())
                            .map(|name| self.find_app_pid(name))
                            .transpose()?;
                        (*coords, pid)
                    }
                    (Some(sel), None) => {
                        let node = self.find_one(sel).await?;
                        let pid = sel
                            .app
                            .as_ref()
                            .map(|name| self.find_app_pid(name))
                            .transpose()?;
                        let center = node.center().ok_or_else(|| Error::PlatformError {
                            message: "element has no position/size — cannot compute click target"
                                .into(),
                        })?;
                        (center, pid)
                    }
                    (None, None) => {
                        return Err(Error::PlatformError {
                            message: "click requires either a selector or coordinates".into(),
                        });
                    }
                };

                match target_pid {
                    Some(pid) => {
                        input::click_to_pid(point, *button, *count, pid)?;
                    }
                    None => {
                        input::click(point, *button, *count)?;
                    }
                }

                Ok(ActionResult {
                    success: true,
                    message: Some(format!("clicked at ({}, {})", point.x, point.y)),
                    path: None,
                    data: None,
                })
            }

            Action::Type {
                text,
                selector,
                submit,
            } => {
                if let Some(sel) = selector {
                    let root = match &sel.app {
                        Some(name) => ax::application_element(self.find_app_pid(name)?),
                        None => ax::system_wide_element(),
                    };

                    let element = ax::find_first_element(root, sel).ok_or_else(|| {
                        Error::ElementNotFound {
                            message: format!("{sel:?}"),
                        }
                    })?;

                    let node = ax::element_to_node(element, Some(0), 0);
                    ax::release_element(element);

                    let point = node.center().ok_or_else(|| Error::PlatformError {
                        message: "element has no position/size".into(),
                    })?;

                    if let Some(ref name) = sel.app {
                        self.activate_app(name)?;
                    }
                    input::click(point, agent_computer_use_core::action::MouseButton::Left, 1)?;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    input::key_press("cmd+a")?;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    input::key_press("backspace")?;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    input::type_text(text)?;

                    if *submit {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        input::key_press("return")?;
                    }
                } else {
                    input::type_text(text)?;

                    if *submit {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        input::key_press("return")?;
                    }
                }

                let msg = if *submit {
                    format!("typed {} characters and submitted", text.len())
                } else {
                    format!("typed {} characters", text.len())
                };

                Ok(ActionResult {
                    success: true,
                    message: Some(msg),
                    path: None,
                    data: None,
                })
            }

            Action::KeyPress { key, app } => {
                match app {
                    Some(name) => {
                        if let Some(pid) = parse_pid_target(name) {
                            ax::raise_window(pid);
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            input::key_press(key)?;
                        } else {
                            input::stealth_activate(name, || input::key_press(key))?;
                        }
                    }
                    None => {
                        input::key_press(key)?;
                    }
                }

                Ok(ActionResult {
                    success: true,
                    message: Some(format!("pressed {key}")),
                    path: None,
                    data: None,
                })
            }

            Action::Scroll {
                direction,
                amount,
                selector,
                app,
            } => {
                if let Some(app_name) = app {
                    let pid = self.find_app_pid(app_name)?;

                    let scroll_point = if let Some(sel) = selector {
                        let results = ax::find_all_native(ax::application_element(pid), sel);
                        results.first().and_then(|n| n.center())
                    } else {
                        find_best_scroll_point(pid)
                    };

                    self.activate(app_name).await?;

                    if let Some(point) = scroll_point {
                        tracing::debug!("scrolling at ({}, {})", point.x, point.y);
                        input::move_mouse_to_pid(point, pid)?;
                    }

                    std::thread::sleep(std::time::Duration::from_millis(50));
                    input::scroll_with_pid(*direction, *amount, Some(pid))?;

                    Ok(ActionResult {
                        success: true,
                        message: Some(format!("scrolled {direction:?} by {amount}")),
                        path: None,
                        data: None,
                    })
                } else {
                    input::scroll(*direction, *amount)?;
                    Ok(ActionResult {
                        success: true,
                        message: Some(format!("scrolled {direction:?} by {amount}")),
                        path: None,
                        data: None,
                    })
                }
            }

            Action::MoveMouse {
                selector,
                coordinates,
            } => {
                let point = match (selector, coordinates) {
                    (_, Some(coords)) => *coords,
                    (Some(sel), None) => {
                        let node = self.find_one(sel).await?;
                        node.center().ok_or_else(|| Error::PlatformError {
                            message: "element has no position".into(),
                        })?
                    }
                    (None, None) => {
                        return Err(Error::PlatformError {
                            message: "move_mouse requires either a selector or coordinates".into(),
                        });
                    }
                };

                input::move_mouse(point)?;

                Ok(ActionResult {
                    success: true,
                    message: Some(format!("moved mouse to ({}, {})", point.x, point.y)),
                    path: None,
                    data: None,
                })
            }

            Action::Drag { from, to } => {
                input::drag(*from, *to, None)?;
                Ok(ActionResult {
                    success: true,
                    message: Some(format!(
                        "dragged from ({}, {}) to ({}, {})",
                        from.x, from.y, to.x, to.y
                    )),
                    path: None,
                    data: None,
                })
            }

            Action::Focus { selector } => {
                let root = match &selector.app {
                    Some(name) => ax::application_element(self.find_app_pid(name)?),
                    None => ax::system_wide_element(),
                };

                let element = ax::find_first_element(root, selector).ok_or_else(|| {
                    Error::ElementNotFound {
                        message: format!("{selector:?}"),
                    }
                })?;

                let focused = ax::set_focused(element, true);
                ax::release_element(element);

                if focused {
                    Ok(ActionResult {
                        success: true,
                        message: Some("focused element".into()),
                        path: None,
                        data: None,
                    })
                } else {
                    Err(Error::PlatformError {
                        message: "failed to set focus on element".into(),
                    })
                }
            }

            Action::Screenshot { path, app } => {
                let output_path = path.clone().unwrap_or_else(|| {
                    format!(
                        "/tmp/agent-computer-use-screenshot-{}.png",
                        std::process::id()
                    )
                });

                let mut args = vec!["-x".to_string()];

                if let Some(ref app_name) = app {
                    let window_ids = get_window_ids(app_name, self)?;
                    let mut last_error = None;
                    for window_id in window_ids {
                        match capture_window_to_png(window_id, &output_path) {
                            Ok(()) => {
                                return Ok(ActionResult {
                                    success: true,
                                    message: Some(format!("screenshot saved to {output_path}")),
                                    path: Some(output_path),
                                    data: None,
                                });
                            }
                            Err(error) => last_error = Some(error),
                        }
                    }

                    return Err(last_error.unwrap_or_else(|| Error::PlatformError {
                        message: format!("no capturable window found for {app_name}"),
                    }));
                }

                args.push(output_path.clone());

                let status = std::process::Command::new("screencapture")
                    .args(&args)
                    .status()
                    .map_err(|e| Error::PlatformError {
                        message: format!("screencapture failed: {e}"),
                    })?;

                if !status.success() {
                    return Err(Error::PlatformError {
                        message: "screencapture returned non-zero exit code".into(),
                    });
                }

                Ok(ActionResult {
                    success: true,
                    message: Some(format!("screenshot saved to {output_path}")),
                    path: Some(output_path),
                    data: None,
                })
            }
        }
    }

    async fn focused(&self) -> Result<AccessibilityNode> {
        ax::get_focused_element().ok_or_else(|| Error::ElementNotFound {
            message: "no element is currently focused".into(),
        })
    }

    async fn applications(&self) -> Result<Vec<AppInfo>> {
        let apps = self.running_apps();
        Ok(apps
            .into_iter()
            .map(|(pid, name)| {
                let app_el = ax::application_element(pid);
                let is_front = ax::get_bool_attribute(app_el, "AXFrontmost").unwrap_or(false);
                AppInfo {
                    name,
                    pid: pid as u32,
                    frontmost: is_front,
                    bundle_id: None,
                }
            })
            .collect())
    }

    async fn windows(&self, app: Option<&str>) -> Result<Vec<WindowInfo>> {
        let apps = match app {
            Some(name) => self.find_app_pids(name)?,
            None => self.running_apps(),
        };

        let mut windows = Vec::new();

        for (pid, app_name) in apps {
            let app_element = ax::application_element(pid);
            let node = ax::element_to_node(app_element, Some(1), 0);

            for child in &node.children {
                if child.role == agent_computer_use_core::node::Role::Window {
                    let title = child.name.clone().unwrap_or_else(|| "(untitled)".into());

                    windows.push(WindowInfo {
                        title,
                        app: app_name.clone(),
                        pid: pid as u32,
                        position: child.position,
                        size: child.size,
                        minimized: None,
                        frontmost: None,
                    });
                }
            }
        }

        Ok(windows)
    }

    async fn text(&self, app: Option<&str>) -> Result<String> {
        let tree = self.tree(app, None).await?;
        let mut text_parts = Vec::new();
        collect_text(&tree, &mut text_parts);
        Ok(text_parts.join("\n"))
    }

    async fn activate(&self, app: &str) -> Result<()> {
        self.activate_app(app)
    }

    async fn press(&self, selector: &Selector) -> Result<bool> {
        Ok(self.ax_press(selector).is_some())
    }

    async fn scroll_to_visible(&self, selector: &Selector) -> Result<bool> {
        let root = match &selector.app {
            Some(name) => ax::application_element(self.find_app_pid(name)?),
            None => ax::system_wide_element(),
        };
        Ok(ax::scroll_to_visible(root, selector))
    }

    async fn set_value(&self, selector: &Selector, value: &str) -> Result<bool> {
        let root = match &selector.app {
            Some(name) => ax::application_element(self.find_app_pid(name)?),
            None => ax::system_wide_element(),
        };
        let element = match ax::find_first_element(root, selector) {
            Some(el) => el,
            None => return Ok(false),
        };
        let result = ax::set_value(element, value);
        ax::release_element(element);
        Ok(result)
    }

    async fn open_application(&self, app: &str) -> Result<()> {
        let status = std::process::Command::new("open")
            .arg("-a")
            .arg(app)
            .status()
            .map_err(|e| Error::PlatformError {
                message: format!("failed to open '{app}': {e}"),
            })?;

        if !status.success() {
            return Err(Error::ApplicationNotFound {
                name: app.to_string(),
            });
        }
        Ok(())
    }

    async fn check_permissions(&self) -> Result<bool> {
        Ok(ax::is_trusted())
    }

    async fn move_window(&self, app: &str, x: f64, y: f64) -> Result<bool> {
        let pid = self.find_app_pid(app)?;
        Ok(ax::set_window_position(pid, x, y))
    }

    async fn resize_window(&self, app: &str, width: f64, height: f64) -> Result<bool> {
        let pid = self.find_app_pid(app)?;
        Ok(ax::set_window_size(pid, width, height))
    }

    fn platform_name(&self) -> &'static str {
        "macOS"
    }
}

fn find_best_scroll_point(pid: i32) -> Option<agent_computer_use_core::node::Point> {
    let app_el = ax::application_element(pid);
    let tree = ax::element_to_node(app_el, Some(8), 0);

    let mut best_area: f64 = 0.0;
    let mut best_center: Option<agent_computer_use_core::node::Point> = None;
    find_largest_scroll_area(&tree, &mut best_center, &mut best_area);

    if best_center.is_some() {
        return best_center;
    }

    tree.children.first().and_then(|window| {
        let pos = window.position?;
        let size = window.size?;
        Some(agent_computer_use_core::node::Point {
            x: pos.x + size.width * 0.65,
            y: pos.y + size.height * 0.5,
        })
    })
}

fn find_largest_scroll_area(
    node: &AccessibilityNode,
    best_center: &mut Option<agent_computer_use_core::node::Point>,
    best_area: &mut f64,
) {
    if node.role == agent_computer_use_core::node::Role::ScrollArea {
        if let (Some(size), Some(_)) = (node.size, node.position) {
            let area = size.width * size.height;
            if area > *best_area {
                *best_area = area;
                *best_center = node.center();
            }
        }
    }
    for child in &node.children {
        find_largest_scroll_area(child, best_center, best_area);
    }
}

fn collect_text(node: &AccessibilityNode, parts: &mut Vec<String>) {
    match node.role {
        agent_computer_use_core::node::Role::StaticText
        | agent_computer_use_core::node::Role::TextField
        | agent_computer_use_core::node::Role::TextArea
        | agent_computer_use_core::node::Role::Heading
        | agent_computer_use_core::node::Role::Paragraph
        | agent_computer_use_core::node::Role::Link => {
            if let Some(ref value) = node.value {
                parts.push(value.clone());
            } else if let Some(ref name) = node.name {
                parts.push(name.clone());
            }
        }
        _ => {
            if let Some(ref name) = node.name {
                if !name.is_empty() {
                    parts.push(name.clone());
                }
            }
        }
    }

    for child in &node.children {
        collect_text(child, parts);
    }
}

fn get_window_ids(app_name: &str, platform: &MacOSPlatform) -> Result<Vec<u32>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::CFString;

    let pids: Vec<i32> = platform
        .find_app_pids(app_name)?
        .into_iter()
        .map(|(pid, _)| pid)
        .collect();
    let info_list = unsafe { CGWindowListCopyWindowInfo(0, 0) };
    if info_list.is_null() {
        return Err(Error::PlatformError {
            message: "failed to get window list".into(),
        });
    }

    let cf_array = unsafe {
        core_foundation::array::CFArray::<CFType>::wrap_under_create_rule(
            info_list as core_foundation::array::CFArrayRef,
        )
    };

    let pid_key = CFString::new("kCGWindowOwnerPID");
    let id_key = CFString::new("kCGWindowNumber");
    let bounds_key = CFString::new("kCGWindowBounds");
    let title_key = CFString::new("kCGWindowName");
    let layer_key = CFString::new("kCGWindowLayer");

    let mut candidates: Vec<(u32, f64, String)> = Vec::new();

    for i in 0..cf_array.len() {
        let Some(entry) = cf_array.get(i) else {
            continue;
        };
        let dict_ref = entry.as_CFTypeRef() as core_foundation::dictionary::CFDictionaryRef;
        if dict_ref.is_null() {
            continue;
        }

        let mut pid_value: *const std::ffi::c_void = std::ptr::null();
        if unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                pid_key.as_concrete_TypeRef() as *const _,
                &mut pid_value,
            )
        } == 0
        {
            continue;
        }
        let mut win_pid: i64 = 0;
        unsafe {
            core_foundation::number::CFNumberGetValue(
                pid_value as core_foundation::number::CFNumberRef,
                core_foundation::number::kCFNumberSInt64Type,
                &mut win_pid as *mut i64 as *mut _,
            );
        }
        if !pids.contains(&(win_pid as i32)) {
            continue;
        }

        let mut layer_value: *const std::ffi::c_void = std::ptr::null();
        if unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                layer_key.as_concrete_TypeRef() as *const _,
                &mut layer_value,
            )
        } != 0
        {
            let mut layer: i64 = 0;
            unsafe {
                core_foundation::number::CFNumberGetValue(
                    layer_value as core_foundation::number::CFNumberRef,
                    core_foundation::number::kCFNumberSInt64Type,
                    &mut layer as *mut i64 as *mut _,
                );
            }
            if layer != 0 {
                continue;
            }
        }

        let mut bounds_value: *const std::ffi::c_void = std::ptr::null();
        if unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                bounds_key.as_concrete_TypeRef() as *const _,
                &mut bounds_value,
            )
        } == 0
        {
            continue;
        }
        let bounds_dict = bounds_value as core_foundation::dictionary::CFDictionaryRef;
        let w_key = CFString::new("Width");
        let h_key = CFString::new("Height");
        let mut w_val: *const std::ffi::c_void = std::ptr::null();
        let mut h_val: *const std::ffi::c_void = std::ptr::null();
        let has_w = unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                bounds_dict,
                w_key.as_concrete_TypeRef() as *const _,
                &mut w_val,
            )
        };
        let has_h = unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                bounds_dict,
                h_key.as_concrete_TypeRef() as *const _,
                &mut h_val,
            )
        };
        if has_w == 0 || has_h == 0 {
            continue;
        }
        let mut width: i64 = 0;
        let mut height: i64 = 0;
        unsafe {
            core_foundation::number::CFNumberGetValue(
                w_val as core_foundation::number::CFNumberRef,
                core_foundation::number::kCFNumberSInt64Type,
                &mut width as *mut i64 as *mut _,
            );
            core_foundation::number::CFNumberGetValue(
                h_val as core_foundation::number::CFNumberRef,
                core_foundation::number::kCFNumberSInt64Type,
                &mut height as *mut i64 as *mut _,
            );
        }

        if width < 100 || height < 100 {
            continue;
        }

        let mut id_value: *const std::ffi::c_void = std::ptr::null();
        if unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                id_key.as_concrete_TypeRef() as *const _,
                &mut id_value,
            )
        } == 0
        {
            continue;
        }
        let mut win_id: i64 = 0;
        unsafe {
            core_foundation::number::CFNumberGetValue(
                id_value as core_foundation::number::CFNumberRef,
                core_foundation::number::kCFNumberSInt64Type,
                &mut win_id as *mut i64 as *mut _,
            );
        }

        let mut title = String::new();
        let mut title_value: *const std::ffi::c_void = std::ptr::null();
        if unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                title_key.as_concrete_TypeRef() as *const _,
                &mut title_value,
            )
        } != 0
        {
            let title_ref = title_value as core_foundation::string::CFStringRef;
            if !title_ref.is_null() {
                title = unsafe { CFString::wrap_under_get_rule(title_ref) }.to_string();
            }
        }

        candidates.push((win_id as u32, (width * height) as f64, title));
    }

    if candidates.is_empty() {
        return Err(Error::PlatformError {
            message: format!("no window found for {app_name}"),
        });
    }

    // If an app has multiple same-named helper/table windows (CoinPoker is one
    // example), the old largest-window heuristic often selects the lobby. Prefer
    // non-dashboard windows and, among plausible app windows, the smaller one.
    // Single-window apps keep the previous behavior.
    let mut filtered: Vec<(u32, f64, String)> = candidates
        .iter()
        .filter(|(_, _, title)| !title.to_lowercase().contains("dashboard"))
        .cloned()
        .collect();
    if filtered.is_empty() {
        filtered = candidates.clone();
    }

    if filtered.len() > 1 {
        filtered.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        filtered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    }

    Ok(filtered.into_iter().map(|(id, _, _)| id).collect())
}

fn capture_window_to_png(window_id: u32, output_path: &str) -> Result<()> {
    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::string::CFString;
    use core_foundation::url::CFURL;
    use core_graphics::window::{
        create_image, kCGWindowImageBestResolution, kCGWindowImageBoundsIgnoreFraming,
        kCGWindowImageDefault, kCGWindowListOptionIncludingWindow,
    };
    use foreign_types::ForeignType;
    use std::path::Path;

    let window_bounds = unsafe { core_graphics::display::CGRectNull };
    let image = create_image(
        window_bounds,
        kCGWindowListOptionIncludingWindow,
        window_id,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
    )
    .or_else(|| {
        create_image(
            window_bounds,
            kCGWindowListOptionIncludingWindow,
            window_id,
            kCGWindowImageDefault,
        )
    })
    .ok_or_else(|| Error::PlatformError {
        message: format!("failed to capture window {window_id}"),
    })?;

    let url =
        CFURL::from_path(Path::new(output_path), false).ok_or_else(|| Error::PlatformError {
            message: format!("invalid screenshot path: {output_path}"),
        })?;
    let png_type = CFString::new("public.png");

    let destination = unsafe {
        CGImageDestinationCreateWithURL(
            url.as_concrete_TypeRef(),
            png_type.as_concrete_TypeRef(),
            1,
            std::ptr::null(),
        )
    };

    if destination.is_null() {
        return Err(Error::PlatformError {
            message: format!("failed to create PNG destination: {output_path}"),
        });
    }

    unsafe {
        CGImageDestinationAddImage(destination, image.as_ptr(), std::ptr::null());
    }

    let ok = unsafe { CGImageDestinationFinalize(destination) != 0 };
    unsafe { CFRelease(destination as *const _) };

    if !ok {
        return Err(Error::PlatformError {
            message: format!("failed to write PNG screenshot: {output_path}"),
        });
    }

    Ok(())
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(
        option: u32,
        relative_to_window: u32,
    ) -> core_foundation::base::CFTypeRef;
}

type CGImageDestinationRef = *mut std::ffi::c_void;

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageDestinationCreateWithURL(
        url: core_foundation::url::CFURLRef,
        type_identifier: core_foundation::string::CFStringRef,
        count: usize,
        options: core_foundation::dictionary::CFDictionaryRef,
    ) -> CGImageDestinationRef;
    fn CGImageDestinationAddImage(
        destination: CGImageDestinationRef,
        image: *mut core_graphics::sys::CGImage,
        properties: core_foundation::dictionary::CFDictionaryRef,
    );
    fn CGImageDestinationFinalize(destination: CGImageDestinationRef) -> u8;
}

const SYSTEM_SERVICE_NAMES: &[&str] = &[
    "Accessibility",
    "AutoFill",
    "Control Centre",
    "Control Centre Helper",
    "ControlCenter",
    "CursorUIViewService",
    "Dock",
    "Notification Centre",
    "NotificationCenter",
    "ScreenCaptureKit",
    "Spotlight",
    "ThemeWidgetControlViewService",
    "Universal Control",
    "Wallpaper",
    "Window Server",
    "WindowManager",
    "coreautha",
    "loginwindow",
];

fn running_apps_native() -> Vec<(i32, String)> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::CFString;
    use std::collections::HashSet;

    let info_list = unsafe { CGWindowListCopyWindowInfo(0, 0) };
    if info_list.is_null() {
        return Vec::new();
    }

    let cf_array = unsafe {
        core_foundation::array::CFArray::<CFType>::wrap_under_create_rule(
            info_list as core_foundation::array::CFArrayRef,
        )
    };

    let pid_key = CFString::new("kCGWindowOwnerPID");
    let name_key = CFString::new("kCGWindowOwnerName");

    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for i in 0..cf_array.len() {
        let Some(entry) = cf_array.get(i) else {
            continue;
        };
        let dict_ref = entry.as_CFTypeRef() as core_foundation::dictionary::CFDictionaryRef;
        if dict_ref.is_null() {
            continue;
        }

        let mut pid_value: *const std::ffi::c_void = std::ptr::null();
        let has_pid = unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                pid_key.as_concrete_TypeRef() as *const _,
                &mut pid_value,
            )
        };
        if has_pid == 0 || pid_value.is_null() {
            continue;
        }
        let mut pid: i64 = 0;
        let ok = unsafe {
            core_foundation::number::CFNumberGetValue(
                pid_value as core_foundation::number::CFNumberRef,
                core_foundation::number::kCFNumberSInt64Type,
                &mut pid as *mut i64 as *mut _,
            )
        };
        if !ok || pid <= 0 {
            continue;
        }

        if !seen.insert(pid as i32) {
            continue;
        }

        let mut name_value: *const std::ffi::c_void = std::ptr::null();
        let has_name = unsafe {
            core_foundation::dictionary::CFDictionaryGetValueIfPresent(
                dict_ref,
                name_key.as_concrete_TypeRef() as *const _,
                &mut name_value,
            )
        };
        if has_name == 0 || name_value.is_null() {
            continue;
        }
        let cf_name = unsafe {
            CFString::wrap_under_get_rule(name_value as core_foundation::string::CFStringRef)
        };
        let name = cf_name.to_string();
        if name.is_empty() {
            continue;
        }

        if SYSTEM_SERVICE_NAMES.iter().any(|s| *s == name) {
            continue;
        }

        result.push((pid as i32, name));
    }

    result
}
