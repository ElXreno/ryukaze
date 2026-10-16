use serde::{Deserialize, Serialize};
use zvariant::{Type, as_value::optional as opt};

/// What ryukazed reports in one call, as an `a{sv}` dictionary. A field is
/// missing when the driver or this EC firmware doesn't expose it; clients
/// ignore keys they don't know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[zvariant(signature = "dict")]
#[serde(default)]
pub struct Status {
    /// EC firmware version, e.g. `17KKIMS1.115`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub firmware: Option<String>,

    /// EC firmware build date as the driver prints it.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub firmware_date: Option<String>,

    /// CPU/GPU power preset: `eco`, `comfort`, `sport` or `turbo`, or
    /// `unspecified`/`unknown (N)` for values the driver can't name.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub shift_mode: Option<String>,

    /// The shift modes this firmware offers.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub shift_modes: Option<Vec<String>>,

    /// `auto`, `silent`, `basic` or `advanced`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub fan_mode: Option<String>,

    /// The fan modes this firmware offers.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub fan_modes: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub super_battery: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cooler_boost: Option<bool>,

    /// What the EC reads off the CPU, in °C.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cpu_temp: Option<u8>,

    /// The GPU's temperature in °C as the EC sees it; 0 while it sleeps.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub gpu_temp: Option<u8>,

    /// Fan duty in percent.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cpu_fan: Option<u8>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub gpu_fan: Option<u8>,

    /// Fan speed off the tachometer; only on EC firmware with a known
    /// layout, and approximate.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cpu_rpm: Option<u16>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub gpu_rpm: Option<u16>,

    /// The curve the fan follows in the advanced fan mode, as (temps,
    /// speeds); see `fans::Curve`. Only where ryukaze can set curves.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub cpu_curve: Option<(Vec<u8>, Vec<u8>)>,

    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub gpu_curve: Option<(Vec<u8>, Vec<u8>)>,

    /// In advanced, ryukazed sets the speeds from the smoothed temperatures
    /// along the curves, rather than the EC stepping through them.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub adaptive_fans: Option<bool>,

    /// While adaptive, the GPU fan runs at least at the CPU curve's speed.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub gpu_fan_follows_cpu: Option<bool>,

    /// ryukazed turns cooler boost on by itself when it gets hot.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub auto_cooler_boost: Option<bool>,

    /// The integrated camera is on.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub webcam: Option<bool>,

    /// Where Fn sits next to Win: `left` or `right`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub fn_key: Option<String>,

    /// The USB ports stay powered while the laptop is off.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub usb_share: Option<bool>,

    /// A Mains supply is online.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub on_ac: Option<bool>,

    /// Percent.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub battery_level: Option<u8>,

    /// As the kernel says: `Charging`, `Discharging`, `Not charging`,
    /// `Full` or `Unknown`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub battery_state: Option<String>,

    /// Watts going in or out.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub battery_watts: Option<f64>,

    /// Full charge now against the design's, percent.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub battery_health: Option<u8>,

    /// Battery charge end threshold in percent.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt")]
    pub charge_limit: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::{LE, serialized::Context, to_bytes};

    #[test]
    fn missing_fields_round_trip() {
        let status = Status {
            shift_mode: Some("turbo".into()),
            charge_limit: Some(80),
            ..Status::default()
        };
        let bytes = to_bytes(Context::new_dbus(LE, 0), &status).unwrap();
        let (decoded, _): (Status, _) = bytes.deserialize().unwrap();
        assert_eq!(decoded, status);
    }
}
