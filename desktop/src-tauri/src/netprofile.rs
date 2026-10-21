//! The network profile of the connected networks (Private, Public or Domain), for the debug window. It matters because the firewall rule
//! of the installer covers Private and Public (D12), and because Windows marks a new network Public by default: a phone that can't reach
//! the PC on a Public network is the first thing to look at. It is read from the Network List Manager (COM), the API behind
//! `Get-NetConnectionProfile`, so no PowerShell process is started.
use serde::Serialize;

/// One connected network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    pub name: String,
    /// `Private`, `Public`, `Domain` or `Unknown`
    pub category: String,
}

/// The word for an `NLM_NETWORK_CATEGORY` value.
pub fn category_name(value: i32) -> &'static str {
    match value {
        0 => "Public",
        1 => "Private",
        2 => "Domain",
        _ => "Unknown",
    }
}

/// The connected networks. An empty list (and the reason) if Windows can't say. Must run on a thread that may use COM: it initialises COM
/// for the thread itself, so call it from the blocking pool.
pub fn connected_networks() -> Result<Vec<Network>, String> {
    use windows::Win32::Networking::NetworkListManager::{INetworkListManager, NLM_ENUM_NETWORK_CONNECTED, NetworkListManager};
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
    // SAFETY: COM is initialised for this thread and undone at the end if it worked. Every interface pointer is used on this thread only
    // and dropped before `CoUninitialize`.
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| -> Result<Vec<Network>, String> {
            let manager: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
            let networks = manager.GetNetworks(NLM_ENUM_NETWORK_CONNECTED).map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            loop {
                let mut item = [None];
                let mut fetched = 0u32;
                if networks.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(network) = item[0].take() else {
                    break;
                };
                let name = network.GetName().map(|n| n.to_string()).unwrap_or_default();
                let category = network.GetCategory().map(|c| category_name(c.0)).unwrap_or("Unknown");
                out.push(Network { name, category: category.to_string() });
            }
            Ok(out)
        })();
        if com {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_have_words() {
        assert_eq!(category_name(0), "Public");
        assert_eq!(category_name(1), "Private");
        assert_eq!(category_name(2), "Domain");
        assert_eq!(category_name(7), "Unknown");
    }

    /// Asks the real Windows (read only). Whatever it answers, it must not fail to run.
    #[test]
    fn the_real_network_list_can_be_read() {
        let networks = connected_networks().expect("the Network List Manager answers");
        for n in &networks {
            assert!(["Private", "Public", "Domain", "Unknown"].contains(&n.category.as_str()));
        }
    }
}
