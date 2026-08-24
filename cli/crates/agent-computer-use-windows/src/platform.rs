#[cfg(not(target_os = "windows"))]
mod stub {
    use agent_computer_use_core::action::{Action, ActionResult};
    use agent_computer_use_core::node::AccessibilityNode;
    use agent_computer_use_core::platform::{AppInfo, Platform, WindowInfo};
    use agent_computer_use_core::selector::Selector;
    use agent_computer_use_core::{Error, Result};
    use async_trait::async_trait;

    pub struct WindowsPlatform;

    impl WindowsPlatform {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for WindowsPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    fn not_implemented() -> Error {
        Error::UnsupportedPlatform {
            platform: "Windows backend requires Windows OS".into(),
        }
    }

    #[async_trait]
    impl Platform for WindowsPlatform {
        async fn tree(
            &self,
            _app: Option<&str>,
            _max_depth: Option<u32>,
        ) -> Result<AccessibilityNode> {
            Err(not_implemented())
        }
        async fn find(&self, _selector: &Selector) -> Result<Vec<AccessibilityNode>> {
            Err(not_implemented())
        }
        async fn perform(&self, _action: &Action) -> Result<ActionResult> {
            Err(not_implemented())
        }
        async fn focused(&self) -> Result<AccessibilityNode> {
            Err(not_implemented())
        }
        async fn applications(&self) -> Result<Vec<AppInfo>> {
            Err(not_implemented())
        }
        async fn windows(&self, _app: Option<&str>) -> Result<Vec<WindowInfo>> {
            Err(not_implemented())
        }
        async fn text(&self, _app: Option<&str>) -> Result<String> {
            Err(not_implemented())
        }
        async fn check_permissions(&self) -> Result<bool> {
            Err(not_implemented())
        }
        fn platform_name(&self) -> &'static str {
            "Windows"
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::WindowsPlatform;

#[cfg(target_os = "windows")]
mod real {
    use agent_computer_use_core::action::{Action, ActionResult};
    use agent_computer_use_core::node::{AccessibilityNode, Point};
    use agent_computer_use_core::platform::{AppInfo, Platform, WindowInfo};
    use agent_computer_use_core::selector::Selector;
    use agent_computer_use_core::{Error, Result};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::*;
    use windows::Win32::System::Threading::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    use crate::input;
    use crate::uia::{self, UiaContext};

    pub struct WindowsPlatform {
        uia: UiaContext,
    }

    unsafe impl Send for WindowsPlatform {}
    unsafe impl Sync for WindowsPlatform {}

    static INIT: OnceLock<()> = OnceLock::new();

    impl WindowsPlatform {
        pub fn new() -> Self {
            INIT.get_or_init(|| unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                );
            });

            let uia = UiaContext::new().expect("failed to initialize UI Automation");
            Self { uia }
        }

        fn find_app_pid(&self, app_name: &str) -> Result<u32> {
            let app_lower = app_name.to_lowercase();
            let apps = self.list_processes()?;

            for (pid, name) in &apps {
                tracing::debug!(
                    "find_app_pid: checking pid={}, name={:?} against {:?}",
                    pid,
                    name,
                    app_lower
                );

                if name.to_lowercase().contains(&app_lower) {
                    tracing::debug!(
                        "find_app_pid: matched app {:?} -> pid={}, name={:?}",
                        app_name,
                        pid,
                        name
                    );
                    return Ok(*pid);
                }
            }

            tracing::debug!("find_app_pid: no process matched app {:?}", app_name);

            Err(Error::ApplicationNotFound {
                name: app_name.to_string(),
            })
        }
        fn find_window_for_app(&self, app_name: &str) -> Result<HWND> {
            // First try the application's own process.
            if let Ok(pid) = self.find_app_pid(app_name) {
                if let Some(hwnd) = find_window_for_pid(pid) {
                    tracing::debug!(
                        "find_window_for_app: found window for {:?} via pid={}",
                        app_name,
                        pid
                    );
                    return Ok(hwnd);
                }

                tracing::debug!(
                    "find_window_for_app: process pid={} has no visible top-level window",
                    pid
                );
            }

            // UWP / Windows Store applications can have their visible window
            // owned by ApplicationFrameHost.exe rather than the application process.
            if let Some(hwnd) = find_window_by_title(app_name) {
                tracing::debug!(
                    "find_window_for_app: found window for {:?} by title",
                    app_name
                );
                return Ok(hwnd);
            }

            Err(Error::PlatformError {
                message: format!("no window found for '{app_name}'"),
            })
        }

