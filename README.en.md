# AnyKey

**简体中文** | English

> A high-performance key remapping tool for Windows — combos, tap-dance, layers, leader sequences, deferred decisions, and kernel-level input interception.

AnyKey is a **keyboard & mouse remapping + runtime control + system-level input interception** tool. It hands you full control over how your keyboard and mouse respond. The core engine is written in **Rust** and, through a **custom Windows kernel filter driver (UpperFilter)**, intercepts and re-injects keyboard/mouse input at the system level. All key processing happens locally: **no telemetry, no network reporting**.

### Why a custom kernel driver? — fixing Interception's device limit

The popular Interception driver has a hard flaw: **10 input devices max for the whole system**. A second keyboard, a macro pad, a few virtual devices — and device #11 just stops working. AnyKey's own UpperFilter driver kills that limit — hook up as many keyboards and mice as you want, and hot-plug plus sleep/wake come fixed for free.

### About Test Mode — the honest truth

Loading a driver the normal way requires Microsoft signing (an EV code-signing certificate plus WHQL certification). **It's just too expensive, and I'm not paying for it right now** — for a solo developer that money buys nothing users can feel. So this version runs in **Windows Test Signing mode**, with a permanent "Test Mode" watermark in the corner of your desktop. If the project ever earns its own signing, the watermark goes away (see the roadmap). If that bothers you, read the [full installation notes](#2-installation) before deciding whether to install.

Compared with other remapping tools, AnyKey has two fundamental advantages rooted in its low-level architecture:

1. **Defer — any key can be a "hold-to-layer" key.** Most tools can only bind "hold to switch layer" to a handful of preset modifier keys; binding it to a normal letter key almost always causes accidental triggers. AnyKey's Defer system lets letters, symbols, and mouse side keys all act as layer activation keys, with an independently adjustable hold threshold per key. Keystrokes pressed during the decision window are queued and replayed in the correct context after the decision — no dropped characters, no false triggers.
2. **Multi-device synergy / independent mappings — more than one keyboard.** Every device can have its own set of mappings; keyboard and mouse cooperate inside the same processing pipeline — a layer switched on the keyboard is immediately followed by the mouse. Using an external numpad as a macro pad, or driving both keyboard and mouse with both hands, is natively supported.

---

## 1. Core Features

- **Combo**: press two keys simultaneously to trigger a new key (e.g. `q+w → Enter`)
- **TapDance**: a single key with four identities — tap / hold / double-tap / double-hold (e.g. `Ctrl tapped twice → {home}`)
- **Layer switching**: base layer + function layers; three layer-activation semantics — `{fnX}` (auto-repeat) / `{bnX}` (swallow repeats) / `{tnX}` (toggle)
- **Leader sequences**: always-on listener — type a configured key sequence at any time to trigger an action; sliding timeout, chained capture, and shield mode supported
- **Defer**: keystrokes downstream of a layer key are delayed until hold/tap is decided, then replayed automatically (chained dependencies)
- **Macros**: `RUN:` (launch programs/URLs/files), `{Sleep N}` (non-blocking delay), `{Select N}`, `{KeyName N}` (repeat a key), `MouseMove(x, y)`
- **Full mouse coverage**: 5 buttons into the pipeline (TD / Combo / layer switching); wheel and movement pass through
- **Kernel driver**: UpperFilter input interception — no device-count limit, hot-plug & sleep friendly
- **Device + application aware runtime**: different mappings per device/app; input state shared across devices by domain
- **Device whitelist**: filter by VID/PID; non-whitelisted devices pass through completely untouched
- **GUI configurator**: CustomTkinter graphical interface, WYSIWYG editing of the config
- **Fail-safe net**: three independent defenses against engine crash, heartbeat loss, and main-thread deadlock, plus a kernel-level emergency escape hotkey

---

## 2. Installation

> ⚠️ **Important limitations — read before installing**: AnyKey's kernel driver **does not carry a Microsoft signing certificate** (signing is too expensive for now — see the note at the top), so it **can only load in Windows Test Signing mode**, and **Test Signing is mutually exclusive with Secure Boot — you must disable Secure Boot in UEFI first**. Once test mode is on, a "Test Mode" watermark stays on the bottom-right of the desktop; some virtualization-based security features (HVCI / Memory Integrity) will block unsigned driver loading — verify your machine's security configuration is compatible before enabling. Proceed only after understanding and accepting these risks.

Get AnyKey one of two ways: download the release package (zip) from GitHub Releases and extract it anywhere, or clone the repository and build from source. **Except for the kernel driver, all components (GUI configurator, system tray, engine) are standalone executables — no installation needed.**

Structure after extracting the release package (these three items sit at the zip root):

```
├── anykey/                  The app: GUI configurator + tray + Rust engine (portable)
├── anykeyFilterDriver/      Kernel filter driver: .sys / .inf / .cer + one-click install/uninstall scripts
└── 安装驱动.bat             Driver install launcher (double-click to run the install script)
```

Three installation steps:

1. **Disable Secure Boot** (full steps below):

   **Option A: enter UEFI setup from Windows**
   - Windows 11: **Settings → System → Recovery → Advanced startup → Restart now**
   - Windows 10: **Settings → Update & Security → Recovery → Advanced startup → Restart now**
   - After rebooting into the blue recovery menu: **Troubleshoot → Advanced options → UEFI Firmware Settings → Restart**; the PC boots straight into BIOS/UEFI
   - Under the **Security / Boot / Authentication** tab (varies by board vendor) find **Secure Boot** and set it to **Disabled**
   - Press **F10** (Save & Exit) to save and reboot

   **Option B: press the BIOS key at power-on**: repeatedly press the board's hotkey while the POST screen shows — usually **Del** on desktops, **F2** on laptops (ThinkPads: Enter then F1). Then disable Secure Boot as above and save.

   **Verify**: back in Windows, run `msinfo32` — "System Summary → Secure Boot State" should read **Off**.

   **Notes**:
   - If **BitLocker** is enabled you may be asked for the 48-digit recovery key when disabling Secure Boot — look it up beforehand in your [Microsoft account](https://account.microsoft.com/devices/recoverykey) or via `manage-bde -protectors -get C:`
   - If the Secure Boot option is grayed out: disable Fast Boot, load factory defaults, or update the BIOS first
   - BIOS menus vary a lot between vendors — consult your motherboard/laptop manual if you can't find it

2. **Run the driver install script**: double-click **`安装驱动.bat`** at the root of the extracted package (or run `anykeyFilterDriver/Install_AnyKey_Filter.bat` as administrator). The script does everything in one pass: detects test mode, runs `bcdedit /set testsigning on` if needed → registers the keyboard + mouse INFs via `pnputil` → binds the UpperFilter to currently connected keyboards and mice → prompts for a reboot. **Run it once; after the reboot, test signing and the driver take effect together.** (If enabling test signing fails, Secure Boot is usually still on — disable it per step 1 and re-run the script.)

3. **Verify and use**: after reboot, the "Test Mode" watermark at the bottom-right of the desktop confirms it's active. Run `anykey/anykey-gui.exe` to edit config and start the engine, or control via the tray menu.

> Manual alternative: if you'd rather not use the script, run `bcdedit /set testsigning on` as administrator and reboot, then register the INFs manually. To turn test mode off: `bcdedit /set testsigning off` (also requires a reboot), after which Secure Boot can be re-enabled.

---

## 3. Uninstallation

1. **Quit the app**: use the tray menu "Full exit" to cascade-close the GUI configurator and engine.
2. **Uninstall the driver**: run `anykeyFilterDriver/Uninstall_AnyKey_Filter.bat` as administrator (double-clicking prompts for elevation). After the script removes the UpperFilter binding and unregisters the INFs, **a reboot is required** for it to fully take effect.
3. **Optional — disable test mode**: run `bcdedit /set testsigning off` as administrator and reboot; the desktop watermark disappears, and Secure Boot can be re-enabled afterwards.
4. **Delete files**: just delete the extracted folder (the config file `anykey_config.json` lives in the program directory and goes with it; back it up first if you want to keep your key mappings).

---

## 4. Feature Usage

### 4.1 Fail-safe Protection

**Emergency escape hotkey (LCtrl + Space + Esc)**: hold all three keys **simultaneously** to trigger an immediate emergency stop — interception off, input queues flushed, all held modifiers released, state reset. Only the **left Ctrl** counts (E0-prefixed right Ctrl is ignored) to prevent accidental triggers. Because it lives in the driver's `ServiceCallback` (DISPATCH_LEVEL, in the path of every keystroke), it works even when the engine's main thread is deadlocked and user-mode code cannot run.

AnyKey is an "input gateway" — if the engine/driver hangs, the whole keyboard locks up. That's why three defenses with **mutually independent detection signals** are built in:

| Layer | Trigger | Mechanism | Location |
|----|---------|------|---------|
| Session | Engine process exits/gets killed | Kernel sees handle close → EvtFileCleanup → interception off immediately | `anykey_flt.c` |
| Heartbeat | Engine heartbeat thread stalls (deadlock) | 30s without heartbeat IOCTL → emergency stop | `anykey_flt.c` + `main.rs` heartbeat thread (every 5s) |
| **Emergency escape hotkey** | Main thread deadlocked but heartbeat still alive (no automatic layer fires) | Hardware-level combo, bypasses the engine entirely | Built into the filter driver |

### 4.2 System Tray

The engine keeps running in the background after the main window closes (tray icon stays). Tray menu:

- **Open main window**: bring back the GUI configurator
- **Help**: quick usage reference
- **Pause / Resume**: temporarily disable/restore all mappings (raw input passes through)
- **Reload settings**: apply edited config without restarting the engine
- **Launch at startup**: writes a registry Run entry to auto-start tray + engine at login
- **Debug logs**: opens the engine log directory for troubleshooting mappings
- **Full exit**: cascades close GUI + engine + tray

The tray icon distinguishes **running / paused** states so you can always tell whether interception is active.

### 4.3 GUI Overview

The GUI is a WYSIWYG config generator that reads/writes `anykey_config.json` and sends IPC commands via the tray. Window layout, top to bottom:

| Area | Content |
|------|------|
| Top bar | Status banner (engine state), **Import / Export** config, **▶ Run / ⏸ Pause**, **? Help** |
| App settings bar | "Global / ● configured apps / ○ running processes" dropdown (auto-refreshes process list on open) + Browse button (see 4.3.6) |
| Device panel | Device list / aliases / device switches / device domains / per-device settings (see 4.3.5) |
| Main editor | Three tabs: **Combo**, **Tapdance**, **Leader**; a status bar above the tabs shows the current editing target; Import / Export / Delete on the right act on the current page |
| Key cheatsheet | Categorized button panel below the main editor (see below) |

Layer config (inside the Tapdance tab) includes a **keyboard visualization**: each key is four-corner colored per its TD settings (top-left tap / top-right hold / bottom-left dbl-tap / bottom-right dbl-hold); mapped / selected / unmapped keys get different background fills, so you can see at a glance where a layer has mappings.

**Double-click any input box for a large editor**: input boxes in the tabs are narrow; double-clicking one opens a large editing window (~500px wide) where long content is fully visible and comfortable to edit. Click "Confirm" to write it back.

**Key cheatsheet panel**: two columns of categorized buttons below the main editor, organizing common key names into nine groups: **Modifiers / Navigation / Editing / Lock / Virtual keys / Fn / Mouse buttons / Media / Numpad**. Clicking a button inserts the corresponding syntax (e.g. `{WheelUp}`, `{Sleep 500}`, `{NumpadEnter}`) at the cursor of the focused input box; **the large editor window accepts them too** — double-click to open it, then click cheatsheet buttons while you edit. The panel scrolls.

#### 4.3.1 Combo

Press two keys simultaneously (within the `comboTime` window, default 200ms, adjustable) to trigger a custom output.

- GUI: in the Combo tab, pick a layer, key 1, key 2, and fill in the output. Layers are covered below.
- Adjustable window: raise it if simultaneous presses often fail to register.
- A combo fires and ends there — it does not chain into other mechanisms.
- Per-layer support: different layers can define different combos.

#### 4.3.2 TapDance

Every physical key, **on every layer**, has four independent behaviors — 7 settings in total, edited per key in the layer table (or by clicking the keyboard visualization) in the Tapdance tab:

| Field | Meaning |
|------|------|
| tap | output on single click (press then release) |
| hold | output on long press |
| ms (hold threshold) | time threshold to trigger hold |
| dbl tap | output on double click |
| ms (double-tap threshold) | interval threshold for double tap |
| dbl hold | output on double-click-and-hold |
| ms (double-hold threshold) | threshold for double hold |

- Leave a field empty and that trigger is disabled.
- Three global thresholds (holdTerm / doubleTapTerm / doubleHoldTerm) are set on the layer config page; per-key values override the globals.
- Applies to keyboard keys and mouse buttons (left/right/middle/side1/side2/wheel).

#### 4.3.3 Leader Sequences

Leader is an **always-on key-sequence listener**. Once configured, the engine watches all input: press a configured sequence of keys (e.g. `he`) within the timeout window and the moment the sequence completes, the mapped output (e.g. `hello`) is emitted automatically. You can also set a shield key to enter shield mode, where triggering keys are swallowed and only the result is emitted.

**Settings (Leader tab)**

| Item | Description |
|----|------|
| Key sequence | The key order to match (e.g. `he`) |
| Output | What gets emitted when the sequence completes |
| Timeout (ms) | Max wait between adjacent keys of the sequence; on timeout the input is discarded and matching restarts |
| Common timeout | Default for sequences without their own timeout (2000ms) |
| Chained capture | Whether a sequence's output feeds back in and keeps participating in sequence matching |

**Sliding timeout**: each sequence can set its own timeout; empty means use the common one. Short fast sequences can use short values, long sequences can be relaxed — the wait always restarts from the last key pressed, sliding per key.

**Chained capture**: when enabled, a sequence's output is fed back through the driver as new input and keeps participating in matching — self-driving chains, where one sequence's output happens to be another sequence's prefix and gets captured in relay. Currently this only works when the output is a single key; multi-character text output breaks the chain.

**Shield mode (optional)**:

- Default is non-shielded: while listening, every key is also delivered to the system as usual; matching happens silently in the background. E.g. configure `he` → `llo`, giving you `hello`
- If you want the sequence keys withheld from the system, set some key's output to the virtual key `{leader}` (as a combo, or any TD output slot): that key becomes a shield switch — once pressed it is swallowed and **all subsequent keys are swallowed too** (never sent to the system) until a sequence completes, input matches nothing, or timeout hits. On success the configured output is emitted. E.g. configure `em` → `John@abc.com`: typing `em` yields `emJohn@abc.com`, while `{leader}em` yields `John@abc.com`

**Common pairing**: use `{Select N}` (general macro, see 5.1) in the output to first select existing text before the cursor — browser-address-bar-style auto-fill — while avoiding stray characters leaking into the input stream.

#### 4.3.4 Layers

Each layer is an independent key mapping set, organized as "base layer + function layers fn1/fn2...". The base layer is the keyboard's default state — you can remap the fundamentals there. Each function layer holds its own independent mapping set. Unlimited function layers supported.

- **Activating a layer**: set any behavior of any key (combo / tap / hold / doubletap / doublehold) to a layer virtual key (e.g. `hold={fn1}`) — that key becomes a layer activation key. Any key works.
- **Stacking**: multiple layers can stack on top of each other.
- **Three activation semantics**: `{fnX}` / `{bnX}` / `{tnX}` — see 5.1 for details.

#### 4.3.5 Device Settings

The device panel manages all connected keyboard/mouse devices:

- **Per-device settings switch**: when on, each device can own its own mapping set (layers / TD / Combos edited while that device is selected apply only to it), while still sharing the base config with others. When off, all devices use the base settings.
- **Device domain (state sharing scope)**: decides "when I switch layers / hold modifiers on one device, do other devices follow?".
  The canonical use: **let keyboard and mouse share layer state** — press the page key on the mouse to switch to layer 2, and what the keyboard types becomes layer-2 content too; switch back and both sides come back together. That's the default (global domain); no setup needed.
  Each device's domain dropdown offers:
  - **Global** (default): all devices share one state. Mouse switches layer, keyboard follows; keyboard switches, mouse follows;
  - **Independent**: this device keeps its own state — e.g. an external macro pad switches layers on its own without touching the main keyboard/mouse;
  - **Domain 2 / 3 / 4**: extra small groups — devices in the same domain follow each other; different domains don't interact.
  Note: domains only share state like "current layer, held modifiers"; each device's own mapping rules stay its own. The domain dropdown appears after enabling that device's "per-device settings switch".
  - Need more? Type your own number — domains are unlimited in theory.
- **Device list**: automatically lists connected keyboards/mice (with VID/PID).
- **Alias**: give devices memorable names ("Custom 68-key", "Logi MX Master").
- **Device switch (subscription)**: **only checked devices are intercepted and processed; unchecked devices pass raw events through untouched**. Only effective while the per-device settings switch is on.
- **Device identification**: to find out which physical device is which, use identify — the driver enters pass-through capture mode (mirrors events, swallows nothing) while you press a few keys on the target keyboard to locate it.
- Some keyboards/mice expose multiple device names, and Windows has its own virtual devices — expect a few extra entries in the list.
- Devices with existing mappings or an alias stay in the list even when unplugged, sorted to the end. To remove one completely, clear its mappings and delete its alias.

#### 4.3.6 App Settings

App settings let **the same key do different things in different software** — e.g. `a` normally types a, but pastes in Excel.

**UI**: in the GUI's left column, above the device panel, sits a small "App settings" panel — two rows:

- Row 1: the "App settings" title with a "Browse..." button on the right;
- Row 2: a dropdown that lists currently running programs when opened.

**How to set up**:

1. Pick an app in the dropdown. The list has three sections: **Global** (default mappings), **●**-prefixed configured apps, **○**-prefixed currently running programs;
2. Target app not running? Click "Browse..." and pick its .exe file;
3. With an app selected, edit Combo / Tapdance / Leader tabs as usual — rules you edit **apply only while that app is the foreground window**; select "Global" to edit the default mappings;
4. Mappings switch automatically as the foreground window changes — no manual action.

**Overlay hierarchy**: the full config is four layers, overridden low to high, **merged per entry** (if a key is individually configured at a higher layer use that; otherwise fall through):

```
Global (base mappings)
  └─ App override          ← edited when "Global + some app" is selected
      └─ Device override   ← edited when "some device + Global" is selected
          └─ Device+app override ← edited when "some device + some app" (highest)
```

Example: Global maps `a→b`, Excel maps `a→c` — pressing a normally types b; with Excel in the foreground, a types c. Note the global TD timing parameters (holdTerm etc.) cannot be overridden per app.

**Border colors**: in the keyboard visualization and all tabs, each rule's border color shows which layer it comes from:

| Border | Meaning |
|---|---|
| Blue | Global base mapping |
| Green | Device override |
| Yellow | Global app override |
| Orange | Device+app override |
| Gray | New, not yet saved |

Border thickness shows ownership: **thick (2px)** = this rule lives right inside your currently selected "device × app" scope — right-click to reset it (fall back to the lower layer); **thin (1px)** = inherited from below, display only.

---

## 5. Output Syntax

The "output" field of every feature (Combo output, TD tap/hold/dt/dh, Leader output) shares one syntax.

### 5.1 Virtual Keys & Macros

**Layer activation virtual keys**:

| Syntax | Behavior | Use case |
|------|------|---------|
| `{fnX}` | while held, every auto-repeat re-sends that key's tap value on the target layer (repeatable) | holding for arrow-key scrolling |
| `{bnX}` | swallows auto-repeat; the key itself emits nothing | hold-only modifiers |
| `{tnX}` | Toggle layer: instantly pins the current layer to fnX; keyup silently stays; layers stack with a push; `{tn0}` clears the stack back to base; idempotent | one key into a persistent "editing/gaming layer" |

**Leader-only**:

| Syntax | Behavior |
|------|------|
| `{leader}` | Leader shield switch: put it on a key's hold / dbl-tap; once pressed the key is swallowed and **subsequent keys are swallowed too** (sequence input never reaches the system; Leader matches and emits) |

**Parameterized macros**:

| Syntax | Behavior |
|------|------|
| `{Sleep N}` | insert a non-blocking N ms delay (SleepTimer-based, doesn't block the pipeline), e.g. `c{Sleep 200}v` |
| `{KeyName N}` | tap the given key N times, with prefix/suffix support: `{a 2}` = press a twice; `hira@126.com{left 12}` = send the text then tap Left 12 times |
| `{Select N}` | select N existing characters before the cursor; subsequent input **replaces** the selection. A general macro, not Leader-specific — the typical use is auto-fill: select the old text before the cursor, then let the output overwrite it, instead of appending to it (like a browser address bar) |

**External commands**:

| Syntax | Behavior |
|------|------|
| `RUN:` prefix | launches programs / folders / URLs / files via `ShellExecuteW`, no cmd window: `RUN:calc.exe`, `RUN:https://example.com`. **Note: spaces after `RUN:` are not escaped** (they're command-argument separators) |

**Mouse**:

| Syntax | Behavior |
|------|------|
| `{MouseLeft}` `{MouseRight}` `{MouseMiddle}` `{MouseSide1}` `{MouseSide2}` | mouse button output (separate channel, press/release fully paired). Legacy names like `{LButton}` still work and are auto-converted to canonical names on save |
| `{WheelUp}` `{WheelDown}` | wheel (self-contained events, no UP is appended) |
| `MouseMove(x, y)` | relative move; positive x is right, positive y is down; negatives allowed, e.g. `MouseMove(-10, 0)` |

**Explicit modifiers**: combine as `{ctrl down}c{ctrl up}` (the `^c`-style prefix notation is not supported); to distinguish sides use single keys `{LCtrl}` `{RCtrl}` `{LAlt}` `{RAlt}` `{LShift}` `{RShift}` `{LWin}` `{RWin}`.

**down / up suffix on any key**: every key supports three forms — `{X}` press and release (tap), `{X down}` press only, `{X up}` release only. Not limited to modifiers; normal keys work too: `{f down}`, `{NumpadEnter down}`, `{WheelUp up}` are all valid. Typical use is precise hold timing:

```
{LCtrl down}{Sleep 100}c{LCtrl up}     ← hold Ctrl first, brief wait, then c to guarantee the combo
{a down}{Sleep 500}{a up}              ← hold a for half a second (game long-press)
```

`down` / `up` must be used in pairs — down without up leaves the key pressed forever.

**Media/system keys**: `{Volume_Up}` `{Volume_Down}` `{Volume_Mute}`, `{Media_Play_Pause}` `{Media_Next}` `{Media_Prev}` `{Media_Stop}`, `{Browser_Back}` `{Browser_Forward}` and other multi-character system key names.

**Numpad**: `{Numpad0}` ~ `{Numpad9}`, `{NumpadAdd}` `{NumpadSub}` `{NumpadMul}` `{NumpadDiv}` `{NumpadDot}` `{NumpadEnter}` (note: multiply is `NumpadMul`, not `NumpadMult`).

### 5.2 Syntax Rules

The output field parses by these rules; understanding them prevents "looks right but doesn't fire" outputs:

1. **`{ }` wrapped = key; unwrapped = plain text**. `{enter}` presses Enter; `hello` types the string "hello" character by character.
2. **Single characters auto-wrap**: a bare single `a`–`z`, `0`–`9`, or basic symbol (`` ` `` `-` `=` `[` `]` `\` `;` `'` `,` `.` `/`) is auto-converted to key format like `{a}` by the GUI.
3. **Multi-character key names must be wrapped**: `{Space}`, `{Enter}`, `{Left}`, `{F1}`... unwrapped multi-character content is always typed as text.
4. **Abbreviations auto-complete**: the GUI normalizes common abbreviations — `{esc}` → `{escape}`, `{del}` → `{delete}`, `{pgup}` → `{pageup}`, `{pgdn}` → `{pagedown}`, `{prtsc}` → `{printscreen}`, `{bs}` → `{backspace}`, etc.
5. **Spaces auto-escape**: literal spaces in the output are auto-replaced with `{space}` by the GUI (the engine trims bare spaces) — just type a space, no need to write `{space}`.
6. **Two spaces are NOT escaped** (important exceptions):
   - spaces **inside `{...}` macro tokens** are kept as-is — `{Sleep 200}`, `{Select 12}`, `{left 12}` rely on spaces to separate parameters; escaping them into `{Sleep{space}200}` would break engine parsing;
   - spaces **inside `RUN:` commands** are kept as-is — path spaces like `RUN:C:\Program Files\app.exe` and command-line argument spaces are part of the syntax.
7. **Case**: lowercase key names inside braces are normalized to lowercase; to type uppercase letters use text or explicit Shift.

---

## 6. Repository Layout

(repo root; dependencies flow one way, no cycles)

```
AnyKey/
├── gui/                   GUI configurator (Python + CustomTkinter)
│   ├── main.py            Main window: top bar / tab dispatch / config IO
│   ├── layout.py          Keyboard visualization layout & key rendering
│   ├── components.py / dialogs.py / app_bar.py / scanner.py
├── lib/
│   └── config.py          Config model & key-name normalization (single source of normalization)
├── anykey-engine/         Rust engine (backend core)
│   ├── src/               Pipeline phases 0-7 / Up1-8, Combo/TD/Layer/Leader/Defer
│   └── tests/             Integration tests (109 unit + scenario tests)
├── anykey-tray/           Rust system tray (engine lifecycle / IPC / autostart)
├── anykey-filter-driver/  C kernel filter driver (WDK)
│   ├── sys/               Driver source (anykey_flt.c / rawpdo.c / public.h)
│   ├── deploy/            Release scripts (one-click install/uninstall + INF + .cer)
│   └── build_driver.bat   Driver build entry
├── assets/                Icons & help doc (help.md)
├── build/                 Build chain
│   ├── build.bat          One-click 7-step pipeline (driver→engine→GUI→tray→assemble→sign→release)
│   ├── build_driver_release.py / build_engine_release.py / build_tray_release.py
│   ├── build_release_package.py   Release package assembly + zip
│   └── anykey.spec        PyInstaller config
├── docs/                  Design docs (DESIGN.md / architecture svg)
├── CODEMAP.md             Code map: structure index for code readers
├── engines/rust/          Engine exe deployment location (build artifact, loaded by GUI)
├── scripts/               Helper scripts (test scenario generation etc.)
├── tests/                 Python config parsing tests
├── requirements.txt       Python deps (customtkinter / Pillow / psutil / pywin32)
├── anykey_config.json     Runtime config (next to the exe; the single source of truth shared by GUI / Tray / Engine)
└── LICENSE / SECURITY.md
```

**Key boundaries** (cross-module invariants):

1. `anykey_config.json` is the single source of truth shared by GUI / Tray / Engine.
2. Key-name normalization happens only in the GUI (`lib/config.py`); the engine treats it as a defensive layer only.
3. Driver IOCTLs/structs are byte-for-byte aligned across three implementations: `public.h`(C) ↔ `filter_driver.rs`(Rust) ↔ `driver.py`(Python).
4. GUI and Tray are separate processes talking over IPC; the Engine is a separate Rust process talking to the kernel through the Filter Driver.

---

## 7. Building from Source

### Rust engine

```bash
cd anykey-engine
cargo build --release          # output: target/release/anykey-engine.exe
cargo test                     # unit tests + scenario tests
```

### Kernel driver (needs VS2022 + WDK 10.0.28000.0)

```bash
cd anykey-filter-driver
build_driver.bat               # WDK build + test signing; output at BIN\X64\RELEASE\ANYKEY_FLT.SYS
```

Or via the unified entry (build + test sign + deploy into the `deploy\` release dir: `anykey_flt.sys` + `anykey_flt.cer`) — also step 1 of `build.bat`:

```bash
cd build
python build_driver_release.py
```

### Release package (GUI + tray + engine + driver distribution)

```bash
cd build
build.bat                                     # build everything; final step assembles the release\ standard layout
python build_release_package.py --zip 1.0.0   # zips release\ into AnyKey_v1.0.0.zip (GitHub Release asset)
```

`build.bat` seven steps: kernel driver (cl/link + test signing → `deploy\`) → Rust engine → GUI (PyInstaller) → Rust tray → assemble `dist\AnyKey\` → code signing (when a cert is present) → assemble `release\` (`anykey\` + `anykeyFilterDriver\` + `安装驱动.bat`).

### Python side (GUI)

```bash
pip install -r requirements.txt
python -m gui.main             # launch the configurator
```

---

## 8. Testing

- Rust engine: `cargo test` (73 unit + scenario tests, including integration tests in `tests/`)
- Python: config parsing tests under `tests/`

---

## 9. Security Notes

> AnyKey contains a **kernel-level driver component**. Download and install only from official channels (the Releases page); verify the **SHA256 checksum** published there before installing. The driver only intercepts events from whitelisted devices; non-whitelisted devices pass through untouched.

**Privacy**: all key processing happens locally — no telemetry, no network reporting, keystroke data never leaves your machine.

**Fail-safes**: three layers (process-exit Session interception-off / 30s heartbeat watchdog / kernel-level LCtrl+Space+Esc emergency escape) — see 4.1.

---

## 10. Documentation

- Design rationale: [`docs/DESIGN.md`](docs/DESIGN.md) — motivation and key decisions behind each core feature
- Code map: root-level [`CODEMAP.md`](CODEMAP.md) + [`docs/code_map_arch.svg`](docs/code_map_arch.svg) — module breakdown and data flow (a structure index for code readers)

---

## 11. Roadmap

- Obtain a Microsoft EV code-signing certificate and complete WHQL / Hardware Lab Kit certification so the driver loads under normal signing mode, without the test watermark
- Config profiles + optional cloud sync
- Macro recording (including mouse trajectories) and replay
- Device page completion: per-device layer mappings
- UI evolution: light/dark themes, keyboard heatmaps, combo conflict detection, CSV batch import/export
- Mouse gestures (right-drag trajectory mapped to actions)
- Community sharing platform + plugin system (open API)
- Implement sending for remaining media/browser control keys

---

## 12. License

This project is licensed under the [Apache License 2.0](LICENSE).

---

## 13. Disclaimer

This is an open-source tool maintained by an individual/community. Using it changes system input behavior — test in a VM or a non-critical environment first. The author is not liable for data loss or system issues caused by misuse.
