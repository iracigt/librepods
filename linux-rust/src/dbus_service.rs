// Session-bus control interface: service me.kavishdevar.librepods, path /LibrePods,
// iface local.librepods.AirPodsTrayApp. See DBUS.md.
// Listening modes are 0=Off, 1=NoiseCancellation, 2=Transparency, 3=Adaptive; the AACP wire value is mode+1.
// Properties describe the connected device with the lowest address that has an AACP manager.
// The poll loop samples AACP state every POLL_MS into a cached snapshot that property getters read,
// and emits PropertiesChanged for fields that differ, rather than hooking each parser path.
use crate::bluetooth::aacp::{
    AACPManager, BatteryComponent, BatteryStatus, ControlCommandIdentifiers as Cmd,
    ControlCommandStatus, EarDetectionStatus,
};
use crate::bluetooth::managers::DeviceManagers;
use crate::devices::enums::DeviceInformation;
use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use zbus::{fdo, interface};

const POLL_MS: u64 = 500;
const WIRE_ANC: u8 = 0x02;
const WIRE_TRANSPARENCY: u8 = 0x03;

type Managers = Arc<RwLock<HashMap<String, DeviceManagers>>>;

// Unknown numeric values are reported as -1, unknown strings as "".
#[derive(Clone, PartialEq)]
struct Snapshot {
    connected: bool,
    name: String,
    model_number: String,
    manufacturer: String,
    serial_number: String,
    firmware_version: String,
    hardware_revision: String,
    left_serial_number: String,
    right_serial_number: String,
    battery_left: i32,
    battery_right: i32,
    battery_case: i32,
    charging_left: bool,
    charging_right: bool,
    charging_case: bool,
    listening_mode: i32,
    ear_primary: String,
    ear_secondary: String,
    conversation_detect: bool,
    conversational_awareness_status: i32,
    allow_off: bool,
    adaptive_strength: i32,
    owns_connection: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            connected: false,
            name: String::new(),
            model_number: String::new(),
            manufacturer: String::new(),
            serial_number: String::new(),
            firmware_version: String::new(),
            hardware_revision: String::new(),
            left_serial_number: String::new(),
            right_serial_number: String::new(),
            battery_left: -1,
            battery_right: -1,
            battery_case: -1,
            charging_left: false,
            charging_right: false,
            charging_case: false,
            listening_mode: -1,
            ear_primary: String::new(),
            ear_secondary: String::new(),
            conversation_detect: false,
            conversational_awareness_status: -1,
            allow_off: false,
            adaptive_strength: -1,
            owns_connection: false,
        }
    }
}

fn ear_str(s: Option<&EarDetectionStatus>) -> String {
    match s {
        Some(EarDetectionStatus::InEar) => "in-ear",
        Some(EarDetectionStatus::OutOfEar) => "out-of-ear",
        Some(EarDetectionStatus::InCase) => "in-case",
        Some(EarDetectionStatus::Disconnected) => "disconnected",
        None => "",
    }
    .to_string()
}

async fn snapshot(aacp: &AACPManager) -> Snapshot {
    let st = aacp.state.lock().await;
    let ctl = |id: Cmd| {
        st.control_command_status_list
            .iter()
            .find(|s| s.identifier == id)
            .and_then(|s| s.value.first().copied())
    };
    let batt = |c: BatteryComponent| {
        st.battery_info
            .iter()
            .find(|b| b.component == c && b.status != BatteryStatus::Disconnected)
            .map(|b| (b.level as i32, b.status == BatteryStatus::Charging))
            .unwrap_or((-1, false))
    };
    let (battery_left, charging_left) = batt(BatteryComponent::Left);
    let (battery_right, charging_right) = batt(BatteryComponent::Right);
    let (battery_case, charging_case) = batt(BatteryComponent::Case);

    let mut s = Snapshot {
        connected: st.sender.is_some(),
        battery_left,
        battery_right,
        battery_case,
        charging_left,
        charging_right,
        charging_case,
        listening_mode: ctl(Cmd::ListeningMode).map_or(-1, |v| v as i32 - 1),
        ear_primary: ear_str(st.ear_detection_status.first()),
        ear_secondary: ear_str(st.ear_detection_status.get(1)),
        conversation_detect: ctl(Cmd::ConversationDetectConfig) == Some(0x01),
        conversational_awareness_status: st.conversational_awareness_status as i32,
        allow_off: ctl(Cmd::AllowOffOption) == Some(0x01),
        // Wire value is inverted: 0 is the strongest setting. Expose 0..=100, 100 = strongest.
        adaptive_strength: ctl(Cmd::AutoAncStrength).map_or(-1, |v| 100 - v.min(100) as i32),
        owns_connection: st.owns,
        ..Default::default()
    };
    if let Some(mac) = st.airpods_mac
        && let Some(dev) = st.devices.get(&mac.to_string())
    {
        s.name = dev.name.clone();
        if let Some(DeviceInformation::AirPods(i)) = &dev.information {
            s.model_number = i.model_number.clone();
            s.manufacturer = i.manufacturer.clone();
            s.serial_number = i.serial_number.clone();
            s.firmware_version = i.version1.clone();
            s.hardware_revision = i.hardware_revision.clone();
            s.left_serial_number = i.left_serial_number.clone();
            s.right_serial_number = i.right_serial_number.clone();
        }
    }
    s
}