        fn list_processes(&self) -> Result<Vec<(u32, String)>> {
            use windows::Win32::System::ProcessStatus::*;

            unsafe {
                let mut pids = vec![0u32; 4096];
                let mut bytes_returned = 0u32;
                EnumProcesses(
                    pids.as_mut_ptr(),
                    (pids.len() * std::mem::size_of::<u32>()) as u32,
                    &mut bytes_returned,
                )
                .map_err(|e| Error::PlatformError {
                    message: format!("EnumProcesses failed: {e}"),
                })?;

                let count = bytes_returned as usize / std::mem::size_of::<u32>();
                let mut result = Vec::new();

                for &pid in &pids[..count] {
                    if pid == 0 {
                        continue;
                    }
                    if let Ok(handle) =
                        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
                    {
                        let mut name_buf = [0u16; 260];
                        let len = GetModuleBaseNameW(handle, None, &mut name_buf);
                        if len > 0 {
                            let name = String::from_utf16_lossy(&name_buf[..len as usize]);
                            let name = name.trim_end_matches(".exe").to_string();
                            result.push((pid, name));
                        }
                        let _ = CloseHandle(handle);
                    }
                }

                Ok(result)
            }
        }

        fn get_app_root(
            &self,
            app: Option<&str>,
        ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationElement> {
            match app {
                Some(name) => self.uia.find_app_element(name),
                None => self.uia.root(),
            }
        }
    }

    impl Default for WindowsPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl Platform for WindowsPlatform {
        async fn tree(
            &self,
            app: Option<&str>,
            max_depth: Option<u32>,
        ) -> Result<AccessibilityNode> {
            let root = self.get_app_root(app)?;
            Ok(uia::element_to_node(&root, max_depth, 0))
        }

        async fn find(&self, selector: &Selector) -> Result<Vec<AccessibilityNode>> {
            let root = self.get_app_root(selector.app.as_deref())?;
            Ok(uia::find_all(&root, selector))
        }

        async fn perform(&self, action: &Action) -> Result<ActionResult> {
            match action {
                Action::Click {
                    selector,
                    coordinates,
                    button,
                    count,
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
                                message: "click requires selector or coordinates".into(),
                            })
                        }
                    };

                    input::click(point, *button, *count)?;
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
                        let root = self.get_app_root(sel.app.as_deref())?;
                        if let Some(element) = uia::find_first(&root, sel) {
                            uia::set_element_value(&element, text);
                        }
                    } else {
                        input::type_text(text)?;
                    }

                    if *submit {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        input::key_press("return")?;
                    }

                    Ok(ActionResult {
                        success: true,
                        message: Some(format!("typed {} characters", text.len())),
                        path: None,
                        data: None,
                    })
                }

