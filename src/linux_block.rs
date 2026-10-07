// Copied verbatim from add_child_webview in owned_browser.rs.
pub fn unregister_ipc(platform: &crate::Platform) {
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::{UserContentManagerExt, WebViewExt};
            if let Some(manager) = platform.inner().user_content_manager() {
                manager.unregister_script_message_handler("ipc");
            }
        }
}
