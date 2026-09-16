//! Read-only native-window gate. Never foregrounds, restores, resizes or reads titles.
use super::*;

fn require_one_window(states: &[bool]) -> AppResult<()> {
    if states != [false] {
        return Err(AppError::new("WORKBUDDY_NOT_PAINTING", "请保持一个 WorkBuddy 主窗口打开且不要最小化，然后刷新画面；不会自动显示窗口或切换会话。"));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn require_available(path: &Path) -> AppResult<()> {
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            callback: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            data: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(window: isize) -> i32;
        fn IsIconic(window: isize) -> i32;
        fn GetClassNameW(window: isize, name: *mut u16, count: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn QueryFullProcessImageNameW(
            process: isize,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    struct Probe {
        expected: String,
        states: Vec<bool>,
    }
    fn normalized(s: &str) -> String {
        s.trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase()
    }
    unsafe extern "system" fn visit(window: isize, data: isize) -> i32 {
        if IsWindowVisible(window) == 0 {
            return 1;
        }
        let mut class = [0u16; 128];
        let len = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
        if len <= 0 || String::from_utf16_lossy(&class[..len as usize]) != "Chrome_WidgetWin_1" {
            return 1;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        let process = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if process == 0 {
            return 1;
        }
        let mut name = [0u16; 32768];
        let mut size = name.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size);
        CloseHandle(process);
        if ok != 0 {
            let probe = &mut *(data as *mut Probe);
            if normalized(&String::from_utf16_lossy(&name[..size as usize])) == probe.expected {
                probe.states.push(IsIconic(window) != 0);
            }
        }
        1
    }
    let mut probe = Probe {
        expected: normalized(&path.to_string_lossy()),
        states: Vec::new(),
    };
    if unsafe { EnumWindows(Some(visit), &mut probe as *mut Probe as isize) } == 0 {
        return require_one_window(&[]);
    }
    require_one_window(&probe.states)
}
#[cfg(not(windows))]
pub(crate) fn require_available(_: &Path) -> AppResult<()> {
    require_one_window(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_minimized_or_ambiguous_windows_cannot_publish_a_capture() {
        assert!(require_one_window(&[false]).is_ok());
        for states in [vec![], vec![true], vec![true, false], vec![false, false]] {
            assert_eq!(
                require_one_window(&states).unwrap_err().code,
                "WORKBUDDY_NOT_PAINTING"
            );
        }
    }
}
