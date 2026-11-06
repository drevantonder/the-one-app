// Every command the window's page may ask the app (lib.rs's), named once: lib.rs's handler takes them from here, and
// build.rs, which includes this file, gives each a permission and grants them all to the app's own windows alone.
macro_rules! commands {
    ($then:ident) => {
        $then![native, address, sign_in, join, wait, fetch, read, cancel, kept, removed, sign_out, notify, copy, open, save, changed, main, login, computer_use, pair, quit, fit, badge, haptic, scan, menu, appearance, menubar, log]
    };
}
