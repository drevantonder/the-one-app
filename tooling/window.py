"""WebDriver on the app's own window on Linux, the one a `one:` link opens: the Home Screen's, or an app's, in a web view.

A strict app's window is drawn natively, and has no web view for WebDriver to reach: `opened` has the launcher open
every app in a web view (`ONE_WEB_VIEW`, which the launcher honours), so a picture here is the DOM's, to compare with
the native window's, which the inspector takes (the qa skill's Screens).

    import sys; sys.path.insert(0, "tooling")
    from window import opened, settled, frame
    with opened("one:settings#vault/accounts") as driver:
        driver.save_screenshot("vault.png")

Run it as the qa skill's Screens says, under `dbus-run-session -- xvfb-run`, with `ONE_HOME` set. It starts `openbox`
(apt's) in that desktop, as a person's has a window manager: without one, X gives a window focus and takes it back
as the launcher resizes, so the launcher puts itself away, and a hidden window draws no frame.

Each window is a process of its own, and WebKitWebDriver reaches one by the inspector port its environment names
(`WEBKIT_INSPECTOR_SERVER`), which every window the app starts inherits and only the first can take. So the app is
started with a port it can't take, as something else holds it until the Home Screen has drawn; then the link opens the
app's window, which takes it, and WebKitWebDriver is attached to that window (`--target`). For the Home Screen the
app takes the port itself.
"""

import contextlib
import os
import socket
import subprocess
import threading

from selenium import webdriver
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.remote.client_config import ClientConfig
from selenium.webdriver.support.ui import WebDriverWait

PROGRAM = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "one")

# The page has drawn twice, so what a resize changes (a media query's `change`) has happened, and what moves has ended:
# a cancelled animation's `finished` rejects.
STILL = """new Promise(drawn => requestAnimationFrame(() => requestAnimationFrame(drawn)))
  .then(() => Promise.all(document.getAnimations().filter(a => a.effect?.getComputedTiming().endTime !== Infinity)
  .map(a => a.finished.catch(() => {})))).then(arguments[0])"""


@contextlib.contextmanager
def opened(link, size=(390, 844), log=os.devnull, timeout=60):
    """A WebDriver session on the window `link` opens (`one:<app>`, `one:<app>/<page>`, `one:home-screen#library`),
    `size` in CSS pixels, drawn (the page has rendered what it first asked for, as the app says in `<app> opened in …`, not
    its skeleton) and still (`settled`). What the app says, its pages' console too, goes to `log`."""
    app = link.removeprefix("one:").split("/")[0].split("#")[0]
    held = socket.create_server(("127.0.0.1", 0))
    port = held.getsockname()[1]
    if app == "home-screen":
        held.close()
    env = {**os.environ, "TAURI_WEBVIEW_AUTOMATION": "true", "WEBKIT_INSPECTOR_SERVER": f"127.0.0.1:{port}", "ONE_WEB_VIEW": "1"}
    args = [PROGRAM, link] if app == "home-screen" else [PROGRAM]
    manager = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    started = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    drawn = {name: threading.Event() for name in ("home-screen", app)}
    seen = []

    def follow():
        with open(log, "a") as out:
            for line in started.stderr:
                out.write(line)
                out.flush()
                seen.append(line.rstrip())
                for name, event in drawn.items():
                    if line.startswith(f"{name} opened in "):
                        event.set()
        # The app ended: whoever waits on a window stops waiting, and says what the app said.
        for event in drawn.values():
            event.set()

    def wait(name, what):
        if not drawn[name].wait(timeout) or started.poll() is not None:
            raise TimeoutError(f"{what} {'ended' if started.poll() is not None else f'did not draw in {timeout} s'}; it said:\n"
                               + "\n".join(seen[-10:]))

    threading.Thread(target=follow, daemon=True).start()
    service = None
    try:
        wait("home-screen", "the Home Screen")
        if app != "home-screen":
            held.close()
            subprocess.run([PROGRAM, link], env=env, check=True, timeout=timeout)
            wait(app, app)
        service = webdriver.WebKitGTKService(service_args=[f"--target=127.0.0.1:{port}"])
        service.start()
        driver = webdriver.Remote(service.service_url, options=ArgOptions(), client_config=ClientConfig(
            remote_server_addr=service.service_url, timeout=timeout))
        try:
            driver.set_window_size(*size)
            settled(driver, timeout)
            yield driver
        finally:
            driver.quit()
    finally:
        held.close()
        if service:
            service.stop()
        started.terminate()
        started.wait()
        manager.terminate()
        manager.wait()


def settled(driver, timeout=30):
    """Waits until the window shows its app and what moves in the window and in the app's frame has ended. It leaves
    the driver in the window, where its picture is taken."""
    driver.switch_to.default_content()
    WebDriverWait(driver, timeout).until(
        lambda d: d.find_elements("css selector", "body:not([aria-busy]) iframe:not([inert]):not([aria-busy])"))
    driver.execute_async_script(STILL)
    frame(driver)
    driver.execute_async_script(STILL)
    driver.switch_to.default_content()


def frame(driver):
    """Switches to the app's frame, where its elements are."""
    driver.switch_to.default_content()
    driver.switch_to.frame(driver.find_element("css selector", "iframe:not([inert])"))