                Action::KeyPress { key, app } => {
                    if let Some(app_name) = app {
                        self.activate(app_name).await?;
                    }
                    input::key_press(key)?;
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
                    selector: _,
                    app,
                } => {
                    if let Some(app_name) = app {
                        self.activate(app_name).await?;
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        unsafe {
                            let hwnd = self.find_window_for_app(app_name)?;
                            let mut rect = RECT::default();
                            if GetWindowRect(hwnd, &mut rect).is_ok() {
                                let cx = (rect.left + rect.right) as f64 / 2.0;
                                let cy = (rect.top + rect.bottom) as f64 / 2.0;
                                input::move_mouse(Point { x: cx, y: cy })?;
                                std::thread::sleep(std::time::Duration::from_millis(20));
                            }
                        }
                    }
                    input::scroll(*direction, *amount)?;
                    Ok(ActionResult {
                        success: true,
                        message: Some(format!("scrolled {direction:?} by {amount}")),
                        path: None,
                        data: None,
                    })
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
                                message: "move requires selector or coordinates".into(),
                            })
                        }
                    };

                    input::move_mouse(point)?;
                    Ok(ActionResult {
                        success: true,
                        message: Some(format!("moved to ({}, {})", point.x, point.y)),
                        path: None,
                        data: None,
                    })
                }

                Action::Screenshot { path, app: _ } => {
                    let save_path = path.clone().unwrap_or_else(|| {
                        format!(
                            "{}\\agent-computer-use-screenshot.png",
                            std::env::temp_dir().display()
                        )
                    });

                    let ps_script = format!(
                        r#"Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height); $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size); $bmp.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()"#,
                        save_path.replace('\'', "''")
                    );

                    let result = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", &ps_script])
                        .output();

                    match result {
                        Ok(output) if output.status.success() => Ok(ActionResult {
                            success: true,
                            message: Some(format!("screenshot saved to {save_path}")),
                            path: Some(save_path),
                            data: None,
                        }),
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            Err(Error::PlatformError {
                                message: format!("screenshot failed: {stderr}"),
                            })
                        }
                        Err(e) => Err(Error::PlatformError {
                            message: format!("failed to run powershell for screenshot: {e}"),
                        }),
                    }
                }

                Action::Focus { selector } => {
                    let root = self.get_app_root(selector.app.as_deref())?;
                    if let Some(element) = uia::find_first(&root, selector) {
                        unsafe {
                            let _ = element.SetFocus();
                        }
                    }
                    Ok(ActionResult {
                        success: true,
                        message: Some("focused".into()),
                        path: None,
                        data: None,
                    })
                }

                Action::Drag { from, to } => {
                    unsafe {
                        let pt = POINT {
                            x: from.x as i32,
                            y: from.y as i32,
                        };
                        let hwnd = WindowFromPoint(pt);
                        if hwnd != HWND::default() {
                            let root = GetAncestor(hwnd, GA_ROOT);
                            let target = if root != HWND::default() { root } else { hwnd };
                            force_foreground(target);
                            std::thread::sleep(std::time::Duration::from_millis(80));
                        }
                    }
                    input::drag(*from, *to)?;
                    Ok(ActionResult {
                        success: true,
                        message: Some(format!(
                            "dragged from ({},{}) to ({},{})",
                            from.x, from.y, to.x, to.y
                        )),
                        path: None,
                        data: None,
                    })
                }
            }
        }

        async fn focused(&self) -> Result<AccessibilityNode> {
            let element = self.uia.focused_element()?;
            Ok(uia::element_to_node(&element, Some(0), 0))
        }

        async fn applications(&self) -> Result<Vec<AppInfo>> {
            let processes = self.list_processes()?;
            let mut apps = Vec::new();
            let mut seen = std::collections::HashSet::new();

            unsafe {
                let foreground = GetForegroundWindow();
                let mut fg_pid = 0u32;
                if foreground != HWND::default() {
                    GetWindowThreadProcessId(foreground, Some(&mut fg_pid));
                }

                for (pid, name) in &processes {
                    if seen.contains(name) {
                        continue;
                    }
                    if has_visible_window(*pid) {
                        seen.insert(name.clone());
                        apps.push(AppInfo {
                            name: name.clone(),
                            pid: *pid,
                            frontmost: *pid == fg_pid,
                            bundle_id: None,
                        });
                    }
                }
            }

            Ok(apps)
        }

        async fn windows(&self, app: Option<&str>) -> Result<Vec<WindowInfo>> {
            let mut windows = Vec::new();

            let pid_to_name: HashMap<u32, String> = self.list_processes()?.into_iter().collect();

            unsafe {
                let foreground = GetForegroundWindow();

                EnumWindows(
                    Some(enum_windows_callback),
                    LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
                )
                .ok();

                if let Some(app_name) = app {
                    let app_normalized = app_name
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .flat_map(|c| c.to_lowercase())
                        .collect::<String>();

                    windows.retain(|w: &WindowInfo| {
                        let title_normalized = w
                            .title
                            .chars()
                            .filter(|c| !c.is_whitespace())
                            .flat_map(|c| c.to_lowercase())
                            .collect::<String>();

                        let pid_match = pid_to_name
                            .get(&w.pid)
                            .map(|name| name.to_lowercase().contains(&app_name.to_lowercase()))
                            .unwrap_or(false);

                        pid_match || title_normalized.contains(&app_normalized)
                    });
                }

                for w in &mut windows {
                    if let Some(name) = pid_to_name.get(&w.pid) {
                        w.app = name.clone();
                    }

                    let hwnd = find_window_by_title(&w.title);

                    if let Some(h) = hwnd {
                        w.frontmost = Some(h == foreground);
                    }
                }
            }

            Ok(windows)
        }

        async fn text(&self, app: Option<&str>) -> Result<String> {
            let root = self.get_app_root(app)?;
            Ok(uia::collect_text(&root, Some(20), 0))
        }

        async fn check_permissions(&self) -> Result<bool> {
            Ok(true)
        }

        async fn activate(&self, app: &str) -> Result<()> {
            let hwnd = self.find_window_for_app(app)?;
            force_foreground(hwnd);
            Ok(())
        }

        async fn press(&self, selector: &Selector) -> Result<bool> {
            let root = self.get_app_root(selector.app.as_deref())?;
            if let Some(element) = uia::find_first(&root, selector) {
                Ok(uia::invoke_element(&element))
            } else {
                Ok(false)
            }
        }

        async fn set_value(&self, selector: &Selector, value: &str) -> Result<bool> {
            let root = self.get_app_root(selector.app.as_deref())?;
            if let Some(element) = uia::find_first(&root, selector) {
                Ok(uia::set_element_value(&element, value))
            } else {
                Ok(false)
            }
        }

        async fn scroll_to_visible(&self, selector: &Selector) -> Result<bool> {
            let root = self.get_app_root(selector.app.as_deref())?;
            if let Some(element) = uia::find_first(&root, selector) {
                Ok(uia::scroll_element_into_view(&element))
            } else {
                Ok(false)
            }
        }

        async fn open_application(&self, app: &str) -> Result<()> {
            use std::process::Command;
            let result = Command::new("cmd").args(["/C", "start", "", app]).spawn();

            match result {
                Ok(_) => Ok(()),
                Err(e) => Err(Error::PlatformError {
                    message: format!("failed to open '{app}': {e}"),
                }),
            }
        }

        async fn move_window(&self, app: &str, x: f64, y: f64) -> Result<bool> {
            unsafe {
                let hwnd = self.find_window_for_app(app)?;

                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);

                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;

                SetWindowPos(
                    hwnd,
                    None,
                    x as i32,
                    y as i32,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .map_err(|e| Error::PlatformError {
                    message: format!("SetWindowPos failed: {e}"),
                })?;

                Ok(true)
            }
        }

        async fn resize_window(&self, app: &str, width: f64, height: f64) -> Result<bool> {
            unsafe {
                let hwnd = self.find_window_for_app(app)?;

                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);

                SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    width as i32,
                    height as i32,
                    SWP_NOZORDER | SWP_NOMOVE | SWP_NOACTIVATE,
                )
                .map_err(|e| Error::PlatformError {
                    message: format!("SetWindowPos failed: {e}"),
                })?;

                Ok(true)
            }
        }

        fn platform_name(&self) -> &'static str {
            "Windows"
        }
    }

    fn has_visible_window(pid: u32) -> bool {
        find_window_for_pid(pid).is_some()
    }

    fn force_foreground(hwnd: HWND) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }

            let foreground = GetForegroundWindow();
            if foreground == hwnd {
                return;
            }

            let current_thread = GetCurrentThreadId();
            let foreground_thread = if foreground != HWND::default() {
                GetWindowThreadProcessId(foreground, None)
            } else {
                0
            };

            let attached = foreground_thread != 0
                && foreground_thread != current_thread
                && AttachThreadInput(current_thread, foreground_thread, true).as_bool();

            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = ShowWindow(hwnd, SW_SHOW);

            if attached {
                let _ = AttachThreadInput(current_thread, foreground_thread, false);
            }
        }
    }

    struct FindPidContext {
        target_pid: u32,
        found: Option<HWND>,
    }
    fn find_window_for_pid(pid: u32) -> Option<HWND> {
        unsafe {
            let mut ctx = FindPidContext {
                target_pid: pid,
                found: None,
            };

            let _ = EnumWindows(
                Some(find_pid_callback),
                LPARAM(&mut ctx as *mut FindPidContext as isize),
            );

            ctx.found
        }
    }

    unsafe extern "system" fn find_pid_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut FindPidContext);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));

        if window_pid == ctx.target_pid {
            let visible = IsWindowVisible(hwnd).as_bool();
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let toolwindow = style & WS_EX_TOOLWINDOW.0 != 0;

            tracing::debug!(
                "find_window_for_pid: hwnd={:?}, pid={}, visible={}, toolwindow={}",
                hwnd,
                window_pid,
                visible,
                toolwindow
            );

            if visible && !toolwindow {
                ctx.found = Some(hwnd);
                return FALSE;
            }
        }
        TRUE
    }

    unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }

        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_EX_TOOLWINDOW.0 != 0 {
            return TRUE;
        }

        let mut title_buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        if len == 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&title_buf[..len as usize]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);

        let windows = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        windows.push(WindowInfo {
            title,
            app: String::new(),
            pid,
            position: Some(Point {
                x: rect.left as f64,
                y: rect.top as f64,
            }),
            size: Some(agent_computer_use_core::node::Size {
                width: (rect.right - rect.left) as f64,
                height: (rect.bottom - rect.top) as f64,
            }),
            minimized: Some(IsIconic(hwnd).as_bool()),
            frontmost: None,
        });

        TRUE
    }

    fn find_window_by_title(title: &str) -> Option<HWND> {
        unsafe {
            let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            match FindWindowW(None, windows::core::PCWSTR(wide.as_ptr())) {
                Ok(hwnd) if hwnd != HWND::default() => Some(hwnd),
                _ => None,
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use real::WindowsPlatform;
