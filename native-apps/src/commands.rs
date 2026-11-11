// Every command the window's page may ask the app (lib.rs's), named once: lib.rs's handler takes them from here, and
// build.rs, which includes this file, gives each a permission and grants them all to the app's own windows alone.
macro_rules! commands {
    ($then:ident) => {
        $then![native, address, sign_in, join, wait, fetch, read, cancel, kept, renew, removed, sign_out, notify, copy, open, save, changed, main, login, pair, quit, fit, badge, haptic, scan, menu, appearance, log, device_keys, device_sign, device_shared, keep_get, keep_put, keep_delete, access_token, waiting, joined, title_bar_double_clicked]
    };
}
