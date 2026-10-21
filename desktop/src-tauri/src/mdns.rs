//! mDNS advertising of `_kinrelay._tcp` with TXT `id` (the short ID) and `v=1` (protocol.md, "Discovery").
use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::Result;

pub const SERVICE_TYPE: &str = "_kinrelay._tcp.local.";

pub struct Advertiser {
    daemon: ServiceDaemon,
}

/// The service record. The addresses are the PC's own, found by the library on every interface.
pub fn service_info(short_id: &str, port: u16) -> Result<ServiceInfo> {
    let instance = format!("Kin-{short_id}");
    let host = format!("kin-{}.local.", short_id.to_lowercase());
    let txt = [("id", short_id), ("v", "1")];
    Ok(ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, &txt[..])?.enable_addr_auto())
}

impl Advertiser {
    /// Starts advertising. This opens multicast sockets, so it is skipped in developer mode (loopback only).
    pub fn start(short_id: &str, port: u16) -> Result<Advertiser> {
        let daemon = ServiceDaemon::new()?;
        daemon.register(service_info(short_id, port)?)?;
        log::info!("mDNS: advertising {SERVICE_TYPE} on port {port} with id={short_id}");
        Ok(Advertiser { daemon })
    }

    pub fn stop(self) {
        // `shutdown` hands back a receiver for the daemon's answer. Nothing to wait for here.
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_records_and_name() {
        let info = service_info("ZHVU5QER", 47821).unwrap();
        assert_eq!(info.get_type(), "_kinrelay._tcp.local.");
        assert_eq!(info.get_port(), 47821);
        assert_eq!(info.get_property_val_str("id"), Some("ZHVU5QER"));
        assert_eq!(info.get_property_val_str("v"), Some("1"));
        assert!(info.is_addr_auto());
    }
}
