/**
 * Shutting down the headless Chromium that the browser checks start. It asks
 * the browser to quit over the DevTools protocol and leaves the launching and
 * the profile folder to the caller.
 */

/** How long to wait for Chromium to acknowledge Browser.close, in milliseconds. */
const CLOSE_TIMEOUT_MS = 3000;

/**
 * Ask Chromium to quit over the DevTools protocol. On Windows the process
 * spawn() started is a launcher that has already exited, so killing it would
 * leave the browser running with its profile locked.
 * @param {number} debugPort the --remote-debugging-port the browser listens on
 */
export async function closeBrowser(debugPort) {
  try {
    const res = await fetch(`http://127.0.0.1:${debugPort}/json/version`);
    const { webSocketDebuggerUrl } = await res.json();
    await new Promise((done) => {
      const socket = new WebSocket(webSocketDebuggerUrl);
      const timer = setTimeout(done, CLOSE_TIMEOUT_MS);
      const finish = () => {
        clearTimeout(timer);
        done();
      };
      socket.addEventListener('open', () => socket.send(JSON.stringify({ id: 1, method: 'Browser.close' })));
      socket.addEventListener('close', finish);
      socket.addEventListener('error', finish);
    });
  } catch { /* the browser has already gone */ }
}
