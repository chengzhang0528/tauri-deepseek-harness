use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

pub fn error(title: &str, description: impl Into<String>) {
    let _ = MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(MessageLevel::Error)
        .set_buttons(MessageButtons::Ok)
        .show();
}

pub fn info(title: &str, description: impl Into<String>) {
    let _ = MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(MessageLevel::Info)
        .set_buttons(MessageButtons::Ok)
        .show();
}

pub fn confirm(title: &str, description: impl Into<String>) -> bool {
    matches!(
        MessageDialog::new()
            .set_title(title)
            .set_description(description)
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
}

/// Native preparation feedback. Closing this window only hides feedback; the
/// serialized operation continues and its final result remains in the menu.
pub struct Progress(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Progress {
    pub fn show(message: std::sync::Arc<std::sync::Mutex<String>>) -> Self {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(windows)]
        {
            let finished = done.clone();
            std::thread::spawn(move || show_progress(message, finished));
        }
        Self(done)
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
#[cfg(windows)]
fn show_progress(
    message: std::sync::Arc<std::sync::Mutex<String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOGCONFIG, TDCBF_OK_BUTTON, TDE_CONTENT, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_CALLBACK_TIMER, TDF_SHOW_MARQUEE_PROGRESS_BAR, TDM_CLICK_BUTTON, TDM_SET_ELEMENT_TEXT,
        TDM_SET_PROGRESS_BAR_MARQUEE, TDN_CREATED, TDN_TIMER, TaskDialogIndirect,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;
    struct State {
        message: std::sync::Arc<std::sync::Mutex<String>>,
        done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    unsafe extern "system" fn callback(
        hwnd: windows_sys::Win32::Foundation::HWND,
        event: u32,
        _: usize,
        _: isize,
        data: isize,
    ) -> i32 {
        // SAFETY: the stack state remains alive for the synchronous TaskDialog call.
        let state = unsafe { &*(data as *const State) };
        if event == TDN_CREATED as u32 {
            unsafe {
                SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_MARQUEE as u32, 1, 0);
            }
        }
        if event == TDN_TIMER as u32 {
            if state.done.load(std::sync::atomic::Ordering::SeqCst) {
                unsafe {
                    SendMessageW(hwnd, TDM_CLICK_BUTTON as u32, 1, 0);
                }
            } else if let Ok(message) = state.message.lock() {
                let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
                unsafe {
                    SendMessageW(
                        hwnd,
                        TDM_SET_ELEMENT_TEXT as u32,
                        TDE_CONTENT as usize,
                        text.as_ptr() as isize,
                    );
                }
            }
        }
        0
    }
    let state = State { message, done };
    let title: Vec<u16> = "DSH Desktop".encode_utf16().chain(Some(0)).collect();
    let instruction: Vec<u16> = "正在准备运行环境".encode_utf16().chain(Some(0)).collect();
    let text: Vec<u16> = "正在处理，请稍候。关闭此提示后可从托盘查看进度。"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>()).expect("config fits u32"),
        dwFlags: TDF_CALLBACK_TIMER | TDF_SHOW_MARQUEE_PROGRESS_BAR | TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: instruction.as_ptr(),
        pszContent: text.as_ptr(),
        pfCallback: Some(callback),
        lpCallbackData: (&raw const state) as isize,
        ..Default::default()
    };
    // SAFETY: config strings and callback state outlive the synchronous dialog.
    unsafe {
        TaskDialogIndirect(
            &raw const config,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
}
