//! Wi-Fi and Bluetooth-headset connection watcher (WinRT, polled on a worker thread).
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::*;
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
use windows::Networking::Connectivity::NetworkInformation;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

#[derive(Clone, Debug)]
pub enum NetEvent {
    WifiUp(String),
    WifiDown,
    BtUp(String),
    BtDown(String),
}

#[derive(Clone, Default)]
pub struct NetMon {
    pub events: Arc<Mutex<Vec<NetEvent>>>,
}

impl NetMon {
    pub fn drain(&self) -> Vec<NetEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

fn wifi_ssid() -> Option<String> {
    let p = NetworkInformation::GetInternetConnectionProfile().ok()?;
    if !p.IsWlanConnectionProfile().ok()? {
        return None;
    }
    let d = p.WlanConnectionProfileDetails().ok()?;
    Some(d.GetConnectedSsid().ok()?.to_string())
}

fn bt_connected() -> Option<BTreeSet<String>> {
    // Connected Bluetooth endpoints (classic + LE), filtered by the Bluetooth protocol id.
    let aqs = h!("System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" AND System.Devices.Aep.IsConnected:=System.StructuredQueryType.Boolean#True");
    let list = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(aqs, None, DeviceInformationKind::AssociationEndpoint)
        .ok()?
        .get()
        .ok()?;
    let mut set = BTreeSet::new();
    for d in list {
        if let Ok(n) = d.Name() {
            let n = n.to_string();
            if !n.is_empty() {
                set.insert(n);
            }
        }
    }
    Some(set)
}

pub fn spawn() -> NetMon {
    let mon = NetMon::default();
    let events = mon.events.clone();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut wifi: Option<Option<String>> = None; // None = not sampled yet
        let mut bt: Option<BTreeSet<String>> = None;
        loop {
            let now = wifi_ssid();
            if let Some(prev) = &wifi {
                if *prev != now {
                    let mut e = events.lock().unwrap();
                    match &now {
                        Some(s) => e.push(NetEvent::WifiUp(s.clone())),
                        None => e.push(NetEvent::WifiDown),
                    }
                }
            }
            wifi = Some(now);

            if let Some(cur) = bt_connected() {
                if let Some(prev) = &bt {
                    let mut e = events.lock().unwrap();
                    for n in cur.difference(prev) {
                        e.push(NetEvent::BtUp(n.clone()));
                    }
                    for n in prev.difference(&cur) {
                        e.push(NetEvent::BtDown(n.clone()));
                    }
                }
                bt = Some(cur);
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
    mon
}
