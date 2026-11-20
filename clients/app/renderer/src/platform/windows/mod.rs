//! Windows' windows, in the launcher's process (`window`), each drawn by wgpu on Direct3D 12 into a DirectComposition
//! swap chain, and the windows' thread's message loop, which also runs the jobs the rest of the program
//! sends it.

mod keyboard;
mod window;

use std::sync::mpsc;

pub use window::{Display, Handle, NativeWindow};
use windows::Win32::Foundation::{HANDLE, LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PostThreadMessageW, QS_ALLINPUT, WM_NULL};
use windows::core::w;

use super::{Job, Native, Windows};

/// Where the windows' thread takes jobs: each sent wakes the thread's message loop with a message.
#[derive(Clone)]
pub struct Sender {
    jobs: mpsc::Sender<Job>,
    thread: u32,
}

impl Sender {
    pub fn send(&self, job: Job) -> Result<(), mpsc::SendError<Job>> {
        self.jobs.send(job)?;
        // Safety: a message with no window, to a thread that's running, wakes its wait.
        let _ = unsafe { PostThreadMessageW(self.thread, WM_NULL, WPARAM(0), LPARAM(0)) };
        Ok(())
    }
}

/// The windows' thread: its windows, and a loop that waits for a message, a job or the next timer of a window's
/// screen, then runs whatever's due and draws what changed.
pub(super) fn start() -> anyhow::Result<Native> {
    let (jobs_to, jobs) = mpsc::channel::<Job>();
    let (ready_to, ready) = mpsc::channel();
    std::thread::Builder::new().name("native".into()).spawn(move || {
        // It takes the person's input and draws what they see, so it runs ahead of work nobody's waiting on, as an
        // app's main thread does: as far as the system lets it.
        let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max);
        let mut windows = match Display::connect().and_then(Windows::new) {
            Ok(windows) => windows,
            Err(error) => return drop(ready_to.send(Err(error))),
        };
        let _ = ready_to.send(Ok(unsafe { GetCurrentThreadId() }));
        loop {
            let timeout = windows.due().map_or(u32::MAX, |due| due.as_millis().min(u128::from(u32::MAX - 1)) as u32);
            // Safety: no handles, only the thread's own messages.
            unsafe { MsgWaitForMultipleObjectsEx(Some(&[] as &[HANDLE]), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
            while let Ok(job) = jobs.try_recv() {
                job(&mut windows);
            }
            windows.turn();
        }
    })?;
    let thread = ready.recv()??;
    Ok(Native(Sender { jobs: jobs_to, thread }))
}

/// Windows' clipboard, as arboard serves it, one for every window on this thread. It has no primary selection.
struct Clipboard(std::rc::Rc<std::cell::RefCell<arboard::Clipboard>>);

impl crate::editing::Clipboard for Clipboard {
    fn read(&mut self, primary: bool) -> Option<String> {
        if primary {
            return None;
        }
        self.0.borrow_mut().get_text().ok()
    }

    fn write(&mut self, primary: bool, text: &str) {
        if primary {
            return;
        }
        if let Err(error) = self.0.borrow_mut().set_text(text.to_owned()) {
            log::warn!("nothing went on the clipboard: {error}");
        }
    }
}

/// Windows' clipboard, for a window's screen, where it has one.
pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    thread_local! {
        static SHARED: Option<std::rc::Rc<std::cell::RefCell<arboard::Clipboard>>> = arboard::Clipboard::new()
            .map_err(|error| log::warn!("there's no clipboard: {error}"))
            .ok()
            .map(|clipboard| std::rc::Rc::new(std::cell::RefCell::new(clipboard)));
    }
    SHARED.with(|shared| shared.clone()).map(|shared| Box::new(Clipboard(shared)) as Box<dyn crate::editing::Clipboard>)
}

/// Whether the person asked for less motion: Windows' Accessibility › Visual effects › Animation effects off
/// (`SPI_GETCLIENTAREAANIMATION`).
pub fn reduced_motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
    let mut animation = windows::core::BOOL(1);
    // Safety: a BOOL read into one.
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some((&raw mut animation).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    read.is_ok() && !animation.as_bool()
}

/// Light or dark, as Windows' Personalization › Colours says for apps (`AppsUseLightTheme`).
pub fn appearance() -> crate::Appearance {
    let mut light = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // Safety: a DWORD read into one.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as _),
            Some(&mut size),
        )
    };
    if read.is_ok() && light == 0 { crate::Appearance::Dark } else { crate::Appearance::Light }
}

/// Shows the system's own picker of files to open, as rfd shows it, of `accept`'s kinds where they're extensions
/// (`.png`), and tells `chosen` the files chosen, none where it was cancelled.
pub fn open_files(accept: &[String], multiple: bool, chosen: Box<dyn FnOnce(Vec<std::path::PathBuf>) + Send>) {
    let extensions: Vec<String> = accept.iter().filter_map(|kind| kind.strip_prefix('.')).map(str::to_owned).collect();
    let mut dialog = rfd::AsyncFileDialog::new();
    if !extensions.is_empty() {
        dialog = dialog.add_filter("", &extensions);
    }
    std::thread::spawn(move || {
        let files = futures_lite::future::block_on(async move {
            if multiple {
                dialog.pick_files().await.unwrap_or_default()
            } else {
                dialog.pick_file().await.into_iter().collect()
            }
        });
        chosen(files.into_iter().map(|file| file.path().to_owned()).collect());
    });
}
