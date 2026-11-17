# Security and failure safety

What the key and the encryption are for, what they protect against and what not, what happens when the network, the app or the board fails, and why the web interface works the way it does. The packet format itself is in [protocol/SPEC.md](../protocol/SPEC.md).

## Why a key at all

The board sits in your network and turns network packets into key presses and mouse movements on your PlayStation. Without protection, **any device in the same network** could do the same: another PC, a phone, a compromised smart TV, a guest's laptop. That matters wherever you do not control everything in the network: a student hall, a shared flat, a LAN party, a network with guests.

Someone in such a network could otherwise:

- **control your PlayStation:** move through the menus, type messages, open the store, or press keys in the middle of your match;
- **read what you type:** keyboard input travels over the network, including a password you enter on the PlayStation;
- **replay what they recorded:** send your own earlier packets again;
- **install their own firmware** on the board over the network.

With the key, all of this is refused. You create the key once in the board's web interface and paste it into the app; it never travels over the network again after that.

## What it protects against

| Threat | Protection |
|---|---|
| Input from other devices in the network | Every packet is encrypted and authenticated with the key; the board silently drops anything else (and only counts it, see the "Status" tab). |
| Reading your keystrokes | The input is encrypted on its way to the board. |
| Replaying recorded packets | Every packet carries a counter that must grow; the board also starts a new session at every restart, so packets from before are worthless. |
| A forged firmware update | An update over the network needs the web password, or a signature made with the key over the exact file; the board checks it before it writes anything. |
| Changing the board's settings | The web interface needs your own password; after three wrong attempts the address they came from has to wait, up to 64 seconds. The board keeps eight addresses apart; any more share one count, so changing addresses does not get around the wait. |
| The app or the network failing in the middle of a game | If no valid packet arrives for 250 ms, the board lets go of every key and button. `Ctrl+Alt+Shift+Esc` always gives mouse and keyboard back to the PC. See [When something goes wrong](#when-something-goes-wrong). |

## What it does not protect against

- **Someone who can use your PC account.** The key is stored there (`~/.config/ps-hid-bridge/psk`, readable by your user only); whoever can run programs as you can also use the key. Protect your PC account as usual.
- **Someone who records the network traffic while you use the web interface.** The web interface uses plain HTTP in the local network (see [Why the web interface uses HTTP](#why-the-web-interface-uses-http)): your web password, and the key when it is shown after "Generate new key", are visible to whoever captures that traffic at that moment. Generate the key at a time and place where you trust the network (e.g. with the PC and the board on your own router). The input itself is always encrypted, independently of this.
- **Someone with the board in their hands.** BOOTSEL and USB allow a factory reset or new firmware; physical access is not protected.
- **Disruption.** Nothing can stop a device in the network from sending junk to the board. The board drops it cheaply, before any decryption, but a flood can still delay your input.

## When something goes wrong

Every part of the way has its own safety net, so a failure never leaves a key held on the PlayStation and never leaves you without mouse and keyboard on the PC.

| What happens | On the PlayStation | On the PC |
|---|---|---|
| **The network drops** (cable pulled, Wi-Fi gone) while you hold a key | The board lets go of every key and button after 250 ms without valid input: a dead man's switch, your character does not keep running. | The bridge stays on: mouse and keyboard stay with it, not with the desktop. After 3 s the app shows "Board not reachable" (tray icon grey) and keeps trying; once the board answers again, input goes on. To get mouse and keyboard back meanwhile: your trigger or `Ctrl+Alt+Shift+Esc`. |
| **The app hangs** | Input stops, so the board lets go of everything after 250 ms. | After one second without a sign of life, the app's watchdog gives mouse and keyboard back to the desktop and stops sending. |
| **The app is closed or crashes** | Closed: it lets go of everything on the board first. Crashed: the board lets go after 250 ms. | The system ends the app's hold on mouse and keyboard with the process: they are back with the desktop. |
| **The PC goes to sleep** or loses power | The board lets go after 250 ms. | — |
| **The board restarts** (power, update, its watchdog) | Keyboard and mouse disappear and come back, with nothing held. | The app notices the board's new session in its first answer and goes on without you doing anything. |
| **The board hangs** | Its watchdog restarts it within a few seconds (see the row above). | "Board not reachable" until it is back. |
| **A firmware update does not start properly** | The new firmware runs on trial; if it does not run by itself for a few seconds, the board restarts into the previous version. | If the app installed the update, it says that the board went back. |
| **200 MHz does not run stably** on your board | A start at 200 MHz is on trial under the watchdog; if it hangs, the next start runs at 150 MHz and keeps that. | The app and the web interface say why. |
| **A mouse or its receiver loses a button release** | The button stays pressed as long as the mouse reports it held. | "Release all keys" (status page, tray, or a trigger with that action) lets go of it on the PlayStation; the app also compares with the real key state every 250 ms, for releases lost on the PC. See [A mouse button stays pressed](troubleshooting.md#a-mouse-button-stays-pressed). |

## Why the web interface uses HTTP

The web interface is only for setting things up: password, key, network, firmware. It uses plain HTTP, on purpose:

- **HTTPS needs a certificate**, and a device in a home network has no domain name a certificate could be issued for. A self-made certificate makes every browser show a red warning page at every visit, and teaches you to click such warnings away, which is worse than the risk it would remove.
- **Settings and input are separate ways.** The password guards the settings only. Your input never goes through the web interface: it goes its own way, encrypted with the key (UDP, see [protocol/SPEC.md](../protocol/SPEC.md)).
- **The app never sends the password.** It reads the board's status, sets its clock and installs firmware with the key, through signed one-time requests that are worthless when recorded.
- **Guessing is slow:** after three wrong passwords the address they came from has to wait, growing to 64 seconds, so guessing runs at about one try a minute; addresses beyond the eight the board keeps apart share one count. Choose a password that is not easy to guess.
- **To read the traffic, someone has to be in your network already**, and capture it at the moment you log in or generate the key. If that is a concern (a shared network), set the board up where you trust the network.

## How it works

- **The key:** 32 random bytes, generated by the board's random number generator ("Security" tab) or entered there, and stored in the app.
- **Input packets:** encrypted and authenticated with ChaCha20-Poly1305. The header (version, type, counter) is not encrypted but covered by the authentication tag, so it cannot be changed either.
- **No replays:** the board accepts a packet only if its counter is higher than the last valid one, and only authentic packets move that counter. The app's counter is based on the clock and stays unique across restarts. The board creates a new session at every boot; input packets must name the current session.
- **Sender filter** (optional, "Security" tab): only one address or subnet may send input. It is an extra layer: in a local network, addresses can be faked, the key is what protects.
- **Signed requests and updates:** the app reads the board's status and installs firmware with the key instead of the web password. The board hands out a one-time challenge; the app proves it knows the key with a MAC over the challenge and the request (for an update: the file's hash). A recorded request cannot be sent again.
- **Web interface:** your own password (the factory password must be changed at the first login); failed logins counted per sender address; only the board's address or its own name are accepted as host name, so no web page can pass itself off as the board through its own DNS name (DNS rebinding).

Details, test vectors and the order of the checks: [protocol/SPEC.md](../protocol/SPEC.md).
