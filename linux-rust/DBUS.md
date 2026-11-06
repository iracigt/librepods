# D-Bus Interface

LibrePods (linux-rust) exposes a control and status interface on the D-Bus session bus. It is intended for scripts, status bars, and desktop integrations that need to read device state or change listening settings without using the GUI.

## 1. Endpoint

| Item           | Value                              |
|----------------|------------------------------------|
| Bus            | Session                            |
| Well-known name| `me.kavishdevar.librepods`         |
| Object path    | `/LibrePods`                       |
| Interface      | `local.librepods.AirPodsTrayApp`   |

The service is registered at application startup. If the session bus is unavailable or the name is already owned, registration fails, a log message is emitted, and the application continues without the D-Bus interface.

The object also implements the standard `org.freedesktop.DBus.Properties`, `org.freedesktop.DBus.Introspectable`, and `org.freedesktop.DBus.Peer` interfaces.

## 2. Device Selection

The interface describes a single device: among connected devices with an active AACP manager, the one with the lowest Bluetooth address (lexicographic order of its string form). The selection is therefore stable while the set of connected devices is unchanged. When none is connected, all properties report their unknown values (Section 4.1) and all methods and property writes fail with `org.freedesktop.DBus.Error.Failed` ("No AirPods connected").

## 3. Methods

### 3.1 `SetNoiseMode(i mode)`

Sets the listening mode.

| `mode` | Meaning            |
|--------|--------------------|
| 0      | Off                |
| 1      | Noise Cancellation |
| 2      | Transparency       |
| 3      | Adaptive           |

Values outside 0–3 fail with `org.freedesktop.DBus.Error.InvalidArgs`. The value transmitted to the device is `mode + 1`. Whether Off is accepted depends on the device and on the `AllowOff` setting.

### 3.2 `ToggleNoiseMode()`

If the current listening mode is Transparency (2), sets Noise Cancellation (1); otherwise sets Transparency (2). Off and Adaptive therefore toggle to Transparency.

## 4. Properties

### 4.1 Conventions

- Integer properties report `-1` when the value is unknown.
- String properties report the empty string when the value is unknown.
- Boolean properties report `false` when the value is unknown.

### 4.2 Read-only Properties

| Name                     | Type | Description |
|--------------------------|------|-------------|
| `Connected`              | `b`  | AACP control channel is established. |
| `Name`                   | `s`  | Device name. |
| `ModelNumber`            | `s`  | Model number, e.g. `A2084`. |
| `Manufacturer`           | `s`  | Manufacturer string. |
| `SerialNumber`           | `s`  | Serial number of the device (case/system). |
| `FirmwareVersion`        | `s`  | Firmware version. |
| `HardwareRevision`       | `s`  | Hardware revision. |
| `LeftSerialNumber`       | `s`  | Serial number of the left bud. |
| `RightSerialNumber`      | `s`  | Serial number of the right bud. |
| `BatteryLeft`            | `i`  | Left bud charge, 0–100 percent; `-1` if unknown or disconnected. |
| `BatteryRight`           | `i`  | Right bud charge, 0–100 percent; `-1` if unknown or disconnected. |
| `BatteryCase`            | `i`  | Case charge, 0–100 percent; `-1` if unknown or disconnected. |
| `ChargingLeft`           | `b`  | Left bud is charging. |
| `ChargingRight`          | `b`  | Right bud is charging. |
| `ChargingCase`           | `b`  | Case is charging. |
| `ListeningMode`          | `i`  | Current listening mode, encoded as in Section 3.1. |
| `EarPrimary`             | `s`  | Placement of the primary bud (see Section 4.4). |
| `EarSecondary`           | `s`  | Placement of the secondary bud (see Section 4.4). |
| `ConversationalAwarenessStatus` | `i` | Raw conversational-awareness status as last reported by the device. |
| `OwnsConnection`         | `b`  | This host currently owns the audio connection. |

### 4.3 Read-write Properties

| Name                 | Type | Description |
|----------------------|------|-------------|
| `ConversationDetect` | `b`  | Conversation detection (Conversational Awareness) enabled. |
| `AllowOff`           | `b`  | Off is permitted as a listening mode. |
| `AdaptiveStrength`   | `i`  | Adaptive-mode noise reduction strength, 0–100; 100 is strongest. |

Writes are transmitted to the device immediately. The device does not echo these settings, so the service records the written value locally once the command has been sent; reads issued after the write returns observe that value. A write to `AdaptiveStrength` outside 0–100 fails with `org.freedesktop.DBus.Error.InvalidArgs`. On the wire the strength is inverted (wire value = `100 - AdaptiveStrength`).

### 4.4 Ear Placement Values

`EarPrimary` and `EarSecondary` refer to the primary and secondary buds as designated by the device, not to left and right.

| Value            | Meaning |
|------------------|---------|
| `in-ear`         | Bud is in an ear. |
| `out-of-ear`     | Bud is out of the ear and out of the case. |
| `in-case`        | Bud is in the case. |
| `disconnected`   | Bud is not connected. |
| (empty)          | Unknown. |

## 5. Change Notification

All properties emit `org.freedesktop.DBus.Properties.PropertiesChanged`. Device state is sampled every 500 ms into a cache from which property reads are served; a signal is emitted for each property whose value differs from the previous sample. Consequently:

- property reads may lag the device by up to 500 ms (except immediately after a property write, see Section 4.3);
- notification latency is up to 500 ms;
- a value that changes and reverts within one sampling interval produces no signal;
- changes caused by a property write are reported by the next sample rather than synchronously with the write.

## 6. Examples

Read all properties:

