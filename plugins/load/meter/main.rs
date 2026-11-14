//! Measures the machine it runs on: for each line it reads, it writes one line of JSON with the machine's
//! processors, its load over the last 1, 5 and 15 minutes, and when this program started. It stops when its
//! input closes, which is how it's told that the service that started it has gone.

use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

extern "C" {
    /// The C library's load averages, on Linux and Mac.
    fn getloadavg(averages: *mut f64, count: i32) -> i32;
}

fn main() {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let processors = std::thread::available_parallelism().map_or(1, |processors| processors.get());
    let mut out = io::stdout();
    for asked in io::stdin().lock().lines() {
        if asked.is_err() {
            break;
        }
        let mut load = [0f64; 3];
        // SAFETY: `load` has room for the three averages asked for.
        unsafe { getloadavg(load.as_mut_ptr(), 3) };
        let [one, five, fifteen] = load;
        let measured =
            format!("{{\"processors\":{processors},\"load\":[{one:.2},{five:.2},{fifteen:.2}],\"since\":{since}}}\n");
        if out.write_all(measured.as_bytes()).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}