pub struct AirPodsTrayApp {
    managers: Managers,
    cache: Mutex<Snapshot>,
}

impl AirPodsTrayApp {
    // Lowest address wins so the selection is stable when several devices are connected.
    async fn aacp_opt(&self) -> Option<Arc<AACPManager>> {
        let m = self.managers.read().await;
        let mut v: Vec<_> = m.iter().filter_map(|(k, d)| d.get_aacp().map(|a| (k, a))).collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v.into_iter().next().map(|(_, a)| a)
    }

    async fn aacp(&self) -> fdo::Result<Arc<AACPManager>> {
        self.aacp_opt()
            .await
            .ok_or_else(|| fdo::Error::Failed("No AirPods connected".into()))
    }

    // Fresh sample from the device state; used by the poll loop.
    async fn sample(&self) -> Snapshot {
        match self.aacp_opt().await {
            Some(a) => snapshot(&a).await,
            None => Snapshot::default(),
        }
    }

    fn snap(&self) -> Snapshot {
        self.cache.lock().unwrap().clone()
    }

    async fn send_to(aacp: &AACPManager, id: Cmd, value: u8) -> fdo::Result<()> {
        aacp.send_control_command(id, &[value])
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    async fn send(&self, id: Cmd, value: u8) -> fdo::Result<()> {
        Self::send_to(&*self.aacp().await?, id, value).await
    }

    // Settings other than ListeningMode are not echoed back by the AirPods, so record the new
    // value locally after sending, as the GUI and tray do. The cache is refreshed so an
    // immediate read returns the written value; the poll loop still emits PropertiesChanged.
    async fn send_remember(&self, id: Cmd, value: u8) -> fdo::Result<()> {
        let aacp = self.aacp().await?;
        Self::send_to(&aacp, id, value).await?;
        {
            let mut st = aacp.state.lock().await;
            match st.control_command_status_list.iter_mut().find(|s| s.identifier == id) {
                Some(s) => s.value = vec![value],
                None => st
                    .control_command_status_list
                    .push(ControlCommandStatus { identifier: id, value: vec![value] }),
            }
        }
        *self.cache.lock().unwrap() = snapshot(&aacp).await;
        Ok(())
    }
}

#[interface(name = "local.librepods.AirPodsTrayApp")]
impl AirPodsTrayApp {
    async fn set_noise_mode(&self, mode: i32) -> fdo::Result<()> {
        if !(0..=3).contains(&mode) {
            return Err(fdo::Error::InvalidArgs(format!("invalid noise mode {mode}")));
        }
        self.send(Cmd::ListeningMode, mode as u8 + 1).await
    }

    async fn toggle_noise_mode(&self) -> fdo::Result<()> {
        let next = if self.snap().listening_mode == WIRE_TRANSPARENCY as i32 - 1 {
            WIRE_ANC
        } else {
            WIRE_TRANSPARENCY
        };
        self.send(Cmd::ListeningMode, next).await
    }