```sh
busctl --user introspect me.kavishdevar.librepods /LibrePods local.librepods.AirPodsTrayApp
```

Read a single property:

```sh
busctl --user get-property me.kavishdevar.librepods /LibrePods \
    local.librepods.AirPodsTrayApp BatteryLeft
```

Set Noise Cancellation:

```sh
busctl --user call me.kavishdevar.librepods /LibrePods \
    local.librepods.AirPodsTrayApp SetNoiseMode i 1
```

Toggle between Noise Cancellation and Transparency:

```sh
gdbus call --session -d me.kavishdevar.librepods -o /LibrePods \
    -m local.librepods.AirPodsTrayApp.ToggleNoiseMode
```

Disable Conversation Detection:

```sh
busctl --user set-property me.kavishdevar.librepods /LibrePods \
    local.librepods.AirPodsTrayApp ConversationDetect b false
```

Monitor changes:

```sh
dbus-monitor --session "type='signal',sender='me.kavishdevar.librepods',path='/LibrePods'"
```

## 7. Integration Examples

The examples below assume the service is running on the current user's session bus. Each reads or writes only the members defined in Sections 3 and 4.

### 7.1 Shell: Status Line

Prints a one-line battery summary, suitable for a status bar module polled at a fixed interval. `busctl` prints a property as its type signature followed by its value, so the second field is the value.

```sh
#!/bin/sh
dest=me.kavishdevar.librepods
path=/LibrePods
iface=local.librepods.AirPodsTrayApp

prop() { busctl --user get-property "$dest" "$path" "$iface" "$1" 2>/dev/null | cut -d' ' -f2-; }

[ "$(prop Connected)" = true ] || { echo "AirPods: disconnected"; exit 0; }

fmt() { [ "$1" -ge 0 ] && echo "$1%" || echo "--"; }
echo "L $(fmt "$(prop BatteryLeft)") R $(fmt "$(prop BatteryRight)") C $(fmt "$(prop BatteryCase)")"
```

### 7.2 Shell: Cycle Listening Modes

Cycles Noise Cancellation → Transparency → Adaptive → Noise Cancellation. Bind it to a hotkey in the desktop environment.

```sh
#!/bin/sh
dest=me.kavishdevar.librepods
path=/LibrePods
iface=local.librepods.AirPodsTrayApp

mode=$(busctl --user get-property "$dest" "$path" "$iface" ListeningMode | cut -d' ' -f2)
case "$mode" in
    1) next=2 ;;
    2) next=3 ;;
    *) next=1 ;;
esac
busctl --user call "$dest" "$path" "$iface" SetNoiseMode i "$next"
```

### 7.3 Shell: Read All Properties at Once

`GetAll` returns every property in one call, which costs less than issuing one `get-property` per field.

```sh
gdbus call --session -d me.kavishdevar.librepods -o /LibrePods \
    -m org.freedesktop.DBus.Properties.GetAll local.librepods.AirPodsTrayApp
```

### 7.4 Python: Read, Write, and Subscribe

Uses PyGObject (`python3-gi` / `python-gobject`), which most desktop distributions install by default. The script prints the current state, sets Adaptive strength to 50, and then prints every property change until it is interrupted.

```python
#!/usr/bin/env python3
from gi.repository import Gio, GLib

DEST = "me.kavishdevar.librepods"
PATH = "/LibrePods"
IFACE = "local.librepods.AirPodsTrayApp"

bus = Gio.bus_get_sync(Gio.BusType.SESSION)
proxy = Gio.DBusProxy.new_sync(bus, Gio.DBusProxyFlags.NONE, None, DEST, PATH, IFACE)

# Cached properties are populated from GetAll when the proxy is created.
for name in proxy.get_cached_property_names():
    print(f"{name} = {proxy.get_cached_property(name).unpack()}")

# Method call: SetNoiseMode(3) selects Adaptive.
proxy.call_sync("SetNoiseMode", GLib.Variant("(i)", (3,)), Gio.DBusCallFlags.NONE, -1, None)

# Property write goes through org.freedesktop.DBus.Properties.Set.
bus.call_sync(DEST, PATH, "org.freedesktop.DBus.Properties", "Set",
              GLib.Variant("(ssv)", (IFACE, "AdaptiveStrength", GLib.Variant("i", 50))),
              None, Gio.DBusCallFlags.NONE, -1, None)

def on_changed(_proxy, changed, _invalidated):
    for k, v in changed.unpack().items():
        print(f"changed: {k} = {v}")

proxy.connect("g-properties-changed", on_changed)
GLib.MainLoop().run()
```

Errors are raised as `GLib.Error`. For example, calling a method while no device is connected raises an error whose message contains `org.freedesktop.DBus.Error.Failed`.

### 7.5 Rust: zbus Client

Requires `zbus = "5"` and `tokio`. The `proxy` macro generates a typed client from the interface definition.

```rust
use zbus::{proxy, Connection};

#[proxy(
    interface = "local.librepods.AirPodsTrayApp",
    default_service = "me.kavishdevar.librepods",
    default_path = "/LibrePods"
)]
trait LibrePods {
    fn set_noise_mode(&self, mode: i32) -> zbus::Result<()>;
    fn toggle_noise_mode(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn battery_left(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn listening_mode(&self) -> zbus::Result<i32>;
    #[zbus(property)]
    fn conversation_detect(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_conversation_detect(&self, on: bool) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let p = LibrePodsProxy::new(&conn).await?;
    println!("left battery: {}%", p.battery_left().await?);
    p.set_conversation_detect(false).await?;
    p.toggle_noise_mode().await?;
    Ok(())
}
```
