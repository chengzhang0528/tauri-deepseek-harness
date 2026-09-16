use rfd::{MessageButtons, MessageDialog, MessageLevel};

pub fn error(title: &str, description: impl Into<String>) {
    let _ = MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(MessageLevel::Error)
        .set_buttons(MessageButtons::Ok)
        .show();
}

pub fn info(title: &str, description: impl Into<String>) {
    notice(title, &description.into());
}

pub fn action(heading: &str, description: &str, label: &str) -> bool {
    prompt(heading, description, &[label.to_owned()], &[], None).0 == 100
}

pub fn notice(heading: &str, description: &str) {
    let _ = prompt(heading, description, &[], &[], None);
}

pub fn commands(heading: &str, description: &str, labels: &[String]) -> Option<usize> {
    let (button, _) = prompt(heading, description, labels, &[], None);
    usize::try_from(button - 100)
        .ok()
        .filter(|index| *index < labels.len())
}

pub fn select(
    heading: &str,
    description: &str,
    labels: &[String],
    selected: usize,
) -> Option<usize> {
    let (button, radio) = prompt(
        heading,
        description,
        &["保存".into()],
        labels,
        Some(selected),
    );
    if button != 100 {
        return None;
    }
    usize::try_from(radio - 200)
        .ok()
        .filter(|index| *index < labels.len())
}

#[cfg(windows)]
fn prompt(
    heading: &str,
    description: &str,
    actions: &[String],
    choices: &[String],
    selected: Option<usize>,
) -> (i32, i32) {
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDCBF_CLOSE_BUTTON,
        TDF_ALLOW_DIALOG_CANCELLATION, TaskDialogIndirect,
    };
    let close_only = actions.len() != 1;
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let title = wide("DSH Desktop");
    let heading = wide(heading);
    let description = wide(description);
    let actions = actions.iter().map(|text| wide(text)).collect::<Vec<_>>();
    let choices = choices.iter().map(|text| wide(text)).collect::<Vec<_>>();
    let buttons = actions
        .iter()
        .enumerate()
        .map(|(index, text)| TASKDIALOG_BUTTON {
            nButtonID: 100 + i32::try_from(index).expect("bounded dialog choices"),
            pszButtonText: text.as_ptr(),
        })
        .collect::<Vec<_>>();
    let radios = choices
        .iter()
        .enumerate()
        .map(|(index, text)| TASKDIALOG_BUTTON {
            nButtonID: 200 + i32::try_from(index).expect("bounded dialog choices"),
            pszButtonText: text.as_ptr(),
        })
        .collect::<Vec<_>>();
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>()).expect("config fits u32"),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: if close_only {
            TDCBF_CLOSE_BUTTON
        } else {
            TDCBF_CANCEL_BUTTON
        },
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        pszContent: description.as_ptr(),
        cButtons: u32::try_from(buttons.len()).expect("bounded dialog choices"),
        pButtons: buttons.as_ptr(),
        cRadioButtons: u32::try_from(radios.len()).expect("bounded dialog choices"),
        pRadioButtons: radios.as_ptr(),
        nDefaultRadioButton: selected.map_or(0, |index| {
            200 + i32::try_from(index).expect("bounded dialog choices")
        }),
        nDefaultButton: if close_only { 8 } else { 2 },
        cxWidth: 360,
        ..Default::default()
    };
    let mut button = 0;
    let mut radio = 0;
    // SAFETY: every string and button array stays alive for this synchronous call.
    let result = unsafe {
        TaskDialogIndirect(
            &raw const config,
            &raw mut button,
            &raw mut radio,
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        error(
            "无法显示对话框",
            "请关闭并重新打开应用后重试；当前操作未执行。",
        );
        return (0, 0);
    }
    (button, radio)
}

#[cfg(not(windows))]
fn prompt(_: &str, _: &str, _: &[String], _: &[String], _: Option<usize>) -> (i32, i32) {
    (0, 0)
}

/// Native preparation feedback. Closing this window only hides feedback; the
/// serialized operation continues and its final result remains available in diagnostics.
pub struct Progress {
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Progress {
    pub fn show(message: std::sync::Arc<std::sync::Mutex<String>>) -> Self {
        Self::with_heading(message, "正在准备运行环境")
    }

    pub fn with_heading(message: std::sync::Arc<std::sync::Mutex<String>>, heading: &str) -> Self {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(windows)]
        let worker = {
            let finished = done.clone();
            let heading = heading.to_owned();
            Some(std::thread::spawn(move || {
                show_progress(message, finished, &heading);
            }))
        };
        #[cfg(not(windows))]
        let worker = None;
        Self { done, worker }
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(windows)]
fn show_progress(
    message: std::sync::Arc<std::sync::Mutex<String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    heading: &str,
) {
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDE_CONTENT, TDF_ALLOW_DIALOG_CANCELLATION,
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
    let instruction: Vec<u16> = heading.encode_utf16().chain(Some(0)).collect();
    let text: Vec<u16> = "正在处理，请稍候。关闭此提示不会取消操作。"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let hide: Vec<u16> = "隐藏进度".encode_utf16().chain(Some(0)).collect();
    let button = TASKDIALOG_BUTTON {
        nButtonID: 1,
        pszButtonText: hide.as_ptr(),
    };
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>()).expect("config fits u32"),
        dwFlags: TDF_CALLBACK_TIMER | TDF_SHOW_MARQUEE_PROGRESS_BAR | TDF_ALLOW_DIALOG_CANCELLATION,
        cButtons: 1,
        pButtons: &raw const button,
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