    // Read-only properties.
    #[zbus(property)]
    async fn connected(&self) -> bool { self.snap().connected }
    #[zbus(property)]
    async fn name(&self) -> String { self.snap().name }
    #[zbus(property)]
    async fn model_number(&self) -> String { self.snap().model_number }
    #[zbus(property)]
    async fn manufacturer(&self) -> String { self.snap().manufacturer }
    #[zbus(property)]
    async fn serial_number(&self) -> String { self.snap().serial_number }
    #[zbus(property)]
    async fn firmware_version(&self) -> String { self.snap().firmware_version }
    #[zbus(property)]
    async fn hardware_revision(&self) -> String { self.snap().hardware_revision }
    #[zbus(property)]
    async fn left_serial_number(&self) -> String { self.snap().left_serial_number }
    #[zbus(property)]
    async fn right_serial_number(&self) -> String { self.snap().right_serial_number }
    #[zbus(property)]
    async fn battery_left(&self) -> i32 { self.snap().battery_left }
    #[zbus(property)]
    async fn battery_right(&self) -> i32 { self.snap().battery_right }
    #[zbus(property)]
    async fn battery_case(&self) -> i32 { self.snap().battery_case }
    #[zbus(property)]
    async fn charging_left(&self) -> bool { self.snap().charging_left }
    #[zbus(property)]
    async fn charging_right(&self) -> bool { self.snap().charging_right }
    #[zbus(property)]
    async fn charging_case(&self) -> bool { self.snap().charging_case }
    #[zbus(property)]
    async fn listening_mode(&self) -> i32 { self.snap().listening_mode }
    #[zbus(property)]
    async fn ear_primary(&self) -> String { self.snap().ear_primary }
    #[zbus(property)]
    async fn ear_secondary(&self) -> String { self.snap().ear_secondary }
    #[zbus(property)]
    async fn conversational_awareness_status(&self) -> i32 {
        self.snap().conversational_awareness_status
    }
    #[zbus(property)]
    async fn owns_connection(&self) -> bool { self.snap().owns_connection }

    // Read-write properties.
    #[zbus(property)]
    async fn conversation_detect(&self) -> bool { self.snap().conversation_detect }
    #[zbus(property)]
    async fn set_conversation_detect(&self, on: bool) -> zbus::Result<()> {
        Ok(self.send_remember(Cmd::ConversationDetectConfig, if on { 0x01 } else { 0x02 }).await?)
    }

    #[zbus(property)]
    async fn allow_off(&self) -> bool { self.snap().allow_off }
    #[zbus(property)]
    async fn set_allow_off(&self, on: bool) -> zbus::Result<()> {
        Ok(self.send_remember(Cmd::AllowOffOption, if on { 0x01 } else { 0x02 }).await?)
    }

    #[zbus(property)]
    async fn adaptive_strength(&self) -> i32 { self.snap().adaptive_strength }
    #[zbus(property)]
    async fn set_adaptive_strength(&self, strength: i32) -> zbus::Result<()> {
        if !(0..=100).contains(&strength) {
            return Err(fdo::Error::InvalidArgs(format!("strength {strength} not in 0..=100")).into());
        }
        Ok(self.send_remember(Cmd::AutoAncStrength, (100 - strength) as u8).await?)
    }
}

pub async fn serve(managers: Managers) {
    let res = zbus::connection::Builder::session()
        .and_then(|b| b.name("me.kavishdevar.librepods"))
        .and_then(|b| b.serve_at("/LibrePods", AirPodsTrayApp { managers, cache: Mutex::default() }));
    let builder = match res {
        Ok(b) => b,
        Err(e) => {
            warn!("D-Bus service setup failed: {e}");
            return;
        }
    };
    let conn = match builder.build().await {
        Ok(c) => c,
        Err(e) => {
            error!("D-Bus service failed to start: {e}");
            return;
        }
    };
    info!("D-Bus service registered on session bus");

    let iref = match conn
        .object_server()
        .interface::<_, AirPodsTrayApp>("/LibrePods")
        .await
    {
        Ok(i) => i,
        Err(e) => {
            error!("D-Bus interface lookup failed: {e}");
            return;
        }
    };
    let mut last = Snapshot::default();
    loop {
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
        let obj = iref.get().await;
        let cur = obj.sample().await;
        *obj.cache.lock().unwrap() = cur.clone();
        if cur == last {
            continue;
        }
        let em = iref.signal_emitter();
        macro_rules! notify {
            ($($f:ident => $sig:ident),* $(,)?) => {$(
                if cur.$f != last.$f {
                    if let Err(e) = obj.$sig(em).await { warn!("PropertiesChanged failed: {e}"); }
                }
            )*};
        }
        notify!(
            connected => connected_changed, name => name_changed,
            model_number => model_number_changed, manufacturer => manufacturer_changed,
            serial_number => serial_number_changed, firmware_version => firmware_version_changed,
            hardware_revision => hardware_revision_changed,
            left_serial_number => left_serial_number_changed,
            right_serial_number => right_serial_number_changed,
            battery_left => battery_left_changed, battery_right => battery_right_changed,
            battery_case => battery_case_changed, charging_left => charging_left_changed,
            charging_right => charging_right_changed, charging_case => charging_case_changed,
            listening_mode => listening_mode_changed, ear_primary => ear_primary_changed,
            ear_secondary => ear_secondary_changed,
            conversational_awareness_status => conversational_awareness_status_changed,
            owns_connection => owns_connection_changed,
            conversation_detect => conversation_detect_changed, allow_off => allow_off_changed,
            adaptive_strength => adaptive_strength_changed,
        );
        last = cur;
    }
}
