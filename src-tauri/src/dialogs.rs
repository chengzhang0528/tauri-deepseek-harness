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
